//! Scheduler policy + context switch.
//!
//! Cherry-pick source: `goose-os/kernel/src/sched.rs` (214 LOC).
//! Round-robin across `PROCS`, scan-from-current, O(MAX_PROCS) per
//! decision. Fine at MAX_PROCS=64; revisit if we ever need 1000+
//! Tier-1 instances per board (Phase 1 scaling target).
