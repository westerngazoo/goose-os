/// Process management — process table, IPC, context switching.
///
/// Phase 9: Call/Reply IPC (seL4-style RPC).
///
/// Design:
///   - Fixed-size process table (no dynamic allocation in kernel)
///   - Synchronous send/receive: sender blocks until receiver picks up
///   - Call/Reply: RPC in one ecall (send + wait for reply)
///   - Context switch via TrapFrame save/restore on the kernel stack
///   - Each process has its own Sv39 page table
///
/// IPC model:
///   SYS_SEND(target, msg)   — blocks until target calls RECEIVE
///   SYS_RECEIVE(from)       — blocks until someone sends to us
///   SYS_CALL(target, msg)   — send + block for SYS_REPLY (RPC)
///   SYS_REPLY(target, reply)— deliver reply to BlockedCall process (non-blocking)
///   Rendezvous: when send and receive meet, message transfers, both unblock.
///   This is the seL4 model — no kernel-side message queues, no allocation.

use core::arch::{asm, global_asm};
use crate::page_alloc::{BitmapAllocator, PAGE_SIZE};
use crate::page_table::*;
use crate::trap::TrapFrame;
use crate::kvm;
use crate::println;

// ── Constants ──────────────────────────────────────────────────

const MAX_PROCS: usize = 8;

// ── Process State Machine ──────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessState {
    Free,           // Slot is unused
    Ready,          // Can be scheduled
    Running,        // Currently executing
    BlockedSend,    // Waiting for receiver to call RECEIVE
    BlockedRecv,    // Waiting for sender to call SEND
    BlockedCall,    // Sent via SYS_CALL, waiting for SYS_REPLY
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
}

impl Process {
    const fn empty() -> Self {
        Process {
            pid: 0,
            state: ProcessState::Free,
            satp: 0,
            context: TrapFrame::zero(),
            ipc_target: 0,
            ipc_value: 0,
        }
    }
}

// ── Process Table (kernel global state) ────────────────────────

/// Process table — fixed size, no heap allocation.
/// PID 0 is reserved (kernel). Processes use PIDs 1..MAX_PROCS-1.
static mut PROCS: [Process; MAX_PROCS] = [Process::empty(); MAX_PROCS];

/// PID of the currently running process.
static mut CURRENT_PID: usize = 0;

// ── Embedded User Programs ─────────────────────────────────────

