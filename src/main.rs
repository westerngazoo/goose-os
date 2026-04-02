//! GooseOS — A RISC-V operating system written in Rust
//!
//! Part 4: Platform abstraction — runs on QEMU virt and VisionFive 2.

#![no_std]
#![no_main]

mod console;
mod platform;
mod plic;
mod trap;
mod uart;

use core::arch::{asm, global_asm};

// Include the RISC-V assembly boot code.
global_asm!(include_str!("boot.S"));

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
    println!("       __( o)>     GooseOS v0.1.0");
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

    println!();
    println!("  Interrupts active! Type something...");
    if cfg!(feature = "qemu") {
        println!("  (timer ticks every 10s, Ctrl-A X to exit QEMU)");
    } else {
        println!("  (timer ticks every 10s)");
    }
    println!();

    // Idle loop — wakes on each interrupt, then sleeps again
    loop {
        unsafe { asm!("wfi") };
    }
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
