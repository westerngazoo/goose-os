/// PLIC (Platform-Level Interrupt Controller) driver for QEMU virt.
///
/// The PLIC routes external hardware interrupts (like UART) to
/// specific harts. Each hart has two "contexts":
///   - Context 0: M-mode (owned by OpenSBI, don't touch)
///   - Context 1: S-mode (ours!)
///
/// MMIO register layout:
///   Base: 0x0C00_0000
///   Priority[irq]:    base + irq * 4       (u32, 0=disabled, 1-7=priority)
///   Enable[ctx][word]: base + 0x2000 + ctx*0x80 + word*4  (u32 bitfield)
///   Threshold[ctx]:   base + 0x200000 + ctx*0x1000  (u32)
///   Claim[ctx]:       base + 0x200004 + ctx*0x1000  (u32, read=claim, write=complete)

use core::ptr;

use crate::println;

const PLIC_BASE: usize = 0x0C00_0000;

// S-mode context for hart 0 = context 1
const CONTEXT: usize = 1;

// Derived addresses
const fn priority_addr(irq: u32) -> usize {
    PLIC_BASE + (irq as usize) * 4
}
const ENABLE_BASE: usize = PLIC_BASE + 0x2000 + CONTEXT * 0x80;
const THRESHOLD: usize = PLIC_BASE + 0x20_0000 + CONTEXT * 0x1000;
const CLAIM_COMPLETE: usize = PLIC_BASE + 0x20_0000 + CONTEXT * 0x1000 + 4;

/// UART0 IRQ on QEMU virt
const UART0_IRQ: u32 = 10;

/// Initialize the PLIC for S-mode on hart 0.
///   1. Set UART0 priority to 1 (any nonzero enables it)
///   2. Enable UART0 in context 1
///   3. Set threshold to 0 (accept all priorities > 0)
pub fn init() {
    unsafe {
        // Set UART0 priority = 1
        ptr::write_volatile(priority_addr(UART0_IRQ) as *mut u32, 1);

        // Enable UART0 (IRQ 10) in context 1, word 0 (bits 0-31)
        let enable_word0 = ENABLE_BASE as *mut u32;
        ptr::write_volatile(enable_word0, 1 << UART0_IRQ);

        // Set priority threshold = 0 (allow all)
        ptr::write_volatile(THRESHOLD as *mut u32, 0);
    }

    println!("  [plic] UART0 (IRQ {}) enabled, threshold=0", UART0_IRQ);
}

/// Claim the highest-priority pending interrupt.
/// Returns the IRQ number (0 = no interrupt pending / spurious).
pub fn claim() -> u32 {
    unsafe { ptr::read_volatile(CLAIM_COMPLETE as *const u32) }
}

/// Signal that we're done handling an interrupt.
/// MUST be called after claim() or the PLIC won't deliver more.
pub fn complete(irq: u32) {
    unsafe { ptr::write_volatile(CLAIM_COMPLETE as *mut u32, irq); }
}
