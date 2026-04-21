---
sidebar_position: 5
sidebar_label: "Ch 46: dwmac Blueprint"
title: "Chapter 46: dwmac — A Blueprint for Real-Silicon Networking"
---

# Chapter 46: dwmac — A Blueprint for Real-Silicon Networking

Chapter 45 closed with the claim that the repo restructure was "architecture-neutral." That is true of filesystem layout. It is not true of drivers. The VirtIO-MMIO driver that powered Chapters 41–44 is a QEMU-only device. The VisionFive 2 has two Gigabit Ethernet PHYs, neither of them virtual, neither of them at `0x10001000`. If we want a user-space packet on real silicon, someone has to write the driver.

This chapter is a blueprint, not a build log. The dwmac driver does not exist yet. What this chapter does is scout the terrain — what the hardware is, where it lives, what its contract with software looks like, and what shape the first testable skeleton will take. It is the same flavor as Chapter 27 "The Blueprint": a design pass before the first `unsafe` block, so that the implementation chapters that follow have somewhere to stand.

## What dwmac Is

**dwmac** is shorthand for **Synopsys DesignWare MAC** — a licensable Ethernet MAC IP block that SoC vendors drop into their silicon the way they drop in a UART. The kernel community's Linux driver is `stmmac` (originally from STMicroelectronics), and every board that ships this IP reuses essentially the same register layout with vendor-specific glue around it.

The JH7110 on the VisionFive 2 has two dwmac-compatible GMACs. So do the Kendryte K230, the SpacemiT X60, and most mainline-supported RISC-V SBCs released since 2023. A dwmac driver is not a one-board detour — it is the single piece of code that unlocks real-silicon networking across the RISC-V ecosystem we care about.

> :sharpgoose: One driver, many boards. The reason Linux's stmmac is thousands of lines is that it covers every register revision from every SoC vendor since 2012. GooseOS does not need that. We need the modern JH7110 variant, with a clean subset of features — and because dwmac is a family, the "easy subset" we write first will boot on the K230 with almost no changes. Writing for the VisionFive 2 is writing for an ecosystem.
>
> :nerdygoose: The Synopsys DesignWare IP is *also* in countless ARM SoCs — Allwinner, Rockchip, NXP, Intel. A kernel driver that handles dwmac on RISC-V is structurally 95% the same code as the ARM version. The register offsets do not care what ISA is decoding the load instruction.

## The JH7110 Memory Map

From the StarFive JH7110 TRM, the two GMAC controllers sit here:

| Block | Base | Size |
|-------|------|------|
| GMAC0 | `0x16030000` | 64 KB |
| GMAC1 | `0x16040000` | 64 KB |
| SYSCRG (clock/reset) | `0x13020000` | 64 KB |
| AON SYSCON (PHY interface mode) | `0x17000000` | 4 KB |
| PLIC (interrupts) | `0x0C000000` | 64 MB |

Each GMAC's internal layout (from the DesignWare TRM) is a few major register blocks:

| Offset | Block | Purpose |
|--------|-------|---------|
| `0x0000` | MAC | Address, configuration, status |
| `0x0100` | MMC | Statistics counters (not needed at first) |
| `0x0700` | GMAC_EXT | RGMII/RMII mode, delay lines |
| `0x1000` | DMA | Descriptor rings, transmit/receive control |

Compared to VirtIO MMIO's flat register window, dwmac is baroque — different subsystems live at different offsets within the same 64 KB window, and getting a packet to move requires touching at least three of them. The compensating virtue is that the layout is documented down to the bit, both in the DesignWare TRM and in every dwmac driver you can read in Linux.

> :angrygoose: "64 KB of registers" is a lot more surface area than VirtIO's ~100 bytes. Most of it is features we do not need — VLAN offload, flow control, hardware timestamping, checksum offload, energy-efficient Ethernet. The trap is to try to initialize it all. The discipline is to zero everything and only turn on what the minimum viable path needs: link negotiation, one TX ring, one RX ring, one interrupt.

## What the First Skeleton Looks Like

A driver does not have to do everything to be useful. A driver that only detects the PHY and reports link state is a meaningful step — it proves that the MMIO window is mapped, the clocks are on, the resets are released, and the GMII handshake with the external PHY chip works. Zero packets. Zero DMA. Zero interrupts. But every one of those passed conditions unblocks the next set of chapters.

