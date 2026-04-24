//! Memory subsystem — physical allocator, Sv39 page tables, kernel VM.
//!
//! Pure/impure split:
//!   - `page_alloc`  — pure bitmap allocator logic (host-testable)
//!   - `page_table`  — pure Sv39 data structures + walker (host-testable)
//!   - `kvm`         — impure glue: linker symbols, MMU enable, MMIO maps
//!
//! Cherry-picked from `goose-os/kernel/src/{page_alloc,page_table,kvm}.rs`.

pub mod kvm;
pub mod page_alloc;
pub mod page_table;
