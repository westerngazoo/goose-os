# Net driver — Phase 1

Targets:
  - QEMU: VirtIO-net-device (MMIO v2)
  - VF2:  JH7110 dwmac (Phase 1c)

Reference: `../../../goose-os/kernel/src/{virtio,net}.rs`. Includes the
smoltcp integration as a WASM dependency — smoltcp is `no_std` and
compiles to wasm32 cleanly (Phase 1 proposal PR will confirm).
