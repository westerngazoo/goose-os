//! Process Control Block — types, globals, and read-only accessors.
//!
//! This is the data module: it defines what a process *is* (the PCB
//! struct + state enum + network-op enum) and owns the three globals
//! that record the process table at runtime.
//!
//! It does *not* contain policy:
//!   - Scheduling           → `sched.rs`
//!   - Creation / exit      → `lifecycle.rs`
//!   - IPC handlers         → `ipc.rs`
//!   - Memory / IRQ / spawn → `syscall.rs`
//!
//! Every other kernel module talks to the process table through the
//! `pub(crate)` statics here. When a capability system lands, those
//! statics will grow a per-process capability slab; when SMP lands,
//! they'll grow per-hart `CURRENT_PID` slots. The shape of the struct
//! is the kernel's most frequently-read API and deserves its own
//! small, boring file.
//!
//! See docs/unsafe-audit.md invariants INV-1 and INV-2 for why the
//! `static mut` globals are sound in single-hart context.

use crate::security::{MAX_PROCS, MAX_IRQS};
use crate::trap::TrapFrame;
use crate::println;

// Re-export for modules (e.g., net.rs) that iterate the process table.
pub(crate) use crate::security::MAX_PROCS as MAX_PROCS_PUB;

// ── Process State Machine ──────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Free,           // Slot is unused
    Ready,          // Can be scheduled
    Running,        // Currently executing
    BlockedSend,    // Waiting for receiver to call RECEIVE
    BlockedRecv,    // Waiting for sender to call SEND
    BlockedCall,    // Sent via SYS_CALL, waiting for SYS_REPLY
    BlockedWait,    // Waiting for child process to exit (SYS_WAIT)
    BlockedNet,     // Waiting for a network event (recv data, TCP connect) — Phase B.next
}

/// What network event a BlockedNet process is waiting on.
///
/// Stored in `Process.net_op` while `state == BlockedNet`.
/// `net.rs::wake_blocked` iterates all BlockedNet processes after each
/// poll and tries to complete the matching op.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetOp {
    None,
    Recv,    // completed when socket has at least one byte
    Connect, // completed when TCP socket reaches Established (or Closed = failed)
}

// ── Process Control Block ──────────────────────────────────────

#[derive(Clone, Copy)]
pub struct Process {
    pub pid: usize,
    pub state: ProcessState,
    pub satp: u64,
    pub context: TrapFrame,     // Saved registers (for context switch)
    // IPC state
    pub ipc_target: usize,      // Who we're sending to / expecting from (0 = any)
    pub ipc_value: usize,       // Message being sent
    pub ipc_arg2: usize,        // Additional IPC argument (a2) — Phase B
    pub ipc_arg3: usize,        // Additional IPC argument (a3) — Phase B
    // Lifecycle
    pub parent: usize,          // Parent PID (0 = kernel-spawned)
    pub exit_code: usize,       // Stored when process exits
    // IRQ ownership (Phase 13)
    pub irq_num: u32,           // Registered IRQ number (0 = none)
    pub irq_pending: bool,      // IRQ fired while not in BlockedRecv
    // Network blocking state (Phase B.next) — valid when state == BlockedNet
    pub net_op: NetOp,          // NetOp::None when not blocked on a net event
    pub net_socket: usize,      // Socket handle index (TCP 0..MAX_TCP, UDP offset by MAX_TCP)
    pub net_buf_va: usize,      // User VA for recv buffer (recv only)
    pub net_buf_len: usize,     // Max bytes to copy (recv only)
}

impl Process {
    pub const fn empty() -> Self {
        Process {
            pid: 0,
            state: ProcessState::Free,
            satp: 0,
            context: TrapFrame::zero(),
            ipc_target: 0,
            ipc_value: 0,
            ipc_arg2: 0,
            ipc_arg3: 0,
            parent: 0,
            exit_code: 0,
            irq_num: 0,
            irq_pending: false,
            net_op: NetOp::None,
            net_socket: 0,
            net_buf_va: 0,
            net_buf_len: 0,
        }
    }
}

// ── Process Table (kernel global state) ────────────────────────

/// Process table — fixed size, no heap allocation.
/// PID 0 is reserved (kernel). Processes use PIDs 1..MAX_PROCS-1.
pub(crate) static mut PROCS: [Process; MAX_PROCS] = [Process::empty(); MAX_PROCS];

/// PID of the currently running process.
pub(crate) static mut CURRENT_PID: usize = 0;

/// IRQ ownership table — maps IRQ number to owning PID (0 = kernel/unclaimed).
pub(crate) static mut IRQ_OWNER: [usize; MAX_IRQS] = [0; MAX_IRQS];

// ── Read-only Accessors ────────────────────────────────────────

/// Get the currently running PID (for trap handler diagnostics).
pub fn current_pid() -> usize {
    // SAFETY: INV-1.
    unsafe { CURRENT_PID }
}

/// Get the satp value of the currently running process.
/// Returns 0 if CURRENT_PID is 0 (kernel context).
pub fn current_satp() -> u64 {
    // SAFETY: INV-1.
    unsafe {
        let pid = CURRENT_PID;
        if pid == 0 { 0 } else { PROCS[pid].satp }
    }
}

/// Dump all non-Free process slots. Called by `kdump_procs!` macro.
///
/// Always compiled (body is empty without `debug-kernel` feature).
/// The macro already gates the call site — putting `#[cfg]` on the
/// function itself triggered a rustc nightly ICE in
/// lint_mod/check_mod_deathness.
#[allow(dead_code)]
pub fn dump_procs() {
    #[cfg(feature = "debug-kernel")]
    // SAFETY: INV-1. Diagnostic read only.
    unsafe {
        println!("[procs] CURRENT_PID={}", CURRENT_PID);
        for i in 1..MAX_PROCS {
            if PROCS[i].state != ProcessState::Free {
                println!("[procs] PID {} state={:?} irq={} irq_pending={} ipc_target={} sepc={:#x}",
                    i, PROCS[i].state, PROCS[i].irq_num, PROCS[i].irq_pending,
                    PROCS[i].ipc_target, PROCS[i].context.sepc);
            }
        }
    }
}

// ── Trivial Syscall Handlers ───────────────────────────────────

/// SYS_GETPID — return the current process's PID.
///
/// Convention: no arguments. Returns: a0 = PID.
/// Lives here (rather than lifecycle.rs) because it's a pure read
/// of the scheduler-owned global, not a lifecycle transition.
pub fn sys_getpid(frame: &mut TrapFrame) {
    frame.sepc += 4;
    // SAFETY: INV-1.
    frame.a0 = unsafe { CURRENT_PID };
}
