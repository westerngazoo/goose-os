//! GooseOS — A RISC-V operating system written in Rust
//!
//! Phase 12: Preemptive Scheduling — timer-driven context switches.

// When running `cargo test`, use host std library.
// When building for RISC-V, use no_std/no_main.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
// Workaround: rustc nightly ICE on dead_code lint analysis in process module.
// https://github.com/rust-lang/rust/issues/ — crashes in lint_mod/check_mod_deathness.
// Also moved #[cfg(feature)] from fn dump_procs item to inner body to avoid the ICE.
#![allow(dead_code)]

#[cfg(not(test))]
mod console;
#[cfg(not(test))]
mod platform;
#[cfg(not(test))]
mod plic;
#[cfg(not(test))]
mod trap;
#[cfg(not(test))]
mod uart;

mod abi;
#[cfg(not(test))]
mod boot;
#[cfg(not(test))]
mod handlers;
mod page_alloc;
#[allow(dead_code)]
mod page_table;
#[cfg(not(test))]
mod kvm;
#[cfg(not(test))]
mod process;
#[cfg(not(test))]
mod sched;
#[cfg(not(test))]
mod lifecycle;
#[cfg(not(test))]
mod ipc;
#[cfg(not(test))]
mod syscall;
#[cfg(not(test))]
mod elf;
#[allow(dead_code)]
mod security;
#[allow(dead_code)]
mod wasm;
#[allow(dead_code)]
mod interp;
#[allow(dead_code)]
mod wasi;

// Phase B: Networking modules
#[cfg(not(test))]
mod driver;
#[cfg(all(not(test), feature = "net"))]
mod virtio;
#[cfg(all(not(test), feature = "net"))]
mod net;


// ── Kernel code (only compiled for RISC-V target, not during host tests) ──

#[cfg(not(test))]
mod kernel {
    use core::arch::{asm, global_asm};
    use crate::println;

    // Include the RISC-V assembly boot code.
    global_asm!(include_str!("boot.S"));

    /// Call SBI System Reset extension to reboot the machine.
    ///
    /// SBI SRST extension:
    ///   EID = 0x53525354 ("SRST")
    ///   FID = 0
    ///   a0  = reset_type  (0=shutdown, 1=cold reboot, 2=warm reboot)
    ///   a1  = reset_reason (0=no reason, 1=system failure)
    pub fn sbi_system_reset() -> ! {
        unsafe {
            asm!(
                "ecall",
                in("a0") 1usize,       // reset_type = cold reboot
                in("a1") 0usize,       // reset_reason = no reason
                in("a6") 0usize,       // FID = 0
                in("a7") 0x53525354usize, // EID = SRST
                options(noreturn)
            );
        }
    }

    /// Kernel entry point — called from boot.S after stack setup.
    ///
    /// Hands off to the stage-structured boot sequence in crate::boot.
    /// Everything past here — console init, interrupt wiring, MMU,
    /// device discovery, userspace launch — lives in boot.rs so that
    /// the boot sequence can be read as a flat list of named stages.
    ///
    /// # Arguments
    /// * `hart_id`  - Hardware thread ID (from OpenSBI via a0)
    /// * `dtb_addr` - Device tree blob address (from OpenSBI via a1)
    #[no_mangle]
    pub extern "C" fn kmain(hart_id: usize, dtb_addr: usize) -> ! {
        crate::boot::run(hart_id, dtb_addr)
    }

    /// Panic handler — prints location and message, then halts.
    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        // Disable interrupts so panic output isn't interleaved
        unsafe { asm!("csrc sstatus, {}", in(reg) 1usize << 1); }

        println!();
        println!("!!! KERNEL PANIC !!!");

        if let Some(location) = info.location() {
            println!(
                "  at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            );
        }

        if let Some(message) = info.message().as_str() {
            println!("  {}", message);
        } else {
            println!("  {}", info.message());
        }

        println!();
        println!("System halted.");

        loop {
            unsafe { asm!("wfi") };
        }
    }
}
