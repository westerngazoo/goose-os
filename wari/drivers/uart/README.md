# UART driver — Phase 1

Target: NS16550A-compatible UART at platform-specific MMIO base.

Reference implementation: `../../../goose-os/kernel/src/uart.rs` (native
Rust). Phase 1 task is to port to WASM + host functions that expose
just enough MMIO surface for the driver to work.
