# GooseOS Unsafe Audit

**Status:** Pass 1 — catalog + invariants. Inline `// SAFETY:` comments being added file-by-file.
**Scope:** `kernel/src/*.rs`. Userspace `unsafe` is out of scope (different TCB).
**Count:** 144 unsafe sites across 15 files.

The kernel's TCB claim rests on these unsafe blocks being sound. This document exists so that claim can be *audited*, not just asserted. Every unsafe site in the kernel is mapped to one of the invariants below. When an invariant is violated (e.g. SMP lands and INV-1 no longer holds), every site that depends on it needs revisiting.

## Invariants

These are the assumptions that make unsafe code in GooseOS sound. Each is load-bearing. Each site in the per-file catalog cites the invariant(s) it depends on.

### INV-1 · Single-Hart Kernel

> Only one hart executes kernel code at a time. Interrupts are disabled on entry to the trap vector (`trap.S`) and not re-enabled until sret back to userspace.

**Consequence:** `static mut` access without synchronization is sound. No hart can race another on `PROCS`, `CURRENT_PID`, `TICKS`, `ALLOC`, `NET_*`, `VIRTIO_NET_*`, `IRQ_OWNER`.

**When this breaks:** SMP. Every INV-1 site needs per-hart or locked access when a second core starts. Audit tag: search `INV-1` in this file.

### INV-2 · Trap Frame Exclusivity

> While a syscall handler runs, the current hart owns the `TrapFrame` it was handed. No other code path touches it until sret.

**Consequence:** `&mut TrapFrame` parameters in `ipc::sys_*`, `syscall::sys_*`, and `trap::handle_*` do not alias. The frame was pushed by `trap.S`, lives on the kernel stack for this trap, and is consumed exactly once.

**When this breaks:** Reentrant traps (nested interrupts). Currently prevented by SIE=0 during S-mode trap service.

### INV-3 · MMIO Address Validity

> Hardcoded MMIO bases are fixed by hardware spec: UART `0x1000_0000` (NS16550A on QEMU virt; identical on VF2), PLIC `0x0C00_0000`, VirtIO MMIO slots `0x1000_1000`–`0x1000_8000`.

**Consequence:** `ptr::read_volatile(ADDR as *const u32)` and its write twin are sound — the address resolves to a device register, not random memory.

