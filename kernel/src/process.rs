/// Process management — process table, context switching, scheduling, lifecycle.
///
/// Core module: PCB, process table globals, boot/launch, scheduling.
/// IPC syscalls live in ipc.rs, memory/IRQ/spawn syscalls in syscall.rs.

use core::arch::{asm, global_asm};
use crate::page_alloc::PAGE_SIZE;
use crate::page_table::*;
use crate::trap::TrapFrame;
use crate::kvm;
use crate::{println, kdebug, kdump_csrs, kdump_plic, kdump_procs};

// ── Constants ──────────────────────────────────────────────────

use crate::security::{self, MAX_PROCS, MAX_IRQS};

// ── Process State Machine ──────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Free,           // Slot is unused
    Ready,          // Can be scheduled
    Running,        // Currently executing
    BlockedSend,    // Waiting for receiver to call RECEIVE
    BlockedRecv,    // Waiting for sender to call SEND
    BlockedCall,    // Sent via SYS_CALL, waiting for SYS_REPLY
    BlockedWait,    // Waiting for child process to exit (SYS_WAIT)
}

// ── Process Control Block ──────────────────────────────────────

#[derive(Clone, Copy)]
pub struct Process {
    pub pid: usize,
    pub state: ProcessState,
    pub satp: u64,
    pub context: TrapFrame,     // Saved registers (for context switch)
    // IPC state
    pub ipc_target: usize,      // Who we're sending to / expecting from (0 = any)
    pub ipc_value: usize,       // Message being sent
    pub ipc_arg2: usize,        // Additional IPC argument (a2) — Phase B
    pub ipc_arg3: usize,        // Additional IPC argument (a3) — Phase B
    // Lifecycle
    pub parent: usize,          // Parent PID (0 = kernel-spawned)
    pub exit_code: usize,       // Stored when process exits
    // IRQ ownership (Phase 13)
    pub irq_num: u32,           // Registered IRQ number (0 = none)
    pub irq_pending: bool,      // IRQ fired while not in BlockedRecv
}

impl Process {
    const fn empty() -> Self {
        Process {
            pid: 0,
            state: ProcessState::Free,
            satp: 0,
            context: TrapFrame::zero(),
            ipc_target: 0,
            ipc_value: 0,
            ipc_arg2: 0,
            ipc_arg3: 0,
            parent: 0,
            exit_code: 0,
            irq_num: 0,
            irq_pending: false,
        }
    }
}

// ── Process Table (kernel global state) ────────────────────────

/// Process table — fixed size, no heap allocation.
/// PID 0 is reserved (kernel). Processes use PIDs 1..MAX_PROCS-1.
pub(crate) static mut PROCS: [Process; MAX_PROCS] = [Process::empty(); MAX_PROCS];

/// PID of the currently running process.
pub(crate) static mut CURRENT_PID: usize = 0;

/// IRQ ownership table — maps IRQ number to owning PID (0 = kernel/unclaimed).
pub(crate) static mut IRQ_OWNER: [usize; MAX_IRQS] = [0; MAX_IRQS];

/// Get the currently running PID (for trap handler diagnostics).
pub fn current_pid() -> usize {
    unsafe { CURRENT_PID }
}

/// Get the satp value of the currently running process.
/// Returns 0 if CURRENT_PID is 0 (kernel context).
pub fn current_satp() -> u64 {
    unsafe {
        let pid = CURRENT_PID;
        if pid == 0 { 0 } else { PROCS[pid].satp }
    }
}

/// Dump all non-Free process slots. Called by kdump_procs! macro.
///
/// Always compiled (body is empty without `debug-kernel` feature).
/// The macro already gates the call site — putting #[cfg] on the function
/// itself triggered a rustc nightly ICE in lint_mod/check_mod_deathness.
#[allow(dead_code)]
pub fn dump_procs() {
    #[cfg(feature = "debug-kernel")]
    unsafe {
        println!("[procs] CURRENT_PID={}", CURRENT_PID);
        for i in 1..MAX_PROCS {
            if PROCS[i].state != ProcessState::Free {
                println!("[procs] PID {} state={:?} irq={} irq_pending={} ipc_target={} sepc={:#x}",
                    i, PROCS[i].state, PROCS[i].irq_num, PROCS[i].irq_pending,
                    PROCS[i].ipc_target, PROCS[i].context.sepc);
            }
        }
    }
}

