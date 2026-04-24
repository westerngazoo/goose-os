# GAPU driver — Phase 3

Custom FPGA coprocessor over PCIe. The sovereign-AI differentiator:
customer inference runs on open silicon, no Nvidia, no proprietary
firmware blobs.

Architecture parallels the GPU driver (both are PCIe endpoints behind
a Tier-2 driver WASM). Differences:
  - Custom MMIO register map (we own the RTL)
  - Model formats tuned to FPGA bitstreams, not CUDA kernels
  - Attestation chain includes the FPGA bitstream hash alongside the
    driver .wasm hash

Phase 3 scope finalized in the dedicated planning PR.
