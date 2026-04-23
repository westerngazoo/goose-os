#![no_std]
#![no_main]
#![allow(dead_code)]

//! GooseOS Network Server — userspace PID 3.
//!
//! Step 2a skeleton: receive/reply loop, opcode dispatch, every
//! handler stubbed. No smoltcp yet, no VirtIO access. Proves the
//! crate compiles against the GooseOS userspace ABI and integrates
//! with the kernel's IPC machinery.
//!
//! Step 2b+ will:
//!   - pull smoltcp in as a dep (once the kernel side stops owning it)
//!   - replace each `unimplemented!()` stub with the real smoltcp call
//!   - share the device via a new SYS_REQUEST_DEVICE capability (when
//!     the capability system lands) or a static shared mapping (for now)
//!
//! The client API (userspace/hello/src/gooseos.rs `net` module) does
//! not change. From the client's perspective, PID 3 has always looked
//! like a server — today it's a kernel handler pretending to be one,
//! tomorrow it's this binary actually being one.

use core::arch::global_asm;

#[macro_use]
mod gooseos;

global_asm!(include_str!("start.S"));

// ── Net opcodes (must match kernel/src/net.rs) ────────────────
//
// Once the migration is complete and the kernel intercept goes away,
// this module becomes the single source of truth for the opcode
// numbers. Until then, both copies must agree.

const NET_STATUS:     usize = 0;
const NET_SOCKET_TCP: usize = 1;
const NET_SOCKET_UDP: usize = 2;
const NET_BIND:       usize = 3;
const NET_CONNECT:    usize = 4;
const NET_LISTEN:     usize = 5;
const NET_ACCEPT:     usize = 6;
const NET_SEND:       usize = 7;
const NET_RECV:       usize = 8;
const NET_CLOSE:      usize = 9;

const ERR: usize = usize::MAX;

// ── Entry point ───────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn main() {
    let pid = gooseos::getpid();
    println!("[netsrv] starting (PID {})", pid);

    // Sanity check: this binary should be spawned as PID 3. If it isn't,
    // clients will still be calling the kernel's intercepted PID 3 and
    // we'll never see a message. Not fatal — just loud.
    if pid != 3 {
        println!("[netsrv] WARNING: expected PID 3, got {} — clients won't find us", pid);
    }

    println!("[netsrv] awaiting IPC requests...");

    // Receive / dispatch / reply forever. seL4 pattern.
    loop {
        // receive_ext captures the full 4-register IPC payload:
        //   a0 = message (here: opcode)
        //   a1 = sender PID
        //   a2 = first argument
        //   a3 = second argument
        // (a4 is not yet carried across rendezvous — see the note in
        // gooseos.rs::receive_ext.)
        let (opcode, sender, arg1, arg2) = gooseos::receive_ext(0);

        let result = dispatch(opcode, arg1, arg2);

        // Reply regardless of success/failure — the caller's SYS_CALL
        // is blocked waiting. A missing reply would hang the client
        // indefinitely. ERR is a valid reply value.
        let _ = gooseos::reply(sender, result);
    }
}

// ── Dispatch ──────────────────────────────────────────────────

fn dispatch(opcode: usize, arg1: usize, arg2: usize) -> usize {
    match opcode {
        NET_STATUS     => handle_status(),
        NET_SOCKET_TCP => handle_socket_tcp(),
        NET_SOCKET_UDP => handle_socket_udp(),
        NET_BIND       => handle_bind(arg1, arg2),
        NET_CONNECT    => handle_connect(arg1, arg2),
        NET_LISTEN     => handle_listen(arg1),
        NET_ACCEPT     => handle_accept(arg1),
        NET_SEND       => handle_send(arg1, arg2),
        NET_RECV       => handle_recv(arg1, arg2),
        NET_CLOSE      => handle_close(arg1),
        _              => ERR,
    }
}

// ── Handlers (Step 2a stubs) ──────────────────────────────────
//
// Every handler returns a single usize. Success returns 0 or a handle.
// Failure returns ERR (usize::MAX). This mirrors the kernel's current
// net.rs convention exactly — swapping this binary in for the kernel
// intercept should be a no-op from the client's point of view.

fn handle_status() -> usize {
    // Advertise readiness. The kernel version returns 1 when smoltcp
    // init has succeeded; we return 1 unconditionally because this
    // skeleton has no stack to fail.
    1
}

fn handle_socket_tcp() -> usize { ERR }
fn handle_socket_udp() -> usize { ERR }

fn handle_bind(_handle: usize, _port: usize) -> usize { ERR }
fn handle_connect(_handle: usize, _packed_ip_port: usize) -> usize { ERR }
fn handle_listen(_handle: usize) -> usize { ERR }
fn handle_accept(_handle: usize) -> usize { ERR }

fn handle_send(_handle: usize, _buf_info: usize) -> usize { ERR }
fn handle_recv(_handle: usize, _buf_info: usize) -> usize { ERR }

fn handle_close(_handle: usize) -> usize { ERR }

// ── Panic handler ─────────────────────────────────────────────

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("[netsrv] PANIC: {}", info);
    gooseos::exit(1);
}