**When this breaks:** Porting to a SoC with a different memory map (Ch 46's dwmac will add new bases at `0x1603_0000` / `0x1604_0000`). Addresses must move behind a `platform::` module.

### INV-4 · Linker Symbol Address Validity

> Linker script exports symbols `_end`, `_heap_end`, `_user_init_start`, `_user_init_end`, `_uart_server_start`, `_uart_server_end`, `_security_test_start`, `_security_test_end`. Each is an address at link time; taking `&X as *const u8 as usize` yields that address.

**Consequence:** `unsafe { &_end as *const u8 as usize }` is sound — no dereference, just address-of. Rust requires `unsafe` because the symbol is `extern "C"`, not because anything hazardous happens.

**When this breaks:** `linker.ld` renames, or a linker flag strips symbols. CI should assert these symbols exist.

### INV-5 · Page Allocator Returns Kernel-Writable PAs

> `BitmapAllocator::alloc()` returns a physical address in the range `[_end, _heap_end)` identified at boot. The kernel identity-maps this entire range with RW permissions.

**Consequence:** Writes through `alloc()`-returned PAs (e.g. `zero_page`, PTE writes in `kvm::map_range`) do not clobber kernel code/text, do not page-fault, do not alias other processes' data.

**When this breaks:** If allocator bounds are wrong (off-by-one, mis-parsed linker symbol), allocations could land in kernel `.text`. Defense: allocator is initialized from linker symbols and unit-tested in `page_alloc` tests.

### INV-6 · Page Table Walker Returns Installed PAs

> `page_table::walk(root, va, callback)` invokes its callback only when `va` resolves to a present leaf PTE. The PA passed to the callback was installed via `kvm::map_range` or `syscall::sys_map`, both of which validate addresses through `security::is_user_va` or come from trusted boot-time mapping.

**Consequence:** Dereferencing the callback's `pa` inside `kvm::copy_to_user` / `copy_from_user` accesses memory owned by the caller's process, not kernel internals or other processes.

**When this breaks:** A bug in the walker (returning non-leaf PAs), or a bypass of `is_user_va` checks in future syscalls. `security.rs` has unit tests for the checker; the walker needs test coverage.

### INV-7 · Privileged ASM Is Privileged

> Inline assembly touching CSRs (`satp`, `sstatus`, `sepc`, `sie`, `sip`), `sret`, `ecall`, `wfi`, and `sfence.vma` is sound because the kernel executes in S-mode. U-mode processes trap on these instructions.

**Consequence:** `asm!("csrc sstatus, ...")` etc. are sound in kernel code. The `unsafe` is Rust's — the instruction itself is permitted at the current privilege level.

**When this breaks:** Never in kernel code. Would break if kernel code ran in M-mode or U-mode (it doesn't).

### INV-8 · Static-Mut Singleton Accessors Are Called Post-Init

> `page_alloc::get()`, `virtio::get()` return `&'static mut` to statics initialized once in boot. Callers obtain these references only after `page_alloc::init()` / `virtio::init()` have run.

**Consequence:** The returned reference is to initialized state, not zero'd uninitialized memory.

**When this breaks:** Calling `get()` from ctor-like code that runs before boot init. Defense: `get()` is only called from syscall handlers and boot code *after* `init()`. Grep enforces this.

### INV-9 · Bytewise Struct Reinterpretation Is Bounds-Checked

> Reinterpreting `&[u8]` as `&Elf64Header` or `&Elf64Phdr` is preceded by a length check ensuring the slice is at least `size_of::<T>()` bytes.

**Consequence:** The read does not extend past the slice.

**When this breaks:** If alignment isn't checked. `Elf64Header` has alignment 8; byte slices from `include_bytes!` are aligned to 1. This is a latent bug — we rely on LLVM generating byte-wise loads for `#[repr(C, packed)]`-like access. Status: **requires follow-up** — should be changed to explicit byte-parsing or `read_unaligned`.

## Per-File Catalog

### `console.rs` — 3 sites (MMIO read)

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 51 | `unsafe { asm!("csrrs ...") }` for sstatus read | INV-7 | Reading S-mode CSR in S-mode |
| 112 | `ptr::read_volatile(base+i as *const u8)` | INV-3 | Reading UART RBR/THR MMIO |
| 123 | `ptr::read_volatile(base+i as *const u8)` | INV-3 | Same, second context |

### `elf.rs` — 2 sites (struct cast)

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 107 | `&*(data.as_ptr() as *const Elf64Header)` | INV-9 | Caller checks `data.len() >= 64` before call |
| 146 | `&*(data.as_ptr().add(offset) as *const Elf64Phdr)` | INV-9 | Caller validates `offset + 56 <= data.len()` |

**Follow-up:** alignment check missing — see INV-9 caveat. Track as issue.

### `ipc.rs` — 9 sites (static mut + schedule)

All sites access `CURRENT_PID` or `PROCS[pid]`. All sound under **INV-1 + INV-2**.

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 34, 86, 127, 177 | `unsafe { CURRENT_PID }` | INV-1 | Reading scheduler-owned static |
| 44, 96, 137, 180 | `unsafe { PROCS[target].state = ... }` | INV-1 | Rendezvous delivery mutation |
| 242 | `unsafe { schedule(frame, current) }` | INV-1 + INV-2 | Scheduler owns PROCS; frame is caller-owned |

### `kvm.rs` — 10 sites (satp + PTE + walker)

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 29 | `unsafe { KERNEL_SATP }` | INV-1 | Read-only after boot init |
| 119 | `pub unsafe fn enable_mmu` | INV-7 | Already has `# Safety` doc |
| 210 | `ptr::read_volatile(addr as *const u64)` for PTE | INV-5 | PA from allocator or linker-identity range |
| 218 | `ptr::write_volatile(addr as *mut u64, pte.bits())` | INV-5 | Same |
| 223 | `unsafe { page_alloc::get() }` | INV-8 | Post-init |
| 225 | `unsafe { BitmapAllocator::zero_page(page) }` | INV-5 | PA just allocated |
| 330, 377 | `walk(root, va, &#124;pa&#124; unsafe { ... })` | INV-6 | Walker guarantees leaf PTE |
| 358, 386, 413 | `unsafe { ... copy_to_user / copy_from_user }` | INV-6 | Callback receives walker-validated PA |

### `main.rs` — 6 sites (boot init)

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 76 | `unsafe { asm!("csrw stvec, {}", ...) }` | INV-7 | Installing trap vector at boot |
| 140 | `unsafe { kvm::enable_mmu(root_pt) }` | INV-7 | Boot sequence |
| 142 | `unsafe { page_alloc::get() }` | INV-8 | Post-init |
| 155 | `unsafe { virtio::get().mac_address() }` | INV-8 | Post-init, `net` feature only |
| 210 | `unsafe { asm!("csrc sstatus, ...") }` | INV-7 | Disable S-mode interrupts |
| 234 | `unsafe { asm!("wfi") }` | INV-7 | Wait-for-interrupt |

### `net.rs` — 21 sites (smoltcp + static muts)

All sites access `NET_DEVICE`, `NET_IFACE`, `NET_SOCKETS`, `NET_READY`, `TCP_HANDLES`, `UDP_HANDLES`, `STAGING`, or call `virtio::get()`. All sound under **INV-1 + INV-8**.

| Lines | Site category | Invariant | Rationale |
|-------|---------------|-----------|-----------|
| 87, 96, 118, 131, 156 | `unsafe { virtio::get() }` | INV-8 | Net feature implies virtio is init'd |
| 162, 175, 177, 186 | `unsafe { &mut NET_* }` in init | INV-1 | Boot-time, single-hart |
| 209, 241, 245, 251 | `unsafe { NET_IFACE.as_mut() }` in poll | INV-1 | Single-hart kernel |
| 323, 346, 375, 404, 439 | `unsafe { &mut SOCKET_STORAGE[...] }` | INV-1 | Handler runs in trap context |
| 490, 495, 567, 568, 611, 618 | `unsafe { &mut STAGING[..] }` | INV-1 | Per-call staging buffer, single-hart |

**Follow-up:** STAGING is a shared static buffer. Two concurrent NET_SEND calls would race — but under INV-1 this can't happen. Revisit for SMP.

### `page_alloc.rs` — 7 sites

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 38 | `pub unsafe fn get()` | INV-1 + INV-8 | Declared unsafe with `# Safety` doc |
| 200 | `pub unsafe fn zero_page(addr)` | INV-5 | Caller vouches for PA validity |
| 227, 228 | `&_end` / `&_heap_end` addresses | INV-4 | Linker symbols |
| 234 | `unsafe { ALLOC = BitmapAllocator::new(...) }` | INV-1 | Boot-time init |
| 249 | `unsafe { get() }` | INV-8 | Post-init |

### `plic.rs` — 5 sites (MMIO)

All `ptr::read_volatile` / `ptr::write_volatile` on PLIC MMIO registers (priority, enable, claim/complete). All sound under **INV-3**.

### `process.rs` — 30 sites (PROCS table, linker symbols, scheduler)

The largest concentration. Categories:

| Category | Count | Invariant |
|----------|-------|-----------|
| `unsafe { CURRENT_PID }` | 7 | INV-1 |
| `unsafe { PROCS[i] }` (read/write) | 14 | INV-1 |
| `&_user_init_start` etc. | 6 | INV-4 |
| `asm!("csrs/csrc sstatus")` | 2 | INV-7 |
| `pub(crate) unsafe fn schedule` | 1 | INV-1 + INV-2 (declared unsafe, reloads TrapFrame) |

### `security.rs` — 1 site

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| (unknown, 1 occurrence) | Likely a test `unsafe` | N/A | Module is otherwise unsafe-free by design |

**Note:** module doc claims "No unsafe, no global state." Need to verify the 1 remaining `unsafe` is intentional or remove.

### `syscall.rs` — 22 sites

Same shape as `process.rs`: CURRENT_PID, PROCS, linker symbols, csr manipulation, page_alloc accessor. All sound under **INV-1 / INV-4 / INV-7 / INV-8**.

Two specific callouts:

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 292 | `asm!("csrs sstatus, ...", 1<<18)` | INV-7 | Setting SUM bit for copy_*_user path |
| 295 | ELF byte slice cast in SYS_SPAWN | INV-9 | **Needs bounds check audit** |

### `trap.rs` — 10 sites

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 100, 416, 438 | `unsafe { TICKS }` (read) | INV-1 | Tick counter, single-hart |
| 436 | `unsafe { TICKS += 1 }` (write) | INV-1 | Timer interrupt context |
| 112, 122, 145 | `unsafe { asm!(...) }` for sie/sip/stvec | INV-7 | Trap setup |
| 329 | `unsafe { asm!("wfi") }` | INV-7 | Idle in timer handler |
| 413 | `unsafe { virtio::get() }` | INV-8 | IRQ dispatch |
| 468, 476 | schedule + sret | INV-1 + INV-2 | Control transfer |

### `uart.rs` — 5 sites (MMIO)

All `ptr::read_volatile` / `ptr::write_volatile` on UART registers (RBR/THR, IER, LSR, FCR). All sound under **INV-3**.

### `virtio.rs` — 13 sites

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 202, 206 | MMIO r/w on VirtIO config regs | INV-3 | |
| 304, 560, 569 | Ring descriptor writes via `&mut` on static | INV-1 | Single-hart, descriptor ownership per direction |
| 363 | MAC address byte read | INV-3 | Config space read |
| 431 | virtio-net header cast `&[u8]` from bytes | INV-9 | Fixed 10-byte header; bounds OK |
| 538 | `pub unsafe fn get()` | INV-1 + INV-8 | Same pattern as page_alloc |
| 544, 549, 582, 584 | Static mut flags read/write | INV-1 | Boot init + single-hart |

### `wasi.rs` — 1 site

| Line | Site | Invariant | Rationale |
|------|------|-----------|-----------|
| 294 | `memory` ptr deref in WASI fd_write | INV-6-ish | **Needs audit** — is memory from validated source? |

## Summary by Invariant

| Invariant | Sites | Strength |
|-----------|-------|----------|
| INV-1 (Single-hart) | ~85 | **Load-bearing for the whole kernel.** SMP breaks everything. |
| INV-2 (Trap frame ownership) | ~20 (overlaps INV-1) | Strong — enforced by hardware trap semantics |
| INV-3 (MMIO validity) | ~20 | Strong — hardware spec guarantee |
| INV-4 (Linker symbols) | ~12 | Strong — linker guarantees |
| INV-5 (Allocator PA validity) | ~6 | Medium — relies on correct boot init |
| INV-6 (Walker PA validity) | ~6 | Medium — walker correctness required |
| INV-7 (Privileged asm) | ~15 | Strong — privilege level guarantees |
| INV-8 (Post-init singletons) | ~10 | Procedural — no enforcement beyond code review |
| INV-9 (Struct reinterpretation) | ~4 | **Weak — needs follow-up** on alignment |

## Follow-ups Identified

The audit surfaced five items that should be tracked as issues (not drive-by-fixed here):

1. **`elf.rs` alignment** — `&*(ptr as *const Elf64Header)` assumes 8-byte alignment of a `#[repr(C)]` struct being cast from a `&[u8]` slice. `include_bytes!` gives alignment 1. Switch to explicit byte parsing or `read_unaligned`.
2. **`syscall.rs:295` SYS_SPAWN ELF bounds check** — verify length is validated before struct cast.
3. **`wasi.rs:294` memory provenance** — audit how the WASI memory pointer is obtained and whether it's validated.
4. **`net.rs` STAGING race under SMP** — when SMP lands, STAGING needs per-hart copies or a lock.
5. **`security.rs` residual unsafe** — module claims "no unsafe"; grep found 1 occurrence. Resolve or remove the claim.

## Enforcement Going Forward

New rule: every `unsafe` block added to `kernel/src/*.rs` must carry a `// SAFETY:` comment citing the invariant number. Example:

```rust
// SAFETY: INV-1 (single-hart). CURRENT_PID is only mutated by the scheduler,
// which runs in trap context with interrupts disabled.
let current = unsafe { CURRENT_PID };
```

Pass 2 of this audit will add these comments inline for all 144 sites.

## When to Re-Audit

- SMP is introduced (INV-1 breaks)
- A new platform with different MMIO bases is added (INV-3 changes)
- The allocator is swapped (INV-5 changes)
- A capability system is added (adds invariants around capability validity)
- Anything in the "Follow-ups" list above is resolved

---

*Last audited: Build 84+ · Kernel LOC: 10,048 · Unsafe sites: 144*