/// Kill the currently running process due to an unrecoverable fault.
///
/// Called by the trap handler when a U-mode exception occurs (page fault,
/// illegal instruction, etc.). This is the microkernel equivalent of
/// Linux's SIGSEGV + process termination.
///
/// `exit_code` follows Unix convention: 128 + signal number (here, scause code).
pub fn kill_current(frame: &mut TrapFrame, exit_code: usize) {
    let current = unsafe { CURRENT_PID };
    if current == 0 {
        panic!("kill_current called with no running process");
    }

    println!("  [kernel] PID {} killed (exit code {})", current, exit_code);

    unsafe {
        PROCS[current].state = ProcessState::Free;
        PROCS[current].exit_code = exit_code;

        // Clean up IRQ ownership
        if PROCS[current].irq_num != 0 {
            let irq = PROCS[current].irq_num;
            if security::is_valid_irq(irq as usize) {
                IRQ_OWNER[irq as usize] = 0;
            }
            PROCS[current].irq_num = 0;
        }

        // Wake any parent that's BlockedWait on us
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::BlockedWait
                && PROCS[i].ipc_target == current
            {
                PROCS[i].context.a0 = exit_code;
                PROCS[i].state = ProcessState::Ready;
                break;
            }
        }

        // Find next ready process
        let mut next = 0;
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Ready {
                next = i;
                break;
            }
        }

        if next == 0 {
            // Check if any processes are still alive
            let mut any_alive = false;
            for i in 1..MAX_PROCS {
                if PROCS[i].state != ProcessState::Free {
                    any_alive = true;
                    break;
                }
            }

            let kernel_satp = crate::kvm::kernel_satp();
            asm!(
                "csrw satp, {0}",
                "sfence.vma zero, zero",
                in(reg) kernel_satp,
            );

            if any_alive {
                CURRENT_PID = 0;
                println!("  [kernel] Idle (waiting for events)...");
                frame.sstatus = (1 << 8) | (1 << 5);
                frame.sepc = crate::trap::kernel_idle as *const () as usize;
                return;
            }

            CURRENT_PID = 0;
            println!("  [kernel] All processes finished.");
            frame.sstatus = (1 << 8) | (1 << 5);
            frame.sepc = crate::trap::post_process_exit as *const () as usize;
            return;
        }

        // Switch to next process
        *frame = PROCS[next].context;
        PROCS[next].state = ProcessState::Running;
        CURRENT_PID = next;

        let next_satp = PROCS[next].satp;
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) next_satp,
        );
    }
}

// ── Embedded User Programs ─────────────────────────────────────

