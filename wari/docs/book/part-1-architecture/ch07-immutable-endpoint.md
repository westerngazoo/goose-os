---
sidebar_position: 7
sidebar_label: "Ch 7: The Immutable Endpoint"
title: "Chapter 7 — The Immutable Endpoint"
---

# Chapter 7 — The Immutable Endpoint

*Draft stub. The Phase 4 vision — where the architecture logically
leads if every prior phase succeeds.*

## What this chapter covers

### The four properties of an immutable kernel
1. Functionally pure state transitions (FC-IS pattern, Rust-friendly)
2. Hash-attested boot (equivalent to Secure Boot but open + small)
3. No self-modification (kernel text RO, no JIT, no dynamic loading)
4. Burnable to mask ROM (final endpoint on custom silicon)

### Singularity's dream, 20 years later
- What Singularity proved and what it couldn't ship
- Why WASM + wasmi is the 2026 enabler that CLR wasn't in 2003
- Tock OS as proof this works in production-scale embedded
- RedLeaf as the academic path from general-purpose to specialized

### The custom-SoC implications
- What "MMU-free variant" means concretely
- Why the MMU can become defense-in-depth rather than primary
- The verification obligations: formal `wasmi` + formal Tier 0
- The attestation chain: ROM hash → kernel → driver signatures →
  module attestation

### What this means for Phase 0 decisions
- Why every Phase 0 module avoids future-locking patterns
- Which Phase 0 invariants directly transfer to the MMU-free model
- The architectural commitments that make Phase 4 possible

## Closing hook

End of Part 1. Part 2+ is the build log.
