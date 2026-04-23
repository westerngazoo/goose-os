/// GooseOS userspace syscall wrappers — netsrv edition.
///
/// Candidate for extraction into userspace/lib/gooseos once we have
/// two programs sharing this (today: hello + netsrv). See Ch 44
/// closing note for the planned refactor.
///
/// NOTE: this is a near-duplicate of userspace/hello/src/gooseos.rs.
/// Changes here need to be mirrored until the lib split happens.

use core::arch::asm;

// ── Syscall Numbers (must match kernel trap.rs) ──────────────

const SYS_PUTCHAR:      usize = 0;
const SYS_EXIT:         usize = 1;
const SYS_SEND:         usize = 2;
const SYS_RECEIVE:      usize = 3;
const SYS_CALL:         usize = 4;
const SYS_REPLY:        usize = 5;
const SYS_GETPID:       usize = 12;
const SYS_YIELD:        usize = 13;

const ERR: usize = usize::MAX;

// ── Syscall Wrappers ─────────────────────────────────────────

#[inline(always)]
pub fn putchar(c: u8) {
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_PUTCHAR,
            in("a0") c as usize,
            options(nostack),
        );
    }
}

#[inline(always)]
pub fn exit(code: usize) -> ! {
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_EXIT,
            in("a0") code,
            options(noreturn),
        );
    }
}

/// Standard 2-register receive. Returns (message, sender_pid).
///
/// Use this when the client is expected to pack everything into a0/a1.
/// For multi-arg protocols (like the net server's IPC), use
/// `receive_ext` below.
#[inline(always)]
pub fn receive(from: usize) -> (usize, usize) {
    let msg: usize;
    let sender: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_RECEIVE,
            inlateout("a0") from => msg,
            lateout("a1") sender,
            options(nostack),
        );
    }
    (msg, sender)
}

/// Extended receive that captures a2 and a3 alongside a0/a1.
///
/// The kernel's Phase B IPC (see kernel/src/ipc.rs) delivers a2 and a3
/// during rendezvous alongside the message value. This wrapper exposes
/// them to the receiver. Needed by servers whose protocol carries more
/// than one argument per request — like the net server's (opcode,
/// sender, arg1, arg2) convention.
///
/// Returns (message, sender_pid, arg2, arg3).
///
/// The extra arg4 (delivered via SYS_CALL's a4 to the kernel) is NOT
/// propagated to the receiver by the current kernel IPC — follow-up
/// #A on the netsrv migration plan. When that lands this wrapper will
/// gain a fourth return value.
#[inline(always)]
pub fn receive_ext(from: usize) -> (usize, usize, usize, usize) {
    let msg: usize;
    let sender: usize;
    let arg2: usize;
    let arg3: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_RECEIVE,
            inlateout("a0") from => msg,
            lateout("a1") sender,
            lateout("a2") arg2,
            lateout("a3") arg3,
            options(nostack),
        );
    }
    (msg, sender, arg2, arg3)
}

#[inline(always)]
pub fn reply(target: usize, val: usize) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_REPLY,
            inlateout("a0") target => ret,
            in("a1") val,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

#[inline(always)]
pub fn getpid() -> usize {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_GETPID,
            lateout("a0") ret,
            options(nostack),
        );
    }
    ret
}

#[inline(always)]
pub fn yield_() {
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_YIELD,
            options(nostack),
        );
    }
}

// ── Console Output (println! via SYS_PUTCHAR) ────────────────

struct SyscallWriter;

impl core::fmt::Write for SyscallWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            putchar(byte);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    SyscallWriter.write_fmt(args).unwrap();
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::gooseos::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
