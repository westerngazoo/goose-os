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
}
