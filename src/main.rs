//! GooseOS — A RISC-V operating system written in Rust
//!
//! Phase 12: Preemptive Scheduling — timer-driven context switches.

// When running `cargo test`, use host std library.
// When building for RISC-V, use no_std/no_main.
#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

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

mod page_alloc;
#[allow(dead_code)]
mod page_table;
#[cfg(not(test))]
mod kvm;
#[cfg(not(test))]
mod process;
#[cfg(not(test))]
mod elf;
#[allow(dead_code)]
mod wasm;
#[allow(dead_code)]
mod interp;


// ── Kernel code (only compiled for RISC-V target, not during host tests) ──

#[cfg(not(test))]
mod kernel {
    use core::arch::{asm, global_asm};
    use crate::{page_alloc, kvm, process, println, platform, trap, plic, uart};

    // Include the RISC-V assembly boot code.
    global_asm!(include_str!("boot.S"));

    /// Call SBI System Reset extension to reboot the machine.
    ///
    /// SBI SRST extension:
    ///   EID = 0x53525354 ("SRST")
    ///   FID = 0
    ///   a0  = reset_type  (0=shutdown, 1=cold reboot, 2=warm reboot)
    ///   a1  = reset_reason (0=no reason, 1=system failure)
    fn sbi_system_reset() -> ! {
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

    /// Kernel main — called from boot.S after stack setup.
    ///
    /// # Arguments
    /// * `hart_id`  - Hardware thread ID (from OpenSBI via a0)
    /// * `dtb_addr` - Device tree blob address (from OpenSBI via a1)
    #[no_mangle]
    pub extern "C" fn kmain(hart_id: usize, dtb_addr: usize) -> ! {
        // === Phase 1: UART init (polling) ===
        let uart = uart::Uart::platform();
        uart.init();

        println!();
        println!("          __");
        println!("       __( o)>     GooseOS v0.1.0 build {}", option_env!("GOOSE_BUILD").unwrap_or("dev"));
        println!("      \\  _/        RISC-V 64-bit");
        println!("       \\\\\\         Written in Rust");
        println!("        \\\\\\        Platform: {}", platform::PLATFORM_NAME);
        println!("         \\   )_    Honk.");
        println!("      ~~~^~~~~");
        println!();

        println!("  Booted on hart {}", hart_id);
        println!("  DTB address:   {:#010x}", dtb_addr);
        println!("  Kernel entry:  {:#010x}", kmain as *const () as usize);
        println!();

        // === Phase 2: Set up trap vector (but don't enable IRQs yet) ===
        trap::trap_init();

        // === Phase 3: Configure PLIC ===
        plic::init();

        // === Phase 4: Enable UART RX interrupts ===
        uart.enable_rx_interrupt();
        println!("  [uart] RX interrupts enabled");

        // === Phase 5: Arm the timer ===
        trap::timer_init();

        // === Phase 6: Go live — enable interrupts globally ===
        trap::interrupts_enable();

        // === Phase 7: Initialize page allocator ===
        page_alloc::init();
        page_alloc::self_test();

        // === Phase 8: Build kernel page table + enable MMU ===
        let root_pt = kvm::init();
        unsafe { kvm::enable_mmu(root_pt); }

        let alloc = unsafe { page_alloc::get() };
        println!("  [page_alloc] {} pages used for page tables, {} free",
            alloc.allocated_count(), alloc.free_count());
        println!();

        // === Phase 12: Create processes + launch scheduler ===
        // Creates init (PID 1) + UART server (PID 2), then srets to PID 1.
        // RPC between processes — init calls server, server prints + replies.
        // After all processes exit, control returns to post_process_exit().
        process::launch();
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
