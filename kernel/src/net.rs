/// Network stack — smoltcp integration with static buffers.
///
/// Phase B: Networking.
///
/// Provides:
///   - smoltcp Device implementation wrapping VirtIO-net
///   - Static socket set (4 TCP + 4 UDP)
///   - IP configuration (static 10.0.2.15/24 for QEMU user-mode)
///   - Poll function called from timer interrupt and VirtIO IRQ
///   - IPC-based network server (userspace calls PID 3 via SYS_CALL)

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet, SocketStorage};
use smoltcp::phy::{self, Medium};
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr, Ipv4Address};
use smoltcp::socket::tcp;
use smoltcp::socket::udp;
use smoltcp::time::Instant;

use crate::driver::NetworkDevice;
use crate::trap::TrapFrame;

// ── Network Server PID ───────────────────────────────────────

/// The network server is dispatched as kernel code when SYS_CALL targets this PID.
pub const NET_SERVER_PID: usize = 3;

// ── IPC Message Opcodes ──────────────────────────────────────

pub const NET_STATUS:     usize = 0;  // Query: is network up?
pub const NET_SOCKET_TCP: usize = 1;  // Create TCP socket → handle
pub const NET_SOCKET_UDP: usize = 2;  // Create UDP socket → handle
pub const NET_BIND:       usize = 3;  // Bind socket to port
pub const NET_CONNECT:    usize = 4;  // Connect TCP socket
pub const NET_LISTEN:     usize = 5;  // Listen on TCP socket
pub const NET_ACCEPT:     usize = 6;  // Accept TCP connection
pub const NET_SEND:       usize = 7;  // Send data
pub const NET_RECV:       usize = 8;  // Receive data
pub const NET_CLOSE:      usize = 9;  // Close socket

// ── Static Socket Storage ────────────────────────────────────

const MAX_TCP_SOCKETS: usize = 4;
const MAX_UDP_SOCKETS: usize = 4;
const MAX_SOCKETS: usize = MAX_TCP_SOCKETS + MAX_UDP_SOCKETS;
const TCP_RX_BUF_SIZE: usize = 4096;
const TCP_TX_BUF_SIZE: usize = 4096;
const UDP_RX_BUF_SIZE: usize = 2048;
const UDP_TX_BUF_SIZE: usize = 2048;
const UDP_RX_META_COUNT: usize = 4;
const UDP_TX_META_COUNT: usize = 4;

// Static buffers for TCP sockets
static mut TCP_RX_BUFS: [[u8; TCP_RX_BUF_SIZE]; MAX_TCP_SOCKETS] = [[0; TCP_RX_BUF_SIZE]; MAX_TCP_SOCKETS];
static mut TCP_TX_BUFS: [[u8; TCP_TX_BUF_SIZE]; MAX_TCP_SOCKETS] = [[0; TCP_TX_BUF_SIZE]; MAX_TCP_SOCKETS];

// Static buffers for UDP sockets
static mut UDP_RX_BUFS: [[u8; UDP_RX_BUF_SIZE]; MAX_UDP_SOCKETS] = [[0; UDP_RX_BUF_SIZE]; MAX_UDP_SOCKETS];
static mut UDP_TX_BUFS: [[u8; UDP_TX_BUF_SIZE]; MAX_UDP_SOCKETS] = [[0; UDP_TX_BUF_SIZE]; MAX_UDP_SOCKETS];
static mut UDP_RX_META: [[udp::PacketMetadata; UDP_RX_META_COUNT]; MAX_UDP_SOCKETS] =
    [[udp::PacketMetadata::EMPTY; UDP_RX_META_COUNT]; MAX_UDP_SOCKETS];
static mut UDP_TX_META: [[udp::PacketMetadata; UDP_TX_META_COUNT]; MAX_UDP_SOCKETS] =
    [[udp::PacketMetadata::EMPTY; UDP_TX_META_COUNT]; MAX_UDP_SOCKETS];

