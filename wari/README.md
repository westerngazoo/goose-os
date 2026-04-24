# Wari

A world-class, formally-verifiable, WASM-native operating system for
RISC-V. Targeting sovereign cloud infrastructure in Latin America.

**Status:** Phase 0 kickoff. Foundation being cherry-picked from
`../goose-os/`.

## What makes Wari different

- **WASM-only process model.** No ELF in the customer ABI, ever.
- **Two-tier sandbox.** Customer code (Tier 1, MMU + WASM) and drivers
  (Tier 2, WASM-only) are both WASM modules, executed with different
  privilege levels via capability grants.
- **Tiny native kernel.** Tier 0 is ~5–10 KLOC of Rust, formal-
  verification-scale.
- **LATAM sovereignty.** Open hardware (RISC-V) + open drivers
  (auditable `.wasm`) + confidential computing (CoVE, Phase 3) +
  custom silicon (GAPU FPGA, Phase 3).

See `CLAUDE.md` for the full architectural specification and
co-architect protocol. See `docs/book/` for the narrative derivation.

## Getting started

This project is Phase 0 scaffold. The execution agent populates it
per the roadmap. If you're here to contribute, read in this order:

1. `CLAUDE.md` — rules, invariants, phases
2. `docs/architecture.md` — current architecture
3. `docs/prior-art.md` — what we inherit and what we reject
4. `docs/invariants.md` — the `INV-N` catalog
5. `docs/pr-workflow.md` — how to propose a change
6. `docs/testing.md` — test layers + adversarial coverage
7. `docs/security-model.md` — threat model
8. `docs/book/` — Wari, Volume 2

## License

TBD before first external release.
