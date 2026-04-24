//! Wari QEMU runner — deterministic integration-test harness.
//!
//! Launches QEMU with a kernel binary + optional WASM blob, captures
//! UART output, asserts against expected markers (PASS / FAIL / regex
//! match). Used by `tests/integration/*` and `tests/security/*`.
//!
//! Phase 0b: the first integration test's PR introduces this runner.
//! Today: placeholder `main` so the crate compiles.

fn main() {
    println!("wari-qemu-runner — scaffold placeholder");
}
