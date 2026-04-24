//! Typed volatile MMIO wrappers — CLAUDE R3.
//!
//! Raw `ptr::read_volatile`/`write_volatile` is **banned outside this
//! module**. Drivers use `VolatilePtr<T>` or device-specific typed
//! register definitions.
//!
//! Why: compiler optimization + MMIO = silent correctness bugs. A
//! typed wrapper makes the intent explicit at every call site, and
//! a future lint or audit can verify R3 mechanically by checking
//! that no other file imports `core::ptr::{read,write}_volatile`.
//!
//! Phase 0a agent populates `volatile.rs` with the wrapper types.
//! Phase 1+ adds device-specific register struct layouts.

pub mod volatile;
