//! Kernel Virtual Memory — impure glue between `page_alloc`,
//! `page_table`, and the hardware MMU.
//!
//! This is the ONLY module that writes to page table memory or touches
//! `satp`. Everything above it uses typed helpers; everything below it
//! is pure data.
//!
//! Cherry-pick source: `goose-os/kernel/src/kvm.rs` (352 LOC). Phase 0a
//! agent adds `// SAFETY: INV-N` comments to every unsafe block during
//! the pick, per CLAUDE R1.
