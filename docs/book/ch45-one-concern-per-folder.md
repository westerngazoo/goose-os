---
sidebar_position: 1
sidebar_label: "Ch 45: One Concern Per Folder"
title: "Chapter 45: One Concern Per Folder — Restructuring the Repo"
---

# Chapter 45: One Concern Per Folder — Restructuring the Repo

At the end of Chapter 44, the kernel compiled, the userspace compiled, the network came up, and PID 1 talked to PID 3 over IPC. Every line of code was in the right place logically. Every line of code was in the wrong place physically.

The repo root looked like this:

```
goose-os/
  .build_number
  .cargo/
  Cargo.lock
  Cargo.toml
  Makefile
  README.md
  fw_payload.img
  goose-upgrade.sh
  kernel.bin
  linker-vf2.ld
  linker.ld
  rust-toolchain.toml
  rustc-ice-2026-04-15T08_28_13-1102522.txt
  rustc-ice-2026-04-15T08_28_29-1102577.txt
  ... (20 more rustc-ice crash dumps)
  src/
  u-boot-spl.bin.normal.out
  user/
```

Kernel source. Build outputs. Linker scripts. VisionFive 2 firmware blobs. A shell script. Twenty-two `rustc-ice-*.txt` crash dumps from a bad afternoon with the Rust toolchain. All mixed together at the top level with no organizing principle except "ls ran out of ideas."

This chapter is not about a new capability. It is about the engineering discipline that keeps the new capabilities from drowning each other. The work took about forty minutes. The delta is entirely moves and path updates. The point of writing it down is that the shape of the repo is the shape of the project, and an unfolded pile of files tells the next reader that the authors had stopped paying attention.

> :angrygoose: Twenty-two rustc-ice-\*.txt files. Twenty-two. Each one is the Rust compiler crashing on a proc-macro panic and politely writing a 10 KB backtrace into the current working directory. I had `git add .`'d them without thinking. They sat in the root for a week. They were committed to main. Never commit what you have not looked at.
>
> :sarcasticgoose: The defense, if you want one: "the compiler put them there." Sure. And the compiler also spends three seconds on every build walking the root directory to check what changed. Every cargo build for a week got slower because I refused to press Delete.

## The Principle

One concern per folder. That is the whole rule.

A concern is a thing that has its own lifetime, its own tools, its own audience. The kernel source is a concern. User programs are a concern. VisionFive 2 firmware blobs are a concern. Build outputs are a concern. Device-side deployment scripts are a concern. Book chapters are a concern. When a concern does not have its own folder, one of two things happens: either it sprawls across the root and contaminates unrelated concerns, or it gets jammed into another concern's folder and the mental model stops matching the filesystem.

The rule is older than software. A well-organized workshop has a shelf for screws, a shelf for saws, a shelf for lumber, and a shelf for finished pieces. Nobody argues with this in a workshop. They accept it in a repo, one hasty commit at a time, until the shelf with screws on it also has three saws and a half-built chair.

> :sharpgoose: "One concern per folder" is a constraint that pays interest. Every time you add a new file, the folder tells you whether it belongs. If it does not fit any existing folder, you have discovered a new concern, and the right move is to make it a folder — not to jam it in somewhere approximate. The rule scales up naturally; it breaks only when you give up on it.

## The New Layout

```
goose-os/
  Makefile               -- the single driver script, stays at root
  .build_number          -- auto-incrementing build number, stays at root
  .gitignore             -- stays at root
  LICENSE, README.md     -- stay at root
  kernel/                -- the kernel crate
    .cargo/config.toml
    Cargo.toml, Cargo.lock
    rust-toolchain.toml
    linker.ld, linker-vf2.ld
    src/
      main.rs, boot.S, trap.S, trap.rs, ...
      net.rs, virtio.rs, page_table.rs, ...
  userspace/             -- user programs
    hello/
      Cargo.toml, linker.ld, rust-toolchain.toml, .cargo/
      src/main.rs, src/gooseos.rs, src/start.S
  platform/              -- hardware-specific artifacts
    vf2/
      u-boot-spl.bin.normal.out
      fw_payload.img
  scripts/               -- device-side shell scripts
    goose-upgrade.sh
  docs/                  -- book chapters, design notes, social posts
    book/
    social-posts.md
  build/                 -- build outputs
    kernel.bin
```

Each folder answers a single question.

