//! Scheduler — context switching and process selection.
//!
//! All scheduler policy and mechanism lives here. Callers:
//!   - Timer IRQ (trap.rs) calls `preempt` from U-mode, `schedule_from_idle`
//!     from S-mode idle.
//!   - Blocking syscalls (ipc.rs, lifecycle.rs, net.rs) call `schedule`
//!     after setting their own state to BlockedXxx.
//!   - `sys_yield` is here because yielding is scheduler mechanism, not
//!     lifecycle.
//!
//! Selection is round-robin from the blocked/current PID + 1. The scan
//! is O(MAX_PROCS); the kernel is still single-hart so no locking.
//!
//! Invariants
//!
//!   - After any function in this module returns, the TrapFrame on the
//!     kernel stack describes the process that sret will return to.
//!   - `CURRENT_PID` and `satp` agree: both reflect the same process.
//!   - A process observed in `Running` is either CURRENT_PID, or was
//!     just swapped out and will be marked Ready/Blocked on the next
//!     scheduler entry. No third case.
//!
//! See docs/unsafe-audit.md INV-1 (single-hart) and INV-2 (TrapFrame
//! exclusivity) for why the `unsafe` here is sound.

use core::arch::asm;

use crate::process::{PROCS, CURRENT_PID, ProcessState};
use crate::security::MAX_PROCS;
use crate::trap::TrapFrame;

// ── Voluntary Yield ────────────────────────────────────────────

/// SYS_YIELD — voluntarily give up the remainder of the time slice.
///
/// Convention: no arguments. Returns: a0 = 0.
/// Marks the caller `Ready` and switches to the next process.
/// If no other process is Ready, returns immediately (caller keeps
/// running with a fresh time slice's worth of quiet).
pub fn sys_yield(frame: &mut TrapFrame) {
    frame.sepc += 4;

    // SAFETY: INV-1. CURRENT_PID is scheduler-owned.
    let current = unsafe { CURRENT_PID };
    if current == 0 { return; }

    // SAFETY: INV-1. PROCS is scheduler-owned; interrupts are disabled
    // inside a trap handler.
    unsafe {
        // Check if there's anyone else to switch to.
        let mut found = false;
        for offset in 1..(MAX_PROCS - 1) {
            let i = ((current - 1 + offset) % (MAX_PROCS - 1)) + 1;
            if PROCS[i].state == ProcessState::Ready {
                found = true;
                break;
            }
        }
        if !found {
            frame.a0 = 0;
            return; // we're the only one — keep running
        }

        PROCS[current].state = ProcessState::Ready;
        schedule(frame, current);
    }

    frame.a0 = 0;
}

// ── Preemptive Scheduling ──────────────────────────────────────

/// Timer-driven preemption — called from the timer interrupt handler.
///
/// If a user process is running, forcibly save its state and switch
/// to the next Ready process (round-robin). This is what prevents a
/// busy-loop process from starving everyone else.
pub fn preempt(frame: &mut TrapFrame) {
    // SAFETY: INV-1. Timer interrupt in single-hart kernel.
    unsafe {
        let current = CURRENT_PID;
        if current == 0 { return; } // no user process running

        // Find next Ready process (round-robin from current+1).
        let mut next = 0;
        for offset in 1..(MAX_PROCS - 1) {
            let i = ((current - 1 + offset) % (MAX_PROCS - 1)) + 1;
            if PROCS[i].state == ProcessState::Ready {
                next = i;
                break;
            }
        }

        if next == 0 {
            return; // no one else to run — let current continue
        }

        // Preempt: save current, load next.
        PROCS[current].context = *frame;
        PROCS[current].state = ProcessState::Ready;

        *frame = PROCS[next].context;
        PROCS[next].state = ProcessState::Running;
        CURRENT_PID = next;

        let next_satp = PROCS[next].satp;
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) next_satp,
        );
    }
}

/// Schedule from kernel idle — called by the timer handler when in S-mode.
///
/// When the kernel is idle (CURRENT_PID=0, waiting in WFI), an interrupt
/// might wake a blocked process (e.g., IRQ notification → BlockedRecv → Ready).
/// The next timer tick calls this to check if any process became Ready and
/// switch to it.
pub fn schedule_from_idle(frame: &mut TrapFrame) {
    // SAFETY: INV-1.
    unsafe {
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Ready {
                *frame = PROCS[i].context;
                PROCS[i].state = ProcessState::Running;
                CURRENT_PID = i;

                let satp = PROCS[i].satp;
                asm!(
                    "csrw satp, {0}",
                    "sfence.vma zero, zero",
                    in(reg) satp,
                );
                return;
            }
        }
    }
}

// ── Core Scheduler ─────────────────────────────────────────────

/// Save current process and switch to the next ready process.
///
/// Called when a process blocks (SEND / RECEIVE / WAIT / BlockedNet
/// with no rendezvous). Uses round-robin: scans from blocked_pid + 1,
/// wrapping around. The frame on the kernel stack is overwritten with
/// the next process's saved context. When trap.S restores and srets,
/// we land in the next process.
///
/// If no Ready process exists, enters the kernel idle loop (waiting
/// for a timer tick to wake a blocked process). If no process is
/// alive at all, panics — that's a deadlock.
///
/// # Safety
/// Caller must have set `PROCS[blocked_pid].state` to the correct
/// BlockedXxx variant before calling. INV-1 + INV-2 apply.
pub(crate) unsafe fn schedule(frame: &mut TrapFrame, blocked_pid: usize) {
    // Save current process's registers.
    PROCS[blocked_pid].context = *frame;

    // Find next ready process (round-robin from blocked_pid + 1).
    let mut next = 0;
    for offset in 1..MAX_PROCS {
        let i = ((blocked_pid - 1 + offset) % (MAX_PROCS - 1)) + 1;
        if PROCS[i].state == ProcessState::Ready {
            next = i;
            break;
        }
    }

    if next == 0 {
        // No Ready process — check if any are still alive (waiting for events).
        let mut any_alive = false;
        for i in 1..MAX_PROCS {
            if PROCS[i].state != ProcessState::Free {
                any_alive = true;
                break;
            }
        }

        if !any_alive {
            panic!("Deadlock: no runnable processes (PID {} blocked)", blocked_pid);
        }

        // Enter kernel idle — timer interrupts will schedule when a
        // process wakes.
        CURRENT_PID = 0;
        let kernel_satp = crate::kvm::kernel_satp();
        asm!(
            "csrw satp, {0}",
            "sfence.vma zero, zero",
            in(reg) kernel_satp,
        );

        frame.sstatus = (1 << 8) | (1 << 5); // SPP=S, SPIE=1
        frame.sepc = crate::trap::kernel_idle as *const () as usize;
        return;
    }

    // Load next process's context onto the kernel stack.
    *frame = PROCS[next].context;
    PROCS[next].state = ProcessState::Running;
    CURRENT_PID = next;

    // Switch page table.
    let next_satp = PROCS[next].satp;
    asm!(
        "csrw satp, {0}",
        "sfence.vma zero, zero",
        in(reg) next_satp,
    );
}
