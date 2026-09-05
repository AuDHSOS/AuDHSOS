// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Encoding the file names and diagnostics the loader passes to the
//! firmware.
//!
//! Invariant: the slice that comes back always ends with the terminating
//! zero the firmware expects.

use core::fmt;

/// Why a string could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Utf16Error {
    /// The string holds a character outside ASCII.
    NotAscii,
    /// The buffer cannot hold the string and its terminator.
    BufferTooSmall {
        /// Number of code units the string needs, terminator included.
        needed: usize,
    },
}

impl fmt::Display for Utf16Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Utf16Error::NotAscii => f.write_str("the string is not ASCII"),
            Utf16Error::BufferTooSmall { needed } => {
                write!(f, "the buffer needs {needed} code units")
            }
        }
    }
}

/// Writes `ascii` into `out` as UTF-16 with a terminating zero and returns
/// the part that was written, terminator included.
///
/// # Errors
///
/// [`Utf16Error::NotAscii`] for a character outside ASCII;
/// [`Utf16Error::BufferTooSmall`] if `out` is shorter than the string plus
/// its terminator.
pub fn encode<'a>(ascii: &str, out: &'a mut [u16]) -> Result<&'a [u16], Utf16Error> {
    let needed = ascii.len().saturating_add(1);
    if out.len() < needed {
        return Err(Utf16Error::BufferTooSmall { needed });
    }
    if !ascii.is_ascii() {
        return Err(Utf16Error::NotAscii);
    }
    for (slot, byte) in out.iter_mut().zip(ascii.bytes()) {
        *slot = u16::from(byte);
    }
    if let Some(slot) = out.get_mut(ascii.len()) {
        *slot = 0;
    }
    out.get(..needed)
        .ok_or(Utf16Error::BufferTooSmall { needed })
}
