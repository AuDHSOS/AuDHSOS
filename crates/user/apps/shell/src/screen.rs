// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The scrollback: [`ROWS`] rows of [`COLUMNS`] characters, the oldest
//! dropping off the top.
//!
//! What is printed is cut into rows: at every line break, and at the right
//! edge, so a line longer than the window continues on the next row rather
//! than being lost. A byte that is not a printable character of ASCII is
//! written as a full stop, because the font draws those and what comes out
//! of a network is not known to be text.
//!
//! Invariants: every row holds at most [`COLUMNS`] bytes, and every byte
//! of every row is a printable character of ASCII, so a row is a string
//! without being checked again.

use user_proto::Bytes;

/// How many characters one row holds.
pub const COLUMNS: usize = 56;

/// How many rows of scrollback stand above the line being typed.
pub const ROWS: usize = 22;

/// What is written for a byte the font has no glyph for.
pub const REPLACEMENT: u8 = b'.';

/// One row of the scrollback.
pub type Row = Bytes<COLUMNS>;

/// The rows above the line being typed.
#[derive(Clone, Copy, Debug)]
pub struct Screen {
    /// The rows, oldest at `top`.
    rows: [Row; ROWS],
    /// How many of them hold text.
    len: usize,
    /// Where the oldest row stands.
    top: usize,
}

impl Default for Screen {
    fn default() -> Self {
        Screen::new()
    }
}

impl Screen {
    /// A scrollback of no rows.
    #[must_use]
    pub const fn new() -> Self {
        Screen {
            rows: [const { Row::empty() }; ROWS],
            len: 0,
            top: 0,
        }
    }

    /// How many rows hold text.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` when nothing has been printed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Forgets every row.
    pub const fn clear(&mut self) {
        self.rows = [const { Row::empty() }; ROWS];
        self.len = 0;
        self.top = 0;
    }

    /// The row at `index`, the oldest first, and the empty string for an
    /// index below no row.
    #[must_use]
    pub fn row(&self, index: usize) -> &str {
        self.rows
            .iter()
            .cycle()
            .skip(self.top)
            .take(self.len)
            .nth(index)
            .map_or("", Row::as_str)
    }

    /// Writes `text`, cut into rows at every line break and at the right
    /// edge.
    pub fn print(&mut self, text: &str) {
        let mut row = [0_u8; COLUMNS];
        let mut len = 0;
        for byte in text.bytes() {
            if byte == b'\n' {
                self.push(Row::filled(row, len));
                len = 0;
                continue;
            }
            for slot in row.iter_mut().skip(len).take(1) {
                *slot = printable(byte);
            }
            len = len.saturating_add(1).min(COLUMNS);
            if len == COLUMNS {
                self.push(Row::filled(row, len));
                len = 0;
            }
        }
        if len > 0 {
            self.push(Row::filled(row, len));
        }
    }

    /// Puts one row at the bottom, dropping the oldest when the scrollback
    /// is full.
    fn push(&mut self, row: Row) {
        if self.len < ROWS {
            let at = self.top.wrapping_add(self.len).wrapping_rem(ROWS);
            for slot in self.rows.iter_mut().skip(at).take(1) {
                *slot = row;
            }
            self.len = self.len.saturating_add(1);
            return;
        }
        for slot in self.rows.iter_mut().skip(self.top).take(1) {
            *slot = row;
        }
        self.top = self.top.wrapping_add(1).wrapping_rem(ROWS);
    }
}

/// `byte` when the font draws it, and [`REPLACEMENT`] when it does not.
const fn printable(byte: u8) -> u8 {
    if byte >= 0x20 && byte < 0x7F {
        byte
    } else {
        REPLACEMENT
    }
}
