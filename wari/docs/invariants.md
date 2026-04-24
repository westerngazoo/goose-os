# Wari — Invariants (INV-N catalog)

> This document is the **source of truth** for what makes Wari's unsafe
> code sound. Every `unsafe` block in the kernel carries a
> `// SAFETY: INV-N` comment citing an invariant below. When an
> invariant is violated (e.g., SMP lands and INV-1 changes), every
> citing site needs revisiting.

Format inherited from `../goose-os/docs/unsafe-audit.md`.

---

## Loaded-bearing invariants (Phase 0 baseline)

### INV-1 · Single-Hart Kernel

> Only one hart executes kernel code at a time. Interrupts are disabled
> on entry to the trap vector and not re-enabled until sret back to
> userspace.

**Consequence**: `static mut` access without synchronization is sound
for scheduler-owned state (`PROCS`, `CURRENT_PID`, `TICKS`, etc.).

**When this breaks**: SMP. Every INV-1 citation needs per-hart or
locked access.

### INV-2 · Trap Frame Exclusivity

> While a syscall handler runs, the current hart owns the `TrapFrame`
> it was handed. No other code path touches it until sret.

**Consequence**: `&mut TrapFrame` parameters in syscall handlers do
not alias.

**When this breaks**: nested interrupts (reentrant traps). Prevented
by SIE=0 during S-mode trap service.

### INV-3 · MMIO Address Validity

> Hardcoded MMIO bases are fixed by hardware spec. Writes/reads to
> these addresses are hardware register operations, not arbitrary
> memory access.

**Consequence**: `VolatilePtr`/`VolatileRef` wrapping of fixed MMIO
addresses is sound.

**When this breaks**: porting to a different SoC layout. MMIO bases
move behind `platform::` module.

### INV-4 · Linker Symbol Addresses Are Valid

> Linker script exports symbols (`_end`, `_heap_end`, etc.) whose
> addresses are bound at link time. Taking `&X as *const u8 as usize`
> yields that address.

**Consequence**: reading linker symbol addresses is sound; no deref.

**When this breaks**: linker script renames or symbol-stripping builds.
CI asserts the symbols exist in the final binary.

### INV-5 · Page Allocator Returns Kernel-Writable PAs

> `BitmapAllocator::alloc()` returns a PA in the range `[_end,
> _heap_end)`. The kernel identity-maps this entire range RW.

**Consequence**: writes through allocator-returned PAs don't clobber
kernel text.

### INV-6 · Page-Table Walker Returns Installed PAs

> `page_table::walk(root, va, cb)` invokes the callback only when VA
> resolves to a present leaf PTE whose PA was installed via validated
> mapping.

**Consequence**: callbacks receive PAs owned by the caller's process.

### INV-7 · Privileged ASM Is Privileged

> Inline assembly touching CSRs, `sret`, `ecall`, `wfi`, `sfence.vma`
> is sound because the kernel executes in S-mode.

**Consequence**: unsafe-block reason is "Rust requires `unsafe` around
asm"; the instruction itself is permitted at this privilege level.

### INV-8 · Static-Mut Singleton Accessors Are Called Post-Init

> `page_alloc::get()`, `runtime::get()`, driver accessors return
> `&'static mut` to statics initialized once in boot. Callers obtain
> these references only after the corresponding `init()` has run.

**Consequence**: returned references are to initialized state.

### INV-9 · Bytewise Struct Reinterpretation Is Bounds-Checked

> Reinterpreting `&[u8]` as a `&StructT` is preceded by a length check
> (`slice.len() >= size_of::<StructT>()`) AND alignment verification
> (or `read_unaligned`).

**Consequence**: struct reads don't extend past the slice, don't cause
unaligned access faults.

**Open**: goose-os followed this for length but not alignment — see
`../goose-os/docs/unsafe-audit.md` follow-up #1. Wari cherry-picks
with the alignment fix.

---

## Phase-1 invariants (added when capability system lands)

### INV-10 · Capability Monotonicity *(Phase 1)*

> A process's capability table is append-only within a single IPC
> call. Capabilities are revoked only by explicit `SYS_CAP_REVOKE`,
> never implicitly.

### INV-11 · Tier-2 Grants Are Signed *(Phase 1)*

> A Tier-2 module is loaded only with a matching signature on its
> manifest. The signature is verified against a compiled-in public key
> before any bytecode executes.

---

## Per-file sites

*(Populated as the kernel is cherry-picked.)*

| File                        | Site | Invariant | Rationale |
|-----------------------------|------|-----------|-----------|
| `kernel/src/main.rs:45`     | `wfi` in placeholder loop | INV-7 | S-mode WFI |
| `kernel/src/main.rs:57`     | `wfi` in panic loop       | INV-7 | S-mode WFI |

---

## Enforcement

- `cargo clippy -- -D warnings` with `undocumented_unsafe_blocks = "warn"`
- Every PR that adds `unsafe` must update this file (CLAUDE §PR Workflow)
- Phase gate audits cross-check: for every `unsafe` in the codebase,
  is there a matching row in this file?

---

*Last audited: Phase 0 scaffold, April 2026.*
