//! Pure argument validators for the syscall boundary.
//!
//! No `unsafe`, no MMIO, no statics — host-testable. The `validate`
//! module is the standing answer to "did userspace give us coherent
//! arguments?" It never decides policy (that's the capability system);
//! it only decides shape.
//!
//! Cherry-picked from `goose-os/kernel/src/security.rs`, renamed because
//! (a) it's validation, not enforcement; (b) we want to reserve the
//! name "security" for the capability module that lands in Phase 1.

#![allow(dead_code)]

/// 4 KB page — RISC-V Sv39 leaf.
pub const PAGE_SIZE: usize = 4096;

/// Maximum number of processes. Single source of truth; referenced by
/// the process table, IPC validators, and capability table.
pub const MAX_PROCS: usize = 64;

/// Maximum number of PLIC IRQs the kernel will track.
pub const MAX_IRQS: usize = 64;

/// User-mappable VA range. Below `USER_VA_START` is MMIO; at or above
/// `USER_VA_END` is kernel space. Phase-0 scaffold — revisit when the
/// capability system gates mappings per-module.
pub const USER_VA_START: usize = 0x5000_0000;
pub const USER_VA_END:   usize = 0x8000_0000;

/// Is `addr` page-aligned?
#[inline]
pub const fn is_page_aligned(addr: usize) -> bool {
    addr % PAGE_SIZE == 0
}

/// Is `va` in the user-mappable VA range?
#[inline]
pub const fn is_user_va(va: usize) -> bool {
    va >= USER_VA_START && va < USER_VA_END
}

/// Is `target` a valid IPC target from `current`?
///
/// Rules:
///   - target != 0  (PID 0 is the kernel; no direct IPC to it)
///   - target < MAX_PROCS
///   - target != current  (no self-IPC; would deadlock on sync rendezvous)
#[inline]
pub const fn is_valid_ipc_target(target: usize, current: usize) -> bool {
    target > 0 && target < MAX_PROCS && target != current
}

/// Is `irq` a valid PLIC IRQ number?
#[inline]
pub const fn is_valid_irq(irq: usize) -> bool {
    irq < MAX_IRQS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_alignment_boundaries() {
        assert!(is_page_aligned(0));
        assert!(is_page_aligned(PAGE_SIZE));
        assert!(!is_page_aligned(1));
        assert!(!is_page_aligned(PAGE_SIZE - 1));
    }

    #[test]
    fn user_va_exclusive_endpoints() {
        assert!(is_user_va(USER_VA_START));
        assert!(!is_user_va(USER_VA_END));      // exclusive upper bound
        assert!(!is_user_va(USER_VA_START - 1));
    }

    #[test]
    fn ipc_target_rules() {
        assert!(!is_valid_ipc_target(0, 1));          // no kernel target
        assert!(!is_valid_ipc_target(2, 2));          // no self
        assert!(!is_valid_ipc_target(MAX_PROCS, 1));  // out of bounds
        assert!(is_valid_ipc_target(2, 1));           // ok
    }
}
