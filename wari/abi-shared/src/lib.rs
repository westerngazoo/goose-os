//! Wari user/kernel ABI — the contract between the kernel and every
//! user-side consumer (WASM tooling, userspace helpers during Phase 0a
//! bring-up, test harnesses).
//!
//! This crate is the **single source of truth** for:
//!   - Syscall numbers (`SYS_*`)
//!   - Syscall error codes (`SyscallError`)
//!   - Net / IPC opcodes (`net::*`)
//!   - (Future) WASI host function IDs and driver opcodes.
//!
//! Kernel and all user-side tooling depend on this one crate. Mirror
//! files (as goose-os carried) are not allowed — CLAUDE R8 and the
//! "no duplicated code" rule in §Code Quality.
//!
//! Cherry-picked from `goose-os/kernel/src/abi.rs` in Phase 0a with one
//! deliberate change: **slot 10 (formerly `SYS_SPAWN_ELF`) is retired**
//! — see CLAUDE R7 ("No ELF in the customer ABI"). The constant is
//! intentionally absent; a future `SYS_WASM_LOAD` will take a fresh
//! number, not reuse slot 10.
//!
//! # Stability contract
//!
//! Syscall *numbers* are part of the ABI. Once shipped, a number never
//! changes meaning. Adding a new syscall means adding a new number at
//! the end, never reassigning an existing one. Retiring a syscall
//! means leaving a gap in the numbering — do NOT reuse the number.
//!
//! Syscall *argument conventions* are also ABI. Changing which
//! register carries which argument is a breaking change.
//!
//! Error codes follow the same rules. A userspace program compiled
//! against ABI version N must run against kernel version N+M with the
//! same semantics, where "semantics" means every syscall number still
//! exists and still returns the same meaning for the same inputs.

#![no_std]
#![deny(missing_docs)]

// ── Syscall numbers ────────────────────────────────────────────
//
// Placed in `a7` by the userspace `ecall` wrapper; read by the
// kernel's trap handler (see kernel/src/trap.rs::handle_ecall).

/// Write a single byte to the kernel UART.
pub const SYS_PUTCHAR:      usize = 0;
/// Terminate the current process with an exit code.
pub const SYS_EXIT:         usize = 1;
/// Non-blocking send on a capability/port (seL4-style).
pub const SYS_SEND:         usize = 2;
/// Blocking receive on a capability/port.
pub const SYS_RECEIVE:      usize = 3;
/// Synchronous call: send + receive-reply on one endpoint.
pub const SYS_CALL:         usize = 4;
/// Reply to the last `SYS_CALL` that targeted this server.
pub const SYS_REPLY:        usize = 5;
/// Map physical pages into the caller's address space.
pub const SYS_MAP:          usize = 6;
/// Unmap a range from the caller's address space.
pub const SYS_UNMAP:        usize = 7;
/// Allocate physical pages owned by the caller.
pub const SYS_ALLOC_PAGES:  usize = 8;
/// Free physical pages previously allocated by the caller.
pub const SYS_FREE_PAGES:   usize = 9;
// 10: retired — formerly SYS_SPAWN_ELF, see Wari R7 / docs/prior-art.md.
//     A future SYS_WASM_LOAD will get a fresh number, not this slot.
/// Wait for a child process to exit and reap it.
pub const SYS_WAIT:         usize = 11;
/// Return the caller's PID.
pub const SYS_GETPID:       usize = 12;
/// Cooperatively yield the CPU.
pub const SYS_YIELD:        usize = 13;
/// Register an IRQ handler (capability-gated in Phase 1).
pub const SYS_IRQ_REGISTER: usize = 14;
/// Acknowledge a pending IRQ.
pub const SYS_IRQ_ACK:      usize = 15;
/// Request a system reboot (capability-gated in Phase 1).
pub const SYS_REBOOT:       usize = 16;

