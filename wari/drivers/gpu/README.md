# GPU driver — Phase 2

PCIe GPU driver, exposed to Tier-1 through `wari_ai_infer` host function
(WASI-NN surface). Driver owns the GPU MMIO + DMA rings; Tier-1 modules
never touch the device directly.

Scope to be finalized in the Phase 2 planning PR.
