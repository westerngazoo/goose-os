/// IPC syscall handlers — synchronous send/receive, call/reply, IRQ notification.
///
/// seL4-style rendezvous IPC:
///   SYS_SEND(target, msg)   — block until target calls RECEIVE
///   SYS_RECEIVE(from)       — block until someone sends to us
///   SYS_CALL(target, msg)   — send + block for SYS_REPLY (RPC)
///   SYS_REPLY(target, reply)— deliver reply to BlockedCall (non-blocking)
///
/// No kernel-side message queues, no allocation.

use crate::process::{PROCS, CURRENT_PID, ProcessState, schedule};
use crate::security::{self, MAX_PROCS};
use crate::trap::TrapFrame;

// ── SYS_CALL ──────────────────────────────────────────────────

/// SYS_CALL(target_pid, msg_value) — synchronous RPC (send + wait for reply).
///
/// Convention: a0 = target PID, a1 = message value.
/// Returns:    a0 = reply value (set by SYS_REPLY from server).
///
/// The caller sends a message and blocks until the target calls SYS_REPLY.
/// This halves the syscall cost of an RPC round-trip compared to
/// separate SYS_SEND + SYS_RECEIVE — one ecall instead of two.
///
/// Behavior:
///   - If target is BlockedRecv: rendezvous (deliver msg), but caller
///     stays BlockedCall until the target does SYS_REPLY.
///   - If target is NOT BlockedRecv: caller blocks as BlockedCall,
///     msg waits until target calls SYS_RECEIVE.
pub fn sys_call(frame: &mut TrapFrame) {
    frame.sepc += 4; // advance past ecall (saved with context)

    let current = unsafe { CURRENT_PID };
    let target_pid = frame.a0;
    let msg_value = frame.a1;

    // Validate target
    if !security::is_valid_ipc_target(target_pid, current) {
        frame.a0 = usize::MAX; // error
        return;
    }

    unsafe {
        let target_state = PROCS[target_pid].state;
        let target_wants = PROCS[target_pid].ipc_target;

        // Check if target is blocked on RECEIVE (from us or from any)
        if target_state == ProcessState::BlockedRecv
            && (target_wants == 0 || target_wants == current)
        {
            // Rendezvous! Deliver message to receiver's saved context.
            PROCS[target_pid].context.a0 = msg_value;  // message
            PROCS[target_pid].context.a1 = current;     // sender PID
            PROCS[target_pid].context.a2 = frame.a2;   // extra arg (Phase B)
            PROCS[target_pid].context.a3 = frame.a3;   // extra arg (Phase B)
            PROCS[target_pid].state = ProcessState::Ready;
        }

        // Caller ALWAYS blocks — even after rendezvous.
        // The difference from SYS_SEND: SYS_SEND unblocks on rendezvous,
        // SYS_CALL stays blocked waiting for SYS_REPLY.
        PROCS[current].ipc_target = target_pid;
        PROCS[current].ipc_value = msg_value;
        PROCS[current].ipc_arg2 = frame.a2;
        PROCS[current].ipc_arg3 = frame.a3;
        PROCS[current].state = ProcessState::BlockedCall;

        schedule(frame, current);
    }
}

// ── SYS_REPLY ─────────────────────────────────────────────────

/// SYS_REPLY(target_pid, reply_value) — deliver reply to a SYS_CALL caller.
///
/// Convention: a0 = caller PID, a1 = reply value.
/// Returns:    a0 = 0 (success), usize::MAX (error).
///
/// Non-blocking for the server — it continues running after replying.
/// The target must be in BlockedCall state and must have called US.
/// This prevents random processes from replying to calls they didn't receive.
pub fn sys_reply(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    let target_pid = frame.a0;
    let reply_value = frame.a1;

    // Validate target
    if !security::is_valid_ipc_target(target_pid, current) {
        frame.a0 = usize::MAX;
        return;
    }

    unsafe {
        // Target must be BlockedCall AND must have called us
        if PROCS[target_pid].state == ProcessState::BlockedCall
            && PROCS[target_pid].ipc_target == current
        {
            // Deliver reply to caller's saved context
            PROCS[target_pid].context.a0 = reply_value;
            PROCS[target_pid].state = ProcessState::Ready;

            // Server continues — non-blocking
            frame.a0 = 0;
            return;
        }

        // No matching BlockedCall — error
        frame.a0 = usize::MAX;
    }
}

// ── SYS_SEND ──────────────────────────────────────────────────