// Socket set storage
static mut SOCKET_STORAGE: [SocketStorage<'static>; MAX_SOCKETS] =
    [SocketStorage::EMPTY; MAX_SOCKETS];

// IP address is set during init(), not in a static initializer
// (IpCidr::new is not const)

// ── smoltcp PHY Device wrapper ───────────────────────────────

/// Wraps VirtIO-net for smoltcp's Device trait.
pub struct SmoltcpDevice;

/// RX token — consumes one received packet.
pub struct VirtioRxToken;

/// TX token — transmits one packet.
pub struct VirtioTxToken;

impl phy::Device for SmoltcpDevice {
    type RxToken<'a> = VirtioRxToken;
    type TxToken<'a> = VirtioTxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let dev = unsafe { crate::virtio::get() };
        if dev.can_receive() && dev.can_transmit() {
            Some((VirtioRxToken, VirtioTxToken))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        let dev = unsafe { crate::virtio::get() };
        if dev.can_transmit() {
            Some(VirtioTxToken)
        } else {
            None
        }
    }

    fn capabilities(&self) -> phy::DeviceCapabilities {
        let mut caps = phy::DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps
    }
}

impl phy::RxToken for VirtioRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        let mut buf = [0u8; 1514];
        let dev = unsafe { crate::virtio::get() };
        let len = dev.receive(&mut buf).unwrap_or(0);
        f(&buf[..len])
    }
}

impl phy::TxToken for VirtioTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 1514];
        let result = f(&mut buf[..len]);
        let dev = unsafe { crate::virtio::get() };
        let _ = dev.transmit(&buf[..len]);
        result
    }
}

// ── Global Network State ─────────────────────────────────────

static mut NET_DEVICE: SmoltcpDevice = SmoltcpDevice;
static mut NET_IFACE: Option<Interface> = None;
static mut NET_SOCKETS: Option<SocketSet<'static>> = None;
static mut NET_READY: bool = false;

// Socket handle tracking (maps user handle index → smoltcp SocketHandle)
static mut TCP_HANDLES: [Option<SocketHandle>; MAX_TCP_SOCKETS] = [None; MAX_TCP_SOCKETS];
static mut UDP_HANDLES: [Option<SocketHandle>; MAX_UDP_SOCKETS] = [None; MAX_UDP_SOCKETS];
static mut TCP_ALLOC_COUNT: usize = 0;
static mut UDP_ALLOC_COUNT: usize = 0;

// ── Initialization ───────────────────────────────────────────

/// Initialize the smoltcp network stack.
///
/// Call after VirtIO-net is initialized.
pub fn init() {
    let mac = unsafe { crate::virtio::get().mac_address() };
    let hw_addr = EthernetAddress(mac);

    let config = Config::new(hw_addr.into());

    // Create the interface with our device
    let mut iface = Interface::new(config, unsafe { &mut NET_DEVICE }, Instant::from_millis(0));

    // Set IP address: 10.0.2.15/24 (QEMU user-mode default)
    iface.update_ip_addrs(|addrs| {
        addrs.push(IpCidr::new(IpAddress::v4(10, 0, 2, 15), 24)).ok();
    });

    // Default gateway: 10.0.2.2 (QEMU user-mode)
    iface.routes_mut()
        .add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2))
        .ok();

    // Create socket set with static storage
    let sockets = SocketSet::new(unsafe { &mut SOCKET_STORAGE[..] });

    unsafe {
        NET_IFACE = Some(iface);
        NET_SOCKETS = Some(sockets);
        NET_READY = true;
    }
}

/// Check if the network stack is initialized.
pub fn is_ready() -> bool {
    unsafe { NET_READY }
}

/// Kernel smoke test: allocate a UDP socket, bind it, and queue an outgoing
/// packet to the gateway. Forces ARP + TX so the pcap shows traffic without
/// needing a userspace client.
///
/// Call after init(). Subsequent poll() calls will drive the state machine.
pub fn smoke_test() {
    // Allocate a UDP socket via the existing handler.
    let udp_handle = handle_socket_udp();
    if udp_handle == usize::MAX {
        crate::println!("  [net] smoke_test: UDP socket alloc failed");
        return;
    }

    // Bind the socket to a local port (required before send).
    if handle_bind(udp_handle, 12345) != 0 {
        crate::println!("  [net] smoke_test: bind failed");
        return;
    }

    // Directly queue a UDP packet to 10.0.2.2:12345 using smoltcp.
    unsafe {
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return,
        };
        let udp_idx = udp_handle - MAX_TCP_SOCKETS;
        let handle = match UDP_HANDLES[udp_idx] {
            Some(h) => h,
            None => return,
        };
        let socket = sockets.get_mut::<udp::Socket>(handle);
        let endpoint = smoltcp::wire::IpEndpoint {
            addr: IpAddress::v4(10, 0, 2, 2),
            port: 12345,
        };
        match socket.send_slice(b"GooseOS smoke\n", endpoint) {
            Ok(()) => crate::println!("  [net] smoke_test: UDP packet queued to 10.0.2.2:12345"),
            Err(_) => crate::println!("  [net] smoke_test: UDP send_slice failed"),
        }
    }

    // Poll once to push the packet out (will trigger ARP first).
    poll(0);
}