// Program 1: init (PID 1) — RPC client
// Uses SYS_CALL to send "Honk! RPC works!\n" to PID 2, one char per call.
// Each SYS_CALL blocks until the server replies — true RPC.
// Then exits with code 0.
global_asm!(r#"
.section .text
.balign 4
.global _user_init_start
.global _user_init_end

_user_init_start:
    # ─── GooseOS init process (PID 1) ───
    # RPC client: calls server (PID 2) with each character via SYS_CALL.
    # SYS_CALL: a7=4, a0=target PID, a1=message value
    # Returns: a0=reply value (0 = ACK from server)

    li      s0, 2           # target PID (s0 survives ecalls)

    li a7, 4
    mv a0, s0
    li a1, 0x48             # 'H'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x6F             # 'o'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x6E             # 'n'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x6B             # 'k'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x21             # '!'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x20             # ' '
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x52             # 'R'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x50             # 'P'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x43             # 'C'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x20             # ' '
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x77             # 'w'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x6F             # 'o'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x72             # 'r'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x6B             # 'k'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x73             # 's'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x21             # '!'
    ecall
    li a7, 4
    mv a0, s0
    li a1, 0x0A             # '\n'
    ecall

    # Exit
    li      a7, 1           # SYS_EXIT
    li      a0, 0           # exit code 0
    ecall

1:  j       1b
_user_init_end:
"#);

// Program 2: UART server (PID 2) — RPC server
// Infinite loop: RECEIVE a call, print character via PUTCHAR, REPLY to caller.
// This is the server side of the SYS_CALL/SYS_REPLY RPC pattern.
global_asm!(r#"
.section .text
.balign 4
.global _user_srv_start
.global _user_srv_end

_user_srv_start:
    # ─── GooseOS UART server (PID 2) ───
    # RPC server: receives messages, prints them, replies with ACK.
    # SYS_RECEIVE: a7=3, a0=from_pid (0=any)
    # Returns: a0=message value, a1=sender PID
    # SYS_REPLY: a7=5, a0=caller PID, a1=reply value
1:
    li      a7, 3           # SYS_RECEIVE
    li      a0, 0           # from any sender
    ecall
    # a0 = character, a1 = sender PID

    mv      s0, a0          # save character
    mv      s1, a1          # save sender PID (need it for REPLY)

    li      a7, 0           # SYS_PUTCHAR
    mv      a0, s0          # the character
    ecall

    li      a7, 5           # SYS_REPLY
    mv      a0, s1          # reply to the caller
    li      a1, 0           # reply value = 0 (ACK)
    ecall

    j       1b              # loop forever

_user_srv_end:
"#);

// ── Process Creation ───────────────────────────────────────────

/// Create a new process from an embedded program.
///
/// Allocates:
///   - 1 page for user code (copied from kernel .text)
///   - 1 page for user stack
///   - N pages for user page table (kernel regions + user pages)
///
/// Sets up initial context so first context-switch srets to user entry.
fn create_process(
    pid: usize,
    code_start: usize,
    code_size: usize,
    alloc: &mut BitmapAllocator,
) {
    assert!(pid > 0 && pid < MAX_PROCS, "invalid PID");

    // Allocate user code page and copy program
    let user_code = kvm::alloc_zeroed_page(alloc);
    unsafe {
        let src = code_start as *const u8;
        let dst = user_code as *mut u8;
        for i in 0..code_size {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
        }
    }

    // Allocate user stack (one page, sp starts at top)
    let user_stack = kvm::alloc_zeroed_page(alloc);
    let user_sp = user_stack + PAGE_SIZE;

    // Build user page table
    let user_root = kvm::alloc_zeroed_page(alloc);
    kvm::map_kernel_regions(user_root, alloc);
    kvm::map_range(user_root, user_code, user_code + PAGE_SIZE, USER_RX, alloc);
    kvm::map_range(user_root, user_stack, user_stack + PAGE_SIZE, USER_RW, alloc);

    let satp = make_satp(user_root, pid as u16);

    // Initial context: U-mode, interrupts enabled, entry at code page
    let mut ctx = TrapFrame::zero();
    ctx.sepc = user_code;       // entry point
    ctx.sp = user_sp;           // user stack top
    ctx.sstatus = 1 << 5;       // SPIE=1 (enable interrupts on sret), SPP=0 (U-mode)

    unsafe {
        PROCS[pid] = Process {
            pid,
            state: ProcessState::Ready,
            satp,
            context: ctx,
            ipc_target: 0,
            ipc_value: 0,
        };
    }

    println!("  [proc] PID {} created (code={:#x}, {}) bytes, sp={:#x}, satp={:#018x})",
        pid, user_code, code_size, user_sp, satp);
}

// ── Boot: Create Processes and Launch ──────────────────────────

/// Create all initial processes and launch the first one.
///
/// This is called from kmain as Phase 9. It never returns —
/// after all processes exit, control goes to post_process_exit().
pub fn launch(alloc: &mut BitmapAllocator) -> ! {
    extern "C" {
        static _user_init_start: u8;
        static _user_init_end: u8;
        static _user_srv_start: u8;
        static _user_srv_end: u8;
    }

    let init_start = unsafe { &_user_init_start as *const u8 as usize };
    let init_size = unsafe { &_user_init_end as *const u8 as usize } - init_start;

    let srv_start = unsafe { &_user_srv_start as *const u8 as usize };
    let srv_size = unsafe { &_user_srv_end as *const u8 as usize } - srv_start;

    println!("  [proc] Creating processes...");

    create_process(1, init_start, init_size, alloc);
    create_process(2, srv_start, srv_size, alloc);

    println!();
    println!("  [page_alloc] {} pages used, {} free",
        alloc.allocated_count(), alloc.free_count());
    println!();

    // Launch PID 1 as the first running process
    let proc1 = unsafe { &PROCS[1] };
    unsafe {
        CURRENT_PID = 1;
        PROCS[1].state = ProcessState::Running;
    }

    let entry = proc1.context.sepc;
    let user_sp = proc1.context.sp;
    let satp = proc1.satp;

    println!("  [proc] Launching PID 1 (init)...");
    println!();

    unsafe {
        asm!(
            "csrw sepc, {entry}",
            "csrr t0, sstatus",
            "li t1, -257",              // clear SPP (bit 8)
            "and t0, t0, t1",
            "ori t0, t0, 32",           // set SPIE (bit 5)
            "csrw sstatus, t0",
            "csrw sscratch, sp",        // save kernel sp
            "csrw satp, {satp}",
            "sfence.vma zero, zero",
            "mv sp, {user_sp}",
            "sret",
            entry = in(reg) entry,
            satp = in(reg) satp,
            user_sp = in(reg) user_sp,
            options(noreturn),
        );
    }
}

// ── Syscall Handlers ───────────────────────────────────────────

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
    if target_pid == 0 || target_pid >= MAX_PROCS || target_pid == current {
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
            PROCS[target_pid].state = ProcessState::Ready;
        }

        // Caller ALWAYS blocks — even after rendezvous.
        // The difference from SYS_SEND: SYS_SEND unblocks on rendezvous,
        // SYS_CALL stays blocked waiting for SYS_REPLY.
        PROCS[current].ipc_target = target_pid;
        PROCS[current].ipc_value = msg_value;
        PROCS[current].state = ProcessState::BlockedCall;

        schedule(frame, current);
    }
}

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
    if target_pid == 0 || target_pid >= MAX_PROCS || target_pid == current {
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
    if target_pid == 0 || target_pid >= MAX_PROCS || target_pid == current {
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

/// SYS_EXIT(code) — terminate the current process.
///
/// Frees the process slot and switches to the next ready process.
/// If no processes remain, returns to kernel idle loop.
pub fn sys_exit(frame: &mut TrapFrame) {
    let current = unsafe { CURRENT_PID };
    let exit_code = frame.a0;

    println!();
    println!("  [kernel] PID {} exited with code {}", current, exit_code);

    unsafe {
        PROCS[current].state = ProcessState::Free;

        // Find next ready process
        let mut next = 0;
        for i in 1..MAX_PROCS {
            if PROCS[i].state == ProcessState::Ready {
                next = i;
                break;
            }
        }

        if next == 0 {
            // No runnable processes — return to kernel
            println!("  [kernel] All processes finished.");

            let kernel_satp = crate::kvm::kernel_satp();
            asm!(
                "csrw satp, {0}",
                "sfence.vma zero, zero",
                in(reg) kernel_satp,
            );

            // Rewrite frame to return to S-mode at post_process_exit
            frame.sstatus |= 1 << 8;  // SPP = S-mode
            frame.sstatus |= 1 << 5;  // SPIE = 1
            frame.sepc = crate::trap::post_process_exit as *const () as usize;
            return;
        }

        // Switch to next process
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

// ── Scheduler ──────────────────────────────────────────────────

/// Save current process and switch to the next ready process.
///
/// Called when a process blocks (SEND/RECEIVE with no rendezvous).
/// The frame on the kernel stack is overwritten with the next process's
/// saved context. When we return to trap.S, it restores and srets to
/// the next process. Elegant — no special context switch code needed.
unsafe fn schedule(frame: &mut TrapFrame, blocked_pid: usize) {
    // Save current process's registers
    PROCS[blocked_pid].context = *frame;

    // Find next ready process (simple linear scan)
    let mut next = 0;
    for i in 1..MAX_PROCS {
        if PROCS[i].state == ProcessState::Ready {
            next = i;
            break;
        }
    }

    if next == 0 {
        // All processes blocked — deadlock
        panic!("Deadlock: no runnable processes (PID {} blocked)", blocked_pid);
    }

    // Load next process's context onto the kernel stack
    *frame = PROCS[next].context;
    PROCS[next].state = ProcessState::Running;
    CURRENT_PID = next;

    // Switch page table
    let next_satp = PROCS[next].satp;
    asm!(
        "csrw satp, {0}",
        "sfence.vma zero, zero",
        in(reg) next_satp,
    );
}
