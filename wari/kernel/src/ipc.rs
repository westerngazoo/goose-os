//! Synchronous rendezvous IPC — seL4 pattern.
//!
//! Cherry-picked from `goose-os/kernel/src/ipc.rs` (~250 LOC, clean).
//! Minimal edits expected:
//!   - `schedule(...)` call now routes to `crate::sched::schedule`
//!   - `CURRENT_PID` / `PROCS` imports adjust to `crate::sched::*`
//!   - Error returns migrate to `KernelError` (CLAUDE R5)
//!
//! Phase 1 capability work adds one argument to each operation:
//! the capability index instead of a raw PID. See Phase 1 plan in
//! `CLAUDE.md` roadmap.