// ── Polling ──────────────────────────────────────────────────

/// Poll the network stack.
///
/// Called from timer interrupt (every ~100ms) and after VirtIO IRQ.
/// Drives smoltcp's internal state machine (ARP, TCP timers, etc.).
pub fn poll(timestamp_ms: i64) {
    let iface = match unsafe { NET_IFACE.as_mut() } {
        Some(i) => i,
        None => return,
    };
    let sockets = match unsafe { NET_SOCKETS.as_mut() } {
        Some(s) => s,
        None => return,
    };

    let instant = Instant::from_millis(timestamp_ms);
    iface.poll(instant, unsafe { &mut NET_DEVICE }, sockets);
}

// ── IPC Request Handler ──────────────────────────────────────

/// Handle a network IPC request from userspace.
///
/// Called from trap.rs when SYS_CALL targets NET_SERVER_PID.
/// Register convention:
///   a7 = SYS_CALL (4)
///   a0 = target PID (NET_SERVER_PID = 3)
///   a1 = opcode (NET_SOCKET_TCP, NET_CONNECT, etc.)
///   a2 = arg1 (socket handle, IP address, buffer VA, etc.)
///   a3 = arg2 (port, buffer length, etc.)
///   a4 = arg3 (additional argument)
///
/// Returns result in a0 (0 = success, usize::MAX = error).
pub fn handle_request(frame: &mut TrapFrame) {
    frame.sepc += 4; // Advance past ecall

    if !is_ready() {
        frame.a0 = usize::MAX;
        return;
    }

    let opcode = frame.a1;
    let arg1 = frame.a2;
    let arg2 = frame.a3;
    let _arg3 = frame.a4;

    match opcode {
        NET_STATUS => {
            frame.a0 = 1; // Network is up
        }
        NET_SOCKET_TCP => {
            frame.a0 = handle_socket_tcp();
        }
        NET_SOCKET_UDP => {
            frame.a0 = handle_socket_udp();
        }
        NET_BIND => {
            frame.a0 = handle_bind(arg1, arg2);
        }
        NET_CONNECT => {
            // arg1 = socket handle, arg2 = packed IPv4, arg3 = port
            frame.a0 = handle_connect(arg1, arg2, frame.a4);
        }
        NET_LISTEN => {
            frame.a0 = handle_listen(arg1, arg2);
        }
        NET_SEND => {
            // a2 = socket handle, a3 = buffer VA, a4 = length
            // For UDP: a5 = packed IPv4 destination, a6 = dest port
            // For TCP: a5/a6 are ignored (socket must be connected).
            frame.a0 = handle_send(frame, arg1, arg2, frame.a4, frame.a5, frame.a6);
        }
        NET_RECV => {
            // a2 = socket handle, a3 = buffer VA, a4 = max length
            frame.a0 = handle_recv(frame, arg1, arg2, frame.a4);
        }
        NET_CLOSE => {
            frame.a0 = handle_close(arg1);
        }
        _ => {
            frame.a0 = usize::MAX; // Unknown opcode
        }
    }
}

// ── Socket Operation Handlers ────────────────────────────────

