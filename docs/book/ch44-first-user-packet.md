---
sidebar_position: 4
sidebar_label: "Ch 44: The First User Packet"
title: "Chapter 44: The First User Packet — Closing the Microkernel Loop"
---

# Chapter 44: The First User Packet — Closing the Microkernel Loop

Chapter 43 ended with the goose online. That was a useful lie. What came online was *the kernel*. The VirtIO driver moved an Ethernet frame, smoltcp generated an ARP request, and QEMU's `filter-dump` caught it in a pcap. PID 1 — the Rust userspace program that calls itself "hello" — was still just printing to the UART. It had never so much as looked at a socket.

This chapter closes that loop. By the end of it, PID 1 will walk through the socket lifecycle — status, open, bind, close — using the IPC protocol from Chapter 43, and every step will print its result to the console. When the last line reads `[net-test] PASS`, the pipeline from user `ecall` to smoltcp and back has been exercised for real.

## Why UDP Before TCP

We have four socket opcodes that actually return a handle: `NET_SOCKET_TCP` and `NET_SOCKET_UDP`. The natural instinct is to reach for TCP — it's the famous one, the one every tutorial demonstrates, the one with the three-way handshake we could watch in Wireshark.

We are not going to do that. Not first.

TCP's `connect()` opens a socket, sends a SYN, waits for SYN/ACK, sends an ACK. Each of those waits is a kernel-side blocking operation we have not written yet — `NET_CONNECT` currently stubs out its blocking path, which means the first TCP call from userspace would either spin or panic. Either failure mode tells us nothing about whether the IPC pipeline works. A bug in the blocking code would look identical to a bug in the argument decoding, which would look identical to a bug in register passing.

UDP has no handshake. `socket_udp()` allocates a socket from the static pool and returns a handle. `bind()` associates a local port. `close()` releases the slot. None of it waits on the network. None of it needs a process state we do not have yet. Every step that succeeds proves one more layer of the pipeline, with exactly one variable in motion at a time.

> :sharpgoose: When you debug a stack, test the *layer*, not the *feature*. TCP is a feature — reliable byte-streams over an unreliable network. UDP is closer to the layer itself — it is what the socket API looks like with the reliability peeled away. If the IPC layer is broken, UDP will show it just as clearly as TCP, and without the second variable in the way.
>
> :sarcasticgoose: "But TCP is the real test." Sure. And the real test of a new Boeing is carrying four hundred passengers across the Atlantic. Before that, they taxi it around the runway. Taxi first. Atlantic second.

## The Userspace Net Module

The `hello` program already has a `gooseos` module — a thin layer of syscall wrappers around `ecall`, one function per syscall number. `putchar`, `exit`, `send`, `receive`, `getpid`, and so on. Each wrapper is a few lines of inline assembly, and together they are the entire surface between the Rust userspace and the kernel.

Adding networking to this module is not a new pattern. It is the existing pattern, extended one more time:

```rust title="userspace/hello/src/gooseos.rs — net submodule"
pub mod net {
    use core::arch::asm;

    const NET_PID: usize = 3;
    const SYS_CALL: usize = 4;

    pub const NET_STATUS:     usize = 0;
    pub const NET_SOCKET_TCP: usize = 1;
    pub const NET_SOCKET_UDP: usize = 2;
    pub const NET_BIND:       usize = 3;
    pub const NET_CONNECT:    usize = 4;
    // ... through NET_CLOSE = 9

    const ERR: usize = usize::MAX;

    #[inline(always)]
    fn ncall(opcode: usize, a1: usize, a2: usize, a3: usize)
        -> Result<usize, ()>
    {
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

    pub fn status()     -> Result<usize, ()> { ncall(NET_STATUS,     0, 0, 0) }
    pub fn socket_udp() -> Result<usize, ()> { ncall(NET_SOCKET_UDP, 0, 0, 0) }

    pub fn bind(handle: usize, port: u16) -> Result<(), ()> {
        ncall(NET_BIND, handle, port as usize, 0).map(|_| ())
    }
    pub fn close(handle: usize) -> Result<(), ()> {
        ncall(NET_CLOSE, handle, 0, 0).map(|_| ())
    }
}
```

Every net call routes through one function, `ncall`. One inline `ecall`. `a7` carries `SYS_CALL`. `a0` goes in as the target PID and comes back as the result. `a1` through `a4` carry the opcode and up to three arguments. A return of `usize::MAX` becomes `Err(())`; anything else becomes `Ok(value)`.

