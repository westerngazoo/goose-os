//! Bitmap-based physical page allocator.
//!
//! Pure logic: one `BitmapAllocator` struct with `alloc` / `free` /
//! `free_count` methods. No `unsafe`, no MMIO, no linker symbols in
//! this file — integration glue lives in `kvm.rs`.
//!
//! Invariant: bit `N` set ↔ page at `base + N * PAGE_SIZE` is allocated.
//! Conservation: `alloc_count() + free_count() == total_pages`.
//!
//! Cherry-pick source: `goose-os/kernel/src/page_alloc.rs` (484 LOC,
//! host-tested).
