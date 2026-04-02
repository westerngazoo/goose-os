/// Console output via UART — provides print! and println! macros.
///
/// For now this is single-hart only (no lock needed since we park
/// all other harts in boot.S). We'll add a spin lock in Part 9
/// when we bring up SMP.

use core::fmt;
use core::fmt::Write;
use crate::uart::Uart;

/// QEMU virt machine UART0 base address.
const UART0_BASE: usize = 0x1000_0000;

/// Print to the console UART. Called by the print!/println! macros.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut uart = Uart::new(UART0_BASE);
    uart.write_fmt(args).unwrap();
}

/// Print formatted text to the console (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

/// Print formatted text to the console with a trailing newline.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
