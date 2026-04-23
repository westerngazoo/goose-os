/// GooseOS userspace syscall wrappers and runtime.
///
/// Provides safe Rust interfaces to all GooseOS syscalls,
/// plus println! macro for console output via SYS_PUTCHAR.

use core::arch::asm;

// ── Syscall Numbers (MUST MATCH kernel/src/abi.rs) ──────────────
// The canonical definition lives in kernel/src/abi.rs. When a shared
// `abi` crate is introduced via a Cargo workspace, these consts will
// become `use abi::*`. Until then: manual mirror.

const SYS_PUTCHAR: usize = 0;
const SYS_EXIT: usize = 1;
const SYS_SEND: usize = 2;
const SYS_RECEIVE: usize = 3;
const SYS_CALL: usize = 4;
const SYS_REPLY: usize = 5;
const SYS_MAP: usize = 6;
const SYS_UNMAP: usize = 7;
const SYS_ALLOC_PAGES: usize = 8;
const SYS_FREE_PAGES: usize = 9;
const SYS_SPAWN: usize = 10;
const SYS_WAIT: usize = 11;
const SYS_GETPID: usize = 12;
const SYS_YIELD: usize = 13;
const SYS_IRQ_REGISTER: usize = 14;
const SYS_IRQ_ACK: usize = 15;
const SYS_REBOOT: usize = 16;

// ── Error sentinel ───────────────────────────────────────────

const ERR: usize = usize::MAX;

// ── Syscall Wrappers ─────────────────────────────────────────

/// Write a single character to the console.
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

/// Terminate the current process. Does not return.
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

/// Synchronous send. Blocks until receiver calls receive.
#[inline(always)]
pub fn send(target: usize, msg: usize) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_SEND,
            inlateout("a0") target => ret,
            in("a1") msg,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Synchronous receive. Blocks until sender calls send.
/// Returns (message, sender_pid).
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

/// RPC call: send message, block until reply. Returns reply value.
#[inline(always)]
pub fn call(target: usize, msg: usize) -> Result<usize, ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_CALL,
            inlateout("a0") target => ret,
            in("a1") msg,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(ret) }
}

/// Reply to an RPC caller.
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

/// Map a physical page into the caller's address space.
/// flags: 0 = USER_RW, 1 = USER_RX
#[inline(always)]
pub fn map_page(phys: usize, virt: usize, flags: usize) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_MAP,
            inlateout("a0") phys => ret,
            in("a1") virt,
            in("a2") flags,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Remove a page mapping.
#[inline(always)]
pub fn unmap_page(virt: usize) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_UNMAP,
            inlateout("a0") virt => ret,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Allocate physical pages. Returns physical address.
#[inline(always)]
pub fn alloc_pages(count: usize) -> Result<usize, ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_ALLOC_PAGES,
            inlateout("a0") count => ret,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(ret) }
}

/// Free physical pages.
#[inline(always)]
pub fn free_pages(addr: usize, count: usize) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_FREE_PAGES,
            inlateout("a0") addr => ret,
            in("a1") count,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Spawn a new process from an ELF binary. Returns new PID.
#[inline(always)]
pub fn spawn(elf_ptr: *const u8, elf_len: usize) -> Result<usize, ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_SPAWN,
            inlateout("a0") elf_ptr as usize => ret,
            in("a1") elf_len,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(ret) }
}

/// Wait for a child process to exit. Returns exit code.
#[inline(always)]
pub fn wait(child_pid: usize) -> Result<usize, ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_WAIT,
            inlateout("a0") child_pid => ret,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(ret) }
}

/// Get the current process ID.
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

/// Voluntarily yield the timeslice.
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

/// Register to receive a hardware IRQ via IPC.
#[inline(always)]
pub fn irq_register(irq: u32) -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_IRQ_REGISTER,
            inlateout("a0") irq as usize => ret,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Acknowledge IRQ handling complete.
#[inline(always)]
pub fn irq_ack() -> Result<(), ()> {
    let ret: usize;
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_IRQ_ACK,
            lateout("a0") ret,
            options(nostack),
        );
    }
    if ret == ERR { Err(()) } else { Ok(()) }
}

/// Reboot the system. Does not return.
#[inline(always)]
pub fn reboot() -> ! {
    unsafe {
        asm!(
            "ecall",
            in("a7") SYS_REBOOT,
            options(noreturn),
        );
    }
}

// ── Network IPC (Phase B) ────────────────────────────────────
//
// The kernel net server lives at PID 3 and is invoked via SYS_CALL.
// Register convention:
//   a7 = SYS_CALL (4)
//   a0 = 3  (NET_SERVER_PID)  — returns result
//   a1 = opcode
//   a2 = arg1
//   a3 = arg2
//   a4 = arg3

#[cfg(feature = "net")]
pub mod net {
    use core::arch::asm;

    const NET_PID: usize = 3;
    const SYS_CALL: usize = 4;

