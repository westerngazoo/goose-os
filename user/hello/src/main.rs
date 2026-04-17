#![no_std]
#![no_main]
#![allow(dead_code)]

// GooseOS userspace hello — Phase B net test.
//
// Exercises the kernel net server (PID 3) via SYS_CALL:
//   1. NET_STATUS  → verify stack is up
//   2. NET_SOCKET_UDP → allocate a UDP socket, get a handle
//   3. NET_BIND    → bind to port 9999
//   4. NET_CLOSE   → release it

use core::arch::global_asm;

#[macro_use]
mod gooseos;

global_asm!(include_str!("start.S"));

#[no_mangle]
pub extern "C" fn main() {
    println!("Hello from Rust userspace!");
    println!("My PID is {}", gooseos::getpid());

    println!("[net-test] Calling NET_STATUS...");
    match gooseos::net::status() {
        Ok(v) => println!("[net-test] net up (status={})", v),
        Err(_) => {
            println!("[net-test] net down — bailing");
            gooseos::exit(1);
        }
    }

    println!("[net-test] Opening UDP socket...");
    let handle = match gooseos::net::socket_udp() {
        Ok(h) => {
            println!("[net-test] got UDP handle {}", h);
            h
        }
        Err(_) => {
            println!("[net-test] socket_udp FAILED");
            gooseos::exit(1);
        }
    };

    println!("[net-test] Binding handle {} to port 9999...", handle);
    match gooseos::net::bind(handle, 9999) {
        Ok(()) => println!("[net-test] bound OK"),
        Err(_) => {
            println!("[net-test] bind FAILED");
            gooseos::exit(1);
        }
    }

    println!("[net-test] Closing handle {}...", handle);
    match gooseos::net::close(handle) {
        Ok(()) => println!("[net-test] close OK"),
        Err(_) => println!("[net-test] close FAILED"),
    }

    println!("[net-test] PASS");
    gooseos::exit(0);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    gooseos::exit(1);
}
