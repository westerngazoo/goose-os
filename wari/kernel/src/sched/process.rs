//! Process Control Block + process table globals + trivial accessors.
//!
//! Cherry-pick source: `goose-os/kernel/src/process.rs` (168 LOC
//! after Build 98 split). The PCB struct gains fields across phases:
//!   - Phase 0: IPC state, basic lifecycle
//!   - Phase 1: capability table index
//!   - Phase 2: per-process fuel budget for WASM
//!
//! Every addition to the PCB is an explicit PR; fields don't accrete
//! silently (CLAUDE §Co-Architect).
