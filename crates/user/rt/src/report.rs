// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A line of text in a fixed number of bytes, which is what a program that
//! has no heap can format into.
//!
//! Everything a user program says — a log line, the report of a panic —
//! goes out as bytes in a message, and the bytes have to be built
//! somewhere. [`Line`] is that somewhere: an array the program owns, a
//! [`core::fmt::Write`] over it, and a length.
//!
//! A line that does not fit is cut and says so. It does not fail: a panic
//! that cannot report because its own message was too long would be the
//! worst of both, and a reader that sees the mark knows the rest is
//! missing.
//!
//! Invariant: the bytes handed out are the ones that were written, in
//! order, and never more than the array holds.

use core::fmt::{self, Write};

/// The mark a cut line ends with.
pub const ELLIPSIS: &[u8] = b"...";

/// A line of at most `N` bytes.
#[derive(Clone, Copy, Debug)]
pub struct Line<const N: usize> {
    bytes: [u8; N],
    len: usize,
    truncated: bool,
}

impl<const N: usize> Default for Line<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Line<N> {
    /// An empty line.
    #[must_use]
    pub const fn new() -> Self {
        Line {
            bytes: [0; N],
            len: 0,
            truncated: false,
        }
    }

    /// How many bytes the line holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` before anything was written.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `true` when something did not fit and was dropped.
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// The bytes that were written.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// Forgets everything written so far.
    pub const fn clear(&mut self) {
        self.len = 0;
        self.truncated = false;
    }

    /// Appends what fits of `bytes` and marks the line when something did
    /// not.
    pub fn put(&mut self, bytes: &[u8]) {
        let room = N.saturating_sub(self.len);
        let taken = bytes.len().min(room);
        let end = self.len.wrapping_add(taken);
        if let (Some(slot), Some(source)) = (self.bytes.get_mut(self.len..end), bytes.get(..taken))
        {
            slot.copy_from_slice(source);
            self.len = end;
        }
        if taken < bytes.len() {
            self.truncated = true;
            self.mark();
        }
    }

    /// Writes [`ELLIPSIS`] over the last bytes of the line, which is how a
    /// reader of the output sees that something was cut: `is_truncated`
    /// says it to the program, and the mark says it to whoever reads the
    /// line.
    ///
    /// A line with no room for the mark carries none. Three dots in place
    /// of the only three bytes there was room for say less than the bytes
    /// do.
    fn mark(&mut self) {
        let Some(from) = self.len.checked_sub(ELLIPSIS.len()) else {
            return;
        };
        if let Some(slot) = self.bytes.get_mut(from..self.len) {
            slot.copy_from_slice(ELLIPSIS);
        }
    }

    /// Formats `arguments` into a fresh line.
    ///
    /// This is the whole of what a caller needs: `Line::of(format_args!(…))`
    /// gives back the line and never fails, because a line that does not
    /// fit is cut rather than refused.
    #[must_use]
    pub fn of(arguments: fmt::Arguments<'_>) -> Self {
        let mut line = Line::new();
        // The writer absorbs what does not fit, so this cannot fail; the
        // result is dropped rather than checked because there is nothing
        // it could say.
        let _written = line.write_fmt(arguments);
        line
    }
}

impl<const N: usize> Write for Line<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.put(text.as_bytes());
        Ok(())
    }
}
