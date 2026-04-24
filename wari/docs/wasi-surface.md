# Wari — WASI Surface Specification

> The complete host-function surface exposed to Tier-1 and Tier-2 WASM
> modules. Every function here is a potential attack vector: it's
> specified here, defined in `/wari/wasi/src/`, implemented in
> `/wari/kernel/src/runtime/`, and tested adversarially in
> `/wari/tests/security/`.

---

## Phase 0 (baseline — WASI Preview 1 subset)

Module name: `wasi_snapshot_preview1` (for maximum compatibility with
existing WASI toolchains like wasi-libc).

| Function | Purpose | Capability required |
|---|---|---|
| `fd_write(fd, iovs, iovs_len) → errno` | Write to fd (Phase 0: only fd=1 stdout, routed to UART) | `CAP_STDOUT` |
| `fd_read(fd, iovs, iovs_len) → errno` | Read from fd (Phase 0: only fd=0 stdin) | `CAP_STDIN` |
| `fd_close(fd) → errno` | Close fd | (always granted) |
| `proc_exit(code) → !` | Terminate module, pass exit code | (always granted) |
| `clock_time_get(clock_id) → (u64, errno)` | Monotonic time | (always granted) |
| `args_get` / `args_sizes_get` | Module arguments from manifest | (always granted) |
| `environ_get` / `environ_sizes_get` | Environment vars from manifest | (always granted) |
| `random_get(buf, len) → errno` | CSPRNG bytes | (always granted; hw-backed Phase 2) |

**Not implemented (by design)**:
  - `path_open`, `fd_seek`, `path_*` — no filesystem in Phase 0–1.
    Object store arrives Phase 2 via `wari_store_*`.
  - `sock_accept` (WASI P1 variant) — networking lives in `wari_ext`.
  - `thread_spawn` — single-threaded wasmi per module, Phase 0–1.

---

## Phase 1 (Wari extensions — networking + caps)

Module name: `wari`. Available only to modules granted relevant
capabilities.

| Function | Purpose | Capability |
|---|---|---|
| `wari_net_tcp_socket() → (fd, errno)` | Create TCP socket | `CAP_NET` |
| `wari_net_udp_socket() → (fd, errno)` | Create UDP socket | `CAP_NET` |
| `wari_net_bind(fd, port)` | Bind to local port | `CAP_NET_BIND` |
| `wari_net_connect(fd, ip, port)` | Connect TCP | `CAP_NET` |
| `wari_net_listen(fd, backlog)` | Listen on TCP | `CAP_NET_BIND` |
| `wari_net_accept(fd) → (conn_fd, errno)` | Accept incoming connection | `CAP_NET_BIND` |
| `wari_net_send(fd, buf, len) → (n, errno)` | Send data | `CAP_NET` |
| `wari_net_recv(fd, buf, max) → (n, errno)` | Receive data | `CAP_NET` |
| `wari_cap_drop(cap_idx)` | Voluntarily drop a capability | (any) |
| `wari_cap_delegate(peer_pid, cap_idx, rights)` | Grant a subset to another module | (cap-owner) |

---

## Phase 2 (AI + crypto + Docker ingress)

| Function | Purpose | Capability |
|---|---|---|
| `wari_crypto_encrypt(key_id, plaintext, ...) → ciphertext` | AES-256-GCM via Zkn | `CAP_CRYPTO` |
| `wari_crypto_decrypt(key_id, ciphertext, ...) → plaintext` | AES-256-GCM via Zkn | `CAP_CRYPTO` |
| `wari_crypto_hash(data, len, alg) → digest` | BLAKE3 / SHA-256 | (always granted) |
| `wari_ai_load_model(bundle_id) → model_handle` | Load a signed AI model | `CAP_AI` |
| `wari_ai_infer(model, input, output) → errno` | Run inference on GPU/GAPU | `CAP_AI` |
| `wari_store_open(name) → fd` | Object-store blob | `CAP_STORE` |
| `wari_store_read/write` | Object blob I/O | `CAP_STORE` |

---

## Phase 3 (GAPU + CoVE)

| Function | Purpose | Capability |
|---|---|---|
| `wari_gapu_*` | Direct GAPU FPGA offload | `CAP_GAPU` |
| `wari_cove_attest() → attestation` | Get CoVE attestation report | (always granted) |
| `wari_cove_seal(data) → sealed` | Bind data to this tenant's CoVE context | (always granted) |

---

## Stability contract

- Once a host function is shipped, its signature and semantics never
  change. Extensions are new functions, never revisions.
- Capabilities can be added (new CAP_X) but never renumbered.
- Removing a host function is a breaking change and requires a
  deliberate major-version bump of `wari-abi`.

See `/wari/CLAUDE.md` §Absolute Rules R8.
