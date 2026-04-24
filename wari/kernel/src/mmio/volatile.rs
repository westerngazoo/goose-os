//! `VolatilePtr<T>` and `VolatileRef<T>` — typed volatile MMIO access.
//!
//! Phase 0a: agent implements these, cherry-picking inspiration from
//! the `volatile` crate (BSD-licensed, small, auditable) without
//! adding the dependency — we want every line in-tree.
//!
//! Target API (proposal for the cherry-pick PR):
//!
//! ```ignore
//! // Reading a 32-bit register:
//! let uart_lsr: VolatilePtr<u32> =
//!     unsafe { VolatilePtr::new(0x1000_0005 as *mut u32) };
//! let lsr = uart_lsr.read();
//!
//! // Writing:
//! uart_lsr.write(0x01);
//!
//! // Device-specific typed register group:
//! #[repr(C)]
//! struct UartRegs { rbr: u32, ier: u32, /* ... */ }
//! let uart: VolatileRef<UartRegs> = unsafe { VolatileRef::new(0x1000_0000) };
//! let byte = uart.field(|u| &u.rbr).read() as u8;
//! ```
//!
//! The wrapper carries no runtime cost over raw volatile ops; the
//! value is in making the unsafe boundary a one-line wrapper, not
//! scattered across drivers.
