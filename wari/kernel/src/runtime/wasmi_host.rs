//! wasmi embedding — `Engine`, `Store`, `Linker` wiring.
//!
//! Phase 0a PR introduces `wasmi = "0.x"` in `wari/kernel/Cargo.toml`
//! (exact version to be proposed in the PR — pinned, not a caret
//! range, per CLAUDE R8). The agent proposes which wasmi features
//! are enabled and what's turned off; Gustavo approves before the
//! dependency lands.
//!
//! Design constraint: **no `alloc`**. wasmi's `no_std` mode requires
//! careful feature selection — the agent's first PR body must show
//! which wasmi features are on, off, and why.
