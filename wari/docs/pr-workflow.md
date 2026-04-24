# Wari — PR Workflow

> Every change lands via PR. Gustavo reviews locally, tests locally,
> merges. Claude never merges. Expanded form of `../CLAUDE.md`
> §PR Workflow.

---

## The loop

```mermaid
flowchart LR
    P[Propose] --> A[Approve]
    A --> B[Branch]
    B --> I[Implement]
    I --> T[Test locally]
    T --> PR[Push &amp; open PR]
    PR --> R[Gustavo reviews]
    R --> L[Gustavo pulls &amp; tests]
    L --> D{decision}
    D -->|approve| M[Merge squash]
    D -->|change| I
    M --> N[Next PR]

    style P fill:#2b6cb0,stroke:#90cdf4,color:#fff
    style A fill:#2f855a,stroke:#9ae6b4,color:#fff
    style M fill:#2f855a,stroke:#9ae6b4,color:#fff
    style D fill:#c53030,stroke:#feb2b2,color:#fff
```

---

## Branch naming

```
phase-<N>/<subsystem>-<kebab-case-summary>
```

Examples:
  - `phase-0/kernel-cherry-pick-page-alloc`
  - `phase-0/runtime-wasmi-embedding`
  - `phase-1/drivers-uart-wasm-skeleton`

---

## PR size discipline

- **Preferred**: 100–400 changed lines. One conceptual change.
- **Acceptable**: ~800 lines if atomic.
- **Requires pre-approval** (propose split): beyond that.

Cherry-picking a whole goose-os file counts as one conceptual change.

---

## PR title

```
<phase>: <subsystem>: <imperative summary>
```

Examples:
  - `phase-0: mem: cherry-pick bitmap allocator`
  - `phase-0: runtime: pin wasmi 0.32 with no_std features`
  - `phase-1: cap: introduce per-process capability table`

---

## PR body — template (mandatory)

Exact sections, exact order. Empty sections are filled with `None —
<reason>`.

```markdown
## What
<one paragraph>

## Why
<phase + exit-criterion ref; link to book chapter or architecture
section if applicable>

## How
<2–5 bullets on approach, modules touched, trade-offs>

## Invariants affected
<INV-N citations; new invariants land in docs/invariants.md in this
PR>

## Security considerations
- Attack surface change: <where>
- Trust-boundary crossing: <which, gated by which capability>
- New host functions to Tier-1: <list>
- Assumptions about caller trust: <explicit list>

## Tests
- Unit: <files>
- Integration (QEMU): <files>
- Security (adversarial): <files>
- Fuzz: <targets>

## Local verification
```
<exact commands + expected output>
```

## Out of scope
<what this PR does not do>

## Rollback
<how to revert>
```

---

## Reviewer checklist (Gustavo runs through)

- [ ] R1: every new `unsafe` cites INV-N in SAFETY comment
- [ ] R2: no heap alloc in trap/dispatch paths
- [ ] R3: no raw volatile outside `kernel/src/mmio/`
- [ ] R4: public APIs have contracts
- [ ] R5: no `unwrap`/`expect` in syscall paths
- [ ] R6: memory barriers documented
- [ ] R7: no ELF path added
- [ ] R8: `Cargo.lock` intact, toolchain not silently bumped
- [ ] Security considerations section is thoughtful (not boilerplate)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test --workspace` passes (host)
- [ ] QEMU integration passes (if applicable)
- [ ] `docs/invariants.md` updated if new unsafe landed
- [ ] PR size within discipline
- [ ] Local-verification commands reproduce claimed output

---

## Merge strategy

**Squash merge.** PR body becomes the commit message on `main`.
Branch-local commits are scratch.

## Build numbering

Continue monotonic numbering from goose-os. Each squash-merge commit
bumps `.build_number` and carries the number in the first line of the
commit message.

## Branch hygiene

- Delete branches after merge.
- Don't force-push a branch after Gustavo has started reviewing unless
  he says so.
- `main` is protected; direct pushes blocked.
