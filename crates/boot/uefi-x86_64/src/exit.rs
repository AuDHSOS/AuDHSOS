// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! How the loader ends when it cannot go on.
//!
//! Invariant: every failure of the loader reaches the machine as the same
//! exit byte, so that the test runner can tell a loader failure from a
//! kernel failure.

use core::arch::asm;
use core::fmt::{self, Write};

use crate::firmware::Firmware;

/// Port of the `isa-debug-exit` device.
pub(crate) const EXIT_PORT: u16 = 0xF4;

/// The byte that reports a loader failure; the machine exits with `37`.
pub(crate) const FAILURE_BYTE: u32 = 0x12;

/// Number of bytes a diagnostic may occupy.
const MESSAGE_BYTES: usize = 192;

/// A diagnostic built without a heap. A piece that does not fit is
/// dropped, so that a failure report never becomes a second failure.
struct Message {
    bytes: [u8; MESSAGE_BYTES],
    len: usize,
}

impl Message {
    const fn new() -> Self {
        Message {
            bytes: [0; MESSAGE_BYTES],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        self.bytes
            .get(..self.len)
            .and_then(|bytes| core::str::from_utf8(bytes).ok())
            .unwrap_or("")
    }
}

impl Write for Message {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.saturating_add(text.len());
        if let Some(slot) = self.bytes.get_mut(self.len..end) {
            slot.copy_from_slice(text.as_bytes());
            self.len = end;
        }
        Ok(())
    }
}

/// Reports `message` on the firmware console and ends the machine.
pub(crate) fn fail(firmware: &Firmware<'_>, message: &str) -> ! {
    firmware.output_string("[loader] ");
    firmware.output_string(message);
    firmware.output_string("\r\n");
    die()
}

/// Reports a formatted diagnostic and ends the machine.
pub(crate) fn fail_with(firmware: &Firmware<'_>, arguments: fmt::Arguments<'_>) -> ! {
    let mut message = Message::new();
    let _ = message.write_fmt(arguments);
    fail(firmware, message.as_str())
}

/// Ends the machine with the loader failure status. Nothing after the
/// write runs; the halt loop is there for a machine without the device.
pub(crate) fn die() -> ! {
    // SAFETY: the port belongs to the exit device of the machine the tests
    // run on, which no firmware driver claims, and the machine ends here.
    unsafe {
        asm!("out dx, eax", in("dx") EXIT_PORT, in("eax") FAILURE_BYTE, options(nomem, nostack, preserves_flags));
    }
    loop {
        core::hint::spin_loop();
    }
}