// Program 1: init (PID 1) — Phase 13: UART server client
// Sends "Hello from userspace UART!\r\n" to PID 2 (UART server)
// via SYS_CALL, one character at a time. Then exits.
global_asm!(r#"
.section .text
.balign 4
.global _user_init_start
.global _user_init_end

_user_init_start:
    # ─── GooseOS init process (PID 1) ───
    # Phase 13: Userspace device servers demo.
    #
    # Sends a greeting via IPC to the UART server (PID 2).
    # Each character is sent as SYS_CALL(2, char) — the server
    # writes it to the UART and replies.
    #
    # Uses auipc + %pcrel relocation to compute the string address.
    # This handles compressed instructions correctly (unlike hardcoded offsets).
    # Works after code is copied to a different physical page because
    # the auipc→string distance is preserved in the copied bytes.

1:  auipc   s0, %pcrel_hi(.hello_str)
    addi    s0, s0, %pcrel_lo(1b)

.init_send_loop:
    lbu     t0, 0(s0)           # load next char
    beqz    t0, .init_done      # null terminator → done

    li      a7, 4               # SYS_CALL
    li      a0, 2               # target = UART server (PID 2)
    mv      a1, t0              # message = char
    ecall                       # blocks until server replies

    addi    s0, s0, 1           # next char
    j       .init_send_loop

.init_done:
    # Exit
    li      a7, 1               # SYS_EXIT
    li      a0, 0               # code 0
    ecall

1:  j       1b

.balign 4
.hello_str:
    .asciz "Hello from userspace UART!\r\n"

_user_init_end:
"#);

// Program 2: UART server (PID 2) — Phase 13: userspace device driver
//
// Provides character I/O via IPC:
//   TX: clients send chars via SYS_CALL(2, char), server writes to UART THR
//   RX: kernel delivers UART IRQ via IPC, server reads FIFO and echoes
//
// UART MMIO is identity-mapped into the server's address space by the kernel.
// Platform-specific register offsets are set via .equ constants.

// Platform-specific UART constants for assembly
#[cfg(feature = "qemu")]
global_asm!(r#"
.equ UART_IRQ_NUM, 10
.equ UART_LSR_OFF, 5
.equ UART_IER_OFF, 1
"#);

#[cfg(feature = "vf2")]
global_asm!(r#"
.equ UART_IRQ_NUM, 32
.equ UART_LSR_OFF, 20
.equ UART_IER_OFF, 4
"#);

// UART server code
global_asm!(r#"
.section .text
.balign 4
.global _uart_server_start
.global _uart_server_end

_uart_server_start:
    # ─── GooseOS UART Server (PID 2) ───
    #
    # Registers:
    #   s0 = UART base address (0x10000000, identity-mapped)
    #   s1 = saved sender PID (for SYS_REPLY)
    #   s2 = saved char (for TX / echo)

    li      s0, 0x5E000000      # UART base (user VA, maps to PA 0x10000000)

    # Register for UART IRQ
    li      a7, 14              # SYS_IRQ_REGISTER
    li      a0, UART_IRQ_NUM
    ecall

    # Enable RX data-available interrupt on UART chip
    li      t0, 0x01            # IER: ERBFI bit
    sb      t0, UART_IER_OFF(s0)

.uart_server_loop:
    # Wait for message from anyone
    li      a7, 3               # SYS_RECEIVE
    li      a0, 0               # from = any
    ecall
    # Returns: a0 = message, a1 = sender PID

    # sender == 0 → kernel IRQ notification
    beqz    a1, .uart_handle_irq

    # ─── TX path: write char for client ───
    mv      s1, a1              # save sender PID
    mv      s2, a0              # save char

.uart_tx_wait:
    lbu     t0, UART_LSR_OFF(s0)
    andi    t0, t0, 0x20        # THR empty? (bit 5)
    beqz    t0, .uart_tx_wait
    sb      s2, 0(s0)           # write char to THR

    # Reply to sender (unblocks their SYS_CALL)
    li      a7, 5               # SYS_REPLY
    mv      a0, s1              # target = original sender
    li      a1, 0               # reply = success
    ecall

    j       .uart_server_loop

.uart_handle_irq:
    # ─── RX path: drain FIFO and echo ───
.uart_rx_loop:
    lbu     t0, UART_LSR_OFF(s0)
    andi    t0, t0, 0x01        # data ready? (bit 0)
    beqz    t0, .uart_rx_done

    lbu     s2, 0(s0)           # read char from RBR

    # Ctrl-R (0x12) → print message then reboot
    li      t1, 0x12
    bne     s2, t1, .uart_not_reboot

    # Server-side UX: print what we received before kernel resets
    # Pattern: pre-syscall notification — server owns the UX, kernel owns the action
.uart_reboot_msg:
    la      t2, .uart_reboot_str
.uart_reboot_putc:
    lbu     t3, 0(t2)
    beqz    t3, .uart_reboot_go
.uart_reboot_thr_wait:
    lbu     t0, UART_LSR_OFF(s0)
    andi    t0, t0, 0x20        # THR empty?
    beqz    t0, .uart_reboot_thr_wait
    sb      t3, 0(s0)
    addi    t2, t2, 1
    j       .uart_reboot_putc
.uart_reboot_go:
    li      a7, 16              # SYS_REBOOT
    ecall

.uart_not_reboot:

    # Echo the char back
.uart_echo_wait:
    lbu     t0, UART_LSR_OFF(s0)
    andi    t0, t0, 0x20        # THR empty?
    beqz    t0, .uart_echo_wait
    sb      s2, 0(s0)           # echo char

    # CR → CRLF
    li      t1, 13              # '\r'
    bne     s2, t1, .uart_rx_loop

.uart_lf_wait:
    lbu     t0, UART_LSR_OFF(s0)
    andi    t0, t0, 0x20
    beqz    t0, .uart_lf_wait
    li      t1, 10              # '\n'
    sb      t1, 0(s0)

    j       .uart_rx_loop

.uart_rx_done:
    # Acknowledge IRQ (completes PLIC cycle)
    li      a7, 15              # SYS_IRQ_ACK
    ecall

    j       .uart_server_loop

    # ── String data (inlined in .text to stay within the server's mapped pages) ──
    .balign 4
.uart_reboot_str:
    .asciz "\r\n  [Ctrl-R] Rebooting...\r\n"

_uart_server_end:
"#);

// ── Security Test Process (conditional) ───────────────────────
//
// Embedded assembly that tests kernel security boundaries from U-mode.
// Only compiled when the `security-test` feature is enabled.
//
// Tests:
//   T1: Unknown syscall → returns error
//   T2: SYS_MAP with kernel VA → returns error
//   T3: SYS_UNMAP with kernel VA → returns error
//   T4: SYS_SEND to PID 0 (kernel) → returns error
//   T5: SYS_SEND to self → returns error
//   T6: SYS_MAP with unaligned address → returns error
//   T7: SYS_FREE_PAGES on kernel address → returns error
//   T8: SYS_SEND to out-of-bounds PID → returns error
//   T9: Read kernel memory → page fault → process killed
//
// Prints "P<n>" for pass, "F<n>" for fail. Final test triggers a kill.

#[cfg(feature = "security-test")]
global_asm!(r#"
.section .text
.balign 4
.global _security_test_start
.global _security_test_end

_security_test_start:
    # ─── GooseOS Security Test Process ───
    #
    # Each test does a syscall that should be rejected, checks a0 == -1,
    # and prints P<n> (pass) or F<n> (fail) via SYS_PUTCHAR.
    #
    # Final test reads kernel memory — should trigger page fault and kill.

    # ── T1: Unknown syscall number → error ──
    li      a7, 255
    li      a0, 0
    ecall
    li      t0, -1
    bne     a0, t0, .t1_fail
    li      a7, 0
    li      a0, 0x50            # 'P'
    ecall
    j       .t1_done
.t1_fail:
    li      a7, 0
    li      a0, 0x46            # 'F'
    ecall
.t1_done:
    li      a7, 0
    li      a0, 0x31            # '1'
    ecall
    li      a7, 0
    li      a0, 10              # '\n'
    ecall

    # ── T2: SYS_MAP with kernel VA → error ──
    li      a7, 6               # SYS_MAP
    li      a0, 0x80200000      # phys = kernel text
    li      a1, 0x80200000      # virt = kernel VA (out of user range)
    li      a2, 0
    ecall
    li      t0, -1
    bne     a0, t0, .t2_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t2_done
.t2_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t2_done:
    li      a7, 0
    li      a0, 0x32            # '2'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T3: SYS_UNMAP kernel VA → error ──
    li      a7, 7               # SYS_UNMAP
    li      a0, 0x80200000      # kernel text page
    ecall
    li      t0, -1
    bne     a0, t0, .t3_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t3_done
.t3_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t3_done:
    li      a7, 0
    li      a0, 0x33            # '3'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T4: SYS_SEND to PID 0 (kernel) → error ──
    li      a7, 2               # SYS_SEND
    li      a0, 0               # target = kernel
    li      a1, 0xDEAD
    ecall
    li      t0, -1
    bne     a0, t0, .t4_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t4_done
.t4_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t4_done:
    li      a7, 0
    li      a0, 0x34            # '4'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T5: SYS_SEND to self → error ──
    li      a7, 12              # SYS_GETPID
    ecall
    mv      s0, a0              # s0 = my PID

    li      a7, 2               # SYS_SEND
    mv      a0, s0              # target = self
    li      a1, 0xDEAD
    ecall
    li      t0, -1
    bne     a0, t0, .t5_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t5_done
.t5_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t5_done:
    li      a7, 0
    li      a0, 0x35            # '5'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T6: SYS_MAP with unaligned phys → error ──
    li      a7, 6               # SYS_MAP
    li      a0, 0x5E000001      # unaligned phys
    li      a1, 0x5E001000      # valid user VA
    li      a2, 0
    ecall
    li      t0, -1
    bne     a0, t0, .t6_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t6_done
.t6_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t6_done:
    li      a7, 0
    li      a0, 0x36            # '6'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T7: SYS_FREE_PAGES on kernel address → error ──
    li      a7, 9               # SYS_FREE_PAGES
    li      a0, 0x80200000      # kernel text
    li      a1, 1
    ecall
    li      t0, -1
    bne     a0, t0, .t7_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t7_done
.t7_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t7_done:
    li      a7, 0
    li      a0, 0x37            # '7'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T8: SYS_SEND to PID 99 → error (out of bounds) ──
    li      a7, 2               # SYS_SEND
    li      a0, 99              # PID 99 — way beyond MAX_PROCS
    li      a1, 0xDEAD
    ecall
    li      t0, -1
    bne     a0, t0, .t8_fail
    li      a7, 0
    li      a0, 0x50
    ecall
    j       .t8_done
.t8_fail:
    li      a7, 0
    li      a0, 0x46
    ecall
.t8_done:
    li      a7, 0
    li      a0, 0x38            # '8'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # ── T9: Read kernel memory → page fault → should be killed ──
    # Print "K9" before the attempt so we know we got here
    li      a7, 0
    li      a0, 0x4B            # 'K'
    ecall
    li      a7, 0
    li      a0, 0x39            # '9'
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    # This load MUST fault. Kernel .text at 0x80200000 has no U bit.
    # If this succeeds, the kernel's page table isolation is broken.
    li      t0, 0x80200000
    lw      t1, 0(t0)

    # ── IF WE REACH HERE, THE KERNEL IS BROKEN ──
    # Print "!!!" as a distress signal
    li      a7, 0
    li      a0, 0x21            # '!'
    ecall
    li      a7, 0
    li      a0, 0x21
    ecall
    li      a7, 0
    li      a0, 0x21
    ecall
    li      a7, 0
    li      a0, 10
    ecall

    li      a7, 1               # SYS_EXIT
    li      a0, 99              # exit code 99 = security failure
    ecall
1:  j       1b

_security_test_end:
"#);

// ── Boot: Create Processes and Launch ──────────────────────────

/// Create processes and launch.
///
/// Normal mode (default): Two kernel-created processes:
///   PID 1 = init — sends greeting via IPC to UART server, exits
///   PID 2 = UART server — handles TX (via IPC) and RX (via IRQ)
///
/// Security test mode (`--features security-test`): One process:
///   PID 1 = security test — exercises all attack vectors, then
///   deliberately reads kernel memory to verify page fault kills it.
pub fn launch() -> ! {
    // Disable interrupts while creating processes.
    // Without this, a timer interrupt after PID 1 is created (state=Ready)
    // could call schedule_from_idle() and switch to PID 1 BEFORE PID 2 exists.
    // PID 1 would then SYS_CALL(2, char) to a nonexistent PID and block forever.
    // This race was observed on VF2 where serial output is slower than QEMU.
    unsafe { asm!("csrc sstatus, {}", in(reg) 1usize << 1); }

    #[cfg(feature = "security-test")]
    {
        extern "C" {
            static _security_test_start: u8;
            static _security_test_end: u8;
        }
        let test_start = unsafe { &_security_test_start as *const u8 as usize };
        let test_size = unsafe { &_security_test_end as *const u8 as usize } - test_start;

        println!("  [security] === SECURITY TEST MODE ===");
        println!("  [security] Creating security test (PID 1)...");
        create_process(1, test_start, test_size);
    }

    #[cfg(feature = "rust-user")]
    {
        // Compiled Rust userspace binary (built by: make build-user)
        static HELLO_ELF: &[u8] = include_bytes!(
            "../../userspace/hello/target/riscv64gc-unknown-none-elf/release/hello"
        );

        println!("  [proc] === RUST USERSPACE MODE ===");
        println!("  [proc] Loading Rust hello ({} bytes)...", HELLO_ELF.len());

        let info = match crate::elf::parse(HELLO_ELF) {
            Ok(info) => info,
            Err(e) => panic!("Failed to parse user ELF: {:?}", e),
        };

        create_process_from_elf(1, &info, HELLO_ELF);
    }

    #[cfg(not(any(feature = "security-test", feature = "rust-user")))]
    {
        extern "C" {
            static _user_init_start: u8;
            static _user_init_end: u8;
            static _uart_server_start: u8;
            static _uart_server_end: u8;
        }

        let init_start = unsafe { &_user_init_start as *const u8 as usize };
        let init_size = unsafe { &_user_init_end as *const u8 as usize } - init_start;
        let uart_start = unsafe { &_uart_server_start as *const u8 as usize };
        let uart_size = unsafe { &_uart_server_end as *const u8 as usize } - uart_start;

        println!("  [proc] Creating init (PID 1)...");
        create_process(1, init_start, init_size);

        println!("  [proc] Creating UART server (PID 2)...");
        create_process(2, uart_start, uart_size);

        // Map UART MMIO into PID 2's address space at a USER-accessible VA.
        const UART_USER_VA: usize = 0x5E00_0000;
        let root2 = kvm::satp_to_root(unsafe { PROCS[2].satp });
        kvm::map_user_page(root2, UART_USER_VA, crate::platform::UART_BASE, USER_MMIO);
        println!("  [proc] Mapped UART MMIO PA {:#x} at user VA {:#x} into PID 2",
            crate::platform::UART_BASE, UART_USER_VA);
    }

    let alloc = unsafe { crate::page_alloc::get() };
    println!();
    println!("  [page_alloc] {} pages used, {} free",
        alloc.allocated_count(), alloc.free_count());
    println!();

    // Launch PID 1 as the first running process
    let proc1 = unsafe { &PROCS[1] };
    unsafe {
        CURRENT_PID = 1;
        PROCS[1].state = ProcessState::Running;
    }

    let entry = proc1.context.sepc;
    let user_sp = proc1.context.sp;
    let satp = proc1.satp;

    // NOTE: Do NOT re-enable SIE here. We disabled interrupts (cleared SIE)
    // before creating processes. The sret below sets SPIE, and sret copies
    // SPIE→SIE — so interrupts are automatically re-enabled when we enter
    // U-mode. Re-enabling SIE before sret would allow a timer interrupt to
    // fire while still in S-mode, calling schedule_from_idle() and preempting
    // the launch sequence. This race was observed on VF2 build 48.

    println!("  [proc] Launching PID 1 (init)...");
    println!();

    unsafe {
        asm!(
            "csrw sepc, {entry}",
            "csrr t0, sstatus",
            "li t1, -257",              // clear SPP (bit 8)
            "and t0, t0, t1",
            "ori t0, t0, 32",           // set SPIE (bit 5)
            "csrw sstatus, t0",
            "csrw sscratch, sp",        // save kernel sp
            "csrw satp, {satp}",
            "sfence.vma zero, zero",
            "mv sp, {user_sp}",
            "sret",
            entry = in(reg) entry,
            satp = in(reg) satp,
            user_sp = in(reg) user_sp,
            options(noreturn),
        );
    }
}

/// Kernel-level spawn for boot processes (not via syscall).
/// Used by launch() to create initial processes from embedded assembly.
fn create_process(
    pid: usize,
    code_start: usize,
    code_size: usize,
) {
    assert!(pid > 0 && pid < MAX_PROCS, "invalid PID");

    let user_code = kvm::alloc_zeroed_page();
    unsafe {
        let src = code_start as *const u8;
        let dst = user_code as *mut u8;
        for i in 0..code_size {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }

    let user_stack = kvm::alloc_zeroed_page();
    let user_sp = user_stack + PAGE_SIZE;

    let user_root = kvm::alloc_zeroed_page();
    kvm::map_kernel_regions(user_root);
    kvm::map_range(user_root, user_code, user_code + PAGE_SIZE, USER_RX);
    kvm::map_range(user_root, user_stack, user_stack + PAGE_SIZE, USER_RW);

    let satp = make_satp(user_root, pid as u16);

    let mut ctx = TrapFrame::zero();
    ctx.sepc = user_code;
    ctx.sp = user_sp;
    ctx.sstatus = 1 << 5;

    unsafe {
        PROCS[pid] = Process {
            pid,
            state: ProcessState::Ready,
            satp,
            context: ctx,
            ipc_target: 0,
            ipc_value: 0,
            ipc_arg2: 0,
            ipc_arg3: 0,
            parent: 0,
            exit_code: 0,
            irq_num: 0,
            irq_pending: false,
        };
    }

    println!("  [proc] PID {} created (code={:#x}, {} bytes, sp={:#x}, satp={:#018x})",
        pid, user_code, code_size, user_sp, satp);
}

/// Kernel-level spawn from a parsed ELF binary in kernel memory.
///
/// Like sys_spawn but reads ELF data from kernel .rodata (via include_bytes!)
/// instead of user memory. Used at boot to load compiled Rust userspace binaries.
fn create_process_from_elf(
    pid: usize,
    info: &crate::elf::ElfInfo,
    elf_data: &[u8],
) {
    assert!(pid > 0 && pid < MAX_PROCS, "invalid PID");

    let user_root = kvm::alloc_zeroed_page();
    kvm::map_kernel_regions(user_root);

    // Load each PT_LOAD segment
    for seg_idx in 0..info.num_segments {
        let seg = &info.segments[seg_idx];
        let flags = if seg.executable { USER_RX } else { USER_RW };

        let num_pages = crate::elf::pages_needed(seg.memsz, seg.vaddr);
        let base_va = seg.vaddr & !(PAGE_SIZE - 1);

        for p in 0..num_pages {
            let va = base_va + p * PAGE_SIZE;
            let page = kvm::alloc_zeroed_page();

            let page_start = va;
            let page_end = va + PAGE_SIZE;
            let seg_file_start = seg.vaddr;
            let seg_file_end = seg.vaddr + seg.filesz;

            let copy_start = if seg_file_start > page_start { seg_file_start } else { page_start };
            let copy_end = if seg_file_end < page_end { seg_file_end } else { page_end };

            if copy_start < copy_end {
                let file_offset = seg.file_offset + (copy_start - seg.vaddr);
                let dst_offset = copy_start - page_start;
                let len = copy_end - copy_start;

                unsafe {
                    let src = elf_data.as_ptr().add(file_offset);
                    let dst = (page as *mut u8).add(dst_offset);
                    for b in 0..len {
                        core::ptr::write_volatile(dst.add(b), core::ptr::read_volatile(src.add(b)));
                    }
                }
            }

            kvm::map_user_page(user_root, va, page, flags);
        }
    }

    // Allocate user stack
    let user_stack = kvm::alloc_zeroed_page();
    let stack_va = 0x7FFF_0000;
    kvm::map_user_page(user_root, stack_va, user_stack, USER_RW);
    let user_sp = stack_va + PAGE_SIZE;

    let satp = make_satp(user_root, pid as u16);

    let mut ctx = TrapFrame::zero();
    ctx.sepc = info.entry;
    ctx.sp = user_sp;
    ctx.sstatus = 1 << 5; // SPIE=1, SPP=0 (U-mode)

    unsafe {
        PROCS[pid] = Process {
            pid,
            state: ProcessState::Ready,
            satp,
            context: ctx,
            ipc_target: 0,
            ipc_value: 0,
            ipc_arg2: 0,
            ipc_arg3: 0,
            parent: 0,
            exit_code: 0,
            irq_num: 0,
            irq_pending: false,
        };
    }

    println!("  [proc] PID {} created from ELF (entry={:#x}, {} segments, sp={:#x})",
        pid, info.entry, info.num_segments, user_sp);
}

/// SYS_EXIT(code) — terminate the current process.
///
/// Frees the process slot and switches to the next ready process.
/// If no processes remain, returns to kernel idle loop.
pub fn sys_exit(frame: &mut TrapFrame) {
    let current = unsafe { CURRENT_PID };
    let exit_code = frame.a0;

    println!();
    println!("  [kernel] PID {} exited with code {}", current, exit_code);

    unsafe {
        PROCS[current].state = ProcessState::Free;
        PROCS[current].exit_code = exit_code;

        // Phase 13: Clean up IRQ ownership
        if PROCS[current].irq_num != 0 {
            let irq = PROCS[current].irq_num;
            if security::is_valid_irq(irq as usize) {
                IRQ_OWNER[irq as usize] = 0;
            }
            PROCS[current].irq_num = 0;
        }

        // Wake any parent that's BlockedWait on us
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::BlockedWait
                && PROCS[i].ipc_target == current
            {
                // Deliver exit code to parent's saved context
                PROCS[i].context.a0 = exit_code;
                PROCS[i].state = ProcessState::Ready;
                break; // only one parent
            }
        }

        // Find next ready process
        let mut next = 0;
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Ready {
                next = i;
                break;
            }
        }

        if next == 0 {
            // No Ready process — check if any are still alive (blocked)
            let mut any_alive = false;
            for i in 1..MAX_PROCS {
                if PROCS[i].state != ProcessState::Free {
                    any_alive = true;
                    break;
                }
            }

            let kernel_satp = crate::kvm::kernel_satp();
            asm!(
                "csrw satp, {0}",
                "sfence.vma zero, zero",
                in(reg) kernel_satp,
            );

            if any_alive {
                // Processes exist but none Ready — enter kernel idle.
                // Timer interrupts will schedule them when they wake.
                CURRENT_PID = 0;
                println!("  [kernel] Idle (waiting for events)...");
                kdump_csrs!();
                kdump_procs!();
                kdump_plic!();
                frame.sstatus = (1 << 8) | (1 << 5); // SPP=S, SPIE=1
                frame.sepc = crate::trap::kernel_idle as *const () as usize;
                return;
            }

            // Truly done — all processes exited
            println!("  [kernel] All processes finished.");

            // Rewrite frame to return to S-mode at post_process_exit
            frame.sstatus |= 1 << 8;  // SPP = S-mode
            frame.sstatus |= 1 << 5;  // SPIE = 1
            frame.sepc = crate::trap::post_process_exit as *const () as usize;
            return;
        }

        // Switch to next process
        *frame = PROCS[next].context;
        PROCS[next].state = ProcessState::Running;
        CURRENT_PID = next;

        let next_satp = PROCS[next].satp;
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) next_satp,
        );
    }
}

// ── Lifecycle Syscall Handlers ────────────────────────────────

/// SYS_GETPID() — return the current process's PID.
///
/// Convention: no arguments. Returns: a0 = PID.
pub fn sys_getpid(frame: &mut TrapFrame) {
    frame.sepc += 4;
    frame.a0 = unsafe { CURRENT_PID };
}

/// SYS_WAIT(child_pid) — block until a child process exits.
///
/// Convention: a0 = child PID.
/// Returns: a0 = child's exit code, usize::MAX on error.
///
/// If the child has already exited (Free state), returns immediately
/// with the stored exit code. Otherwise blocks until the child calls SYS_EXIT.
pub fn sys_wait(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    let child_pid = frame.a0;

    // Validate
    if !security::is_valid_ipc_target(child_pid, current) {
        frame.a0 = usize::MAX;
        return;
    }

    unsafe {
        // Check if the child exists and belongs to us
        if PROCS[child_pid].parent != current {
            frame.a0 = usize::MAX;
            return;
        }

        // Already exited? Return immediately.
        if PROCS[child_pid].state == ProcessState::Free {
            frame.a0 = PROCS[child_pid].exit_code;
            return;
        }

        // Child still running — block
        PROCS[current].ipc_target = child_pid; // who we're waiting for
        PROCS[current].state = ProcessState::BlockedWait;

        schedule(frame, current);
    }
}

// ── Voluntary Yield ──────────────────────────────────────────

/// SYS_YIELD() — voluntarily give up the time slice.
///
/// Convention: no arguments. Returns: a0 = 0.
/// Marks the caller Ready and switches to the next process.
/// If no other process is Ready, returns immediately.
pub fn sys_yield(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    if current == 0 { return; }

    unsafe {
        // Check if there's anyone else to switch to
        let mut found = false;
        for offset in 1..(MAX_PROCS - 1) {
            let i = ((current - 1 + offset) % (MAX_PROCS - 1)) + 1;
            if PROCS[i].state == ProcessState::Ready {
                found = true;
                break;
            }
        }
        if !found {
            frame.a0 = 0;
            return; // we're the only one — keep running
        }

        PROCS[current].state = ProcessState::Ready;
        schedule(frame, current);
    }

    frame.a0 = 0;
}

// ── Preemptive Scheduling ────────────────────────────────────

/// Timer-driven preemption — called from the timer interrupt handler.
///
/// If a user process is running, forcibly save its state and switch
/// to the next Ready process (round-robin). This is what prevents
/// a busy-loop process from starving everyone else.
pub fn preempt(frame: &mut TrapFrame) {
    unsafe {
        let current = CURRENT_PID;
        if current == 0 { return; } // no user process running

        // Find next Ready process (round-robin from current+1)
        let mut next = 0;
        for offset in 1..(MAX_PROCS - 1) {
            let i = ((current - 1 + offset) % (MAX_PROCS - 1)) + 1;
            if PROCS[i].state == ProcessState::Ready {
                next = i;
                break;
            }
        }

        if next == 0 {
            return; // no one else to run — let current continue
        }

        // Preempt: save current, load next
        PROCS[current].context = *frame;
        PROCS[current].state = ProcessState::Ready;

        *frame = PROCS[next].context;
        PROCS[next].state = ProcessState::Running;
        CURRENT_PID = next;

        let next_satp = PROCS[next].satp;
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) next_satp,
        );
    }
}