/// SYS_SEND(target_pid, msg_value) — synchronous send.
///
/// Convention: a0 = target PID, a1 = message value.
/// Returns: a0 = 0 (success).
///
/// Blocks the sender until the target calls SYS_RECEIVE.
/// If the target is already blocked on RECEIVE, rendezvous immediately.
pub fn sys_send(frame: &mut TrapFrame) {
    frame.sepc += 4; // advance past ecall

    let current = unsafe { CURRENT_PID };
    let target_pid = frame.a0;
    let msg_value = frame.a1;

    // Validate target
    if !security::is_valid_ipc_target(target_pid, current) {
        frame.a0 = usize::MAX; // error
        return;
    }

    unsafe {
        let target_state = PROCS[target_pid].state;
        let target_wants = PROCS[target_pid].ipc_target;

        // Check if target is blocked on RECEIVE (from us or from any)
        if target_state == ProcessState::BlockedRecv
            && (target_wants == 0 || target_wants == current)
        {
            // Rendezvous! Transfer message directly to receiver's saved context.
            PROCS[target_pid].context.a0 = msg_value;     // message
            PROCS[target_pid].context.a1 = current;        // sender PID
            PROCS[target_pid].state = ProcessState::Ready;

            // Sender continues — send returns success
            frame.a0 = 0;
            return;
        }

        // No rendezvous — block the sender
        PROCS[current].ipc_target = target_pid;
        PROCS[current].ipc_value = msg_value;
        PROCS[current].state = ProcessState::BlockedSend;

        // Context switch to the next ready process
        schedule(frame, current);
    }
}

// ── SYS_RECEIVE ───────────────────────────────────────────────

/// SYS_RECEIVE(from_pid) — synchronous receive.
///
/// Convention: a0 = expected sender PID (0 = accept from anyone).
/// Returns: a0 = message value, a1 = sender PID.
///
/// Blocks the receiver until someone calls SYS_SEND targeting us.
/// If a sender is already blocked, rendezvous immediately.
pub fn sys_receive(frame: &mut TrapFrame) {
    frame.sepc += 4;

    let current = unsafe { CURRENT_PID };
    let from_pid = frame.a0; // 0 = any

    unsafe {
        // Phase 13: Check for pending IRQ before checking senders.
        // If an IRQ fired while we weren't in BlockedRecv, irq_pending is set.
        // Return it immediately as a synthetic IPC message (sender=0, msg=irq_num).
        if PROCS[current].irq_pending && (from_pid == 0) {
            PROCS[current].irq_pending = false;
            frame.a0 = PROCS[current].irq_num as usize;  // message = IRQ number
            frame.a1 = 0;                                  // sender = kernel (PID 0)
            return;
        }

        // Check if any sender is blocked waiting to send to us.
        // Match both BlockedSend (from SYS_SEND) and BlockedCall (from SYS_CALL).
        // The server's RECEIVE doesn't care how the sender sent — it gets the
        // message either way. The difference is what happens when the server
        // replies: BlockedSend processes were already unblocked, BlockedCall
        // processes stay blocked until SYS_REPLY delivers the response.
        for i in 1..MAX_PROCS {
            let is_sender = PROCS[i].state == ProcessState::BlockedSend
                || PROCS[i].state == ProcessState::BlockedCall;

            if is_sender
                && PROCS[i].ipc_target == current
                && (from_pid == 0 || from_pid == i)
            {
                // Rendezvous! Transfer message.
                let msg = PROCS[i].ipc_value;
                let sender = i;

                // Unblock sender ONLY if it was a plain SEND.
                // BlockedCall stays blocked — it's waiting for SYS_REPLY.
                if PROCS[i].state == ProcessState::BlockedSend {
                    PROCS[i].state = ProcessState::Ready;
                    PROCS[i].context.a0 = 0; // send returns 0
                }
                // BlockedCall: leave state as BlockedCall, target stays set.
                // SYS_REPLY will unblock it later.

                // Receiver gets the message
                frame.a0 = msg;
                frame.a1 = sender;
                return;
            }
        }

        // No sender found — block the receiver
        PROCS[current].ipc_target = from_pid;
        PROCS[current].state = ProcessState::BlockedRecv;

        // Context switch to the next ready process
        schedule(frame, current);
    }
}

// ── IRQ Notification ──────────────────────────────────────────

/// Deliver an IRQ notification to the owning process.
///
/// If the process is in BlockedRecv: deliver immediately as a synthetic
/// IPC message (sender=0, msg=irq_num), mark Ready.
/// If not: set irq_pending flag — next SYS_RECEIVE returns immediately.
pub fn irq_notify(irq: u32, owner: usize) {
    unsafe {
        if PROCS[owner].state == ProcessState::BlockedRecv {
            // Deliver immediately — overwrite the saved context
            PROCS[owner].context.a0 = irq as usize;  // message = IRQ number
            PROCS[owner].context.a1 = 0;              // sender = kernel (PID 0)
            PROCS[owner].state = ProcessState::Ready;
        } else {
            // Process not ready to receive — queue for later
            PROCS[owner].irq_pending = true;
        }
    }
}
