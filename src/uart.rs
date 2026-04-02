/// NS16550A UART driver for QEMU virt machine.
///
/// Register map (offsets from base address):
///   0x00  THR  - Transmit Holding Register (write)
///   0x00  RBR  - Receive Buffer Register (read)
///   0x01  IER  - Interrupt Enable Register
///   0x02  FCR  - FIFO Control Register (write)
///   0x03  LCR  - Line Control Register
///   0x05  LSR  - Line Status Register
///
/// All registers are 8-bit (byte-wide) MMIO.

use core::ptr;

pub struct Uart {
    base: usize,
}

impl Uart {
    pub const fn new(base: usize) -> Self {
        Uart { base }
    }

    /// Initialize UART: 8-bit words, FIFOs enabled, no interrupts.
    pub fn init(&self) {
        let base = self.base as *mut u8;
        unsafe {
            // LCR: 8 data bits, 1 stop bit, no parity
            ptr::write_volatile(base.add(3), 0x03);
            // FCR: enable FIFOs
            ptr::write_volatile(base.add(2), 0x01);
            // IER: disable all interrupts (for now)
            ptr::write_volatile(base.add(1), 0x00);
        }
    }

    /// Write a single byte, waiting until the transmitter is ready.
    pub fn putc(&self, c: u8) {
        let base = self.base as *mut u8;
        unsafe {
            // Spin until LSR bit 5 (THR empty) is set
            while ptr::read_volatile(base.add(5)) & (1 << 5) == 0 {}
            // Write the character to THR
            ptr::write_volatile(base.add(0), c);
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
}
