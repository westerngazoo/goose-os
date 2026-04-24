---
sidebar_position: 2
sidebar_label: "Ch 2: Shoulders We Stand On"
title: "Chapter 2 — The Shoulders We Stand On"
---

# Chapter 2 — The Shoulders We Stand On

*Draft stub. Narrative version of `docs/prior-art.md` — same content,
woven into prose with the book's voice.*

## What this chapter covers

The commercial landscape:
  - Cloudflare Workers and the density bet
  - Fastly Compute@Edge and the WASM boundary
  - AWS Lambda + Firecracker and narrow-purpose VMMs
  - AWS Nitro and HW/SW co-design
  - gVisor and the attack-surface argument
  - KataContainers and why OCI compat is a trap for us

The research inheritance:
  - seL4 and what "formally verified" actually bought
  - Singularity and the managed-code OS that was 20 years too early
  - Tock OS shipping Rust isolation in production
  - RedLeaf and the academic precedent closest to ours
  - MirageOS, Unikraft, Hubris as adjacent inspirations

The explicit rejections:
  - V8 / JavaScript
  - OCI compatibility
  - Syscall shimming
  - Proprietary silicon isolation (SGX lineage)

## Source material

- `docs/prior-art.md` — the structured citation table
- Reading list from that doc — each entry gets a paragraph of
  "what we took from it" in the book

## Closing hook

Points to Ch 3 — given all this context, why did we pick WASM-only?
