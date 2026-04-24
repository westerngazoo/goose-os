/// VirtIO MMIO transport and virtio-net device driver.
///
/// Phase B: Networking.
///
/// Implements VirtIO MMIO v2 device discovery, virtqueue management,
/// and the virtio-net device driver. All buffers are statically allocated.
///
/// Unsafe: contains MMIO register access and static mutable state.

use core::ptr;
use core::sync::atomic::{fence, Ordering};
use crate::driver::{Device, NetworkDevice, DriverError};
use crate::platform;
use crate::println;

// ── VirtIO MMIO Register Offsets (v2 spec) ───────────────────

const VIRTIO_MMIO_MAGIC:            usize = 0x000;
const VIRTIO_MMIO_VERSION:          usize = 0x004;
const VIRTIO_MMIO_DEVICE_ID:        usize = 0x008;
const VIRTIO_MMIO_VENDOR_ID:        usize = 0x00C;
const VIRTIO_MMIO_DEV_FEATURES:     usize = 0x010;
const VIRTIO_MMIO_DEV_FEATURES_SEL: usize = 0x014;
const VIRTIO_MMIO_DRV_FEATURES:     usize = 0x020;
const VIRTIO_MMIO_DRV_FEATURES_SEL: usize = 0x024;
const VIRTIO_MMIO_QUEUE_SEL:        usize = 0x030;
const VIRTIO_MMIO_QUEUE_NUM_MAX:    usize = 0x034;
const VIRTIO_MMIO_QUEUE_NUM:        usize = 0x038;
const VIRTIO_MMIO_QUEUE_READY:      usize = 0x044;
const VIRTIO_MMIO_QUEUE_NOTIFY:     usize = 0x050;
const VIRTIO_MMIO_IRQ_STATUS:       usize = 0x060;
const VIRTIO_MMIO_IRQ_ACK:          usize = 0x064;
const VIRTIO_MMIO_STATUS:           usize = 0x070;
const VIRTIO_MMIO_QUEUE_DESC_LOW:   usize = 0x080;
const VIRTIO_MMIO_QUEUE_DESC_HIGH:  usize = 0x084;
const VIRTIO_MMIO_QUEUE_AVAIL_LOW:  usize = 0x090;
const VIRTIO_MMIO_QUEUE_AVAIL_HIGH: usize = 0x094;
const VIRTIO_MMIO_QUEUE_USED_LOW:   usize = 0x0A0;
const VIRTIO_MMIO_QUEUE_USED_HIGH:  usize = 0x0A4;
const VIRTIO_MMIO_CONFIG:           usize = 0x100;

// Status bits
const STATUS_ACK:         u32 = 1;
const STATUS_DRIVER:      u32 = 2;
const STATUS_DRIVER_OK:   u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;
const STATUS_FAILED:      u32 = 128;

// VirtIO magic value
const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt"

// Feature bits for virtio-net
const VIRTIO_NET_F_MAC:    u32 = 1 << 5;
const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
// Mandatory feature bits
const VIRTIO_F_VERSION_1:  u32 = 1 << 0; // In features_sel=1 (high 32 bits)

// Virtqueue descriptor flags
const VRING_DESC_F_NEXT:     u16 = 1;
const VRING_DESC_F_WRITE:    u16 = 2;

// ── Buffer Pool Configuration ────────────────────────────────

const RX_BUFFERS: usize = 32;
const TX_BUFFERS: usize = 16;
const PACKET_SIZE: usize = 1514;   // Max Ethernet frame
// virtio-net header size. In VirtIO 1.0+ (modern, with VIRTIO_F_VERSION_1)
// this struct is always 12 bytes — there's a num_buffers field even without
// VIRTIO_NET_F_MRG_RXBUF. Legacy transport was 10 bytes. We negotiate
// VERSION_1, so use 12.
const VIRTIO_NET_HDR_SIZE: usize = 12;
const BUF_SIZE: usize = PACKET_SIZE + VIRTIO_NET_HDR_SIZE;

