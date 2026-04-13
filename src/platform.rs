/// Platform-specific constants.
///
/// GooseOS supports two platforms:
///   - QEMU virt machine (default, for development)
///   - StarFive VisionFive 2 / JH7110 (real hardware)
///
/// Select at build time: `cargo build --features vf2 --no-default-features`

// ──────────────────────────────────────────
// UART
// ──────────────────────────────────────────

/// UART0 MMIO base address.
/// Same on both platforms (happy coincidence).
pub const UART_BASE: usize = 0x1000_0000;

/// UART register stride (bytes between consecutive registers).
///
/// QEMU virt: NS16550A with 1-byte register spacing
///   THR=0x00, IER=0x01, FCR=0x02, LCR=0x03, LSR=0x05
///
/// VisionFive 2: DesignWare 8250 with 4-byte register spacing
///   THR=0x00, IER=0x04, FCR=0x08, LCR=0x0C, LSR=0x14
#[cfg(feature = "qemu")]
pub const UART_REG_STRIDE: usize = 1;

#[cfg(feature = "vf2")]
pub const UART_REG_STRIDE: usize = 4;

// ──────────────────────────────────────────
// PLIC
// ──────────────────────────────────────────

/// PLIC base address (same on both platforms).
pub const PLIC_BASE: usize = 0x0C00_0000;

/// UART0 IRQ number at the PLIC.
#[cfg(feature = "qemu")]
pub const UART0_IRQ: u32 = 10;

#[cfg(feature = "vf2")]
pub const UART0_IRQ: u32 = 32;

// ──────────────────────────────────────────
// Hart (CPU core) configuration
// ──────────────────────────────────────────

/// The hart ID that should boot the kernel.
///
/// QEMU virt: all harts are identical, hart 0 is conventional boot hart.
///
/// VisionFive 2: hart 0 is the SiFive S7 *monitor* core (no MMU!).
///   Harts 1-4 are U74 application cores. Hart 1 is the boot hart.
#[cfg(feature = "qemu")]
pub const BOOT_HART: usize = 0;

#[cfg(feature = "vf2")]
pub const BOOT_HART: usize = 1;

/// PLIC context for S-mode on the boot hart.
///
/// PLIC contexts are assigned sequentially to each (hart, mode) pair.
/// The S7 monitor core (hart 0) has NO S-mode, so it only gets ONE context,
/// which shifts all subsequent context numbers down by one.
///
/// QEMU virt (all harts have M+S, 2 contexts each):
///   Context 0: hart 0 M-mode
///   Context 1: hart 0 S-mode  ← we use this
///
/// VisionFive 2 / JH7110 (hart 0 = S7 M-only, harts 1-4 = U74 M+S):
///   Context 0: hart 0 M-mode (S7, no S-mode → only 1 context)
///   Context 1: hart 1 M-mode
///   Context 2: hart 1 S-mode  ← we use this
///   Context 3: hart 2 M-mode
///   ...
#[cfg(feature = "qemu")]
pub const PLIC_S_CONTEXT: usize = 1;

#[cfg(feature = "vf2")]
pub const PLIC_S_CONTEXT: usize = 2;

// ──────────────────────────────────────────
// Timer
// ──────────────────────────────────────────

/// Timer frequency in Hz.
/// Both QEMU virt and JH7110 use 10 MHz by convention.
pub const TIMER_FREQ: u64 = 10_000_000;

/// Timer tick interval (1 second) — used for wallclock display.
pub const TIMER_INTERVAL: u64 = TIMER_FREQ;

/// Preemption timeslice (10 ms).
/// This is how long a process runs before the scheduler checks for preemption.
/// 10 ms gives fine-grained interleaving on real hardware while keeping
/// context-switch overhead under 0.01%.
pub const TIMESLICE: u64 = TIMER_FREQ / 100;

// ──────────────────────────────────────────
// Platform name (for boot banner)
// ──────────────────────────────────────────

#[cfg(feature = "qemu")]
pub const PLATFORM_NAME: &str = "QEMU virt";

#[cfg(feature = "vf2")]
pub const PLATFORM_NAME: &str = "VisionFive 2 (JH7110)";
