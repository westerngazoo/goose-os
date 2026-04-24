---
sidebar_position: 4
sidebar_label: "Ch 4: Two Tiers"
title: "Chapter 4 — Two Tiers"
---

# Chapter 4 — Two Tiers

*Draft stub. The centerpiece architectural chapter. Contains the
main architecture diagram from `docs/architecture.md` with narrative.*

## What this chapter covers

- Why pure WASM-in-ring-0 for everything is tempting and wrong —
  defense-in-depth matters for untrusted customer code
- Why pure WASM-in-userspace for drivers is tempting and slow —
  trap overhead for every MMIO op is unacceptable
- The two-tier answer: customer WASM in U-mode (double-sandboxed),
  driver WASM in S-mode (WASM-only sandbox, signed)
- Capability-gated crossing between tiers
- Comparison with Singularity's SIPs, RedLeaf's domains, MirageOS's
  single-address-space

## Closing hook

Ch 5 — before we build this, what do we inherit from goose-os?
