//! Tier-1 "hello world" WASM module — Phase 0 exit demo.
//!
//! When the Phase 0 milestone is reached, this module is built to
//! `wari-hello.wasm`, signed, loaded as Tier-1 PID 1, and calls
//! `fd_write(1, "Hello from Wari\n")` followed by `proc_exit(0)`.
//!
//! Phase 0a agent lands the actual bytes. Today: an empty crate that
//! compiles as a cdylib for `wasm32-unknown-unknown`.

#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // On a real panic, a Tier-1 module should call proc_exit(1). That
    // binding arrives with the wasmi integration PR.
    loop {}
}
