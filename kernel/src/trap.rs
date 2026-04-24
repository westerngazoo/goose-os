/// Trap handling — interrupts and exceptions for S-mode.
///
/// RISC-V traps come in two flavors:
///   - Interrupts (scause bit 63 = 1): timer, external (PLIC), software
///   - Exceptions (scause bit 63 = 0): illegal insn, page fault, ecall, etc.
///
/// All traps enter through _trap_vector (trap.S), which saves registers
/// and calls trap_handler() here. We read scause to figure out what
/// happened and dispatch accordingly.

use core::arch::{asm, global_asm};
use crate::{plic, platform, println, kdebug, kdump_csrs};
use crate::abi::SYS_MAX;
use crate::{handlers, ipc, lifecycle, process, sched, syscall};

// Include the trap vector assembly
global_asm!(include_str!("trap.S"));

// Platform-aware constants
const UART0_IRQ: u32 = platform::UART0_IRQ;

/// Trap frame layout — must match trap.S exactly.
/// 31 GPRs + sstatus + sepc = 33 fields.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct TrapFrame {
    pub ra: usize,      // x1   offset 0x00
    pub sp: usize,      // x2   offset 0x08
    pub gp: usize,      // x3   offset 0x10
    pub tp: usize,      // x4   offset 0x18
    pub t0: usize,      // x5   offset 0x20
    pub t1: usize,      // x6   offset 0x28
    pub t2: usize,      // x7   offset 0x30
    pub s0: usize,      // x8   offset 0x38
    pub s1: usize,      // x9   offset 0x40
    pub a0: usize,      // x10  offset 0x48
    pub a1: usize,      // x11  offset 0x50
    pub a2: usize,      // x12  offset 0x58
    pub a3: usize,      // x13  offset 0x60
    pub a4: usize,      // x14  offset 0x68
    pub a5: usize,      // x15  offset 0x70
    pub a6: usize,      // x16  offset 0x78
    pub a7: usize,      // x17  offset 0x80
    pub s2: usize,      // x18  offset 0x88
    pub s3: usize,      // x19  offset 0x90
    pub s4: usize,      // x20  offset 0x98
    pub s5: usize,      // x21  offset 0xA0
    pub s6: usize,      // x22  offset 0xA8
    pub s7: usize,      // x23  offset 0xB0
    pub s8: usize,      // x24  offset 0xB8
    pub s9: usize,      // x25  offset 0xC0
    pub s10: usize,     // x26  offset 0xC8
    pub s11: usize,     // x27  offset 0xD0
    pub t3: usize,      // x28  offset 0xD8
    pub t4: usize,      // x29  offset 0xE0
    pub t5: usize,      // x30  offset 0xE8
    pub t6: usize,      // x31  offset 0xF0
    pub sstatus: usize,  //      offset 0xF8
    pub sepc: usize,     //      offset 0x100
}

impl TrapFrame {
    /// Create a zeroed trap frame (all registers = 0).
    pub const fn zero() -> Self {
        TrapFrame {
            ra: 0, sp: 0, gp: 0, tp: 0,
            t0: 0, t1: 0, t2: 0,
            s0: 0, s1: 0,
            a0: 0, a1: 0, a2: 0, a3: 0, a4: 0, a5: 0, a6: 0, a7: 0,
            s2: 0, s3: 0, s4: 0, s5: 0, s6: 0, s7: 0, s8: 0, s9: 0, s10: 0, s11: 0,
            t3: 0, t4: 0, t5: 0, t6: 0,
            sstatus: 0, sepc: 0,
        }
    }
}

/// Tick counter — incremented on each timer interrupt.
static mut TICKS: u64 = 0;

/// Monotonic tick counter. Increments every 10ms (the timer preemption tick).
/// Exposed so subsystems (notably smoltcp) can compute monotonic timestamps.
pub fn ticks() -> u64 {
    unsafe { TICKS }
}

/// Initialize the trap vector.
/// Writes our _trap_vector address into stvec (direct mode).
/// Does NOT enable interrupts yet — call interrupts_enable() after
/// PLIC and UART are configured.
pub fn trap_init() {
    extern "C" {
        fn _trap_vector();
    }
    let trap_addr = _trap_vector as *const () as usize;
    unsafe {
        // stvec[1:0] = 00 means Direct mode (all traps go to one address)
        asm!("csrw stvec, {}", in(reg) trap_addr);
    }
    println!("  [trap] stvec set to {:#010x}", trap_addr);
}