> :nerdygoose: The `inlateout("a0") NET_PID => ret` syntax is the whole reason this module is clean. `a0` is both an input (the target PID, written before `ecall`) and an output (the result, read after). LLVM emits exactly one register use, no redundant move. In C you would either manually clobber, cast through inline-asm outputs, or give up and write the whole thing in a separate assembly file. Rust's inline asm borrows the best of both — type-checked inputs, explicit register naming, and `inlateout` for the register-reuse case.
>
> :happygoose: `ERR = usize::MAX` is not elegant, but it is honest. The kernel writes `usize::MAX` on any error; the userspace maps it to `Err(())`. One sentinel, one branch. No errno. No errno-like global. No "last error" lookup. When we want richer errors later, we change the kernel to return a small negative number and the wrapper to match on ranges. For now, success or failure is enough.

## The Test Itself

`main.rs` becomes the story. Four operations, each with a `println!` before and a `println!` after:

```rust title="userspace/hello/src/main.rs — net-test body"
#[no_mangle]
pub extern "C" fn main() {
    println!("Hello from Rust userspace!");
    println!("My PID is {}", gooseos::getpid());

    println!("[net-test] Calling NET_STATUS...");
    match gooseos::net::status() {
        Ok(v)  => println!("[net-test] net up (status={})", v),
        Err(_) => { println!("[net-test] net down — bailing"); gooseos::exit(1); }
    }

    println!("[net-test] Opening UDP socket...");
    let handle = match gooseos::net::socket_udp() {
        Ok(h)  => { println!("[net-test] got UDP handle {}", h); h }
        Err(_) => { println!("[net-test] socket_udp FAILED"); gooseos::exit(1); }
    };

    println!("[net-test] Binding handle {} to port 9999...", handle);
    match gooseos::net::bind(handle, 9999) {
        Ok(()) => println!("[net-test] bound OK"),
        Err(_) => { println!("[net-test] bind FAILED"); gooseos::exit(1); }
    }

    println!("[net-test] Closing handle {}...", handle);
    match gooseos::net::close(handle) {
        Ok(()) => println!("[net-test] close OK"),
        Err(_) => println!("[net-test] close FAILED"),
    }

    println!("[net-test] PASS");
    gooseos::exit(0);
}
```

No state machine. No error recovery. Each failure path `exit(1)`s immediately. This is a smoke test, not a production network client — its job is to tell us, in the span of a QEMU boot, whether every stage of the pipeline works.

> :weightliftinggoose: The `println!` before the call and the `println!` after the call are not redundant. If a syscall hangs, the "before" line is on the screen and the "after" line is not — you know exactly which call is stuck. If a syscall returns garbage, the "after" line tells you what it got. Logging flanks. Always.

## The Handle-5 Surprise

The first successful run produces this:

```
Hello from Rust userspace!
My PID is 1
[net-test] Calling NET_STATUS...
[net-test] net up (status=1)
[net-test] Opening UDP socket...
[net-test] got UDP handle 5
[net-test] Binding handle 5 to port 9999...
[net-test] bound OK
[net-test] Closing handle 5...
[net-test] close OK
[net-test] PASS
```

Handle 5. Not handle 0. Not handle 4 (the first UDP slot, the one that sits right after the four TCP slots). *Handle 5.*

The reason is in the kernel's boot sequence. When the `net` feature is enabled, `net::init()` runs a `smoke_test()` — exactly the pcap-producing call from Chapter 42 that proves smoltcp can generate an ARP request. That smoke test opens a UDP socket and does not free it, because at the time it was written there was nothing to free it *to*. It holds handle 4.

PID 1 starts, asks for a UDP socket, and gets the next available slot: 5.

> :surprisedgoose: This looked like a bug for about thirty seconds. "Handle 5? There are four TCP slots and four UDP slots. Why is my first UDP handle 5?" Then: oh. The kernel already opened one. The kernel is, in its own way, PID ∞ — it holds a socket that nobody else can see and nobody thought to close. Nothing is broken. Something is just not yet elegant.
>
> :angrygoose: The kernel leaking a socket is fine for a smoke test and not fine for a real system. Socket deallocation after `NET_CLOSE` is still on the backlog. When it lands, the kernel's smoke test will close its socket when it is done, and the first user UDP handle will be 4 again. Leaks are small lies that cost nothing until they do.

## The Makefile Target

Running this end-to-end needs three things at once: the Rust userspace compiled, the kernel compiled with both `rust-user` and `net` features, and QEMU launched with virtio-net plus a packet filter. `make test-net-user` bundles all three:

```makefile title="Makefile — test-net-user"
test-net-user: build-user
	cd kernel && GOOSE_BUILD=$(NEXT_BUILD) cargo build --release \
	    --features "qemu rust-user net" --no-default-features
	@echo $(NEXT_BUILD) > $(BUILD_FILE)
	@echo "=== Rust Userspace Net Test ==="
	timeout 6 $(QEMU) $(QEMU_ARGS) $(NET_ARGS) \
	    -object filter-dump,id=f1,netdev=net0,file=build/goose-net.pcap \
	    -kernel $(KERNEL_ELF) || true
```

