#![no_std]
#![no_main]
#![allow(dead_code)]

// GooseOS userspace hello world — first compiled Rust program on GooseKern.
//
// This binary is compiled separately from the kernel, embedded via
// include_bytes!, and loaded by the kernel's ELF loader at boot.

use core::arch::global_asm;

#[macro_use]
mod gooseos;

// Include the _start entry point (sets gp, zeroes BSS, calls main)
global_asm!(include_str!("start.S"));

#[no_mangle]
pub extern "C" fn main() {
    println!("Hello from Rust userspace!");
    println!("My PID is {}", gooseos::getpid());

    // Clean exit
    gooseos::exit(0);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    gooseos::exit(1);
}