fn handle_socket_tcp() -> usize {
    unsafe {
        if TCP_ALLOC_COUNT >= MAX_TCP_SOCKETS {
            return usize::MAX;
        }
        let idx = TCP_ALLOC_COUNT;
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        let rx_buf = tcp::SocketBuffer::new(&mut TCP_RX_BUFS[idx][..]);
        let tx_buf = tcp::SocketBuffer::new(&mut TCP_TX_BUFS[idx][..]);
        let socket = tcp::Socket::new(rx_buf, tx_buf);
        let handle = sockets.add(socket);
        TCP_HANDLES[idx] = Some(handle);
        TCP_ALLOC_COUNT += 1;

        // Return handle index (0-based, TCP handles are 0..MAX_TCP)
        idx
    }
}

fn handle_socket_udp() -> usize {
    unsafe {
        if UDP_ALLOC_COUNT >= MAX_UDP_SOCKETS {
            return usize::MAX;
        }
        let idx = UDP_ALLOC_COUNT;
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        let rx_buf = udp::PacketBuffer::new(
            &mut UDP_RX_META[idx][..],
            &mut UDP_RX_BUFS[idx][..],
        );
        let tx_buf = udp::PacketBuffer::new(
            &mut UDP_TX_META[idx][..],
            &mut UDP_TX_BUFS[idx][..],
        );
        let socket = udp::Socket::new(rx_buf, tx_buf);
        let handle = sockets.add(socket);
        UDP_HANDLES[idx] = Some(handle);
        UDP_ALLOC_COUNT += 1;

        // Return handle index (TCP count + idx for UDP distinction)
        MAX_TCP_SOCKETS + idx
    }
}

fn handle_bind(handle_idx: usize, port: usize) -> usize {
    unsafe {
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        if handle_idx >= MAX_TCP_SOCKETS {
            // UDP bind
            let udp_idx = handle_idx - MAX_TCP_SOCKETS;
            if udp_idx >= MAX_UDP_SOCKETS {
                return usize::MAX;
            }
            let handle = match UDP_HANDLES[udp_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<udp::Socket>(handle);
            if socket.bind(port as u16).is_err() {
                return usize::MAX;
            }
            0
        } else {
            // TCP doesn't have a standalone bind — it's part of listen/connect
            0
        }
    }
}

fn handle_connect(handle_idx: usize, packed_ip: usize, port: usize) -> usize {
    unsafe {
        if handle_idx >= MAX_TCP_SOCKETS {
            return usize::MAX;
        }
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };
        let iface = match NET_IFACE.as_mut() {
            Some(i) => i,
            None => return usize::MAX,
        };
        let handle = match TCP_HANDLES[handle_idx] {
            Some(h) => h,
            None => return usize::MAX,
        };

        // Unpack IPv4 address from u32
        let ip = Ipv4Address::new(
            ((packed_ip >> 24) & 0xFF) as u8,
            ((packed_ip >> 16) & 0xFF) as u8,
            ((packed_ip >> 8) & 0xFF) as u8,
            (packed_ip & 0xFF) as u8,
        );

        let socket = sockets.get_mut::<tcp::Socket>(handle);
        let cx = iface.context();
        if socket.connect(cx, (IpAddress::Ipv4(ip), port as u16), 49152 + handle_idx as u16).is_err() {
            return usize::MAX;
        }
        0
    }
}

fn handle_listen(handle_idx: usize, port: usize) -> usize {
    unsafe {
        if handle_idx >= MAX_TCP_SOCKETS {
            return usize::MAX;
        }
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };
        let handle = match TCP_HANDLES[handle_idx] {
            Some(h) => h,
            None => return usize::MAX,
        };

        let socket = sockets.get_mut::<tcp::Socket>(handle);
        if socket.listen(port as u16).is_err() {
            return usize::MAX;
        }
        0
    }
}

// Kernel staging buffer for user-buffer send/recv copies.
// Sized to one page (4KB) — matches the single-page limit in
// kvm::copy_{from,to}_user. Callers must chunk larger transfers.
const STAGING_SIZE: usize = 4096;
static mut STAGING: [u8; STAGING_SIZE] = [0; STAGING_SIZE];

/// Monotonic timestamp in milliseconds for smoltcp.
/// Derived from the kernel's 10ms preemption tick.
#[inline]
fn now_ms() -> i64 {
    (crate::trap::ticks() as i64) * 10
}

