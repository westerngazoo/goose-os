//! Syscall handlers that didn't yet have a canonical home.
//!
//! Most handlers live where the domain they operate on lives:
//!   - IPC handlers            → ipc.rs
//!   - process lifecycle       → process.rs
//!   - memory / IRQ / spawn    → syscall.rs
//!
//! This file picks up the three that were previously inlined in
//! trap.rs::handle_ecall, plus the wrapper that mixes a SYS_CALL
//! with the Phase B net-server intercept.
//!
//! After the full handler-consolidation refactor (tracked as the
//! next refactor build), every sys_* handler should live in a
//! per-domain file under kernel/src/handlers/, and this file
//! becomes the handlers/ mod.rs dispatch table. For now, keep
//! scope small: introduce the dispatch pattern without moving
//! existing handler bodies.

use crate::trap::TrapFrame;

/// SYS_PUTCHAR — write one byte to the platform console.
///
/// Previously inlined into trap.rs::handle_ecall. Promoted to a
/// named function so it can be referenced from the dispatch table
/// with the same `fn(&mut TrapFrame)` signature as every other
/// handler.
pub fn sys_putchar(frame: &mut TrapFrame) {
    let ch = frame.a0 as u8;
    crate::uart::Uart::platform().putc(ch);
    frame.a0 = 0;
    frame.sepc += 4;
}

/// SYS_CALL with Phase B network-server intercept.
///
/// Temporary wrapper. When the network stack moves out of the
/// kernel (Item 2 of the architecture refactor), PID 3 becomes a
/// real process and this whole wrapper disappears — the dispatch
/// table entry for SYS_CALL will just be `ipc::sys_call`.
///
/// Until then: if the target PID is the virtual net server, route
/// directly to `net::handle_request` instead of the normal IPC
/// path. Same signature as the other handlers so it drops into
/// the dispatch table cleanly.
pub fn sys_call(frame: &mut TrapFrame) {
    #[cfg(feature = "net")]
    if frame.a0 == crate::net::NET_SERVER_PID {
        crate::net::handle_request(frame);
        return;
    }
    crate::ipc::sys_call(frame);
}

/// SYS_REBOOT — cold-reboot the machine via SBI SRST.
///
/// In practice this never returns (sbi_system_reset is `-> !`), but
/// the function signature is the plain handler shape so the dispatch
/// table stays uniform. The `!` from sbi_system_reset coerces to `()`
/// cleanly as the block's final expression.
pub fn sys_reboot(_frame: &mut TrapFrame) {
    crate::println!("\n  [kernel] SYS_REBOOT — rebooting...");
    crate::kernel::sbi_system_reset();
}

/// Dispatch for unknown syscall numbers.
///
/// Writes legacy ERR (usize::MAX) to a0, advances sepc, and logs.
/// Called from the dispatch table's fallback path.
pub fn sys_unknown(frame: &mut TrapFrame) {
    crate::println!(
        "\n  [kernel] Unknown syscall: {} (a0={:#x})",
        frame.a7, frame.a0
    );
    frame.a0 = crate::abi::ERR;
    frame.sepc += 4;
}
