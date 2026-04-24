//! Wari-WASI extensions — beyond WASI Preview 1.
//!
//! Staged by phase:
//!   Phase 1: `wari_net_*`       — TCP/UDP via Tier-2 net driver
//!   Phase 2: `wari_crypto_*`    — Zkn/Zks hardware crypto
//!   Phase 2: `wari_ai_infer`    — neural net inference via Tier-2 GPU
//!   Phase 3: `wari_gapu_*`      — GAPU FPGA offload
//!
//! Every extension is gated by a capability (Phase 1+). Tier-1 modules
//! receive capabilities at spawn time based on a manifest the operator
//! signs. A module without the capability sees `SyscallError::PermissionDenied`
//! when calling the host function.
