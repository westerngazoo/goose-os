//! WASM runtime embedding — Tier-0's interface to wasmi.
//!
//! This is the kernel's hosting of the wasmi interpreter plus the
//! WASI host-function dispatch table. Load a `.wasm`, validate it,
//! instantiate it as a Tier-1 or Tier-2 process, route its host-fn
//! calls to the appropriate kernel services.
//!
//! Phase 0 scope:
//!   - wasmi embedding (pin version in Cargo.toml)
//!   - Load + validate `.wasm` from kernel memory (no filesystem yet)
//!   - WASI Preview 1 subset: fd_write, fd_read, proc_exit,
//!     clock_time_get, random_get
//!   - Spawn as Tier-1 (U-mode) with MMU page table
//!
//! Phase 1 scope:
//!   - Capability-gated host function access
//!   - Tier-2 driver load path (S-mode, no MMU barrier)
//!   - Module attestation + signing verification before load
//!
//! Phase 2 scope:
//!   - WASI-NN (wari_ai_infer)
//!   - Fuel metering per invocation (wasmi's built-in)
//!   - Docker-to-WASM compatibility surface

pub mod wasmi_host;