/// Enable S-mode interrupts globally.
/// Call this AFTER plic_init() and uart.enable_rx_interrupt().
pub fn interrupts_enable() {
    unsafe {
        // sie: enable supervisor external interrupts (bit 9) + timer (bit 5)
        let sie_bits: usize = (1 << 9) | (1 << 5);
        asm!("csrs sie, {}", in(reg) sie_bits);

        // sstatus: set SIE bit (bit 1) to globally enable interrupts
        asm!("csrs sstatus, {}", in(reg) 1usize << 1);
    }
    println!("  [trap] interrupts enabled (SEIE + STIE)");
}

/// Arm the first timer interrupt.
pub fn timer_init() {
    let time = read_time();
    sbi_set_timer(time + platform::TIMESLICE);
    println!("  [trap] timer armed (10ms timeslice, timebase=10MHz)");
}

/// Rust trap dispatcher — called from trap.S with pointer to TrapFrame.
#[no_mangle]
pub extern "C" fn trap_handler(frame: &mut TrapFrame) {
    let scause: usize;
    let stval: usize;
    unsafe {
        asm!("csrr {}, scause", out(reg) scause);
        asm!("csrr {}, stval", out(reg) stval);
    }

    let is_interrupt = scause >> 63 == 1;
    let code = scause & 0x7FFF_FFFF_FFFF_FFFF;

    if is_interrupt {
        match code {
            5 => handle_timer(frame),
            9 => handle_external(frame),
            _ => {
                println!("\n[trap] unhandled interrupt: code={}", code);
                kdump_csrs!();
            }
        }
    } else {
        // Exception
        match code {
            8 => {
                // ecall from U-mode — handle syscall
                kdebug!("ecall: a7={} a0={:#x} a1={:#x}", frame.a7, frame.a0, frame.a1);
                handle_ecall(frame);
            }
            _ => {
                let cause_name = match code {
                    0 => "instruction address misaligned",
                    1 => "instruction access fault",
                    2 => "illegal instruction",
                    3 => "breakpoint",
                    4 => "load address misaligned",
                    5 => "load access fault",
                    6 => "store address misaligned",
                    7 => "store/AMO access fault",
                    9 => "environment call from S-mode",
                    12 => "instruction page fault",
                    13 => "load page fault",
                    15 => "store/AMO page fault",
                    _ => "unknown",
                };

                // Check SPP bit: did this fault come from U-mode or S-mode?
                let from_user = frame.sstatus & (1 << 8) == 0;

                if from_user {
                    // ── U-mode fault: kill the process, don't crash the kernel ──
                    //
                    // This is the correct behavior for a microkernel. A user
                    // process accessing unmapped memory, executing an illegal
                    // instruction, or hitting any other fault should be killed —
                    // not bring down the entire system.
                    let pid = crate::process::current_pid();
                    println!();
                    println!("  [kernel] Process fault:");
                    println!("    PID:    {}", pid);
                    println!("    cause:  {} (code={})", cause_name, code);
                    println!("    stval:  {:#018x}", stval);
                    println!("    sepc:   {:#018x}", frame.sepc);
                    crate::lifecycle::kill_current(frame, 128 + code);
                } else {
                    // ── S-mode fault: kernel bug, panic ──
                    //
                    // If the kernel itself faults, something is deeply wrong.
                    // Print diagnostics and halt.
                    println!("\n!!! KERNEL EXCEPTION !!!");
                    println!("  cause:  {} (code={})", cause_name, code);
                    println!("  stval:  {:#018x}", stval);
                    println!("  sepc:   {:#018x}", frame.sepc);
                    println!("  ra:     {:#018x}", frame.ra);
                    panic!("unrecoverable kernel exception");
                }
            }
        }
    }
}

