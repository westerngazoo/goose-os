//! Staged boot sequence — kernel entry to first Tier-1 WASM scheduled.
//!
//! Each stage is a standalone function with documented pre- and
//! post-conditions. Reading this file top-to-bottom gives a flat
//! table of contents for the boot sequence.
//!
//! Cherry-picked template: `goose-os/kernel/src/boot.rs`. Phase 0a
//! agent populates the stages in dependency order:
//!
//!   1. `stage_uart`       — early console (allow panics to print)
//!   2. `stage_banner`     — Wari banner, hart id, build info
//!   3. `stage_interrupts` — trap vector, PLIC, timer, SIE on
//!   4. `stage_memory`     — physical page allocator + self-test
//!   5. `stage_mmu`        — Sv39 page tables + `csrw satp`
//!   6. `stage_runtime`    — wasmi embedding + WASI host fn table
//!   7. `stage_tier1_init` — load signed .wasm, spawn as PID 1
//!
//! Staging rule: a stage may not depend on a stage below it. Stage 7
//! never returns — it hands to the scheduler.

// Phase 0 scaffold: no function bodies yet. See `docs/pr-workflow.md`
// for how a cherry-pick PR lands these.
