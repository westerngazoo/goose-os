/// Console output via UART — provides print!, println!, and kdebug! macros.

use core::fmt;
use core::fmt::Write;
use crate::uart::Uart;

/// Print to the console UART. Called by the print!/println! macros.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut uart = Uart::platform();
    uart.write_fmt(args).unwrap();
}

/// Print formatted text to the console (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

/// Print formatted text to the console with a trailing newline.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}

// ── Debug macros (zero-cost when debug-kernel feature is off) ──

/// Print a debug message. Compiles to nothing without `debug-kernel` feature.
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {
        #[cfg(feature = "debug-kernel")]
        $crate::println!("[debug] {}", format_args!($($arg)*));
    };
}

/// Dump S-mode CSRs: sstatus, sie, sip, satp, sepc, scause, stval.
#[macro_export]
macro_rules! kdump_csrs {
    () => {
        #[cfg(feature = "debug-kernel")]
        {
            let sstatus: usize;
            let sie: usize;
            let sip: usize;
            let satp: usize;
            let sepc: usize;
            let scause: usize;
            let stval: usize;
            // SAFETY: INV-7 — reading S-mode CSRs from S-mode (kernel).
            unsafe {
                core::arch::asm!("csrr {}, sstatus", out(reg) sstatus);
                core::arch::asm!("csrr {}, sie", out(reg) sie);
                core::arch::asm!("csrr {}, sip", out(reg) sip);
                core::arch::asm!("csrr {}, satp", out(reg) satp);
                core::arch::asm!("csrr {}, sepc", out(reg) sepc);
                core::arch::asm!("csrr {}, scause", out(reg) scause);
                core::arch::asm!("csrr {}, stval", out(reg) stval);
            }
            $crate::println!("[csrs] sstatus={:#018x} sie={:#06x} sip={:#06x}",
                sstatus, sie, sip);
            $crate::println!("[csrs]   SIE={} SPIE={} SPP={}",
                if sstatus & (1 << 1) != 0 { "on" } else { "off" },
                if sstatus & (1 << 5) != 0 { "on" } else { "off" },
                if sstatus & (1 << 8) != 0 { "S" } else { "U" });
            $crate::println!("[csrs]   SEIE={} STIE={} SSIE={}",
                if sie & (1 << 9) != 0 { "on" } else { "off" },
                if sie & (1 << 5) != 0 { "on" } else { "off" },
                if sie & (1 << 1) != 0 { "on" } else { "off" });
            $crate::println!("[csrs]   SEIP={} STIP={} SSIP={}",
                if sip & (1 << 9) != 0 { "pending" } else { "-" },
                if sip & (1 << 5) != 0 { "pending" } else { "-" },
                if sip & (1 << 1) != 0 { "pending" } else { "-" });
            $crate::println!("[csrs] satp={:#018x} sepc={:#018x}", satp, sepc);
            $crate::println!("[csrs] scause={:#018x} stval={:#018x}", scause, stval);
        }
    };
}

/// Dump PLIC state for the UART IRQ.
#[macro_export]
macro_rules! kdump_plic {
    () => {
        #[cfg(feature = "debug-kernel")]
        $crate::plic::dump();
    };
}

/// Dump all non-Free process slots.
#[macro_export]
macro_rules! kdump_procs {
    () => {
        #[cfg(feature = "debug-kernel")]
        $crate::process::dump_procs();
    };
}

/// Hexdump `len` bytes starting at `addr`.
#[macro_export]
macro_rules! kdump_mem {
    ($addr:expr, $len:expr) => {
        #[cfg(feature = "debug-kernel")]
        {
            let base = $addr as usize;
            let len = $len as usize;
            let mut offset = 0usize;
            while offset < len {
                $crate::print!("[mem] {:#010x}: ", base + offset);
                let line_end = if offset + 16 < len { offset + 16 } else { len };
                let mut i = offset;
                while i < line_end {
                    let byte = unsafe { core::ptr::read_volatile((base + i) as *const u8) };
                    $crate::print!("{:02x} ", byte);
                    i += 1;
                }
                while i < offset + 16 {
                    $crate::print!("   ");
                    i += 1;
                }
                $crate::print!(" |");
                i = offset;
                while i < line_end {
                    let byte = unsafe { core::ptr::read_volatile((base + i) as *const u8) };
                    if byte >= 0x20 && byte <= 0x7e {
                        $crate::print!("{}", byte as char);
                    } else {
                        $crate::print!(".");
                    }
                    i += 1;
                }
                $crate::println!("|");
                offset += 16;
            }
        }
    };
}

/// Dump all registers from a TrapFrame.
#[macro_export]
macro_rules! kdump_trapframe {
    ($frame:expr) => {
        #[cfg(feature = "debug-kernel")]
        {
            let f = $frame;
            $crate::println!("[frame] ra={:#018x}  sp={:#018x}  gp={:#018x}", f.ra, f.sp, f.gp);
            $crate::println!("[frame] a0={:#018x}  a1={:#018x}  a2={:#018x}  a3={:#018x}",
                f.a0, f.a1, f.a2, f.a3);
            $crate::println!("[frame] a4={:#018x}  a5={:#018x}  a6={:#018x}  a7={:#018x}",
                f.a4, f.a5, f.a6, f.a7);
            $crate::println!("[frame] t0={:#018x}  t1={:#018x}  t2={:#018x}",
                f.t0, f.t1, f.t2);
            $crate::println!("[frame] t3={:#018x}  t4={:#018x}  t5={:#018x}  t6={:#018x}",
                f.t3, f.t4, f.t5, f.t6);
            $crate::println!("[frame] s0={:#018x}  s1={:#018x}  s2={:#018x}  s3={:#018x}",
                f.s0, f.s1, f.s2, f.s3);
            $crate::println!("[frame] s4={:#018x}  s5={:#018x}  s6={:#018x}  s7={:#018x}",
                f.s4, f.s5, f.s6, f.s7);
            $crate::println!("[frame] s8={:#018x}  s9={:#018x}  s10={:#018x} s11={:#018x}",
                f.s8, f.s9, f.s10, f.s11);
            $crate::println!("[frame] sstatus={:#018x}  sepc={:#018x}", f.sstatus, f.sepc);
        }
    };
}
