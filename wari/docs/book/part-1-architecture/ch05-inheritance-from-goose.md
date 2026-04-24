---
sidebar_position: 5
sidebar_label: "Ch 5: Inheritance from Goose"
title: "Chapter 5 — Inheritance from Goose"
---

# Chapter 5 — Inheritance from Goose

*Draft stub. The cherry-pick audit as a narrative — what survives,
what is rewritten, what is retired, why each choice.*

## What this chapter covers

### What we keep (verbatim or near-verbatim)
- `page_alloc.rs` — pure bitmap allocator
- `page_table.rs` — pure Sv39 walker + data structures
- `kvm.rs` — impure MMU glue
- `ipc.rs` — synchronous rendezvous
- `process.rs` + `sched.rs` — after the Debt-3 split
- `boot.rs` — staged boot with pre/post conditions
- `trap.rs` — dispatch-table form (Build 88)
- `abi.rs` — syscall numbers, typed errors (extracted to `abi-shared`)
- `security.rs` → renamed `validate.rs` — pure validators
- `unsafe-audit.md` → `invariants.md` — INV-N framework

### What we rewrite
- UART / PLIC / VirtIO drivers — retire native, port as Tier-2 WASM
- `syscall.rs::sys_spawn` — from ELF to WASM module loader

### What we delete
- `elf.rs` — ELF loader (R7 violation)
- `wasm.rs` + `interp.rs` + `wasi.rs` — hand-rolled interpreter
  replaced by wasmi (3,556 LOC of TCB debt retired)
- Native user programs (`_user_init`, `_uart_server` asm blocks)

### The retire rationale
- Why replacing ~4,000 LOC is a net win for the TCB story
- What we lose in short-term progress vs. long-term correctness

## Closing hook

Ch 6 — how do we keep it honest? The invariants.
