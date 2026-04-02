//! GooseOS — A RISC-V operating system written in Rust
//!
//! Part 2: print!/println! macros + panic handler with UART output.

#![no_std]
#![no_main]

mod console;
mod uart;

use core::arch::{asm, global_asm};

// Include the RISC-V assembly boot code.
// This defines _start which the linker script places at 0x80200000.
global_asm!(include_str!("boot.S"));

/// QEMU virt machine UART0 base address (NS16550A compatible).
const UART0_BASE: usize = 0x1000_0000;

/// Kernel main — called from boot.S after stack setup.
///
/// # Arguments
/// * `hart_id`  - Hardware thread ID (from OpenSBI via a0)
/// * `dtb_addr` - Device tree blob address (from OpenSBI via a1)
#[no_mangle]
pub extern "C" fn kmain(hart_id: usize, dtb_addr: usize) -> ! {
    // Initialize UART hardware before any printing
    let uart = uart::Uart::new(UART0_BASE);
    uart.init();

    // Now we can use println! everywhere
    println!();
    println!("          __");
    println!("       __( o)>     GooseOS v0.1.0");
    println!("      \\  _/        RISC-V 64-bit");
    println!("       \\\\\\         Written in Rust");
    println!("        \\\\\\__");
    println!("         \\   )>    Honk.");
    println!("      ~~~^~~~~");
    println!();

    // Formatted output — the whole point of Part 2!
    println!("  Booted on hart {}", hart_id);
    println!("  DTB address:   {:#010x}", dtb_addr);
    println!("  Kernel entry:  {:#010x}", kmain as *const () as usize);
    println!();
    println!("  Hello from GooseOS!");
    println!();

    // Halt — nothing else to do yet.
    loop {
        unsafe { asm!("wfi") };
    }
}

/// Panic handler — prints location and message, then halts.
///
/// This is critical for debugging: without it, any bug
/// (array OOB, unwrap on None, explicit panic) just silently hangs.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
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
