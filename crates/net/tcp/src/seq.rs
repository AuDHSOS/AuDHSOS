// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Sequence numbers, which are points on a circle of `2^32` and not
//! numbers on a line.
//!
//! Every comparison in RFC 9293 is a comparison of distances, never of
//! magnitudes: `a` is before `b` when going forward from `a` reaches `b`
//! in fewer than `2^31` steps. Written that way the wrap at the top of
//! the range costs nothing — the arithmetic is the same on both sides of
//! it — and the one case the relation cannot decide, two numbers exactly
//! `2^31` apart, is the case a window of at most 65535 bytes can never
//! produce.
//!
//! What a window cannot produce, a peer can write. An acknowledgment
//! number is bounded by no window, so a segment may carry one exactly
//! `2^31` past the number it is judged against, and `before` and `after`
//! are then both false. A test written as two comparisons therefore has
//! a third outcome, and whoever writes one has to say what happens in it
//! rather than let a segment fall between the branches.
//!
//! This module is separate and table-driven in its tests because every
//! other decision of the protocol rests on it: whether a segment is
//! acceptable, whether an acknowledgment is new, and where a segment's
//! bytes belong in the receive buffer are all this comparison under
//! different names.

use core::fmt;

/// Half the circle. A distance below it is forward, one above it is
/// backward.
const HALF: u32 = 0x8000_0000;

/// A point on the sequence circle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SeqNumber(u32);

impl SeqNumber {
    /// The number at `value`.
    #[must_use]
    pub const fn new(value: u32) -> SeqNumber {
        SeqNumber(value)
    }

    /// The number on the wire.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// This number `count` steps forward.
    #[must_use]
    pub const fn add(self, count: u32) -> SeqNumber {
        SeqNumber(self.0.wrapping_add(count))
    }

    /// This number `count` steps back.
    #[must_use]
    pub const fn sub(self, count: u32) -> SeqNumber {
        SeqNumber(self.0.wrapping_sub(count))
    }

    /// How many steps forward it is from `earlier` to this number.
    ///
    /// The answer is a distance on the circle, so it is meaningful only
    /// when the caller knows the two are less than half a circle apart —
    /// which for a window of at most 65535 bytes they always are.
    #[must_use]
    pub const fn distance_from(self, earlier: SeqNumber) -> u32 {
        self.0.wrapping_sub(earlier.0)
    }

    /// Whether this number comes before `other`, which is the `<` of
    /// RFC 9293.
    #[must_use]
    pub const fn before(self, other: SeqNumber) -> bool {
        let forward = other.0.wrapping_sub(self.0);
        forward != 0 && forward < HALF
    }

    /// Whether this number comes after `other`.
    #[must_use]
    pub const fn after(self, other: SeqNumber) -> bool {
        other.before(self)
    }

    /// Whether this number comes before `other` or is `other`.
    #[must_use]
    pub const fn before_or_equal(self, other: SeqNumber) -> bool {
        self.0 == other.0 || self.before(other)
    }

    /// Whether this number comes after `other` or is `other`.
    #[must_use]
    pub const fn after_or_equal(self, other: SeqNumber) -> bool {
        self.0 == other.0 || self.after(other)
    }

    /// Whether this number lies in the window of `len` numbers that starts
    /// at `start`. An empty window holds nothing, not even its start.
    #[must_use]
    pub const fn is_in_window(self, start: SeqNumber, len: u32) -> bool {
        len != 0 && self.distance_from(start) < len
    }
}

impl fmt::Display for SeqNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Whether a segment of `len` sequence numbers beginning at `seq` is
/// acceptable to a receiver whose next expected number is `next` and whose
/// window is `window` wide.
///
/// These are the four cases of RFC 9293, section 3.4, in its order. The
/// two that matter are the last: a segment that carries data is acceptable
/// when either end of it falls in the window, so a segment that begins
/// before the window but reaches into it is taken and trimmed rather than
/// dropped, which is what makes a retransmission of a partly acknowledged
/// segment useful instead of wasted.
#[must_use]
pub const fn is_acceptable(seq: SeqNumber, len: u32, next: SeqNumber, window: u32) -> bool {
    match (len, window) {
        (0, 0) => seq.get() == next.get(),
        (0, _) => seq.is_in_window(next, window),
        (_, 0) => false,
        (_, _) => {
            seq.is_in_window(next, window)
                || seq.add(len.wrapping_sub(1)).is_in_window(next, window)
        }
    }
}