The skeleton:

```rust title="kernel/src/dwmac.rs — probe skeleton (planned)"
pub struct Dwmac {
    base: usize,                 // MMIO base, e.g. 0x1603_0000
    mac_addr: [u8; 6],           // read from EFUSE or derived
}

impl Dwmac {
    /// Probe: does a dwmac controller live at this address?
    /// Returns None if hardware is absent or wedged.
    pub fn probe(base: usize) -> Option<Self> {
        // 1. Read MAC_VERSION (offset 0x0020) — should be 0x41 on JH7110
        // 2. Read MAC_HW_FEATURE0 (offset 0x011C) — enabled features
        // 3. If both plausible, construct Self
        unimplemented!()
    }

    /// Release resets and enable clocks for this GMAC.
    /// (JH7110-specific — touches SYSCRG at 0x1302_0000.)
    pub fn enable_clocks(&mut self) -> Result<(), DriverError> {
        unimplemented!()
    }

    /// Read the PHY's link status over MDIO.
    /// Returns Some(speed_mbps) on link-up, None on link-down.
    pub fn link_status(&mut self) -> Option<u32> {
        // 1. Write PHY address + register address to MAC_MDIO_ADDRESS
        // 2. Poll MAC_MDIO_ADDRESS.GB (busy bit) until clear
        // 3. Read MAC_MDIO_DATA
        // 4. Bit 2 of PHY register 1 = link up
        unimplemented!()
    }
}
```

Three functions. One prints a version number. One toggles clock bits. One does a single MDIO read-and-poll. If those three compile, link, deploy to the VF2, and print

```
[dwmac] version=0x41 hw_features=0x10D73F37
[dwmac] link up at 1000 Mbps
```

…we are done with the skeleton and every subsequent chapter gets to assume it works.

> :weightliftinggoose: Ship the skeleton before you ship the feature. The worst way to write a driver is to write all of it and then run it for the first time. The best way is to make one function work, print one line, and stop. The next session you write the next function. The difference in debug time is hours vs. days. Split the progress bar into ten boxes and tick them one at a time.

## The Three Hard Problems

Probing and link detection are straightforward register reads. The hard parts are the three that do not show up until you try to move data.

### 1. DMA Cache Coherency

QEMU's VirtIO is a virtual device; there is no cache between driver and device. Real silicon is not so kind. The JH7110's U74 cores have an 8-way, 32 KB L1 D-cache per core, and the GMAC's DMA controller reads memory directly, not through the cache hierarchy. If the driver writes a descriptor and the write sits in L1 instead of RAM, the DMA controller reads stale data and packets silently fail to transmit.

The fix is explicit: `fence` and cache maintenance instructions. The RISC-V `Zicbom` extension (Cache-Block Management, ratified 2022) provides `cbo.clean`, `cbo.flush`, and `cbo.inval` for exactly this. The U74 implements them. The driver has to call them before notifying the device of a new descriptor, and after receiving an interrupt that says a descriptor has been written by the device.

```rust title="kernel/src/dwmac.rs — planned DMA fence"
/// Flush cache lines spanning `addr..addr+len` back to RAM.
/// Required before pointing the DMA engine at a writable buffer.
#[inline]
fn dma_flush(addr: *const u8, len: usize) {
    const LINE: usize = 64;
    let start = addr as usize & !(LINE - 1);
    let end   = (addr as usize + len + LINE - 1) & !(LINE - 1);
    unsafe {
        for cl in (start..end).step_by(LINE) {
            core::arch::asm!("cbo.clean ({0})", in(reg) cl);
        }
        core::arch::asm!("fence ow,w");
    }
}
```

> :surprisedgoose: The first dwmac bug every single driver author hits is forgetting cache coherency. The driver passes unit tests. The code review is clean. Everything "should work." Then on silicon, packets randomly drop. One in three. One in twenty. No pattern. That is cache incoherence, and the fix is literally two assembly instructions in the right places. Write them in *first*, even before you believe you need them. You do.
>
> :sarcasticgoose: "But my test passed." It passed on QEMU, which models no cache. Congratulations — you have verified that your code runs on a machine that does not exist.

### 2. Clock and Reset Dance

Powering on a peripheral on the JH7110 is not a single write. The `SYSCRG` block at `0x13020000` has separate registers for each clock gate and each reset line associated with GMAC0. The order matters.

