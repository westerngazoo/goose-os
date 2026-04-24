//! Trap entry + syscall dispatch table.
//!
//! Cherry-picked template: `goose-os/kernel/src/trap.rs` (the Build-88
//! dispatch-table form, not the earlier match ladder).
//!
//! The dispatch table has one entry per `SYS_*` number in
//! `wari-abi/src/lib.rs`. Adding a syscall is always a 2-line change:
//! one const in `wari-abi`, one fn pointer here.
//!
//! Phase 0 scaffold: agent lands the `TrapFrame` struct, `handle_ecall`,
//! and the dispatch table in the first post-cherry-pick PR.
