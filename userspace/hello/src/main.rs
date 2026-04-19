#![no_std]
#![no_main]
#![allow(dead_code)]

// GooseOS userspace hello.
//
// Without the `net` feature: plain "Hello from Rust userspace!" + exit.
// With    the `net` feature: exercises the kernel net server (PID 3):
//   NET_STATUS -> NET_SOCKET_UDP -> NET_BIND -> NET_CLOSE.
//
// The `net` feature on this crate must be matched by the kernel being
// built with its own `net` feature, otherwise SYS_CALL to PID 3 hangs.

use core::arch::global_asm;

#[macro_use]
mod gooseos;

global_asm!(include_str!("start.S"));

#[no_mangle]
pub extern "C" fn main() {
    println!("Hello from Rust userspace!");
    println!("My PID is {}", gooseos::getpid());

    #[cfg(feature = "net")]
    run_net_test();

    gooseos::exit(0);
}

#[cfg(feature = "net")]
fn run_net_test() {
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
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    gooseos::exit(1);
}