From the JH7110 datasheet, the sequence is approximately:

1. Enable `GMAC0_AHB` clock.
2. Enable `GMAC0_AXI` clock.
3. Enable `GMAC0_PTP` and `GMAC0_RX` clocks.
4. Deassert `GMAC0_AHB_RSTN` reset.
5. Deassert `GMAC0_AXI_RSTN` reset.
6. Wait for `MAC_DMA_MODE.SWR` (software reset, bit 0 of DMA reg 0x1000) to clear.

Each step is a single 32-bit write. Getting the order wrong means the controller either never comes out of reset (reads return `0xFFFFFFFF`) or comes out in a bad state (reads look plausible but nothing moves). There is no diagnostic that distinguishes "forgot step 3" from "register 0x0020 moved in a silicon revision."

> :nerdygoose: Linux's `stmmac` handles this via the Device Tree. The DT describes which clocks and resets belong to this device; the clock framework walks them in order. GooseOS has no DT parser and deliberately does not want one for Phase C. The alternative is a hard-coded `init_sequence()` function with a comment above every line pointing at the TRM page. Less flexible. Easier to read in isolation. For one SoC, that trade is correct.

### 3. Interrupt Wiring

The GMAC raises two interrupt lines — transmit-complete and receive-available. They arrive at the JH7110's PLIC at specific interrupt IDs (7 and 8 for GMAC0, 78 and 79 for GMAC1, per the JH7110 TRM). The PLIC IRQ subsystem GooseOS already has from Chapter 8 handles PLIC mechanics — enable, claim, complete. What dwmac adds is the handler:

```rust title="kernel/src/dwmac.rs — planned IRQ handler"
pub fn handle_interrupt(&mut self) {
    let status = self.read(DMA_STATUS);  // offset 0x1020
    if status & DMA_STATUS_TI != 0 {     // TX complete
        self.reap_tx();
        self.write(DMA_STATUS, DMA_STATUS_TI);  // W1C
    }
    if status & DMA_STATUS_RI != 0 {     // RX available
        crate::net::poll();              // let smoltcp pick up frames
        self.write(DMA_STATUS, DMA_STATUS_RI);
    }
}
```

The write-1-to-clear semantics (`W1C`) are dwmac convention: writing a 1 to a status bit clears it, writing a 0 leaves it alone. Forget to acknowledge and the PLIC re-fires the interrupt immediately, yielding an infinite IRQ loop that pins one core at 100%. This is also a classic every-driver-author-hits-it bug, which is why the skeleton includes the write even though the skeleton is not yet raising interrupts.

## The Feature-Flag Refactor

The current `Cargo.toml` pins `net` to `qemu`:

```toml
net = ["qemu", "dep:smoltcp"]
```

That has to go. Once dwmac exists, `net` becomes platform-agnostic, and the *driver* is chosen by the platform feature:

```toml
# Planned
net = ["dep:smoltcp"]           # network stack (smoltcp), platform-agnostic
qemu = ["virtio-net"]            # qemu platform pulls in the virtio driver
vf2  = ["dwmac"]                 # vf2 platform pulls in the dwmac driver
virtio-net = []                   # private feature — implementation detail
dwmac = []                        # private feature — implementation detail
```

`kernel/src/net.rs` already abstracts over a `NetworkDevice` trait. Chapter 41 wrote that trait; Chapter 42's `SmoltcpDevice` uses it; dwmac will implement it. The `net` module becomes:

```rust title="kernel/src/net.rs — planned driver selection"
#[cfg(feature = "virtio-net")]
use crate::virtio::get as get_device;

#[cfg(feature = "dwmac")]
use crate::dwmac::get as get_device;
```

One `use` statement compiles in. The rest of the net module never learns which silicon it is talking to.

> :happygoose: This is the payoff of the trait from Chapter 41. When we wrote `NetworkDevice`, there was only one implementation and it felt over-engineered. The point of a trait is not how many implementations you have today — it is how many you can add tomorrow without changing the callers. Adding dwmac adds one `impl` block. smoltcp, the IPC layer, the userspace `gooseos::net` module — none of them change. Zero lines. That is the trait working exactly as designed.

## What Gets Proven Along the Way

Each milestone in the dwmac chapters unlocks a specific test on the VF2:

