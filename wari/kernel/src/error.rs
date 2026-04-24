//! `KernelError` — the single error taxonomy for the kernel (CLAUDE R5).
//!
//! Every fallible operation inside Tier 0 returns `Result<T, KernelError>`.
//! Panics are last-resort only, with a justifying comment.
//!
//! `KernelError` differs from `wari_abi::SyscallError`: the ABI error
//! is the userspace-visible encoding (fits in `a0`). `KernelError` is
//! the internal richer enum, converted at the syscall boundary. This
//! separation lets the kernel distinguish "target process is in state X"
//! from "out of physical pages" — details userspace doesn't need.

#![allow(dead_code)]

/// Internal kernel result type. Mapped to `wari_abi::SyscallError` at
/// the syscall boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelError {
    /// An argument was out of range or malformed.
    InvalidArgument,
    /// Target PID does not exist or is not in the expected state.
    NoSuchProcess,
    /// Caller does not hold the capability required for this operation.
    PermissionDenied,
    /// Operation would block; caller did not opt into blocking.
    WouldBlock,
    /// Out of physical pages.
    OutOfPages,
    /// Out of handles in a fixed pool (sockets, caps, etc.).
    OutOfHandles,
    /// Page/handle/capability not mapped or not owned by caller.
    NotMapped,
    /// WASM module failed validation (Phase 0+).
    BadWasm,
    /// Driver-layer failure — see driver-specific log line for detail.
    DriverError,
}

impl KernelError {
    /// Convert to the userspace-visible `SyscallError`.
    ///
    /// Multiple kernel errors may collapse to the same user error —
    /// userspace rarely needs the internal distinction, and collapsing
    /// limits information leakage across the trust boundary.
    pub const fn into_syscall(self) -> wari_abi::SyscallError {
        use wari_abi::SyscallError as E;
        match self {
            KernelError::InvalidArgument  => E::InvalidArgument,
            KernelError::NoSuchProcess    => E::NoSuchProcess,
            KernelError::PermissionDenied => E::PermissionDenied,
            KernelError::WouldBlock       => E::WouldBlock,
            KernelError::OutOfPages       => E::OutOfResources,
            KernelError::OutOfHandles     => E::OutOfResources,
            KernelError::NotMapped        => E::NotMapped,
            KernelError::BadWasm          => E::BadWasm,
            KernelError::DriverError      => E::Generic,
        }
    }
}
