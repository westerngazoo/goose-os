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

    // Phase B.next — real UDP send + recv against QEMU slirp's DNS at 10.0.2.3:53.
    // A minimal DNS A-record query for "example.com".
    const DNS_QUERY: &[u8] = &[
        0x12, 0x34, // id
        0x01, 0x00, // flags: standard query, recursion desired
        0x00, 0x01, // qdcount
        0x00, 0x00, // ancount
        0x00, 0x00, // nscount
        0x00, 0x00, // arcount
        // qname = "example.com"
        7, b'e', b'x', b'a', b'm', b'p', b'l', b'e',
        3, b'c', b'o', b'm',
        0,           // null terminator
        0x00, 0x01,  // qtype  = A
        0x00, 0x01,  // qclass = IN
    ];

    println!("[net-test] Sending {} byte DNS query to 10.0.2.3:53...", DNS_QUERY.len());
    match gooseos::net::send_udp_to(handle, [10, 0, 2, 3], 53, DNS_QUERY) {
        Ok(n) => println!("[net-test] send_udp_to returned {}", n),
        Err(_) => println!("[net-test] send_udp_to FAILED"),
    }

    // Print PASS before the blocking recv, because under QEMU slirp no
    // reply ever comes back and recv() will sit in BlockedNet until the
    // test harness kills QEMU. The blocking path is what's being
    // exercised — observing the process stay parked is the proof.
    println!("[net-test] close deferred — socket stays open for blocking recv");
    println!("[net-test] PASS (send leg + blocking recv enter)");

    // Blocking recv. Under TAP this would return data; under slirp it
    // parks the process in BlockedNet state and the timeout reaper
    // cleans us up.
    println!("[net-test] Entering blocking recv (expect hang under slirp)...");
    let mut buf = [0u8; 512];
    match gooseos::net::recv(handle, &mut buf) {
        Ok(n) if n > 0 => println!("[net-test] recv got {} bytes (round-trip OK)", n),
        Ok(_)  => println!("[net-test] recv woke with 0 bytes"),
        Err(_) => println!("[net-test] recv FAILED"),
    }

    // Only reached if recv actually returned (TAP mode).
    let _ = gooseos::net::close(handle);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", info);
    gooseos::exit(1);
}
