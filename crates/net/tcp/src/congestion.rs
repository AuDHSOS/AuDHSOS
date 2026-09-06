// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reno congestion control, as RFC 5681 states it (D-50).
//!
//! Two windows decide how much may be in flight: the one the receiver
//! advertises, which says what it has room for, and the one kept here,
//! which says what the path has room for. The smaller of the two wins,
//! and this is the second.
//!
//! It grows two ways. Below the slow start threshold it grows by one
//! segment per segment acknowledged, which doubles it every round trip;
//! above the threshold it grows by roughly one segment per round trip,
//! which is the linear increase of section 3.1. It shrinks two ways as
//! well, and the difference between them is the whole of Reno: three
//! duplicate acknowledgments mean a segment was lost while the ones
//! behind it arrived, so the path still carries traffic and the window is
//! halved; a timeout means nothing came back at all, so the window goes
//! to one segment and slow start begins again.
//!
//! Fast recovery is the part that is easy to get subtly wrong. After the
//! third duplicate acknowledgment the window is set to the halved
//! threshold plus three segments — the three that the duplicates prove
//! have left the network — and each further duplicate adds one more, so
//! that new data keeps flowing while the retransmission is in the air.
//! The first acknowledgment of new data deflates the window back to the
//! threshold and recovery ends.

use crate::seq::SeqNumber;

/// How many duplicate acknowledgments mean a segment was lost rather than
/// reordered (RFC 5681, section 3.2).
pub const DUPLICATE_THRESHOLD: u32 = 3;

/// The threshold a connection starts with. RFC 5681, section 3.1 allows
/// any arbitrarily high value; this one is high enough that slow start
/// ends at a loss and not at a number.
const INITIAL_THRESHOLD: u32 = u32::MAX;

/// The window after a timeout, in segments (RFC 5681, section 3.1).
const LOSS_WINDOW_SEGMENTS: u32 = 1;

/// The smallest a halved threshold may become, in segments.
const MINIMUM_THRESHOLD_SEGMENTS: u32 = 2;

/// What the path is believed to carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Congestion {
    /// The congestion window, in bytes.
    window: u32,
    /// The slow start threshold, in bytes.
    threshold: u32,
    /// The largest segment this connection sends, in bytes.
    max_segment: u32,
    /// How many duplicate acknowledgments have arrived in a row.
    duplicates: u32,
    /// The highest number sent when recovery began, or `None` outside
    /// recovery.
    recovering_until: Option<SeqNumber>,
}

impl Congestion {
    /// A window for a connection whose segments are `max_segment` bytes.
    #[must_use]
    pub const fn new(max_segment: u32) -> Congestion {
        Congestion {
            window: initial_window(max_segment),
            threshold: INITIAL_THRESHOLD,
            max_segment,
            duplicates: 0,
            recovering_until: None,
        }
    }

    /// How many bytes the path is believed to carry.
    #[must_use]
    pub const fn window(&self) -> u32 {
        self.window
    }

    /// Where slow start ends.
    #[must_use]
    pub const fn threshold(&self) -> u32 {
        self.threshold
    }

    /// How many duplicate acknowledgments have arrived in a row.
    #[must_use]
    pub const fn duplicates(&self) -> u32 {
        self.duplicates
    }

    /// Whether the connection is in fast recovery.
    #[must_use]
    pub const fn is_recovering(&self) -> bool {
        self.recovering_until.is_some()
    }

    /// Whether the window is still doubling every round trip.
    #[must_use]
    pub const fn is_slow_start(&self) -> bool {
        self.window < self.threshold
    }

    /// Sets the segment size, which the peer's announcement and the
    /// interface MTU decide, and sizes the initial window from it.
    ///
    /// A connection does this once, before it has sent anything.
    pub const fn set_max_segment(&mut self, max_segment: u32) {
        self.max_segment = max_segment;
        self.window = initial_window(max_segment);
    }

    /// An acknowledgment of `acked` new bytes.
    pub const fn on_ack(&mut self, acked: u32) {
        self.duplicates = 0;
        if self.recovering_until.is_some() {
            // Recovery ends at the first acknowledgment of new data, and
            // the window comes back down to the threshold it was inflated
            // from (RFC 5681, section 3.2, step 6).
            self.recovering_until = None;
            self.window = self.threshold;
            return;
        }
        let growth = if self.window < self.threshold {
            // Slow start: one segment per segment acknowledged.
            if acked < self.max_segment {
                acked
            } else {
                self.max_segment
            }
        } else {
            // Congestion avoidance: roughly one segment per round trip.
            let squared = self.max_segment.saturating_mul(self.max_segment);
            match squared.checked_div(self.window) {
                // One byte per acknowledgment is the floor: a window that
                // has grown past the square of a segment would otherwise
                // stop growing altogether.
                Some(0) | None => 1,
                Some(share) => share,
            }
        };
        self.window = self.window.saturating_add(growth);
    }

    /// A duplicate acknowledgment, with `in_flight` bytes outstanding.
    ///
    /// Answers whether this one is the third, which is when the lost
    /// segment is sent again without waiting for the timer.
    pub const fn on_duplicate_ack(&mut self, in_flight: u32, highest: SeqNumber) -> bool {
        self.duplicates = self.duplicates.saturating_add(1);
        if self.recovering_until.is_some() {
            // Every further duplicate is one more segment that has left
            // the network, so one more may go in.
            self.window = self.window.saturating_add(self.max_segment);
            return false;
        }
        if self.duplicates < DUPLICATE_THRESHOLD {
            return false;
        }
        self.threshold = self.halved(in_flight);
        self.window = self
            .threshold
            .saturating_add(self.max_segment.saturating_mul(DUPLICATE_THRESHOLD));
        self.recovering_until = Some(highest);
        true
    }

    /// The retransmission timer expired, with `in_flight` bytes
    /// outstanding.
    pub const fn on_timeout(&mut self, in_flight: u32) {
        self.threshold = self.halved(in_flight);
        self.window = self.max_segment.saturating_mul(LOSS_WINDOW_SEGMENTS);
        self.duplicates = 0;
        self.recovering_until = None;
    }

    /// Half of what is in flight, but never below two segments
    /// (RFC 5681, section 3.1, equation 4).
    const fn halved(&self, in_flight: u32) -> u32 {
        let half = in_flight.saturating_div(2);
        let floor = self.max_segment.saturating_mul(MINIMUM_THRESHOLD_SEGMENTS);
        if half < floor { floor } else { half }
    }
}

/// The window a connection starts with, from equation 1 of RFC 5681,
/// section 3.1: four segments for a small one, three for a middling one,
/// two for a large one.
const fn initial_window(max_segment: u32) -> u32 {
    let segments = if max_segment > 2190 {
        2
    } else if max_segment > 1095 {
        3
    } else {
        4
    };
    max_segment.saturating_mul(segments)
}
