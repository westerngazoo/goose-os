//! Wari-WASI — the host function surface exposed to Tier-1 WASM
//! modules.
//!
//! Two layers:
//!   - `preview1` — the subset of WASI Preview 1 we implement
//!   - `wari_ext` — Wari-specific extensions (crypto, AI, net, GAPU)
//!
//! This crate defines the *surface*: function signatures, module names,
//! error codes. The kernel crate provides the *implementations* in
//! `kernel/src/runtime/`. Keeping the surface separate lets host-side
//! tools (like `oci2wasm` in Phase 2) link against the same signatures
//! the kernel accepts.

#![no_std]
#![deny(missing_docs)]

pub mod preview1;
pub mod wari_ext;
