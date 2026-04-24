//! Wari user/kernel ABI.
//!
//! This crate is the single source of truth for:
//!   - Syscall numbers (`SYS_*`)
//!   - Syscall error codes (`SyscallError`)
//!   - WASI host function IDs (Phase 0b onward)
//!   - Net / IPC / driver opcodes (Phase 1+)
//!
//! Kernel and all user-side tooling depend on this one crate. Mirror
//! files (like goose-os had) are not allowed — CLAUDE R8 + §Code
//! Quality §1 "no duplicated code."
//!
//! Phase 0a agent cherry-picks `goose-os/kernel/src/abi.rs` contents
//! into this lib. Keep the stability contract: never renumber a shipped
//! syscall, only append.

#![no_std]
#![deny(missing_docs)]

/// Placeholder — Phase 0a agent lands the real contents.
pub const WARI_ABI_VERSION: u32 = 0;

// ── Syscall numbers ────────────────────────────────────────────
// Phase 0a agent ports the SYS_* constants from goose-os/kernel/src/abi.rs.
//
// Example of target layout:
//
//   pub const SYS_PUTCHAR: usize = 0;
//   pub const SYS_EXIT:    usize = 1;
//   ...
//   pub const SYS_MAX: usize = /* last SYS_* */;

// ── SyscallError enum ──────────────────────────────────────────
// Phase 0a agent ports the enum + `into_retval` + tests.

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_placeholder() {
        // Sanity: Phase 0 is pre-ABI-freeze. When the first stable
        // release happens, this test gets rewritten to assert the
        // frozen version number and every SYS_* constant.
        assert_eq!(WARI_ABI_VERSION, 0);
    }
}
