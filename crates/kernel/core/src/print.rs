// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Formatting into a debug console.
//!
//! Invariant: a formatting error is dropped rather than propagated; a
//! console that cannot take the bytes must not stop the kernel.

use core::fmt::{self, Write};

use kernel_hal_api::console::DebugConsole;

/// A [`core::fmt::Write`] over a debug console.
pub struct ConsoleWriter<'a, C: DebugConsole + ?Sized> {
    console: &'a mut C,
}

impl<'a, C: DebugConsole + ?Sized> ConsoleWriter<'a, C> {
    /// A writer over `console`.
    pub const fn new(console: &'a mut C) -> Self {
        ConsoleWriter { console }
    }
}

impl<C: DebugConsole + ?Sized> Write for ConsoleWriter<'_, C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.console.write_bytes(text.as_bytes());
        Ok(())
    }
}

/// Writes `arguments` to `console`, dropping a formatting error.
pub fn write(console: &mut (impl DebugConsole + ?Sized), arguments: fmt::Arguments<'_>) {
    let _ = ConsoleWriter::new(console).write_fmt(arguments);
}

/// Writes a line to a debug console.
#[macro_export]
macro_rules! println {
    ($console:expr, $($arg:tt)*) => {
        $crate::print::write($console, format_args!("{}\n", format_args!($($arg)*)))
    };
}
