# Platform — StarFive VisionFive 2

This directory holds firmware blobs required to boot Wari on the
VisionFive 2 (JH7110 SoC, 4× SiFive U74). Imported from `../goose-os/`:

  - `u-boot-spl.bin.normal.out`  — Secondary Program Loader (SPL)
  - `fw_payload.img`             — OpenSBI firmware (M-mode)

The kernel image produced by `cargo build --release -p wari-kernel`
is loaded by U-Boot after OpenSBI. See `../../scripts/` for the
device-side update script (Phase 1a port from goose-os).