| Milestone | Test on VF2 |
|-----------|-------------|
| Probe + version read | `[dwmac] version=0x41` on UART |
| Clock/reset sequence | No bus hang when reading other GMAC regs |
| MDIO + PHY link | `[dwmac] link up at 1000 Mbps` |
| TX ring + first packet | ARP request visible on a USB-Ethernet dongle sniffer |
| RX ring + first receive | ARP reply logged to UART |
| smoltcp integration | `ping 10.0.0.1` from a host computer works |
| IPC + userspace client | `make deploy-rust-user` on VF2 prints `[net-test] PASS` |

The last row is the answer to "what test can I do on real silicon?" from the question that started this chapter. It is also several chapters of driver work away. That distance is not a bug in the plan — it is what honest driver work looks like on hardware you do not control.

> :sharpgoose: The table above is a bisection-friendly roadmap. If the last row fails, you do not debug "networking." You debug the specific row that stopped printing. Each row is a commit, each commit has its own UART signature, and each signature is one grep on the logs. Discipline in the plan is discipline in the debug.

## Why Not Port a Linux Driver?

A question worth asking honestly: Linux's `stmmac` exists, is mature, is open source. Could we not translate it?

In principle yes; in practice no, and for two reasons.

**The Linux driver is 12,000+ lines** spread across 30+ files, most of it handling variant quirks GooseOS will never see. Cutting it down to a minimum viable driver is, itself, a multi-day reading project. And because the cuts are interleaved with the structure of the Linux driver model (clk framework, reset framework, mdio bus, phylib, rtnl locking), the cuts do not separate cleanly — you end up rewriting, not porting.

**The Linux driver depends on a runtime GooseOS does not have** — kmalloc, DMA pool allocator, IRQ threading, sleepable waits, the device tree. Every dependency is an impedance mismatch. By the time you stub them all out, you have written a new driver with a Linux-shaped comment block on top.

The direct-from-TRM approach is faster, shorter, and produces code that matches the rest of the GooseOS kernel in style and unsafe boundary. It is also, for pedagogical purposes, the whole point — the book's premise is that a microkernel is small enough to read, and its drivers should be too.

> :angrygoose: "Port the Linux driver" is a siren song. It sounds efficient. It is not. Six times out of ten, a clean TRM-based implementation is shorter, faster, and easier to understand than a cut-down port of a mature Linux driver. The other four times you already knew going in that you wanted Linux's quirks coverage. For a single SoC from 2022, you do not.

## What We're Not Doing

The roadmap deliberately excludes a few things.

- **DHCP.** Static IP is fine for the first real-silicon network test. DHCP is a lot of protocol machinery, and smoltcp has it, but the first `[net-test] PASS` on VF2 will use a hardcoded 10.0.0.2/24 config.
- **Both GMACs.** GMAC0 only. GMAC1 exists, but two interfaces means routing policy, and routing policy means complexity that a first-working driver does not need.
- **Hardware offloads.** No checksum offload, no TSO, no VLAN. smoltcp computes everything on the CPU. The U74 at 1.5 GHz has cycles to spare for a 1 Gbps link at microkernel-grade protocol processing.
- **Copper PHY configuration.** The VF2's PHY (Motorcomm YT8531S) has an initialization dance to run — clock skew tuning, LED mapping, RGMII delay. The first version does the minimum — read the datasheet's recommended register writes, apply them, move on.

Each exclusion is a future chapter. None of them blocks the first packet.

## What Comes Next

The implementation chapters follow this blueprint in sequence. Each is scoped to one testable milestone:

- **Chapter 47**: Clock, reset, probe — the first `[dwmac]` line on UART.
- **Chapter 48**: MDIO and the PHY — link detection and speed negotiation.
- **Chapter 49**: Descriptor rings and DMA — the first transmitted ARP frame.
- **Chapter 50**: RX path and interrupts — the first received reply.
- **Chapter 51**: smoltcp integration and the userspace round-trip — `[net-test] PASS` on silicon.

Five chapters, five commits, five UART signatures. The driver is not small. But it is, block by block, writable by one person at a kitchen table with a VF2 and a USB-UART cable — which is the whole point.

> :happygoose: The gap between QEMU and silicon is the gap between "it works" and "it ships." Bridging that gap is what makes an OS a real OS instead of an emulator demo. The dwmac driver is the most concrete form of that bridge we will write this year. When `[net-test] PASS` appears on a UART attached to a physical VisionFive 2, the honk stops being metaphorical. The goose has landed.
