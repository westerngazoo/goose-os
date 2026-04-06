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
use crate::{plic, platform, println};

/// Syscall numbers — must match userspace programs.
pub const SYS_PUTCHAR: usize = 0;
pub const SYS_EXIT: usize = 1;
pub const SYS_SEND: usize = 2;
pub const SYS_RECEIVE: usize = 3;

// Include the trap vector assembly
global_asm!(include_str!("trap.S"));

// Platform-aware constants
const UART0_IRQ: u32 = platform::UART0_IRQ;
const TIMER_INTERVAL: u64 = platform::TIMER_INTERVAL;

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
    sbi_set_timer(time + TIMER_INTERVAL);
    println!("  [trap] timer armed (1s interval, timebase=10MHz)");
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
            5 => handle_timer(),
            9 => handle_external(),
            _ => {
                println!("\n[trap] unhandled interrupt: code={}", code);
            }
        }
    } else {
        // Exception
        match code {
            8 => {
                // ecall from U-mode — handle syscall
                handle_ecall(frame);
            }
            _ => {
                // Unexpected exception — print diagnostics and panic
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
                println!("\n!!! EXCEPTION !!!");
                println!("  cause:  {} (code={})", cause_name, code);
                println!("  stval:  {:#018x}", stval);
                println!("  sepc:   {:#018x}", frame.sepc);
                println!("  ra:     {:#018x}", frame.ra);
                panic!("unrecoverable exception");
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
    let syscall_num = frame.a7;

    match syscall_num {
        SYS_PUTCHAR => {
            let ch = frame.a0 as u8;
            crate::uart::Uart::platform().putc(ch);
            frame.a0 = 0;
            frame.sepc += 4;
        }
        SYS_EXIT => {
            crate::process::sys_exit(frame);
            // sys_exit handles sepc — don't advance
            return;
        }
        SYS_SEND => {
            crate::process::sys_send(frame);
            // sys_send handles sepc — don't advance
            return;
        }
        SYS_RECEIVE => {
            crate::process::sys_receive(frame);
            // sys_receive handles sepc — don't advance
            return;
        }
        _ => {
            println!("\n  [kernel] Unknown syscall: {} (a0={:#x})", syscall_num, frame.a0);
            frame.a0 = usize::MAX;
            frame.sepc += 4;
        }
    }
}

/// Kernel re-entry point after a user process exits.
///
/// We land here via sret after SYS_EXIT rewrites the trap frame.
/// Runs in S-mode with kernel satp. Enters the idle loop.
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
        println!("  (Type R to reboot)");
    }
    println!();
    loop {
        if let Some(c) = uart.getc() {
            match c {
                0x12 | b'R' => {
                    println!("\n  Rebooting...");
                    // SBI System Reset
                    unsafe {
                        asm!(
                            "ecall",
                            in("a0") 1usize,
                            in("a1") 0usize,
                            in("a6") 0usize,
                            in("a7") 0x53525354usize,
                            options(noreturn)
                        );
                    }
                }
                b'\r' | b'\n' => { uart.putc(b'\r'); uart.putc(b'\n'); }
                0x7F | 0x08 => { uart.putc(0x08); uart.putc(b' '); uart.putc(0x08); }
                _ => {
                    // Debug: show hex value of non-printable characters
                    if c < 0x20 {
                        println!("[key: 0x{:02x}]", c);
                    } else {
                        uart.putc(c);
                    }
                }
            }
        }
    }
}

/// Handle supervisor external interrupt (from PLIC).
fn handle_external() {
    let irq = plic::claim();
    if irq == 0 {
        return; // spurious
    }

    match irq {
        UART0_IRQ => {
            crate::uart::handle_interrupt();
        }
        _ => {
            println!("[plic] unhandled IRQ: {}", irq);
        }
    }

    plic::complete(irq);
}

/// Handle supervisor timer interrupt.
fn handle_timer() {
    unsafe { TICKS += 1; }

    let ticks = unsafe { TICKS };
    if ticks % 10 == 0 {
        println!("[timer] {} seconds", ticks);
    }

    // Re-arm for next tick
    let time = read_time();
    sbi_set_timer(time + TIMER_INTERVAL);
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