fn handle_send(
    _frame: &TrapFrame,
    handle_idx: usize,
    buf_va: usize,
    len: usize,
    packed_ip: usize,
    port: usize,
) -> usize {
    if len == 0 || len > STAGING_SIZE {
        return usize::MAX;
    }
    let satp = crate::process::current_satp();
    if satp == 0 {
        return usize::MAX;
    }

    // Copy user payload into kernel staging buffer.
    let staging = unsafe { &mut STAGING[..len] };
    if crate::kvm::copy_from_user(satp, buf_va, staging).is_err() {
        return usize::MAX;
    }

    let sent = unsafe {
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        if handle_idx < MAX_TCP_SOCKETS {
            // TCP send — socket must already be connected.
            let handle = match TCP_HANDLES[handle_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if !socket.may_send() {
                return usize::MAX;
            }
            match socket.send_slice(staging) {
                Ok(n) => n,
                Err(_) => return usize::MAX,
            }
        } else {
            // UDP send_to — destination comes from a5/a6.
            let udp_idx = handle_idx - MAX_TCP_SOCKETS;
            if udp_idx >= MAX_UDP_SOCKETS {
                return usize::MAX;
            }
            let handle = match UDP_HANDLES[udp_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<udp::Socket>(handle);
            let ip = Ipv4Address::new(
                ((packed_ip >> 24) & 0xFF) as u8,
                ((packed_ip >> 16) & 0xFF) as u8,
                ((packed_ip >> 8)  & 0xFF) as u8,
                ( packed_ip        & 0xFF) as u8,
            );
            let endpoint = smoltcp::wire::IpEndpoint {
                addr: IpAddress::Ipv4(ip),
                port: port as u16,
            };
            match socket.send_slice(staging, endpoint) {
                Ok(()) => len,
                Err(_) => return usize::MAX,
            }
        }
    };

    // Drive the stack so the packet actually leaves. Use a real monotonic
    // timestamp so smoltcp's ARP + retransmit logic advances correctly.
    poll(now_ms());

    sent
}

fn handle_recv(
    _frame: &TrapFrame,
    handle_idx: usize,
    buf_va: usize,
    max_len: usize,
) -> usize {
    if max_len == 0 || max_len > STAGING_SIZE {
        return usize::MAX;
    }
    let satp = crate::process::current_satp();
    if satp == 0 {
        return usize::MAX;
    }

    // First poll so any newly-arrived RX packet is processed into the socket.
    poll(now_ms());

    let staging = unsafe { &mut STAGING[..max_len] };
    let copied = unsafe {
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        if handle_idx < MAX_TCP_SOCKETS {
            let handle = match TCP_HANDLES[handle_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if !socket.may_recv() {
                // Not connected yet — return 0 (would-block), not error.
                return 0;
            }
            match socket.recv_slice(staging) {
                Ok(n) => n,
                Err(_) => return usize::MAX,
            }
        } else {
            let udp_idx = handle_idx - MAX_TCP_SOCKETS;
            if udp_idx >= MAX_UDP_SOCKETS {
                return usize::MAX;
            }
            let handle = match UDP_HANDLES[udp_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<udp::Socket>(handle);
            match socket.recv_slice(staging) {
                Ok((n, _endpoint)) => n,
                Err(smoltcp::socket::udp::RecvError::Exhausted) => 0,
                Err(_) => return usize::MAX,
            }
        }
    };

    if copied == 0 {
        return 0;
    }

    // Copy back to user.
    if crate::kvm::copy_to_user(satp, buf_va, &unsafe { &STAGING[..] }[..copied]).is_err() {
        return usize::MAX;
    }
    copied
}

fn handle_close(handle_idx: usize) -> usize {
    unsafe {
        let sockets = match NET_SOCKETS.as_mut() {
            Some(s) => s,
            None => return usize::MAX,
        };

        if handle_idx < MAX_TCP_SOCKETS {
            let handle = match TCP_HANDLES[handle_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            socket.close();
            0
        } else {
            let udp_idx = handle_idx - MAX_TCP_SOCKETS;
            if udp_idx >= MAX_UDP_SOCKETS {
                return usize::MAX;
            }
            let handle = match UDP_HANDLES[udp_idx] {
                Some(h) => h,
                None => return usize::MAX,
            };
            let socket = sockets.get_mut::<udp::Socket>(handle);
            socket.close();
            0
        }
    }
}
