# Wari — Tier 2 Drivers

Each subdirectory is a **WASM module** that the kernel loads as a
Tier-2 driver. Drivers have S-mode privilege (direct MMIO + IRQ),
are isolated from each other only by the WASM type system, and are
signed + attested before load.

Populated by phase:

| Driver    | Phase | Responsibility                                          |
|-----------|-------|---------------------------------------------------------|
| `uart/`   | 1     | NS16550A-compatible UART — console TX/RX               |
| `net/`    | 1     | VirtIO-net (QEMU) + dwmac (VF2) — Ethernet MAC         |
| `gpu/`    | 2     | PCIe GPU — neural-net inference via WASI-NN            |
| `gapu/`   | 3     | Custom FPGA coprocessor over PCIe                      |

**Rule**: a driver is added only via a green-lit PR that includes:
  - the WASM build chain config,
  - a host-side test harness with at least one mocked-MMIO scenario,
  - an adversarial security test (`tests/security/`) exercising the
    tier boundary between Tier-1 and this driver,
  - sign-off against the relevant INV-N invariants.

See `../CLAUDE.md` PR workflow.