/// Schedule from kernel idle — called by the timer handler when in S-mode.
///
/// When the kernel is idle (CURRENT_PID=0, waiting in WFI), an interrupt
/// might wake a blocked process (e.g., IRQ notification → BlockedRecv → Ready).
/// The next timer tick calls this to check if any process became Ready and
/// switch to it.
pub fn schedule_from_idle(frame: &mut TrapFrame) {
    unsafe {
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Ready {
                *frame = PROCS[i].context;
                PROCS[i].state = ProcessState::Running;
                CURRENT_PID = i;

                let satp = PROCS[i].satp;
                asm!(
                    "csrw satp, {0}",
                    "sfence.vma zero, zero",
                    in(reg) satp,
                );
                return;
            }
        }
    }
}

// ── Scheduler ──────────────────────────────────────────────────

/// Save current process and switch to the next ready process.
///
/// Called when a process blocks (SEND/RECEIVE/WAIT with no rendezvous).
/// Uses round-robin: scans from the blocked PID forward, wrapping around.
/// The frame on the kernel stack is overwritten with the next process's
/// saved context. When we return to trap.S, it restores and srets to
/// the next process.
pub(crate) unsafe fn schedule(frame: &mut TrapFrame, blocked_pid: usize) {
    // Save current process's registers
    PROCS[blocked_pid].context = *frame;

    // Find next ready process (round-robin from blocked_pid + 1)
    let mut next = 0;
    for offset in 1..MAX_PROCS {
        let i = ((blocked_pid - 1 + offset) % (MAX_PROCS - 1)) + 1;
        if PROCS[i].state == ProcessState::Ready {
            next = i;
            break;
        }
    }

    if next == 0 {
        // No Ready process — check if any are still alive (waiting for events)
        let mut any_alive = false;
        for i in 1..MAX_PROCS {
            if PROCS[i].state != ProcessState::Free {
                any_alive = true;
                break;
            }
        }

        if !any_alive {
            panic!("Deadlock: no runnable processes (PID {} blocked)", blocked_pid);
        }

        // Enter kernel idle — timer interrupts will schedule when a process wakes
        CURRENT_PID = 0;
        let kernel_satp = crate::kvm::kernel_satp();
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) kernel_satp,
        );

        frame.sstatus = (1 << 8) | (1 << 5); // SPP=S, SPIE=1
        frame.sepc = crate::trap::kernel_idle as *const () as usize;
        return;
    }

    // Load next process's context onto the kernel stack
    *frame = PROCS[next].context;
    PROCS[next].state = ProcessState::Running;
    CURRENT_PID = next;

    // Switch page table
    let next_satp = PROCS[next].satp;
    asm!(
        "csrw satp, {0}",
        "sfence.vma zero, zero",
        in(reg) next_satp,
    );
}