/// Handle ecall from U-mode — syscall dispatch.
///
/// Convention:
///   a7 = syscall number
///   a0 = first argument (and return value)
///   sepc is advanced by 4 so sret goes to the instruction after ecall.
fn handle_ecall(frame: &mut TrapFrame) {
    // Dispatch table: one function pointer per syscall number.
    //
    // Ordered by SYS_* value from abi.rs. The index MUST be kept in
    // sync with abi.rs — adding a syscall means appending one entry
    // here and bumping SYS_MAX in abi.rs. A debug_assert at table
    // construction time would be ideal; until const_fn grows the
    // right primitives, the invariant is enforced by code review +
    // the SYS_MAX bounds check on dispatch.
    //
    // All handlers have the same signature: fn(&mut TrapFrame).
    // Handlers are responsible for advancing sepc themselves (most
    // do `frame.sepc += 4` at the top; exit/send/receive/call/reply
    // handle sepc as part of their blocking/scheduling logic).
    type Handler = fn(&mut TrapFrame);
    static HANDLERS: [Handler; SYS_MAX + 1] = [
        handlers::sys_putchar,      // 0  SYS_PUTCHAR
        lifecycle::sys_exit,        // 1  SYS_EXIT
        ipc::sys_send,              // 2  SYS_SEND
        ipc::sys_receive,           // 3  SYS_RECEIVE
        handlers::sys_call,         // 4  SYS_CALL (with net intercept)
        ipc::sys_reply,             // 5  SYS_REPLY
        syscall::sys_map,           // 6  SYS_MAP
        syscall::sys_unmap,         // 7  SYS_UNMAP
        syscall::sys_alloc_pages,   // 8  SYS_ALLOC_PAGES
        syscall::sys_free_pages,    // 9  SYS_FREE_PAGES
        syscall::sys_spawn,         // 10 SYS_SPAWN
        lifecycle::sys_wait,        // 11 SYS_WAIT
        process::sys_getpid,        // 12 SYS_GETPID
        sched::sys_yield,           // 13 SYS_YIELD
        syscall::sys_irq_register,  // 14 SYS_IRQ_REGISTER
        syscall::sys_irq_ack,       // 15 SYS_IRQ_ACK
        handlers::sys_reboot,       // 16 SYS_REBOOT
    ];

    let syscall_num = frame.a7;
    if syscall_num <= SYS_MAX {
        HANDLERS[syscall_num](frame);
    } else {
        handlers::sys_unknown(frame);
    }
}

/// Kernel idle loop — entered when no user process is Ready but some are alive.
///
/// Executes WFI (Wait For Interrupt) in a loop. When an interrupt fires:
///   - External IRQ → may wake a blocked process via irq_notify
///   - Timer → handle_timer calls schedule_from_idle, switching to any Ready process
///
/// The key insight: schedule_from_idle overwrites the trap frame with a user
/// process's context (SPP=0), so sret goes to U-mode. The WFI loop never
/// resumes — it gets "preempted" by the interrupt handler.
#[no_mangle]
pub extern "C" fn kernel_idle() -> ! {
    loop {
        unsafe { asm!("wfi"); }
    }
}

/// Kernel re-entry point after ALL user processes have exited.
///
/// We land here via sret after SYS_EXIT rewrites the trap frame.
/// Runs in S-mode with kernel satp. Enters the interactive idle loop.
#[no_mangle]
pub extern "C" fn post_process_exit() -> ! {
    // Disable UART RX interrupts — the idle loop polls directly.
    // Without this, handle_interrupt() drains the RX FIFO before
    // our getc() polling loop ever sees the keystroke.
    let uart = crate::uart::Uart::platform();
    uart.disable_rx_interrupt();

    println!("  [kernel] Back in S-mode. Idle loop active.");
    println!();
    if cfg!(feature = "qemu") {
        println!("  (Ctrl-A X to exit QEMU)");
    } else {
        println!("  (Ctrl-R or R to reboot)");
    }
    println!();
    loop {
        if let Some(c) = uart.getc() {
            match c {
                0x12 | b'R' => {
                    println!("\n  Rebooting...");
                    crate::kernel::sbi_system_reset();
                }
                b'\r' | b'\n' => { uart.putc(b'\r'); uart.putc(b'\n'); }
                0x7F | 0x08 => { uart.putc(0x08); uart.putc(b' '); uart.putc(0x08); }
                _ => {
                    if c < 0x20 || c > 0x7E {
                        // Non-printable — show hex so we can debug what arrives
                        println!("[0x{:02x}]", c);
                    } else {
                        uart.putc(c);
                    }
                }
            }
        }
    }
}

