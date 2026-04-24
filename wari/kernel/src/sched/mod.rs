//! Scheduler + process table.
//!
//! Split by concern (learned from goose-os's Debt-3 refactor, Build 98):
//!   - `process`   — PCB struct, `PROCS` / `CURRENT_PID` globals, accessors
//!   - `scheduler` — scheduling policy (round-robin), context switch,
//!                    preempt, `schedule_from_idle`, `sys_yield`
//!
//! Lifecycle (spawn/exit/wait) is Phase 0+1 material and moves here
//! once the ELF lifecycle is retired and WASM lifecycle is in place.

pub mod process;
pub mod scheduler;
