// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod bits;
mod decode;
mod encode;
mod huffman;
mod tables;
mod zlib;

#[cfg(test)]
mod tests;

pub use tables::WINDOW;
pub use zlib::adler32;

use encode::{HASH_SIZE, hash_of};

/// What can go wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The buffer the caller gave has no room for what was written.
    Output,
    /// The stream is not one this crate can read.
    Input,
    /// The stream read back, but not as the checksum in it says.
    Checksum,
}

impl core::fmt::Display for Error {
    fn fmt(&self, out: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let said = match self {
            Error::Output => "the buffer has no room for the result",
            Error::Input => "the stream is not one this crate can read",
            Error::Checksum => "the stream does not match the checksum it carries",
        };
        out.write_str(said)
    }
}

/// The table the compressor finds runs with: for every three bytes, where
/// they were last seen, and a chain back through every earlier place.
///
/// It is a quarter of a megabyte, and the caller owns it, because nothing
/// in this crate allocates. A caller that compresses more than once
/// should keep one and hand it over again: the compressor clears it
/// itself.
pub struct Scratch {
    /// Where each three bytes were last seen, one more than the place so
    /// that nothing stands for `none`.
    head: [u32; HASH_SIZE],
    /// For each place, the place before it with the same three bytes.
    prev: [u32; WINDOW],
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Scratch {
    /// A table with nothing in it.
    ///
    /// It is a quarter of a megabyte, which is why it is a value the
    /// caller holds rather than something this crate makes for itself:
    /// nothing here allocates, so where the table lives is the caller's
    /// to decide, and one that cannot spare the stack puts it behind a
    /// box. That is what the clippy exemptions below say.
    #[must_use]
    #[expect(
        clippy::large_stack_arrays,
        reason = "the table is the caller's to place; this crate allocates nothing"
    )]
    pub const fn new() -> Self {
        Self {
            head: [0; HASH_SIZE],
            prev: [0; WINDOW],
        }
    }

    /// Forgets everything.
    #[expect(
        clippy::large_stack_arrays,
        reason = "the table is the caller's to place; this crate allocates nothing"
    )]
    pub(crate) const fn clear(&mut self) {
        self.head = [0; HASH_SIZE];
        self.prev = [0; WINDOW];
    }

    /// Notes that the three bytes at `at` were seen there.
    pub(crate) fn insert(&mut self, input: &[u8], at: usize) {
        let Some(hash) = hash_of(input, at) else {
            return;
        };
        let earlier = self.head.get(hash).copied().unwrap_or(0);
        if let Some(slot) = self.prev.get_mut(at.wrapping_rem(WINDOW)) {
            *slot = earlier;
        }
        if let Some(slot) = self.head.get_mut(hash) {
            *slot = u32::try_from(at.saturating_add(1)).unwrap_or(0);
        }
    }

    /// Where the three bytes at `at` were last seen before now.
    pub(crate) fn head(&self, input: &[u8], at: usize) -> Option<usize> {
        let hash = hash_of(input, at)?;
        let stored = self.head.get(hash).copied()?;
        usize::try_from(stored.checked_sub(1)?).ok()
    }

    /// Where the three bytes at `at` were seen before that.
    pub(crate) fn previous(&self, at: usize) -> Option<usize> {
        let stored = self.prev.get(at.wrapping_rem(WINDOW)).copied()?;
        usize::try_from(stored.checked_sub(1)?).ok()
    }
}

/// Compresses `input` into `out`, and says how many bytes it wrote.
///
/// What comes out is a DEFLATE stream: the body of what a PDF calls a
/// `FlateDecode` filter, without the wrapper. An input that will not
/// compress is written unchanged in blocks that say so, so the result is
/// never more than a few bytes longer than what went in.
///
/// # Errors
///
/// [`Error::Output`] when `out` has no room for the result.
pub fn compress(input: &[u8], out: &mut [u8], scratch: &mut Scratch) -> Result<usize, Error> {
    encode::compress(input, out, scratch)
}

/// Compresses `input` into `out` as a zlib stream, and says how many
/// bytes it wrote: the two header bytes of RFC 1950, the DEFLATE stream,
/// and the checksum.
///
/// # Errors
///
/// [`Error::Output`] when `out` has no room for the result.
pub fn compress_zlib(input: &[u8], out: &mut [u8], scratch: &mut Scratch) -> Result<usize, Error> {
    zlib::wrap(input, out, scratch)
}

/// Reads a DEFLATE stream into `out`, and says how many bytes it stood
/// for.
///
/// # Errors
///
/// [`Error::Input`] for a stream this crate cannot read, and
/// [`Error::Output`] when `out` has no room for what it holds.
pub fn decompress(input: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    decode::inflate(input, out)
}

/// Reads a zlib stream into `out`, and says how many bytes it stood for.
///
/// # Errors
///
/// [`Error::Input`] for a stream this crate cannot read,
/// [`Error::Output`] when `out` has no room for what it holds, and
/// [`Error::Checksum`] when it reads back as something other than what
/// the checksum in it says.
pub fn decompress_zlib(input: &[u8], out: &mut [u8]) -> Result<usize, Error> {
    zlib::unwrap(input, out)
}

/// How much room `out` needs for an input of `length` bytes, whatever it
/// holds: what a stream that will not compress at all takes.
#[must_use]
pub const fn bound(length: usize) -> usize {
    encode::stored_length(length)
}

/// How much room a zlib stream of that input needs: the same, and six
/// bytes of wrapper.
#[must_use]
pub const fn bound_zlib(length: usize) -> usize {
    bound(length).saturating_add(6)
}