/// Handle supervisor external interrupt (from PLIC).
///
/// Phase 13: If a userspace process owns the IRQ, deliver it via IPC
/// instead of calling the kernel handler. The server must SYS_IRQ_ACK
/// to complete the PLIC cycle.
fn handle_external(frame: &mut TrapFrame) {
    let irq = plic::claim();
    if irq == 0 {
        kdebug!("external: spurious (claim=0)");
        return;
    }

    // Check if a userspace process owns this IRQ
    let owner = crate::syscall::irq_owner(irq);
    kdebug!("external: IRQ {} owner={}", irq, owner);
    if owner != 0 {
        // Deliver as IPC notification — don't complete PLIC yet
        // (server must SYS_IRQ_ACK to re-enable this IRQ)
        crate::ipc::irq_notify(irq, owner);

        // If we're in kernel idle (S-mode), schedule the woken process immediately
        if frame.sstatus & (1 << 8) != 0 {
            kdebug!("external: scheduling from idle");
            crate::sched::schedule_from_idle(frame);
        }
        return;
    }

    // Kernel fallback — no userspace owner
    kdebug!("external: no owner, kernel fallback for IRQ {}", irq);
    match irq {
        UART0_IRQ => {
            crate::uart::handle_interrupt();
        }
        _ => {
            // Check if this is a VirtIO-net IRQ (kernel-handled)
            #[cfg(feature = "net")]
            if crate::virtio::is_ready() && irq == crate::virtio::irq_number() {
                let dev = unsafe { crate::virtio::get() };
                crate::driver::Device::handle_interrupt(dev);
                // Poll smoltcp after receiving packets
                let ticks = unsafe { TICKS };
                let timestamp_ms = (ticks * 10) as i64;
                crate::net::poll(timestamp_ms);
                // Wake any processes blocked on sockets that now have data.
                crate::net::wake_blocked();
                plic::complete(irq);
                return;
            }
            println!("[plic] unhandled IRQ: {}", irq);
        }
    }

    plic::complete(irq);
}

/// Handle supervisor timer interrupt.
///
/// Two roles:
///   1. Wallclock display (every TIMER_INTERVAL ticks)
///   2. Preemptive scheduling (every TIMESLICE) — forcibly switch processes
///      when the timer fires during user-mode execution.
fn handle_timer(frame: &mut TrapFrame) {
    unsafe { TICKS += 1; }

    let ticks = unsafe { TICKS };
    // Wallclock: print every 1000 ticks (1000 × 10ms = 10 seconds)
    if ticks % 1000 == 0 {
        println!("[timer] {} seconds", ticks / 100);
    }

    // Re-arm for next timeslice
    let time = read_time();
    sbi_set_timer(time + platform::TIMESLICE);

    // Poll network stack periodically (every 10 ticks = 100ms)
    #[cfg(feature = "net")]
    if ticks % 10 == 0 && crate::virtio::is_ready() {
        let timestamp_ms = (ticks * 10) as i64;
        crate::net::poll(timestamp_ms);
        // Wake any process blocked on NET_RECV / NET_CONNECT whose condition
        // was satisfied by this poll (new RX packet, TCP handshake complete).
        crate::net::wake_blocked();
    }

    // Preempt or schedule based on where we came from.
    if frame.sstatus & (1 << 8) == 0 {
        // From U-mode — preempt current user process (round-robin)
        crate::sched::preempt(frame);
    } else {
        // From S-mode (kernel idle) — check if any process woke up
        crate::sched::schedule_from_idle(frame);
    }
}

/// Read the RISC-V time CSR.
fn read_time() -> u64 {
    let time: u64;
    unsafe {
        asm!("csrr {}, time", out(reg) time);
    }
    time
}

/// Call SBI set_timer (Timer extension: EID=0x54494D45, FID=0).
fn sbi_set_timer(stime: u64) {
    unsafe {
        asm!(
            "ecall",
            in("a0") stime,
            in("a6") 0usize,          // FID = 0
            in("a7") 0x54494D45usize, // EID = TIME
            lateout("a0") _,
            lateout("a1") _,
        );
    }
}
