//! Kernel/userspace ABI — the contract between kernel and user programs.
//!
//! This file is the ONE SOURCE OF TRUTH for syscall numbers and error
//! codes. Any change here must be mirrored in every userspace program's
//! local copy (today: `userspace/hello/src/gooseos.rs` and
//! `userspace/netsrv/src/gooseos.rs`).
//!
//! The mirroring is manual until a shared `abi` crate is extracted into
//! a Cargo workspace. That's tracked as a follow-up.
//!
//! # Stability contract
//!
//! Syscall *numbers* are part of the ABI. Once shipped, a number never
//! changes meaning. Adding a new syscall means adding a new number at
//! the end, never reassigning an existing one. Removing a syscall
//! means retiring the number (leave a gap in the enum, do NOT reuse).
//!
//! Syscall *argument conventions* are also ABI. Changing which
//! register carries which argument is a breaking change.
//!
//! Error codes follow the same rules. A userspace program compiled
//! against ABI version N must run against kernel version N+M with the
//! same semantics, where "semantics" means every syscall number still
//! exists and still returns the same meaning for the same inputs.

// ── Syscall numbers ────────────────────────────────────────────
//
// Placed in `a7` by the userspace `ecall` wrapper; read by the
// kernel's trap handler (see kernel/src/trap.rs::handle_ecall).

pub const SYS_PUTCHAR:      usize = 0;
pub const SYS_EXIT:         usize = 1;
pub const SYS_SEND:         usize = 2;
pub const SYS_RECEIVE:      usize = 3;
pub const SYS_CALL:         usize = 4;
pub const SYS_REPLY:        usize = 5;
pub const SYS_MAP:          usize = 6;
pub const SYS_UNMAP:        usize = 7;
pub const SYS_ALLOC_PAGES:  usize = 8;
pub const SYS_FREE_PAGES:   usize = 9;
pub const SYS_SPAWN:        usize = 10;
pub const SYS_WAIT:         usize = 11;
pub const SYS_GETPID:       usize = 12;
pub const SYS_YIELD:        usize = 13;
pub const SYS_IRQ_REGISTER: usize = 14;
pub const SYS_IRQ_ACK:      usize = 15;
pub const SYS_REBOOT:       usize = 16;

/// Highest syscall number currently defined. Used for bounds checks
/// in the dispatch path and for the size of any dispatch table.
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
    /// resource exhausted, etc.). Distinct from PermissionDenied,
    /// which is a final "no."
    WouldBlock        = 5,

    /// Out of physical pages, out of socket handles, etc.
    OutOfResources    = 6,

    /// The requested page, handle, or capability is not mapped/owned.
    NotMapped         = 7,

    /// ELF parse error during SYS_SPAWN.
    BadElf            = 8,
}

impl SyscallError {
    /// Convert to the value that goes in `a0`. Encodes as
    /// `usize::MAX - (discriminant - 1)`, so Generic -> MAX, and larger
    /// numbers walk downward. Userspace can recover the error code by
    /// computing `usize::MAX - a0 + 1`.
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
/// Never renumber. Kernel (`net.rs`) and every userspace ABI mirror
/// (`userspace/hello/src/gooseos.rs::net`, `userspace/netsrv/src/main.rs`)
/// must agree byte-for-byte. Until a shared-abi crate lands, each
/// mirror carries a link back to this module as the source of truth.
pub mod net {
    /// PID of the network server. Fixed by convention; the kernel
    /// currently intercepts SYS_CALL to this PID before IPC rendezvous
    /// runs, but clients don't need to know that.
    pub const NET_SERVER_PID: usize = 3;

    pub const NET_STATUS:     usize = 0;  // is network up? -> 1/0
    pub const NET_SOCKET_TCP: usize = 1;  // -> tcp socket handle
    pub const NET_SOCKET_UDP: usize = 2;  // -> udp socket handle
    pub const NET_BIND:       usize = 3;  // (handle, port)
    pub const NET_CONNECT:    usize = 4;  // (handle, packed_ip, port) — blocking
    pub const NET_LISTEN:     usize = 5;  // (handle, port)
    pub const NET_ACCEPT:     usize = 6;  // (handle) — reserved, unimplemented
    pub const NET_SEND:       usize = 7;  // (handle, buf_va, len, packed_ip?, port?)
    pub const NET_RECV:       usize = 8;  // (handle, buf_va, max_len) — blocking
    pub const NET_CLOSE:      usize = 9;  // (handle)

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
        // Hard-coded in trap.rs dispatch. Pinning here so a future
        // change has to touch two places.
        assert_eq!(super::net::NET_SERVER_PID, 3);
    }
}