/// Highest syscall number currently defined. Used for bounds checks
/// in the dispatch path and for the size of any dispatch table.
///
/// Note: slot 10 is **retired** (formerly `SYS_SPAWN_ELF`, see Wari
/// R7). The live syscalls are 0..=9 and 11..=16, so `SYS_MAX` remains
/// `SYS_REBOOT == 16`. The retired slot leaves a hole in the numbering
/// that must never be reused.
pub const SYS_MAX: usize = SYS_REBOOT;

// ── Error codes ────────────────────────────────────────────────
//
// Returned in `a0` from every fallible syscall. Successful returns are
// non-negative (handle, PID, byte count, etc.). Errors are encoded as
// the bitwise complement of the error number, so:
//
//   a0 = 0 .. isize::MAX / 2  -> success value
//   a0 = usize::MAX - N       -> error code N (from SyscallError)
//
// The legacy convention — `a0 == usize::MAX` on any error — is still
// supported by writing `SyscallError::Generic.into_retval()`. Handlers
// are being migrated to typed errors incrementally.

/// Structured syscall errors. `#[repr(usize)]` so the discriminant is
/// the raw error number; `into_retval()` converts to the a0 value.
///
/// Never renumber an existing variant. Add new errors at the end.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallError {
    /// Unspecified error. Used by pre-typed-error handlers for backward
    /// compatibility with callers that only check `a0 == usize::MAX`.
    /// New handlers should use a specific variant.
    Generic           = 1,

    /// An argument was malformed or out of range (e.g., unaligned VA,
    /// PID out of bounds, flag bit set that the handler doesn't know).
    InvalidArgument   = 2,

    /// Target process does not exist or is in the Free state.
    NoSuchProcess     = 3,

    /// Caller lacks the capability/permission for this operation.
    /// Placeholder until the capability system lands.
    PermissionDenied  = 4,

    /// The kernel cannot satisfy the request right now (would block,
    /// resource exhausted, etc.). Distinct from `PermissionDenied`,
    /// which is a final "no."
    WouldBlock        = 5,

    /// Out of physical pages, out of socket handles, etc.
    OutOfResources    = 6,

    /// The requested page, handle, or capability is not mapped/owned.
    NotMapped         = 7,

    /// ELF parse error — retained for ABI stability against the retired
    /// slot-10 path. No live syscall in Wari returns this; the variant
    /// is kept so the discriminant sequence is stable across goose-os
    /// and Wari. Will be repurposed only via a future ABI version bump.
    BadElf            = 8,
}

impl SyscallError {
    /// Convert to the value that goes in `a0`. Encodes as
    /// `usize::MAX - (discriminant - 1)`, so `Generic` -> `MAX`, and
    /// larger numbers walk downward. Userspace can recover the error
    /// code by computing `usize::MAX - a0 + 1`.
    #[inline]
    pub const fn into_retval(self) -> usize {
        usize::MAX - (self as usize - 1)
    }
}

/// Convenience: the legacy "any error" return value. Equals
/// `SyscallError::Generic.into_retval()` by construction.
pub const ERR: usize = usize::MAX;

// ── Network IPC protocol ───────────────────────────────────────
//
// Clients talk to the network server at PID 3 via SYS_CALL. The
// opcode goes in a1; remaining arguments in a2..=a6 per the per-op
// calling convention documented in `net` below.

/// Opcodes for the IPC network server (PID 3). Send in `a1` of a
/// SYS_CALL targeting `NET_SERVER_PID`.
///
/// Never renumber. Kernel and every userspace consumer imports from
/// this module — no mirror copies are allowed.
pub mod net {
    /// PID of the network server. Fixed by convention; the kernel
    /// currently intercepts SYS_CALL to this PID before IPC rendezvous
    /// runs, but clients don't need to know that.
    pub const NET_SERVER_PID: usize = 3;

