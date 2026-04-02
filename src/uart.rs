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

use core::fmt;
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

    /// Enable receive-data-available interrupts.
    /// Call this AFTER PLIC is configured, BEFORE interrupts_enable().
    pub fn enable_rx_interrupt(&self) {
        let base = self.base as *mut u8;
        unsafe {
            // IER bit 0 (ERBFI): interrupt when RX data is available
            // Keep bit 1 (ETBEI) clear — TX stays polling
            ptr::write_volatile(base.add(1), 0x01);
        }
    }

    /// Non-blocking read. Returns Some(byte) if data is available.
    pub fn getc(&self) -> Option<u8> {
        let base = self.base as *mut u8;
        unsafe {
            // LSR bit 0 (DR): data ready
            if ptr::read_volatile(base.add(5)) & 1 != 0 {
                Some(ptr::read_volatile(base.add(0)))
            } else {
                None
            }
        }
    }
}

/// Implement core::fmt::Write so we can use write!() and friends.
///
/// This is the bridge between Rust's formatting machinery and our
/// raw UART. Once this exists, we get formatted output for free:
///   write!(uart, "hart {} at {:#x}", id, addr)
impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.puts(s);
        Ok(())
    }
}

/// Handle a UART interrupt — drain the RX FIFO and echo characters.
/// Called from the trap handler when PLIC claims IRQ 10.
pub fn handle_interrupt() {
    let uart = Uart::new(0x1000_0000);
    // Drain all available characters from the FIFO
    while let Some(c) = uart.getc() {
        match c {
            // Carriage return or newline → echo both
            b'\r' | b'\n' => {
                uart.putc(b'\r');
                uart.putc(b'\n');
            }
            // Backspace (0x7F DEL or 0x08 BS) → erase
            0x7F | 0x08 => {
                uart.putc(0x08); // move cursor back
                uart.putc(b' '); // overwrite with space
                uart.putc(0x08); // move cursor back again
            }
            // Regular character → echo it
            _ => {
                uart.putc(c);
            }
        }
    }
}
