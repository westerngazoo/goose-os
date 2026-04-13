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
///
/// Sets priority and threshold only. Does NOT enable any IRQ yet.
/// IRQs are enabled individually when a process calls SYS_IRQ_REGISTER.
/// This prevents IRQ floods before a userspace server owns the interrupt.
pub fn init() {
    unsafe {
        // Set UART0 priority = 1 (above threshold → eligible for delivery)
        ptr::write_volatile(priority_addr(UART0_IRQ) as *mut u32, 1);

        // Accept all priorities > 0
        ptr::write_volatile(THRESHOLD as *mut u32, 0);
    }

    println!("  [plic] UART0 (IRQ {}) priority=1, context={}, threshold=0", UART0_IRQ, CONTEXT);
    println!("  [plic] IRQ routing deferred until SYS_IRQ_REGISTER");
}

/// Enable a specific IRQ at the PLIC (set the enable bit).
///
/// Called from SYS_IRQ_REGISTER when a process claims an IRQ.
pub fn enable_irq(irq: u32) {
    unsafe {
        let word_index = (irq / 32) as usize;
        let bit_index = irq % 32;
        let enable_addr = (ENABLE_BASE + word_index * 4) as *mut u32;
        // Read-modify-write to preserve other IRQ enable bits
        let current = ptr::read_volatile(enable_addr as *const u32);
        ptr::write_volatile(enable_addr, current | (1 << bit_index));
    }
    println!("  [plic] IRQ {} enabled at PLIC", irq);
}

/// Claim the highest-priority pending interrupt.
pub fn claim() -> u32 {
    unsafe { ptr::read_volatile(CLAIM_COMPLETE as *const u32) }
}

/// Signal that we're done handling an interrupt.
pub fn complete(irq: u32) {
    unsafe { ptr::write_volatile(CLAIM_COMPLETE as *mut u32, irq); }
}

/// Dump PLIC state for debugging.
pub fn dump() {
    unsafe {
        let pri = ptr::read_volatile(priority_addr(UART0_IRQ) as *const u32);
        let word_index = (UART0_IRQ / 32) as usize;
        let enable_addr = (ENABLE_BASE + word_index * 4) as *const u32;
        let en = ptr::read_volatile(enable_addr);
        let thr = ptr::read_volatile(THRESHOLD as *const u32);
        let pending_addr = (PLIC_BASE + 0x1000 + word_index * 4) as *const u32;
        let pend = ptr::read_volatile(pending_addr);
        println!("  [plic] IRQ {} priority={} enable_word[{}]={:#010x} threshold={} pending={:#010x}",
            UART0_IRQ, pri, word_index, en, thr, pend);
    }
}
