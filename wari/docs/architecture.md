# Wari — Architecture (living document)

> **Scope**: the current architecture. Not the vision (see
> `book/part-1-architecture/` for that), not the roadmap (see
> `../CLAUDE.md` §Roadmap). Only what's true *right now*.

**Status**: Phase 0 scaffold. Kernel surface declared; implementations
pending cherry-pick from `../goose-os/`.

---

## Component overview

```mermaid
graph TB
    subgraph T1["Tier 1 — Customer WASM  (U-mode, MMU + WASM sandbox)"]
        A1[app A<br/>.wasm]
        A2[app B<br/>.wasm]
        AN[... 50k instances target]
    end

    subgraph T2["Tier 2 — System WASM  (S-mode, WASM sandbox only)"]
        D1[uart driver<br/>.wasm signed]
        D2[net driver<br/>.wasm signed]
        D3[gpu / ai driver<br/>.wasm signed]
        D4[gapu driver<br/>.wasm signed]
    end

    subgraph T0["Tier 0 — Native Rust Kernel  (S-mode)"]
        K1[boot &middot; trap &middot; MMU]
        K2[wasmi runtime]
        K3[capability table]
        K4[IPC + host-fn dispatch]
        K5[scheduler]
    end

    subgraph HW["Hardware — JH7110 (Phase 0) / + GAPU FPGA (Phase 3)"]
        H1[U74 cores]
        H2[Sv39 MMU + PMP]
        H3[Zkn / Zks crypto]
        H4[CoVE confidential mem - P3]
        H5[PCIe &middot; GPU &middot; FPGA]
    end

    A1 -- "WASI host fn" --> K4
    A2 -- "WASI host fn" --> K4
    AN -- "WASI host fn" --> K4

    K4 -- "cap-gated IPC" --> D1
    K4 -- "cap-gated IPC" --> D2
    K4 -- "cap-gated IPC" --> D3
    K4 -- "cap-gated IPC" --> D4

    D1 -- "typed MMIO" --> H1
    D2 -- "typed MMIO" --> H1
    D3 -- "PCIe / MMIO" --> H5
    D4 -- "PCIe / MMIO" --> H5

    K1 -.-> H1
    K1 -.-> H2
    K2 -.-> K4
    K3 -.-> K4
    K5 -.-> K1
```

## Control flow — Tier-1 syscall

A Tier-1 app calls `fd_write(stdout, "Hello")`; this is the full path
through the system.

```mermaid
sequenceDiagram
    autonumber
    participant App as Tier-1 app (.wasm)
    participant WR as wasmi runtime (Tier 0)
    participant K as Kernel dispatch
    participant D as Tier-2 UART driver (.wasm)
    participant HW as UART MMIO

    App->>WR: fd_write(1, buf, len)
    WR->>K: host_fn_fd_write(stdout, buf, len)
    K->>K: validate caller's stdout cap
    K->>D: IPC CALL (write, buf_copy, len)
    D->>D: wasmi executes driver module
    D->>HW: typed volatile store to THR
    HW-->>D: (bytes out the wire)
    D-->>K: IPC REPLY (bytes_written)
    K-->>WR: return n
    WR-->>App: a0 = n
```

Two WASM sandbox crossings (Tier 1 → Tier 2), two kernel dispatches,
zero process-level context switches. Every crossing is capability-gated.

## State at Phase 0

| Subsystem        | Status | Source                                                    |
|------------------|--------|-----------------------------------------------------------|
| Workspace layout | Done   | This scaffold                                             |
| ABI (syscalls/errors) | Template | Phase 0a cherry-pick from `goose-os/kernel/src/abi.rs` |
| Tier 0 memory    | Scaffold | Cherry-pick from `goose-os/kernel/src/{page_alloc,page_table,kvm}.rs` |
| Tier 0 scheduler | Scaffold | Cherry-pick from `goose-os/kernel/src/{process,sched}.rs` |
| Tier 0 IPC       | Scaffold | Cherry-pick from `goose-os/kernel/src/ipc.rs`             |
| Tier 0 trap      | Scaffold | Cherry-pick from `goose-os/kernel/src/trap.rs` (dispatch-table form) |
| Typed MMIO (R3)  | Scaffold | New in Phase 0a — `mmio/volatile.rs`                      |
| wasmi embedding  | Not started | Phase 0b                                                 |
| WASI host fns    | Not started | Phase 0b                                                 |
| Tier 1 hello     | Scaffold | Phase 0c                                                 |
| Capability system | Not started | Phase 1a                                                 |
| Tier-2 drivers   | Placeholders | Phase 1b–d                                             |

## Open questions — resolve before leaving Phase 0

1. **wasmi pinned version + feature set.** Phase 0b proposal PR.
2. **How do `.wasm` bundles get signed and verified at boot?** Proposal
   PR before Phase 0c.
3. **PID allocation policy for Tier-1 vs Tier-2.** Currently assumed:
   PID 1 = first Tier-1, PID 2+ = Tier-1 pool, PIDs from 16 up are
   Tier-2 drivers. To be confirmed in Phase 0 closeout.

See `book/part-1-architecture/` for the narrative derivation of this
architecture and why it looks like this.
