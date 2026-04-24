//! WASI Preview 1 — the subset Wari implements.
//!
//! Implemented by Phase 0 exit:
//!   `fd_write`, `fd_read`, `fd_close`, `proc_exit`, `clock_time_get`,
//!   `args_get`, `args_sizes_get`, `environ_get`, `environ_sizes_get`,
//!   `random_get`.
//!
//! NOT implemented (by design):
//!   `path_open`, `fd_seek` — no filesystem in Phase 0–1 (object store
//!   arrives Phase 2).
//!   `sock_accept` from WASI P1 — replaced by `wari_ext::net`.
//!   `thread_spawn` — single-threaded wasmi per instance in Phase 0–1.
//!
//! Agent lands function signatures in the cherry-pick / wasmi-integration PR.
