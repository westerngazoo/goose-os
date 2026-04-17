# Networking a Microkernel

> *"The network is the computer."* — John Gage, 1984.
> Thirty years later, the network is also the bug report.

## Why networking, and why now

By the end of Phase A, GooseOS was a small, honest RISC-V microkernel. It had
Sv39 paging, preemptive scheduling, seL4-style synchronous IPC, a WASM
interpreter, and enough discipline to keep its unsafe code in a handful of
files. What it couldn't do was talk to anything other than its own UART.

Phase B is about changing that — without compromising the properties that make
the kernel worth verifying. A TCP/IP stack is not a small thing. `smoltcp` is
~20,000 lines of carefully-written Rust, and every byte of it needs to be
reachable from a no_std, no_alloc environment. The question for this chapter
is not "can we run a network stack?" (we can; the ecosystem makes that easy)
but "can we run one without destroying the boundaries we drew in Phase A?".

## Four decisions that shape everything

Before any code, there are four decisions. Each has a cost; each has a
consequence for the rest of the kernel.

**1. Where does the stack live? In-kernel.**

The puritan choice is to run smoltcp as a userspace server, just like the UART
server at PID 2. Packets come in from the device, go through IPC into the
server, get parsed by smoltcp, and the server replies to client processes via
more IPC. It is microkernel-correct. It is also at least two shared-memory
primitives, a syscall, and a lot of copying away from working.

The pragmatic choice — and the one Phase B takes — is to run smoltcp inside
the kernel, with static buffers, and expose it to userspace through the IPC
system we already have. The kernel is still tiny. The network stack is one
more module. If I'm wrong about this, I'll move it out later; the IPC API at
the user boundary doesn't care where the implementation lives.

**2. What transport? VirtIO MMIO only.**

The QEMU `virt` machine exposes VirtIO devices through eight MMIO slots. No
PCI. No discovery tree. Just fixed addresses and a magic number. For a
microkernel that's identity-mapping its physical memory anyway, this is the
easiest thing in the world. PCI can wait for real hardware.

**3. What user API? IPC to a net server.**

The kernel dispatches `SYS_CALL` to a pseudo-PID (3) directly to a handler
function. No context switch, no second process; just a syscall that happens
to look like an IPC from the outside. This preserves the illusion that
networking is "something a server does", which means a future move to a true
userspace server is a matter of changing one dispatch site, not rewriting
client code.

**4. What testing? QEMU user-mode + TAP.**

`-netdev user` is the default: NAT'd through the host, no setup, works on any
dev machine. `-netdev tap` is the option for anyone who wants a real L2
connection. Both target the same kernel binary.

## The driver trait

Phase A had one driver abstraction: the `HostFunctions` trait for WASM
imports. There was no general notion of "a device". Phase B adds one.

```rust
pub trait Device {
    fn probe(base: usize) -> bool where Self: Sized;
    fn init(&mut self) -> Result<(), DriverError>;
    fn handle_interrupt(&mut self);
}

pub trait NetworkDevice: Device {
    fn mtu(&self) -> usize;
    fn mac_address(&self) -> [u8; 6];
    fn transmit(&mut self, data: &[u8]) -> Result<(), DriverError>;
    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, DriverError>;
    fn can_transmit(&self) -> bool;
    fn can_receive(&self) -> bool;
}
```

The trait is a pure module: no unsafe, no MMIO, no dependency on anything
else in the kernel. An implementor lives somewhere dirty — `virtio.rs`
touches MMIO — but the trait itself is host-testable. You can write a mock
`NetworkDevice` in five lines and stand up the stack in a unit test.

This is the pattern Phase A established and Phase B continues: pure traits
upstairs, unsafe integration downstairs, and a clear contract between them.

## VirtIO in four movements

VirtIO MMIO v2 is a surprisingly readable spec. The driver has four jobs.

**Movement 1: Probe.** Iterate the eight MMIO slots. For each, read the magic
value at offset 0 (should be `0x74726976` — ASCII "virt"), the version at
offset 4 (should be 2), and the device ID at offset 8 (should be 1, for
virtio-net). Stop at the first match.

The first time I ran this, it found nothing. QEMU silently presents VirtIO v1
(legacy) by default, not v2. Once I added `-global
virtio-mmio.force-legacy=false` to the QEMU arguments, slot 7 — not slot 0;
QEMU assigns devices to the highest slot first — lit up with
`magic=0x74726976 version=2 device_id=1`.

**Movement 2: Handshake.** Write `0` to status to reset. Write `ACK` to
acknowledge the device. Write `ACK | DRIVER` to claim it. Read the 64-bit
feature bitmap, write back the subset you support, write `FEATURES_OK`. Set
up the queues. Write `DRIVER_OK`. If at any point the device writes back
`FAILED`, bail.

**Movement 3: Virtqueues.** Each queue is three arrays: the descriptor table
(where buffers live), the available ring (which buffers the driver wants the
device to process), and the used ring (which buffers the device has
finished). Virtio-net uses two queues: queue 0 for RX, queue 1 for TX. All
three arrays per queue are statically allocated in `.bss`. No heap, no
runtime size.

The RX queue is pre-populated at init time: every descriptor points to an
empty buffer, and every buffer is in the available ring. The device fills
them as packets arrive.

**Movement 4: DMA.** VirtIO descriptors carry physical addresses. Since
GooseOS identity-maps the kernel, a static array's virtual address *is* its
physical address. No translation. One atomic `fence(SeqCst)` before notifying
the device, so the ring writes are visible before the notification.

## smoltcp, held at arm's length