`build-user` compiles `userspace/hello` to `hello.elf`, which the kernel's `include_bytes!` embeds into its data segment at compile time. The `rust-user` feature tells the kernel to spawn PID 1 from that embedded ELF instead of a hand-written syscall test. The `net` feature brings in smoltcp and the VirtIO driver. `NET_ARGS` attaches virtio-net to QEMU with user-mode NAT. `filter-dump` writes every frame to `build/goose-net.pcap`.

Six seconds is enough for the whole pipeline: kernel boots, VirtIO probes, smoltcp comes up, PID 1 starts, the four calls go through, PID 1 exits 0. Any longer and you are waiting on slirp's own gravity, not on GooseOS.

> :nerdygoose: `filter-dump` is QEMU's answer to "I want Wireshark, but on the wire between guest and host, not on either side." It sits in the netdev chain and writes raw frames to a pcap-format file. Open it in Wireshark, read it with `tcpdump -r`, or decode it by hand like we did in Chapter 42. It captures what the driver sent and what the host returned — if a frame did not make it to the pcap, the driver never handed it off. If it made it to the pcap and no reply came back, the host side ignored it. The diagnostic resolution is brutal and exact.

## What Just Happened, End to End

Each of the four user-level calls traces the same path through the system, with different opcodes and arguments:

```
userspace/hello/src/main.rs
    |  gooseos::net::socket_udp()
    v
userspace/hello/src/gooseos.rs
    |  ncall(NET_SOCKET_UDP, 0, 0, 0)
    |  ecall with a7=SYS_CALL, a0=3, a1=2
    v
----- user/kernel boundary (ecall traps to S-mode) -----
    |
    v
kernel/src/trap.rs: handle_ecall
    |  SYS_CALL dispatch; a0 == NET_SERVER_PID
    v
kernel/src/net.rs: handle_request
    |  match a1 -> handle_socket_udp
    v
smoltcp: SocketSet::add(udp::Socket::new(...))
    |  returns smoltcp SocketHandle
    v
kernel/src/net.rs: record in UDP_HANDLES, return idx
    |  frame.a0 = idx
    v
----- kernel/user boundary (sret) -----
    |
    v
userspace/hello/src/gooseos.rs: ret != ERR -> Ok(idx)
    |
    v
userspace/hello/src/main.rs: println!("got UDP handle {}", h)
```

Six named layers. Two privilege transitions. One register-width result. And it takes well under a microsecond of CPU time — most of the six-second budget is the kernel boot and the QEMU tear-down.

The pcap from `test-net-user` still shows only the smoke-test ARP frames from boot. PID 1's calls did not generate any packets — they allocated, bound, and closed a socket, none of which touches the wire. That is exactly correct. The pipeline that moves *control* is now proven. The pipeline that moves *data* — `NET_SEND`, `NET_RECV`, VA-to-PA translation — is next.

> :happygoose: The goose chorus has to pause here. This is the moment the microkernel stops being a diagram and starts being a system. A user program asked the kernel for a socket *over IPC*, got one back, used it, gave it back. Nothing special happened. That "nothing special" is the entire point. The machinery behind each line is substantial, and the user program does not care. The interface is the interface.

## What We Changed

| File | Change |
|------|--------|
| `userspace/hello/src/gooseos.rs` | Added `net` submodule: `status`, `socket_udp`, `bind`, `close` over 4-register `SYS_CALL` |
| `userspace/hello/src/main.rs` | Replaced hello-world body with the NET_STATUS → SOCKET_UDP → BIND → CLOSE pipeline |
| `Makefile` | Added `test-net-user` target: `rust-user` + `net` features with pcap capture |

## What's Next

The four calls that work are the ones that never block. The calls that remain — `NET_CONNECT`, `NET_RECV`, `NET_SEND` — need a new process state (`BlockedNet`) and a per-process "what am I waiting for" handle. Once those land, PID 1 can connect to a real TCP server, and the pipeline proves not just control but data.

There is also a structural observation hiding in plain sight. Adding a second user program (a DNS client, a simple HTTP fetcher) would want to share the `gooseos::net` module. Right now that module lives inside `userspace/hello/src/`, which is correct for one program and wrong for two. The shape of that refactor — `userspace/lib/gooseos` and `userspace/bin/{hello,dns}` — is already visible. It just has not happened yet, because one program does not need it.

Chapter 45 turns to that second structural observation: the repo itself had been growing faster than its folders, and it was time to separate concerns at the filesystem level.
