/// Console output via UART — provides print! and println! macros.

use core::fmt;
use core::fmt::Write;
use crate::uart::Uart;

/// Print to the console UART. Called by the print!/println! macros.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut uart = Uart::platform();
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