- **`kernel/`** answers "where is the kernel source and how is it built?"
- **`userspace/`** answers "what user programs exist?"
- **`platform/vf2/`** answers "what do we need to boot on the VisionFive 2 that we didn't write ourselves?"
- **`scripts/`** answers "what runs on the device, not on the host?"
- **`docs/`** answers "what should a human read?"
- **`build/`** answers "what artifacts did the build system produce?"

The root answers "how do I build and deploy?" — with a `Makefile`, a `.build_number`, a `README`, a `LICENSE`, and not one thing more.

> :nerdygoose: `platform/` is the folder that matters most when hardware support grows. Today it is `platform/vf2/`. Tomorrow it will be `platform/vf2/`, `platform/milkv-duo/`, and `platform/qemu-virt/` — each one holding the firmware, device tree overlays, and boot-config specifics of a target board. Nothing in the kernel or userspace folders needs to change when a new board is added, because the board-specific stuff lives in its own drawer.

## The Path Plumbing

Restructuring a repo of moderate size is not difficult. It is tedious. The tedium is entirely in the path references that survived the move.

**Makefile.** Every cargo invocation now `cd`s into its crate before running:

```makefile title="Makefile — before and after"
# Before
build:
	cargo build --release

# After
build:
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release
```

The same pattern applies to `build-user`:

```makefile
build-user:
	cd userspace/hello && CARGO_ENCODED_RUSTFLAGS='-Clink-arg=-Tlinker.ld' \
	    cargo build --release
```

`KERNEL_ELF` and `KERNEL_BIN` also move:

```makefile
KERNEL_ELF := kernel/target/riscv64gc-unknown-none-elf/release/goose-os
KERNEL_BIN := build/kernel.bin
USER_ELF   := userspace/hello/target/riscv64gc-unknown-none-elf/release/hello
```

**`kernel/src/process.rs`.** The kernel embeds the compiled user ELF at build time through `include_bytes!`. That macro is resolved at compile time, relative to the source file, so the path starts from `kernel/src/`:

```rust title="kernel/src/process.rs — include_bytes path"
#[cfg(feature = "rust-user")]
const USER_ELF: &[u8] = include_bytes!(
    "../../userspace/hello/target/riscv64gc-unknown-none-elf/release/hello"
);
```

Two `..` hops — one out of `src/`, one out of `kernel/` — then into `userspace/hello/target/...`. The macro will fail at compile time if the path is wrong, which means a broken path cannot ship.

**`scripts/goose-upgrade.sh`.** The deploy script reads `build/kernel.bin` now rather than the root-level `kernel.bin`:

```bash
# Before:  KERNEL_BIN=./kernel.bin
# After:
KERNEL_BIN=build/kernel.bin
```

**`.gitignore`.** Two new lines:

```gitignore
**/target/
rustc-ice-*.txt
```

The double-star `target/` pattern catches `kernel/target/` and `userspace/hello/target/` with one rule. The `rustc-ice-*.txt` pattern makes sure the compiler's next bad day does not commit itself to main.

> :weightliftinggoose: The `**/target/` pattern is one line that saves gigabytes. Every Rust crate in the tree has its own `target/` with its own build artifacts, and every one of them grows to a few hundred megabytes. Without this line, a careless `git add .` commits binary build output and your PR shows up on GitHub as "Gustavo added 3,847 changed files." The one-line fix is worth its weight in PR diff sanity.

## The Cleanup

Three one-time operations finished the move.

**Delete 22 rustc-ice crash dumps.** The root directory had twelve, `user/hello/` had ten more. All gone. The shell one-liner was `rm rustc-ice-*.txt kernel/rustc-ice-*.txt userspace/hello/rustc-ice-*.txt`.

**Untrack `userspace/hello/target/`.** It was accidentally committed at some point before the `**/target/` rule existed. `git rm -r --cached userspace/hello/target/` removed the tracking without deleting the local files, then the `.gitignore` rule kept them out going forward.

**Remove the old flat `user/` directory.** After verifying `userspace/hello/` was complete, the old `user/hello/` went away with `git rm -r user/`. The branch that did the move had no other changes touching `user/`, so there was nothing to merge.

> :surprisedgoose: "How big was the diff?" The file-tree side of it was dominated by *renames*, which Git is smart enough to detect and display as one line each — `kernel.bin => build/kernel.bin`, `{src => kernel/src}/boot.S`, and so on. The real *content* delta was the Makefile (~40 lines changed), one `include_bytes!` path in `process.rs`, and the script. Three files of real change. The rest was Git bookkeeping.

## Verification

A restructure that breaks the build is worse than no restructure at all. The verification step is boring on purpose: it runs both feature-combination builds, plus the end-to-end net test.

