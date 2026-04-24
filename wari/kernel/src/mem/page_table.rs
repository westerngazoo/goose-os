//! Sv39 page table structures — pure, host-testable.
//!
//! 3-level page table: 9+9+9+12 bits of a 39-bit VA.
//! `walk(root, va, read_closure)` walks the tree without dereferencing
//! anything — the read closure resolves PTE slots to u64 values, so
//! the same function works against real memory (kernel) and fake
//! memory (host tests).
//!
//! No `unsafe` in this file. Hardware integration lives in `kvm.rs`.
//!
//! Cherry-pick source: `goose-os/kernel/src/page_table.rs` (~680 LOC,
//! host-tested including the `walk()` test harness from Build 101).
