---
sidebar_position: 3
sidebar_label: "Ch 3: The WASM-Only Bet"
title: "Chapter 3 — The WASM-Only Bet"
---

# Chapter 3 — The WASM-Only Bet

*Draft stub. The "ELF retires" argument made concretely, with the
Cloudflare/Fastly density-vs-cold-start comparison as evidence.*

## What this chapter covers

- The comparison table (density × cold-start × cold-TCB) for every
  major cloud compute primitive
- Why "the only way to beat Cloudflare on density is to not be Linux"
- The Docker-to-WASM escape hatch (`tools/oci2wasm/`, Phase 2) — how
  compatibility happens without compromise
- What we give up: customers cannot bring arbitrary Docker images
- What we get: 10k–50k tenants per board, μs cold start, auditable
  stack, sovereignty story that's defensible

## Closing hook

Ch 4 — given WASM-only, where do drivers live?