smoltcp's `phy::Device` trait is not our `driver::NetworkDevice` trait. It
uses a token-based model: `receive()` returns a pair of tokens, each of which
can be "consumed" exactly once to read or write a packet. This matches
smoltcp's internal poll loop but imposes a particular shape on the adapter.

The adapter is small:

```rust
impl phy::Device for SmoltcpDevice {
    fn receive(&mut self, _t: Instant) -> Option<(RxToken, TxToken)> {
        let dev = unsafe { virtio::get() };
        (dev.can_receive() && dev.can_transmit())
            .then_some((VirtioRxToken, VirtioTxToken))
    }
    fn transmit(&mut self, _t: Instant) -> Option<TxToken> {
        unsafe { virtio::get() }.can_transmit().then_some(VirtioTxToken)
    }
    // ...
}
```

Each token, when consumed, stages a 1514-byte stack buffer, calls into the
VirtIO driver once, and is done. Copies are expensive in principle and cheap
in practice — we are not yet in the territory where zero-copy matters, and
the 1500-byte memcpy is measured in microseconds.

The socket set is allocated statically: four TCP slots with 4KB RX + 4KB TX
buffers each, four UDP slots with 2KB + 2KB. That's 64KB of socket buffer
storage in `.bss`. The `SocketSet::new(&mut SOCKET_STORAGE[..])` call takes a
mutable slice rather than allocating, which is exactly the affordance we
needed from a network stack that claims to be no_std.

The interface gets a static IP — 10.0.2.15/24, which is QEMU user-mode's
convention — and a default route to 10.0.2.2. DHCP can come later.

## How userspace talks to the stack

The IPC protocol is seL4-adjacent: `SYS_CALL(target, opcode, arg1, arg2, arg3)`
where `target` is PID 3 (the net server) and `opcode` is one of
`NET_SOCKET_TCP`, `NET_CONNECT`, `NET_SEND`, etc. The reply comes back in the
caller's `a0` register.

Extending the existing IPC to carry four argument registers instead of two
was a mechanical change — the `Process` struct gained two fields
(`ipc_arg2`, `ipc_arg3`) and the copy lines in `sys_call` got slightly
longer. Existing clients that only use `a0`/`a1` see zero-valued extra
registers and don't care.

The net server itself is not a separate process. It's a handler function
that the kernel dispatches to directly when it sees a `SYS_CALL` targeting
PID 3:

```rust
if target == NET_SERVER_PID {
    net::handle_request(frame);
} else {
    process::sys_call(frame);
}
```

This is cheaper than a context switch and avoids the chicken-and-egg problem
of getting a real process started before networking is up. If the stack
later moves to userspace, this dispatch point is the single thing that
changes.

## The first packet

The smoke test is a single function: allocate a UDP socket, bind it to port
12345, queue a small packet to 10.0.2.2:12345, poll once. Running this at
the end of kernel boot, with `-object filter-dump` attached to QEMU, produces
a pcap file — 360 bytes, containing six 42-byte frames.

Decoded by hand:

```
Ethernet: dst=ff:ff:ff:ff:ff:ff  src=52:54:00:12:34:56  ethertype=0806 (ARP)
ARP:      request  sender=52:54:00:12:34:56/10.0.2.15
                   target=00:00:00:00:00:00/10.0.2.2
```

Six of them, retries because QEMU's slirp handles IP-level proxying and is
selective about replying to ARP. But the point stands: smoltcp generated a
frame, handed it to a TX token, which copied it into a VirtIO buffer, which
wrote the descriptor ring, which notified the device, which handed the frame
to QEMU, which wrote it to the pcap. Every layer works.

## What's missing, and why that's fine

Phase B's boot path is complete and tested. The pieces that aren't yet in
place:

- **Userspace client.** No process yet makes `NET_SOCKET_TCP` calls. The
  infrastructure is there; a small test program can exercise it. (This is
  next on the list.)
- **Blocking operations.** `NET_CONNECT` and `NET_RECV` currently stub out
  the blocking path. The kernel will need a `BlockedNet` process state and a
  per-process "what am I waiting for" field. That is a small, well-scoped
  addition.
- **RX interrupt path.** The VirtIO RX IRQ fires and the handler runs, but
  smoltcp's view of incoming packets still depends on the 100ms timer poll.
  Wiring the IRQ handler to call `net::poll()` directly will cut receive
  latency by two orders of magnitude.

None of these change the architecture. They are filling in the trait, not
rethinking it — which is the sign that the architecture was right.

## What I learned

Three things are worth carrying forward.

**Diagnostics pay for themselves in seconds.** The probe loop printed one
line per slot. The instant the first QEMU run produced output, I could see
that slot 7 held the device and it was reporting v1. Without that line, I
would have stared at "No virtio-net device found" for an hour and written
three wrong hypotheses.

**The first integration point is always configuration.** The driver was
correct. The spec was respected. The kernel was wrong only in that it asked
QEMU for something QEMU doesn't ship by default. `force-legacy=false` is a
ten-character fix that would have taken an hour to find without the
diagnostic print.

**smoltcp is well-designed for no_std.** It did what it said it would.
`SocketSet::new(&mut [SocketStorage])`, static socket buffers, the phy::Device
trait — all of it composes cleanly into an identity-mapped, heapless kernel.
That is not nothing; a lot of "no_std" libraries quietly require `alloc`
somewhere in the call graph. smoltcp doesn't.

The kernel is 40% larger than it was yesterday. Almost none of that
increase is in the unsafe boundary. The driver trait is pure. The socket
buffers are `.bss`. The stack code is `smoltcp`'s, which is its own concern.
The parts that needed to be proven — Sv39, IPC, scheduling — are unchanged.

That is what "add networking without compromising the kernel" looks like on
the inside.
