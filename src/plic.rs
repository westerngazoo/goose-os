/// PLIC (Platform-Level Interrupt Controller) driver.
///
/// Uses platform constants for context and IRQ numbers.

use core::ptr;
use crate::platform;
use crate::println;

const PLIC_BASE: usize = platform::PLIC_BASE;
const CONTEXT: usize = platform::PLIC_S_CONTEXT;
const UART0_IRQ: u32 = platform::UART0_IRQ;

// Derived addresses
const fn priority_addr(irq: u32) -> usize {
    PLIC_BASE + (irq as usize) * 4
}
const ENABLE_BASE: usize = PLIC_BASE + 0x2000 + CONTEXT * 0x80;
const THRESHOLD: usize = PLIC_BASE + 0x20_0000 + CONTEXT * 0x1000;
const CLAIM_COMPLETE: usize = THRESHOLD + 4;

/// Initialize the PLIC for S-mode on the boot hart.
pub fn init() {
    unsafe {
        // Set UART0 priority = 1
        ptr::write_volatile(priority_addr(UART0_IRQ) as *mut u32, 1);

        // Enable UART0 in our context
        // IRQ might be > 31, so calculate which enable word and bit
        let word_index = (UART0_IRQ / 32) as usize;
        let bit_index = UART0_IRQ % 32;
        let enable_addr = (ENABLE_BASE + word_index * 4) as *mut u32;
        ptr::write_volatile(enable_addr, 1 << bit_index);

        // Accept all priorities > 0
        ptr::write_volatile(THRESHOLD as *mut u32, 0);
    }

    println!("  [plic] UART0 (IRQ {}) enabled, context={}, threshold=0", UART0_IRQ, CONTEXT);
}

/// Claim the highest-priority pending interrupt.
pub fn claim() -> u32 {
    unsafe { ptr::read_volatile(CLAIM_COMPLETE as *const u32) }
}

/// Signal that we're done handling an interrupt.
pub fn complete(irq: u32) {
    unsafe { ptr::write_volatile(CLAIM_COMPLETE as *mut u32, irq); }
}
