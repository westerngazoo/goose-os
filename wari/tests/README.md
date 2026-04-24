# Wari — Test Harness

Three layers. See `../docs/testing.md` for the full strategy and
`../CLAUDE.md` §Testing Strategy for the gates.

| Directory          | Layer       | Run on      | Gate                               |
|--------------------|-------------|-------------|------------------------------------|
| `../abi-shared/`*  | Unit (pure) | Host        | `cargo test -p wari-abi`          |
| `../kernel/src/`*  | Unit (pure) | Host        | `cargo test -p wari-kernel --lib` |
| `integration/`     | Integration | QEMU RV64   | `cargo test -p integration-tests` |
| `security/`        | Adversarial | QEMU RV64   | `cargo test -p security-tests`    |
| `fuzz/`            | Fuzz        | Host (long) | `cargo fuzz run <target>`         |

*\* "Pure" modules place their tests inline under `#[cfg(test)] mod`.*

Per-phase gates (every milestone commits when all of these pass):

**Phase 0 gate (milestone m0):**
  - `boot_smoke.rs` — kernel boots to the wasmi runtime, banner prints
  - `hello_wasm.rs` — Tier-1 WASM prints "Hello from Wari" and exits 0
  - `malformed_wasm.rs` — invalid bytecode rejected cleanly
  - `elf_rejection.rs` — `SYS_SPAWN_ELF` does not exist (compile-time check)
  - fuzz clean for 1 h on wasmi-validator target
