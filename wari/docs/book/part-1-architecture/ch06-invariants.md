---
sidebar_position: 6
sidebar_label: "Ch 6: The Invariants"
title: "Chapter 6 — The Invariants"
---

# Chapter 6 — The Invariants

*Draft stub. Narrative wrapper around `docs/invariants.md` — why
invariants are first-class citizens, how they gate every unsafe block,
how they evolve across phases.*

## What this chapter covers

- Why an invariant catalog is the right formalism-staging artifact
- The Phase 0 invariants (INV-1 through INV-9) — each explained
- How a new unsafe block lands: identify the invariant, cite it,
  document it, test it
- The Phase 1 additions (INV-10, INV-11 — capabilities and signed
  loading)
- What happens when an invariant breaks (e.g., SMP lands and INV-1
  no longer holds) — the cross-reference + migration process
- How this feeds into Phase 3's formal verification: the invariants
  become proof obligations

## Closing hook

Ch 7 — where does all this go? The immutable endpoint.
