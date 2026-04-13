/// Security validation functions — pure, testable checks for syscall arguments.
///
/// Every security boundary in GooseOS has a named validation function here.
/// The syscall handlers in process.rs call these functions, and the test suite
/// below exercises every boundary condition.
///
/// Design principle: if a future change weakens any check, a test fails.
/// No unsafe, no global state, no architecture dependency — runs on any host.

use crate::page_alloc::PAGE_SIZE;

/// Maximum number of processes (must match process.rs).
const MAX_PROCS: usize = 8;

/// Maximum number of IRQs (must match process.rs).
const MAX_IRQS: usize = 64;

/// User VA range boundaries.
///
/// Below USER_VA_START: MMIO devices (UART at 0x1000_0000, PLIC at 0x0C00_0000).
/// At USER_VA_END and above: kernel code/data/heap/stack (0x8020_0000+).
///
/// Carving this range prevents user processes from:
///   - Mapping over kernel pages (privilege escalation)
///   - Mapping over MMIO regions (device hijacking)
///   - Unmapping kernel pages from their own table (DoS via trap fault)
pub const USER_VA_START: usize = 0x5000_0000;
pub const USER_VA_END: usize = 0x8000_0000;

// ── Validation Functions ──────────────────────────────────────

/// Check if an address is page-aligned (4KB boundary).
///
/// Required for all page table operations — unaligned addresses would
/// cause the MMU to behave unpredictably.
pub fn is_page_aligned(addr: usize) -> bool {
    addr % PAGE_SIZE == 0
}

/// Check if a virtual address is in the user-mappable range.
///
/// Only addresses in [USER_VA_START, USER_VA_END) can be mapped/unmapped
/// by user processes. This prevents:
///   - Overwriting kernel page table entries
///   - Mapping over UART/PLIC MMIO regions
///   - Creating executable pages in kernel address space
pub fn is_user_va(va: usize) -> bool {
    va >= USER_VA_START && va < USER_VA_END
}

/// Validate a PID for IPC operations (SEND, RECEIVE, CALL, REPLY).
///
/// Rules:
///   - PID 0 is the kernel — user processes can't IPC to it
///   - Must be within the process table bounds
///   - Can't send to yourself (would deadlock on synchronous IPC)
pub fn is_valid_ipc_target(target: usize, current: usize) -> bool {
    target > 0 && target < MAX_PROCS && target != current
}

/// Validate map flags argument for SYS_MAP.
///
/// Only two flag values are accepted:
///   0 = USER_RW (read-write data, heap, stack)
///   1 = USER_RX (read-execute code)
///
/// Prevents user from requesting kernel-only flags or invalid combinations.
pub fn is_valid_map_flags(flags: usize) -> bool {
    flags <= 1
}

/// Validate an IRQ number for SYS_IRQ_REGISTER.
///
/// Must be within the PLIC's IRQ range.
pub fn is_valid_irq(irq: usize) -> bool {
    irq < MAX_IRQS
}

/// Validate page allocation count for SYS_ALLOC_PAGES.
///
/// Currently only single-page allocations are supported.
/// Multi-page support would need contiguous allocation logic.
pub fn is_valid_alloc_count(count: usize) -> bool {
    count == 1
}

