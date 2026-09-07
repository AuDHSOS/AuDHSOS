// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the thread the interrupt wakes does: empty the output buffer, and
//! let both lines assert again.
//!
//! The controller has one buffer and two lines, so one drain empties what
//! either of them filled. Both lines are therefore acknowledged whichever
//! one woke the thread: a line left masked because the byte in the buffer
//! belonged to the other device would never assert again, and that device
//! would go quiet for good.
//!
//! Invariant: the acknowledgement comes after the drain and never before
//! it, so a byte that arrives while the buffer is being emptied raises the
//! line again instead of being lost.

use driver_i8042::controller::{Controller, MAX_POLLS, Ports};

/// One of the two lines of the controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Line {
    /// The line the keyboard asserts, which is ISA line one.
    Keyboard,
    /// The line the mouse asserts, which is ISA line twelve.
    Mouse,
}

impl Line {
    /// Both lines, in table order.
    pub const ALL: [Line; 2] = [Line::Keyboard, Line::Mouse];

    /// The name of the line.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Line::Keyboard => "keyboard",
            Line::Mouse => "mouse",
        }
    }
}

/// The interrupt objects of the two lines, as the process holds them.
pub trait Lines {
    /// Lets `line` assert again.
    fn acknowledge(&mut self, line: Line);
}

/// How the byte and its `AUX` bit are packed into one word: the byte in the
/// low eight bits and the bit above them, so that a message carries one
/// word per byte and the decoder on the far side knows which device it came
/// from.
pub const AUX_FLAG: u64 = 0x100;

/// Packs one byte of the controller and where it came from.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "widening a byte into the word that carries it, in a const fn"
)]
pub const fn pack(byte: u8, aux: bool) -> u64 {
    let flag = if aux { AUX_FLAG } else { 0 };
    (byte as u64) | flag
}

/// Takes one packed word apart.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "the mask keeps the byte inside u8, in a const fn"
)]
pub const fn unpack(word: u64) -> (u8, bool) {
    ((word & 0xFF) as u8, word & AUX_FLAG != 0)
}

/// Empties the output buffer into `into` and lets both lines assert again,
/// and answers with how many words it wrote.
///
/// The drain stops when the buffer is empty, when `into` is full, or after
/// [`MAX_POLLS`] bytes: a controller that keeps handing bytes out is a
/// controller this thread stops reading rather than one it serves for ever.
pub fn service<P: Ports, L: Lines>(
    controller: &mut Controller<P>,
    lines: &mut L,
    into: &mut [u64],
) -> usize {
    let mut written = 0usize;
    let mut polls = 0u32;
    while polls < MAX_POLLS {
        polls = polls.saturating_add(1);
        let Some(slot) = into.get_mut(written) else {
            break;
        };
        let Some((byte, aux)) = controller.take() else {
            break;
        };
        *slot = pack(byte, aux);
        written = written.saturating_add(1);
    }
    for line in Line::ALL {
        lines.acknowledge(line);
    }
    written
}
