/// NS16550A / DW8250-compatible UART driver.
///
/// Supports two register strides:
///   - 1 byte  (QEMU virt, standard NS16550A)
///   - 4 bytes (VisionFive 2, DesignWare 8250)
///
/// Register indices (multiplied by stride for actual offset):
///   0  THR/RBR  - Transmit/Receive
///   1  IER      - Interrupt Enable
///   2  FCR/IIR  - FIFO Control / Interrupt ID
///   3  LCR      - Line Control
///   4  MCR      - Modem Control
///   5  LSR      - Line Status

use core::fmt;
use core::ptr;
use crate::platform;

pub struct Uart {
    base: usize,
    stride: usize,
}

impl Uart {
    pub const fn new(base: usize, stride: usize) -> Self {
        Uart { base, stride }
    }

    /// Convenience: create a UART using platform defaults.
    pub const fn platform() -> Self {
        Uart {
            base: platform::UART_BASE,
            stride: platform::UART_REG_STRIDE,
        }
    }

    /// Get the address of register at logical index.
    #[inline(always)]
    fn reg(&self, index: usize) -> *mut u8 {
        (self.base + index * self.stride) as *mut u8
    }

    /// Initialize UART: 8-bit words, FIFOs enabled, interrupt output on.
    pub fn init(&self) {
        unsafe {
            // IER: disable all interrupts during setup
            ptr::write_volatile(self.reg(1), 0x00);
            // LCR: 8 data bits, 1 stop bit, no parity
            ptr::write_volatile(self.reg(3), 0x03);
            // FCR: enable + clear both FIFOs, 1-byte RX trigger
            ptr::write_volatile(self.reg(2), 0x07);
            // MCR: OUT2 (bit 3) gates interrupt output to PLIC.
            // DTR (bit 0) + RTS (bit 1) needed for RX on some hardware.
            ptr::write_volatile(self.reg(4), 0x0B);
        }
    }

    /// Write a single byte, waiting until the transmitter is ready.
    pub fn putc(&self, c: u8) {
        unsafe {
            // Spin until LSR bit 5 (THR empty) is set
            while ptr::read_volatile(self.reg(5)) & (1 << 5) == 0 {}
            // Write the character to THR
            ptr::write_volatile(self.reg(0), c);
        }
    }

    /// Write a string, converting \n to \r\n for terminal compatibility.
    pub fn puts(&self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.putc(b'\r');
            }
            self.putc(byte);
        }
    }

    /// Enable receive-data-available interrupts.
    /// Call this AFTER PLIC is configured, BEFORE interrupts_enable().
    pub fn enable_rx_interrupt(&self) {
        unsafe {
            // IER bit 0 (ERBFI): interrupt when RX data is available
            // Keep bit 1 (ETBEI) clear — TX stays polling
            ptr::write_volatile(self.reg(1), 0x01);
        }
    }

    /// Disable receive-data-available interrupts.
    /// Used when switching to polling mode (e.g., kernel idle loop).
    pub fn disable_rx_interrupt(&self) {
        unsafe {
            ptr::write_volatile(self.reg(1), 0x00);
        }
    }

    /// Non-blocking read. Returns Some(byte) if data is available.
    pub fn getc(&self) -> Option<u8> {
        unsafe {
            // LSR bit 0 (DR): data ready
            if ptr::read_volatile(self.reg(5)) & 1 != 0 {
                Some(ptr::read_volatile(self.reg(0)))
            } else {
                None
            }
        }
    }
}

/// Implement core::fmt::Write so we can use write!() and friends.
impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.puts(s);
        Ok(())
    }
}

/// Handle a UART interrupt — drain the RX FIFO and echo characters.
pub fn handle_interrupt() {
    let uart = Uart::platform();
    while let Some(c) = uart.getc() {
        match c {
            b'\r' | b'\n' => {
                uart.putc(b'\r');
                uart.putc(b'\n');
            }
            0x7F | 0x08 => {
                uart.putc(0x08);
                uart.putc(b' ');
                uart.putc(0x08);
            }
            _ => uart.putc(c),
        }
    }
}
