//! GooseOS — A RISC-V operating system written in Rust
//!
//! Part 1: Bare-metal boot + Hello World on QEMU virt machine.

#![no_std]
#![no_main]

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
pub extern "C" fn kmain(hart_id: usize, _dtb_addr: usize) -> ! {
    let uart = uart::Uart::new(UART0_BASE);
    uart.init();

    uart.puts("\n");
    uart.puts("          __\n");
    uart.puts("       __( o)>     GooseOS v0.1.0\n");
    uart.puts("      \\  _/        RISC-V 64-bit\n");
    uart.puts("       \\\\\\         Written in Rust\n");
    uart.puts("        \\\\\\__\n");
    uart.puts("         \\   )>    Honk.\n");
    uart.puts("      ~~~^~~~~\n");
    uart.puts("\n");

    // Print which hart we're running on
    uart.puts("  Booted on hart ");
    uart.putc(b'0' + hart_id as u8);
    uart.puts("\n");
    uart.puts("  Hello from GooseOS!\n");
    uart.puts("\n");

    // Halt — nothing else to do yet.
    loop {
        unsafe { asm!("wfi") };
    }
}

/// Panic handler — required by #![no_std].
/// For now, just halts. Part 2 will print panic info to UART.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe { asm!("wfi") };
    }
}