```bash
# 1. Default feature set (kernel with no optional modules)
make build              # -> kernel.bin compiled, no errors

# 2. Full feature set (rust-user + net)
make test-net-user      # -> compiles both crates, boots QEMU, runs test
```

The expected output of the second command, after the restructure, is:

```
=== Rust Userspace Net Test ===
[net] VirtIO-net detected at slot 7
[net] MAC 52:54:00:12:34:56, IP 10.0.2.15/24
[net] smoltcp stack up
Hello from Rust userspace!
My PID is 1
[net-test] Calling NET_STATUS...
[net-test] net up (status=1)
[net-test] Opening UDP socket...
[net-test] got UDP handle 5
[net-test] Binding handle 5 to port 9999...
[net-test] bound OK
[net-test] Closing handle 5...
[net-test] close OK
[net-test] PASS
```

The kernel boots, VirtIO is detected, smoltcp comes up, PID 1 runs the net pipeline, PASS. The same output Chapter 44 produced. The restructure moved every file; the behavior of the system did not change by a single byte.

> :happygoose: This is the cleanest possible commit message: "verified equivalent behavior." A restructure that preserves semantics is easy to approve, easy to revert, and easy to trust. The worst kind of restructure is the one that is *also* a rewrite — now you cannot tell whether the refactor broke something or whether the rewrite did, and bisecting is a forensic exercise. One commit, one kind of change.

## Why This Matters for a Microkernel

The point of a microkernel is not that it is small. It is that each component has a single job, and the interfaces between them are narrow and inspectable. A filesystem layout with `kernel/`, `userspace/`, `platform/`, and `docs/` at the top level encodes that same principle at the filesystem level. Anyone reading the repo for the first time sees the architecture before they see the code.

Flat repos communicate "this is all one thing." That is sometimes true — a small library, a single binary — and when it is true, flatness is a virtue. GooseOS is not one thing. It is a kernel, a set of user programs, a set of board-specific blobs, a build system, and a book. The folders say so.

> :sharpgoose: Watch what happens when the next user program arrives — a DNS client, say, or a simple HTTP fetcher. Before this restructure, we would have been tempted to jam it into `user/hello/src/` as a second binary. After this restructure, the folder says where it goes: `userspace/dns/`. The shape of the repo told us the shape of the refactor. That is what "structure as documentation" looks like when it works.

## What We Changed

| Path | Change |
|------|--------|
| `kernel/` | New folder, holds all kernel crate contents (Cargo.toml, src/, linker scripts, .cargo/, toolchain) |
| `userspace/hello/` | Renamed from `user/hello/` |
| `platform/vf2/` | New folder for VisionFive 2 firmware blobs (u-boot-spl, fw_payload) |
| `scripts/` | New folder, holds `goose-upgrade.sh` |
| `docs/` | New folder, holds book chapter drafts and social posts |
| `build/` | New folder, holds `kernel.bin` build output |
| `Makefile` | All cargo invocations prefixed with `cd kernel &&` or `cd userspace/hello &&`; paths updated |
| `kernel/src/process.rs` | `include_bytes!` path updated to `../../userspace/hello/target/...` |
| `scripts/goose-upgrade.sh` | Reads `build/kernel.bin` instead of root-level `kernel.bin` |
| `.gitignore` | Added `**/target/` and `rustc-ice-*.txt` |

## What's Next

The next engineering-hygiene chapters have their topics lined up. None of them are glamorous; all of them compound.

- **The unsafe audit.** Every `unsafe` block in the kernel should be catalogued with a one-line justification. Right now there are about 40 of them across 8 files. If we ever want to formally verify any subset of the kernel, the audit is the precondition. Chapter 46 walks through the catalog and explains what each `unsafe` is doing and why it is sound.
- **The build-number ratchet.** `.build_number` auto-increments on every build and every deploy carries its build number in the commit message. Small trick, big payoff for bisecting. Chapter 47 explains why.
- **The feature-flag matrix.** `qemu`, `vf2`, `debug-kernel`, `net`, `rust-user`, `wasm-test`, `security-test` — every combination is a different kernel. Documenting which combinations are tested, and which are not, is the sort of engineering hygiene that is boring to do and catastrophic to skip. Chapter 48 lays out the matrix.

Structural chapters are a different flavor from the ones that came before. They do not produce a new screen-full of output. They produce a system that is easier to read, easier to extend, and easier to trust. That is the payoff, and it is worth writing down.