/// Validate ELF data size for SYS_SPAWN.
///
/// Must be non-zero (can't spawn nothing) and within the 1MB limit
/// (prevents a single spawn from consuming all memory).
pub fn is_valid_elf_size(len: usize) -> bool {
    len > 0 && len <= 1024 * 1024
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Page Alignment ────────────────────────────────────────

    #[test]
    fn test_aligned_zero() {
        assert!(is_page_aligned(0));
    }

    #[test]
    fn test_aligned_4k() {
        assert!(is_page_aligned(0x1000));
    }

    #[test]
    fn test_aligned_kernel_base() {
        assert!(is_page_aligned(0x8020_0000));
    }

    #[test]
    fn test_aligned_large() {
        assert!(is_page_aligned(0x8000_0000));
    }

    #[test]
    fn test_unaligned_1() {
        assert!(!is_page_aligned(1));
    }

    #[test]
    fn test_unaligned_half_page() {
        assert!(!is_page_aligned(0x800));
    }

    #[test]
    fn test_unaligned_4095() {
        assert!(!is_page_aligned(0xFFF));
    }

    #[test]
    fn test_unaligned_page_plus_1() {
        assert!(!is_page_aligned(0x1001));
    }

    // ── User VA Range ─────────────────────────────────────────

    #[test]
    fn test_user_va_start_is_valid() {
        assert!(is_user_va(USER_VA_START));
    }

    #[test]
    fn test_user_va_end_is_exclusive() {
        assert!(!is_user_va(USER_VA_END));
    }

    #[test]
    fn test_user_va_uart_mapping() {
        assert!(is_user_va(0x5E00_0000)); // our UART user VA
    }

    #[test]
    fn test_user_va_stack_region() {
        assert!(is_user_va(0x7FFF_0000)); // spawned process stack VA
    }

    #[test]
    fn test_user_va_just_below_end() {
        assert!(is_user_va(USER_VA_END - 1));
    }

    #[test]
    fn test_user_va_rejects_zero() {
        assert!(!is_user_va(0));
    }

    #[test]
    fn test_user_va_rejects_null_page() {
        assert!(!is_user_va(0x1000));
    }

    #[test]
    fn test_user_va_rejects_uart_mmio() {
        // UART MMIO at 0x1000_0000 is kernel-mapped — user can't remap it
        assert!(!is_user_va(0x1000_0000));
    }

    #[test]
    fn test_user_va_rejects_plic_mmio() {
        // PLIC at 0x0C00_0000 — kernel only
        assert!(!is_user_va(0x0C00_0000));
    }

    #[test]
    fn test_user_va_rejects_kernel_text() {
        assert!(!is_user_va(0x8020_0000));
    }

    #[test]
    fn test_user_va_rejects_kernel_heap() {
        assert!(!is_user_va(0x8800_0000));
    }

    #[test]
    fn test_user_va_rejects_just_below_start() {
        assert!(!is_user_va(USER_VA_START - 1));
    }

    #[test]
    fn test_user_va_rejects_max_address() {
        assert!(!is_user_va(usize::MAX));
    }

    // ── IPC Target Validation ─────────────────────────────────

    #[test]
    fn test_ipc_target_valid_1_to_2() {
        assert!(is_valid_ipc_target(2, 1));
    }

    #[test]
    fn test_ipc_target_valid_2_to_1() {
        assert!(is_valid_ipc_target(1, 2));
    }

    #[test]
    fn test_ipc_target_valid_max_pid() {
        assert!(is_valid_ipc_target(MAX_PROCS - 1, 1));
    }

    #[test]
    fn test_ipc_rejects_pid_zero() {
        // PID 0 = kernel, not a valid IPC target
        assert!(!is_valid_ipc_target(0, 1));
    }

    #[test]
    fn test_ipc_rejects_self() {
        // Sending to self would deadlock on synchronous IPC
        assert!(!is_valid_ipc_target(3, 3));
    }

    #[test]
    fn test_ipc_rejects_at_max() {
        assert!(!is_valid_ipc_target(MAX_PROCS, 1));
    }

    #[test]
    fn test_ipc_rejects_over_max() {
        assert!(!is_valid_ipc_target(100, 1));
    }

    #[test]
    fn test_ipc_rejects_usize_max() {
        assert!(!is_valid_ipc_target(usize::MAX, 1));
    }

    // ── Map Flags Validation ──────────────────────────────────

    #[test]
    fn test_map_flags_rw() {
        assert!(is_valid_map_flags(0));
    }

    #[test]
    fn test_map_flags_rx() {
        assert!(is_valid_map_flags(1));
    }

    #[test]
    fn test_map_flags_rejects_2() {
        // No USER_WX (writable+executable violates W^X)
        assert!(!is_valid_map_flags(2));
    }

    #[test]
    fn test_map_flags_rejects_kernel_like() {
        // Can't request flags that would include kernel-only bits
        assert!(!is_valid_map_flags(0xFF));
    }

    #[test]
    fn test_map_flags_rejects_max() {
        assert!(!is_valid_map_flags(usize::MAX));
    }

    // ── IRQ Validation ────────────────────────────────────────

    #[test]
    fn test_irq_valid_zero() {
        assert!(is_valid_irq(0));
    }

    #[test]
    fn test_irq_valid_qemu_uart() {
        assert!(is_valid_irq(10));
    }

    #[test]
    fn test_irq_valid_vf2_uart() {
        assert!(is_valid_irq(32));
    }

    #[test]
    fn test_irq_valid_last() {
        assert!(is_valid_irq(MAX_IRQS - 1));
    }

    #[test]
    fn test_irq_rejects_at_max() {
        assert!(!is_valid_irq(MAX_IRQS));
    }

    #[test]
    fn test_irq_rejects_over_max() {
        assert!(!is_valid_irq(1000));
    }

    // ── Allocation Count ──────────────────────────────────────

    #[test]
    fn test_alloc_count_one_is_valid() {
        assert!(is_valid_alloc_count(1));
    }

    #[test]
    fn test_alloc_count_rejects_zero() {
        assert!(!is_valid_alloc_count(0));
    }

    #[test]
    fn test_alloc_count_rejects_multi() {
        assert!(!is_valid_alloc_count(2));
    }

    #[test]
    fn test_alloc_count_rejects_large() {
        assert!(!is_valid_alloc_count(1000));
    }

    // ── ELF Size ──────────────────────────────────────────────

    #[test]
    fn test_elf_size_minimum() {
        assert!(is_valid_elf_size(1));
    }

    #[test]
    fn test_elf_size_typical() {
        assert!(is_valid_elf_size(4096));
    }

    #[test]
    fn test_elf_size_at_limit() {
        assert!(is_valid_elf_size(1024 * 1024));
    }

    #[test]
    fn test_elf_size_rejects_zero() {
        assert!(!is_valid_elf_size(0));
    }

    #[test]
    fn test_elf_size_rejects_over_limit() {
        assert!(!is_valid_elf_size(1024 * 1024 + 1));
    }

    #[test]
    fn test_elf_size_rejects_huge() {
        assert!(!is_valid_elf_size(100 * 1024 * 1024));
    }

    // ── Compound Scenarios ────────────────────────────────────
    // These test realistic attack patterns, not just individual functions.

    #[test]
    fn test_attack_map_kernel_text() {
        // Attacker tries to map a page at kernel .text address
        let kernel_text = 0x8020_0000usize;
        assert!(!is_user_va(kernel_text));
    }

    #[test]
    fn test_attack_map_plic() {
        // Attacker tries to map over PLIC to control interrupts
        let plic = 0x0C00_0000usize;
        assert!(!is_user_va(plic));
    }

    #[test]
    fn test_attack_map_uart_direct() {
        // Attacker tries to map own page at UART address to hijack serial
        let uart = 0x1000_0000usize;
        assert!(!is_user_va(uart));
    }

    #[test]
    fn test_attack_unmap_trap_vector() {
        // Attacker tries to unmap the trap vector page
        let trap_vector = 0x8020_0000usize;
        assert!(!is_user_va(trap_vector));
    }

    #[test]
    fn test_attack_unmap_kernel_stack() {
        // Attacker tries to unmap kernel stack → crash on next trap
        let stack = 0x8FEC_0000usize;
        assert!(!is_user_va(stack));
    }

    #[test]
    fn test_attack_ipc_to_kernel() {
        // Attacker tries to IPC to PID 0 (kernel)
        assert!(!is_valid_ipc_target(0, 1));
    }

    #[test]
    fn test_attack_irq_overflow() {
        // Attacker passes huge IRQ number hoping for array OOB
        assert!(!is_valid_irq(0xFFFF_FFFF));
    }

    #[test]
    fn test_attack_alloc_exhaust() {
        // Attacker requests huge allocation hoping for OOM
        assert!(!is_valid_alloc_count(1_000_000));
    }

    #[test]
    fn test_attack_spawn_huge_elf() {
        // Attacker passes huge ELF size hoping to read kernel memory
        assert!(!is_valid_elf_size(usize::MAX));
    }
}