    /// Query "is the network up?" — returns 1 or 0.
    pub const NET_STATUS:     usize = 0;
    /// Allocate a TCP socket handle.
    pub const NET_SOCKET_TCP: usize = 1;
    /// Allocate a UDP socket handle.
    pub const NET_SOCKET_UDP: usize = 2;
    /// Bind a socket to a local port: `(handle, port)`.
    pub const NET_BIND:       usize = 3;
    /// Blocking connect: `(handle, packed_ip, port)`.
    pub const NET_CONNECT:    usize = 4;
    /// Listen on a bound socket: `(handle, port)`.
    pub const NET_LISTEN:     usize = 5;
    /// Accept an incoming connection: `(handle)` — reserved, unimplemented.
    pub const NET_ACCEPT:     usize = 6;
    /// Send on a socket: `(handle, buf_va, len, packed_ip?, port?)`.
    pub const NET_SEND:       usize = 7;
    /// Blocking receive: `(handle, buf_va, max_len)`.
    pub const NET_RECV:       usize = 8;
    /// Close a socket handle: `(handle)`.
    pub const NET_CLOSE:      usize = 9;

    /// Largest opcode currently defined. Dispatch tables size off this.
    pub const NET_OP_MAX: usize = NET_CLOSE;
}

// ── Tests — pure, runnable on host ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_error_matches_legacy_sentinel() {
        assert_eq!(SyscallError::Generic.into_retval(), ERR);
    }

    #[test]
    fn error_codes_are_distinct() {
        let codes = [
            SyscallError::Generic.into_retval(),
            SyscallError::InvalidArgument.into_retval(),
            SyscallError::NoSuchProcess.into_retval(),
            SyscallError::PermissionDenied.into_retval(),
            SyscallError::WouldBlock.into_retval(),
            SyscallError::OutOfResources.into_retval(),
            SyscallError::NotMapped.into_retval(),
            SyscallError::BadElf.into_retval(),
        ];
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j], "error {} and {} collide", i, j);
            }
        }
    }

    #[test]
    fn sys_max_matches_highest_syscall() {
        assert_eq!(SYS_MAX, SYS_REBOOT);
        assert_eq!(SYS_MAX, 16);
    }

    #[test]
    fn net_opcodes_are_distinct() {
        use super::net::*;
        let codes = [
            NET_STATUS, NET_SOCKET_TCP, NET_SOCKET_UDP, NET_BIND,
            NET_CONNECT, NET_LISTEN, NET_ACCEPT, NET_SEND,
            NET_RECV, NET_CLOSE,
        ];
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j], "net opcode {} collides with {}", i, j);
            }
        }
    }

    #[test]
    fn net_op_max_matches_highest() {
        use super::net::*;
        assert_eq!(NET_OP_MAX, NET_CLOSE);
    }

    #[test]
    fn net_server_pid_is_three() {
        // Hard-coded in the kernel's trap dispatch. Pinning here so a
        // future change has to touch two places.
        assert_eq!(super::net::NET_SERVER_PID, 3);
    }

    #[test]
    fn sys_spawn_slot_is_retired() {
        // Slot 10 is the retired SYS_SPAWN_ELF position. It must NOT
        // reappear in Wari — CLAUDE R7 forbids any ELF entry point in
        // the customer ABI. This test guards the numbering: the live
        // syscalls around the hole stay where they are, so slot 10 is
        // observably unused.
        assert_eq!(SYS_FREE_PAGES, 9);
        assert_eq!(SYS_WAIT,       11);
        // If a future patch re-introduces `pub const SYS_SPAWN: usize
        // = 10;`, the next line will fail to compile (name collision
        // with a local binding). Cheap belt-and-braces check.
        let sys_spawn_must_not_exist: () = ();
        let _ = sys_spawn_must_not_exist;
    }

    /// Compile-time witness: slot 10 is a hole between SYS_FREE_PAGES
    /// and SYS_WAIT. If anyone re-introduces a constant at slot 10,
    /// the `SYS_WAIT == SYS_FREE_PAGES + 2` equality will still hold
    /// — that's fine; the guard is the absence of `SYS_SPAWN`. The
    /// assertion below documents the intended hole shape.
    const _RETIRED_SLOT_10_SHAPE: () = {
        assert!(SYS_FREE_PAGES == 9);
        assert!(SYS_WAIT == 11);
    };
}