    // Opcodes — must match kernel src/net.rs.
    pub const NET_STATUS: usize = 0;
    pub const NET_SOCKET_TCP: usize = 1;
    pub const NET_SOCKET_UDP: usize = 2;
    pub const NET_BIND: usize = 3;
    pub const NET_CONNECT: usize = 4;
    pub const NET_LISTEN: usize = 5;
    pub const NET_ACCEPT: usize = 6;
    pub const NET_SEND: usize = 7;
    pub const NET_RECV: usize = 8;
    pub const NET_CLOSE: usize = 9;

    const ERR: usize = usize::MAX;

    /// Raw 3-argument call into the net server.
    #[inline(always)]
    fn ncall(opcode: usize, a1: usize, a2: usize, a3: usize) -> Result<usize, ()> {
        let ret: usize;
        unsafe {
            asm!(
                "ecall",
                in("a7") SYS_CALL,
                inlateout("a0") NET_PID => ret,
                in("a1") opcode,
                in("a2") a1,
                in("a3") a2,
                in("a4") a3,
                options(nostack),
            );
        }
        if ret == ERR { Err(()) } else { Ok(ret) }
    }

    /// Query whether the network stack is up. Returns 1 if ready.
    pub fn status() -> Result<usize, ()> {
        ncall(NET_STATUS, 0, 0, 0)
    }

    /// Create a new UDP socket. Returns an opaque socket handle.
    pub fn socket_udp() -> Result<usize, ()> {
        ncall(NET_SOCKET_UDP, 0, 0, 0)
    }

    /// Bind a UDP socket to a local port.
    pub fn bind(handle: usize, port: u16) -> Result<(), ()> {
        ncall(NET_BIND, handle, port as usize, 0).map(|_| ())
    }

    /// Close a socket.
    pub fn close(handle: usize) -> Result<(), ()> {
        ncall(NET_CLOSE, handle, 0, 0).map(|_| ())
    }

    /// Create a new TCP socket. Returns an opaque socket handle (0..MAX_TCP).
    pub fn socket_tcp() -> Result<usize, ()> {
        ncall(NET_SOCKET_TCP, 0, 0, 0)
    }

    /// Listen on a TCP socket at the given port.
    pub fn listen(handle: usize, port: u16) -> Result<(), ()> {
        ncall(NET_LISTEN, handle, port as usize, 0).map(|_| ())
    }

    /// Pack an IPv4 dotted-quad into a single usize the kernel unpacks.
    #[inline(always)]
    fn pack_ip(ip: [u8; 4]) -> usize {
        ((ip[0] as usize) << 24)
            | ((ip[1] as usize) << 16)
            | ((ip[2] as usize) << 8)
            | (ip[3] as usize)
    }

    /// Connect a TCP socket to a remote endpoint.
    pub fn connect(handle: usize, ip: [u8; 4], port: u16) -> Result<(), ()> {
        // Protocol: a2=handle, a3=packed_ip, a4=port (handled by handle_connect).
        let packed = pack_ip(ip);
        ncall(NET_CONNECT, handle, packed, port as usize).map(|_| ())
    }

    /// Extended 6-arg call — needed for UDP send_to (needs dest IP + port).
    #[inline(always)]
    fn ncall6(
        opcode: usize,
        a1: usize, a2: usize, a3: usize, a4: usize, a5: usize,
    ) -> Result<usize, ()> {
        let ret: usize;
        unsafe {
            asm!(
                "ecall",
                in("a7") SYS_CALL,
                inlateout("a0") NET_PID => ret,
                in("a1") opcode,
                in("a2") a1,
                in("a3") a2,
                in("a4") a3,
                in("a5") a4,
                in("a6") a5,
                options(nostack),
            );
        }
        if ret == ERR { Err(()) } else { Ok(ret) }
    }

    /// Send a UDP datagram to (ip, port) from a bound UDP socket.
    /// Returns the number of bytes the stack accepted.
    pub fn send_udp_to(
        handle: usize,
        ip: [u8; 4],
        port: u16,
        data: &[u8],
    ) -> Result<usize, ()> {
        let packed = pack_ip(ip);
        // a2=handle, a3=buf_va, a4=len, a5=packed_ip, a6=port
        ncall6(NET_SEND, handle, data.as_ptr() as usize, data.len(), packed, port as usize)
    }

    /// Send on a connected TCP socket. Returns bytes written.
    pub fn send(handle: usize, data: &[u8]) -> Result<usize, ()> {
        // a5/a6 are zero for TCP.
        ncall6(NET_SEND, handle, data.as_ptr() as usize, data.len(), 0, 0)
    }

    /// Non-blocking receive. Returns bytes read (0 if nothing pending).
    pub fn recv(handle: usize, buf: &mut [u8]) -> Result<usize, ()> {
        ncall(NET_RECV, handle, buf.as_mut_ptr() as usize, buf.len())
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