// Queue size — must be power of 2, >= RX_BUFFERS + TX_BUFFERS
const QUEUE_SIZE: usize = 64;

// ── Virtqueue Structures (repr(C) for DMA) ───────────────────

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len:  u32,
    flags: u16,
    next: u16,
}

impl VirtqDesc {
    const fn zero() -> Self {
        VirtqDesc { addr: 0, len: 0, flags: 0, next: 0 }
    }
}

#[repr(C, align(2))]
struct VirtqAvail {
    flags: u16,
    idx:   u16,
    ring:  [u16; QUEUE_SIZE],
}

impl VirtqAvail {
    const fn zero() -> Self {
        VirtqAvail { flags: 0, idx: 0, ring: [0; QUEUE_SIZE] }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqUsedElem {
    id:  u32,
    len: u32,
}

impl VirtqUsedElem {
    const fn zero() -> Self {
        VirtqUsedElem { id: 0, len: 0 }
    }
}

#[repr(C, align(4))]
struct VirtqUsed {
    flags: u16,
    idx:   u16,
    ring:  [VirtqUsedElem; QUEUE_SIZE],
}

impl VirtqUsed {
    const fn zero() -> Self {
        VirtqUsed { flags: 0, idx: 0, ring: [VirtqUsedElem::zero(); QUEUE_SIZE] }
    }

    /// Volatile read of `idx`, with a SeqCst fence so prior DMA writes
    /// by the device are observed.
    ///
    /// The device updates `idx` via DMA. A plain field load lets LTO
    /// hoist the value out of loops; we'd never see new completions.
    /// Every reader in the driver funnels through this helper.
    #[inline]
    fn load_idx(&self) -> u16 {
        fence(Ordering::SeqCst);
        // SAFETY: INV-1 (single hart) + the VirtqUsed is owned by the
        // VirtioNet static. Volatile + fence pairs with the device's
        // write on the other side of the DMA.
        unsafe { ptr::read_volatile(&self.idx) }
    }
}

// ── virtio-net header ────────────────────────────────────────

/// virtio-net packet header.
///
/// Exactly `VIRTIO_NET_HDR_SIZE` (12) bytes. In VirtIO 1.0+ modern
/// transport (which we negotiate via VIRTIO_F_VERSION_1), `num_buffers`
/// is always present even without VIRTIO_NET_F_MRG_RXBUF — so we
/// include it here. Without it, `from_raw_parts(ptr, 12)` over a 10-byte
/// struct would read 2 bytes of stack garbage into the TX header.
#[repr(C)]
#[derive(Clone, Copy)]
struct VirtioNetHdr {
    flags:       u8,
    gso_type:    u8,
    hdr_len:     u16,
    gso_size:    u16,
    csum_start:  u16,
    csum_offset: u16,
    num_buffers: u16,
}

// Compile-time assertion: struct size must match the on-wire header size.
const _: () = assert!(
    core::mem::size_of::<VirtioNetHdr>() == VIRTIO_NET_HDR_SIZE,
    "VirtioNetHdr struct size must equal VIRTIO_NET_HDR_SIZE"
);

impl VirtioNetHdr {
    const fn zero() -> Self {
        VirtioNetHdr {
            flags: 0, gso_type: 0, hdr_len: 0,
            gso_size: 0, csum_start: 0, csum_offset: 0,
            num_buffers: 0,
        }
    }
}

// ── VirtioNet Device ─────────────────────────────────────────

pub struct VirtioNet {
    base: usize,
    irq: u32,
    mac: [u8; 6],
    initialized: bool,

    // RX queue (queue index 0)
    rx_desc:  [VirtqDesc; QUEUE_SIZE],
    rx_avail: VirtqAvail,
    rx_used:  VirtqUsed,
    rx_last_used: u16,

    // TX queue (queue index 1)
    tx_desc:  [VirtqDesc; QUEUE_SIZE],
    tx_avail: VirtqAvail,
    tx_used:  VirtqUsed,
    tx_last_used: u16,

    // Buffer pools
    rx_buffers: [[u8; BUF_SIZE]; RX_BUFFERS],
    tx_buffers: [[u8; BUF_SIZE]; TX_BUFFERS],
    tx_free_mask: u16, // Bitmask: bit i set = tx_buffers[i] is free
}

impl VirtioNet {
    const fn new_uninit() -> Self {
        VirtioNet {
            base: 0,
            irq: 0,
            mac: [0; 6],
            initialized: false,
            rx_desc:  [VirtqDesc::zero(); QUEUE_SIZE],
            rx_avail: VirtqAvail::zero(),
            rx_used:  VirtqUsed::zero(),
            rx_last_used: 0,
            tx_desc:  [VirtqDesc::zero(); QUEUE_SIZE],
            tx_avail: VirtqAvail::zero(),
            tx_used:  VirtqUsed::zero(),
            tx_last_used: 0,
            rx_buffers: [[0u8; BUF_SIZE]; RX_BUFFERS],
            tx_buffers: [[0u8; BUF_SIZE]; TX_BUFFERS],
            tx_free_mask: 0xFFFF, // All 16 TX buffers free
        }
    }

    // ── MMIO register helpers ────────────────────────────────

    fn read32(&self, offset: usize) -> u32 {
        unsafe { ptr::read_volatile((self.base + offset) as *const u32) }
    }

    fn write32(&self, offset: usize, val: u32) {
        unsafe { ptr::write_volatile((self.base + offset) as *mut u32, val); }
    }

    // ── Virtqueue setup ──────────────────────────────────────

    /// Set up a virtqueue by writing descriptor/avail/used physical addresses
    /// to the device. Identity-mapped, so VA == PA for static globals.
    fn setup_queue(&mut self, queue_idx: u32) {
        self.write32(VIRTIO_MMIO_QUEUE_SEL, queue_idx);

        let max = self.read32(VIRTIO_MMIO_QUEUE_NUM_MAX);
        if max == 0 {
            println!("  [virtio] Queue {} not available", queue_idx);
            return;
        }
        if (max as usize) < QUEUE_SIZE {
            println!("  [virtio] Queue {} max {} < our {}", queue_idx, max, QUEUE_SIZE);
        }

        self.write32(VIRTIO_MMIO_QUEUE_NUM, QUEUE_SIZE as u32);

        // Get physical addresses of our queue structures
        let (desc_pa, avail_pa, used_pa) = if queue_idx == 0 {
            (
                self.rx_desc.as_ptr() as usize,
                &self.rx_avail as *const _ as usize,
                &self.rx_used as *const _ as usize,
            )
        } else {
            (
                self.tx_desc.as_ptr() as usize,
                &self.tx_avail as *const _ as usize,
                &self.tx_used as *const _ as usize,
            )
        };

        self.write32(VIRTIO_MMIO_QUEUE_DESC_LOW,  desc_pa as u32);
        self.write32(VIRTIO_MMIO_QUEUE_DESC_HIGH, (desc_pa >> 32) as u32);
        self.write32(VIRTIO_MMIO_QUEUE_AVAIL_LOW,  avail_pa as u32);
        self.write32(VIRTIO_MMIO_QUEUE_AVAIL_HIGH, (avail_pa >> 32) as u32);
        self.write32(VIRTIO_MMIO_QUEUE_USED_LOW,   used_pa as u32);
        self.write32(VIRTIO_MMIO_QUEUE_USED_HIGH,  (used_pa >> 32) as u32);

        self.write32(VIRTIO_MMIO_QUEUE_READY, 1);
    }

    /// Pre-populate RX queue with buffer descriptors.
    fn populate_rx_queue(&mut self) {
        for i in 0..RX_BUFFERS {
            let buf_pa = self.rx_buffers[i].as_ptr() as u64;
            self.rx_desc[i] = VirtqDesc {
                addr: buf_pa,
                len: BUF_SIZE as u32,
                flags: VRING_DESC_F_WRITE, // Device writes to this buffer
                next: 0,
            };
            // Add to available ring
            let avail_idx = self.rx_avail.idx;
            self.rx_avail.ring[(avail_idx as usize) % QUEUE_SIZE] = i as u16;
            self.rx_avail.idx = avail_idx.wrapping_add(1);
        }
        fence(Ordering::SeqCst);
        // Notify device that RX buffers are available
        self.write32(VIRTIO_MMIO_QUEUE_SEL, 0);
        self.write32(VIRTIO_MMIO_QUEUE_NOTIFY, 0);
    }

    /// Reclaim used TX descriptors.
    fn reclaim_tx(&mut self) {
        loop {
            let used_idx = self.tx_used.load_idx();
            if self.tx_last_used == used_idx {
                break;
            }
            let elem = self.tx_used.ring[(self.tx_last_used as usize) % QUEUE_SIZE];
            let desc_idx = elem.id as usize;
            // Map descriptor index back to buffer index
            // TX descriptors use indices RX_BUFFERS..RX_BUFFERS+TX_BUFFERS in desc table,
            // but we use separate desc tables, so desc_idx is directly the buffer idx
            if desc_idx < TX_BUFFERS {
                self.tx_free_mask |= 1 << desc_idx;
            }
            self.tx_last_used = self.tx_last_used.wrapping_add(1);
        }
    }

    /// Process received packets from the used ring.
    /// Returns true if any packets were received (caller should check can_receive).
    fn process_rx_used(&mut self) -> bool {
        self.rx_used.load_idx() != self.rx_last_used
    }
}

// ── Device trait implementation ──────────────────────────────

impl Device for VirtioNet {
    fn probe(base: usize) -> bool {
        unsafe {
            let magic = ptr::read_volatile(base as *const u32);
            if magic != VIRTIO_MAGIC {
                return false;
            }
            let version = ptr::read_volatile((base + VIRTIO_MMIO_VERSION) as *const u32);
            if version != 2 {
                return false; // We only support VirtIO MMIO v2
            }
            let device_id = ptr::read_volatile((base + VIRTIO_MMIO_DEVICE_ID) as *const u32);
            device_id == platform::VIRTIO_DEV_NET
        }
    }

    fn init(&mut self) -> Result<(), DriverError> {
        if self.base == 0 {
            return Err(DriverError::NotFound);
        }

        // 1. Reset device
        self.write32(VIRTIO_MMIO_STATUS, 0);

        // 2. Set ACKNOWLEDGE status bit
        self.write32(VIRTIO_MMIO_STATUS, STATUS_ACK);

        // 3. Set DRIVER status bit
        self.write32(VIRTIO_MMIO_STATUS, STATUS_ACK | STATUS_DRIVER);

        // 4. Read device features
        self.write32(VIRTIO_MMIO_DEV_FEATURES_SEL, 0);
        let features_lo = self.read32(VIRTIO_MMIO_DEV_FEATURES);
        self.write32(VIRTIO_MMIO_DEV_FEATURES_SEL, 1);
        let features_hi = self.read32(VIRTIO_MMIO_DEV_FEATURES);

        println!("  [virtio] Device features: lo={:#010x} hi={:#010x}", features_lo, features_hi);

        // 5. Negotiate features — we want MAC and VERSION_1
        let our_features_lo = features_lo & (VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS);
        let our_features_hi = features_hi & VIRTIO_F_VERSION_1;

        self.write32(VIRTIO_MMIO_DRV_FEATURES_SEL, 0);
        self.write32(VIRTIO_MMIO_DRV_FEATURES, our_features_lo);
        self.write32(VIRTIO_MMIO_DRV_FEATURES_SEL, 1);
        self.write32(VIRTIO_MMIO_DRV_FEATURES, our_features_hi);

        // 6. Set FEATURES_OK
        self.write32(VIRTIO_MMIO_STATUS, STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK);

        // 7. Re-read status to verify FEATURES_OK is still set
        let status = self.read32(VIRTIO_MMIO_STATUS);
        if status & STATUS_FEATURES_OK == 0 {
            println!("  [virtio] FEATURES_OK not accepted!");
            self.write32(VIRTIO_MMIO_STATUS, STATUS_FAILED);
            return Err(DriverError::InitFailed);
        }

        // 8. Read MAC address from config space
        if features_lo & VIRTIO_NET_F_MAC != 0 {
            for i in 0..6 {
                self.mac[i] = unsafe {
                    ptr::read_volatile((self.base + VIRTIO_MMIO_CONFIG + i) as *const u8)
                };
            }
        } else {
            // Default MAC
            self.mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
        }

        // 9. Set up virtqueues
        // Queue 0 = RX, Queue 1 = TX
        self.setup_queue(0); // RX
        self.setup_queue(1); // TX

        // 10. Pre-populate RX queue with buffers
        self.populate_rx_queue();

        // 11. Set DRIVER_OK — device is live
        self.write32(VIRTIO_MMIO_STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK);

        self.initialized = true;
        Ok(())
    }

    fn handle_interrupt(&mut self) {
        let irq_status = self.read32(VIRTIO_MMIO_IRQ_STATUS);
        // Acknowledge all interrupt sources
        self.write32(VIRTIO_MMIO_IRQ_ACK, irq_status);

        // Reclaim completed TX buffers
        self.reclaim_tx();

        // RX processing is deferred to receive() calls from smoltcp poll
    }
}

// ── NetworkDevice trait implementation ───────────────────────

impl NetworkDevice for VirtioNet {
    fn mtu(&self) -> usize {
        PACKET_SIZE
    }

    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }

    fn transmit(&mut self, data: &[u8]) -> Result<(), DriverError> {
        if !self.initialized {
            return Err(DriverError::NotReady);
        }
        if data.len() > PACKET_SIZE {
            return Err(DriverError::BufferFull);
        }

        // Reclaim any completed TX
        self.reclaim_tx();

        // Find a free TX buffer
        if self.tx_free_mask == 0 {
            return Err(DriverError::BufferFull);
        }
        let buf_idx = self.tx_free_mask.trailing_zeros() as usize;
        self.tx_free_mask &= !(1 << buf_idx);

        // Write virtio-net header + data into TX buffer
        let hdr = VirtioNetHdr::zero();
        let hdr_bytes: &[u8] = unsafe {
            core::slice::from_raw_parts(
                &hdr as *const VirtioNetHdr as *const u8,
                VIRTIO_NET_HDR_SIZE,
            )
        };
        self.tx_buffers[buf_idx][..VIRTIO_NET_HDR_SIZE].copy_from_slice(hdr_bytes);
        self.tx_buffers[buf_idx][VIRTIO_NET_HDR_SIZE..VIRTIO_NET_HDR_SIZE + data.len()]
            .copy_from_slice(data);

        let total_len = VIRTIO_NET_HDR_SIZE + data.len();

        // Set up TX descriptor
        let buf_pa = self.tx_buffers[buf_idx].as_ptr() as u64;
        self.tx_desc[buf_idx] = VirtqDesc {
            addr: buf_pa,
            len: total_len as u32,
            flags: 0, // Device reads this (no WRITE flag)
            next: 0,
        };

        // Add to TX available ring
        let avail_idx = self.tx_avail.idx;
        self.tx_avail.ring[(avail_idx as usize) % QUEUE_SIZE] = buf_idx as u16;
        fence(Ordering::SeqCst);
        self.tx_avail.idx = avail_idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device
        self.write32(VIRTIO_MMIO_QUEUE_NOTIFY, 1); // Queue 1 = TX

        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, DriverError> {
        if !self.initialized {
            return Err(DriverError::NotReady);
        }

        let used_idx = self.rx_used.load_idx();
        if self.rx_last_used == used_idx {
            return Err(DriverError::BufferEmpty);
        }

        // Get the used element
        let elem = self.rx_used.ring[(self.rx_last_used as usize) % QUEUE_SIZE];
        let desc_idx = elem.id as usize;
        let total_len = elem.len as usize;

        if desc_idx >= RX_BUFFERS || total_len <= VIRTIO_NET_HDR_SIZE {
            // Invalid — re-provision and skip
            self.rx_last_used = self.rx_last_used.wrapping_add(1);
            return Err(DriverError::InvalidState);
        }

        // Copy packet data (skip virtio-net header)
        let data_len = total_len - VIRTIO_NET_HDR_SIZE;
        let copy_len = data_len.min(buf.len());
        buf[..copy_len].copy_from_slice(
            &self.rx_buffers[desc_idx][VIRTIO_NET_HDR_SIZE..VIRTIO_NET_HDR_SIZE + copy_len]
        );

        self.rx_last_used = self.rx_last_used.wrapping_add(1);

        // Re-provision this buffer to the RX available ring
        let buf_pa = self.rx_buffers[desc_idx].as_ptr() as u64;
        self.rx_desc[desc_idx] = VirtqDesc {
            addr: buf_pa,
            len: BUF_SIZE as u32,
            flags: VRING_DESC_F_WRITE,
            next: 0,
        };
        let avail_idx = self.rx_avail.idx;
        self.rx_avail.ring[(avail_idx as usize) % QUEUE_SIZE] = desc_idx as u16;
        fence(Ordering::SeqCst);
        self.rx_avail.idx = avail_idx.wrapping_add(1);
        fence(Ordering::SeqCst);

        // Notify device that new RX buffers are available
        self.write32(VIRTIO_MMIO_QUEUE_NOTIFY, 0); // Queue 0 = RX

        Ok(copy_len)
    }

    fn can_transmit(&self) -> bool {
        self.initialized && self.tx_free_mask != 0
    }

    fn can_receive(&self) -> bool {
        if !self.initialized {
            return false;
        }
        self.rx_used.load_idx() != self.rx_last_used
    }
}

// ── Global State ─────────────────────────────────────────────

static mut VIRTIO_NET: VirtioNet = VirtioNet::new_uninit();
static mut VIRTIO_NET_READY: bool = false;
static mut VIRTIO_NET_IRQ: u32 = 0;

/// Get a mutable reference to the VirtIO-net device.
///
/// # Safety
/// Caller must ensure no concurrent access (single-hart kernel, interrupts
/// disabled or within interrupt handler).
pub unsafe fn get() -> &'static mut VirtioNet {
    &mut *core::ptr::addr_of_mut!(VIRTIO_NET)
}

/// Check if the VirtIO-net device is initialized and ready.
pub fn is_ready() -> bool {
    unsafe { VIRTIO_NET_READY }
}

/// Get the IRQ number for the VirtIO-net device.
pub fn irq_number() -> u32 {
    unsafe { VIRTIO_NET_IRQ }
}

/// Scan all VirtIO MMIO slots for a virtio-net device.
///
/// Returns Some((slot_index, irq)) if found, None otherwise.
#[cfg(feature = "qemu")]
pub fn probe_all() -> Option<(usize, u32)> {
    let mut found: Option<(usize, u32)> = None;
    for slot in 0..platform::VIRTIO_MMIO_SLOTS {
        let base = platform::VIRTIO_MMIO_BASE + slot * platform::VIRTIO_MMIO_STRIDE;
        unsafe {
            let magic = ptr::read_volatile(base as *const u32);
            let version = ptr::read_volatile((base + VIRTIO_MMIO_VERSION) as *const u32);
            let device_id = ptr::read_volatile((base + VIRTIO_MMIO_DEVICE_ID) as *const u32);
            println!("  [virtio] slot {} @ {:#010x}: magic={:#010x} version={} device_id={}",
                slot, base, magic, version, device_id);
        }
        if found.is_none() && VirtioNet::probe(base) {
            let irq = platform::VIRTIO_IRQ_BASE + slot as u32;
            unsafe {
                VIRTIO_NET.base = base;
                VIRTIO_NET.irq = irq;
                VIRTIO_NET_IRQ = irq;
            }
            found = Some((slot, irq));
        }
    }
    found
}

/// Initialize the VirtIO-net device (call after probe_all succeeds).
pub fn init_device() -> Result<(), DriverError> {
    let dev = unsafe { get() };
    dev.init()?;
    unsafe { VIRTIO_NET_READY = true; }
    Ok(())
}
