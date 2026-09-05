// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Path MTU discovery of RFC 8201.
//!
//! It is not optional here. A router never fragments an IPv6 packet
//! (RFC 8200, section 4.5), so a packet that is too big for a link on the
//! way comes back as a packet-too-big message and is otherwise simply
//! lost. A host that ignored those messages would work over a path whose
//! links all carry 1500 bytes and stop dead over one that does not.
//!
//! Three rules make the whole of it, and all three are RFC 8201,
//! section 4:
//!
//! - the estimate is never raised by a message. One that claims more is
//!   a stale packet, a forgery, or a second path, and none of the three
//!   is a reason to send larger packets;
//! - it is never lowered below 1280, the minimum MTU of the format. A
//!   message reporting less than that is discarded outright;
//! - an increase is looked for only by trying a larger packet, and not
//!   sooner than five minutes after the last message. This table forgets
//!   an entry after ten minutes, which is the value the document
//!   recommends, and the estimate goes back to the link MTU until
//!   something says otherwise.
//!
//! Nothing here sends. The table answers *what fits on the way to this
//! address*, and [`crate::send`] asks it before every packet, so a packet
//! larger than the estimate is one this crate cannot write.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use net_wire::Ipv6Addr;

use crate::header::MIN_MTU;

/// How long an estimate is kept before the path is tried at the link MTU
/// again. RFC 8201, section 4 gives five minutes as the floor and
/// recommends twice that.
pub const INCREASE_INTERVAL: Duration = Duration::from_secs(600);

/// One path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    /// Where it goes.
    destination: Ipv6Addr,
    /// What fits.
    mtu: usize,
    /// When that was last lowered.
    learned_at: Instant,
}

/// What fits on the way to each of `N` destinations.
///
/// A new path evicts the one whose estimate is oldest when the table is
/// full: that is the one closest to being forgotten anyway, and losing it
/// costs one packet-too-big message.
#[derive(Debug)]
pub struct PathMtu<const N: usize> {
    /// The paths that have reported.
    entries: ArrayVec<Entry, N>,
    /// How long an estimate is kept.
    interval: Duration,
}

impl<const N: usize> Default for PathMtu<N> {
    fn default() -> PathMtu<N> {
        PathMtu::new()
    }
}

impl<const N: usize> PathMtu<N> {
    /// An empty table on the interval RFC 8201 recommends.
    #[must_use]
    pub const fn new() -> PathMtu<N> {
        PathMtu {
            entries: ArrayVec::new(),
            interval: INCREASE_INTERVAL,
        }
    }

    /// An empty table that forgets an estimate after `interval`.
    #[must_use]
    pub const fn with_interval(interval: Duration) -> PathMtu<N> {
        PathMtu {
            entries: ArrayVec::new(),
            interval,
        }
    }

    /// How many paths have reported.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether none have.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Forgets every estimate.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// What fits on the way to `destination` over a link of `link_mtu`.
    ///
    /// The answer is the smaller of the two. Every estimate this table
    /// holds is at least the minimum MTU, because
    /// [`on_packet_too_big`](Self::on_packet_too_big) discards a message
    /// that reports less — which is the rule of RFC 8201, section 4 and
    /// the whole of what the floor means here.
    ///
    /// A `link_mtu` below the minimum is answered as it stands and not
    /// raised to it. Such a link cannot carry IPv6, and the send path
    /// says so with an error; answering 1280 for a link that carries 576
    /// would turn that into frames the driver silently could not send.
    #[must_use]
    pub fn mtu(&self, destination: Ipv6Addr, link_mtu: usize) -> usize {
        let path = self
            .entries
            .iter()
            .find(|entry| entry.destination == destination)
            .map_or(link_mtu, |entry| entry.mtu);
        path.min(link_mtu)
    }

    /// Takes a packet-too-big message in, and answers whether it lowered
    /// anything.
    ///
    /// `reported` is the MTU the message carries. It is refused when it
    /// is below the minimum, and ignored when it is not smaller than what
    /// is already known.
    pub fn on_packet_too_big(
        &mut self,
        destination: Ipv6Addr,
        reported: u32,
        now: Instant,
    ) -> bool {
        let reported = widen(reported);
        if reported < MIN_MTU {
            return false;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.destination == destination)
        {
            if reported >= entry.mtu {
                return false;
            }
            entry.mtu = reported;
            entry.learned_at = now;
            return true;
        }
        let entry = Entry {
            destination,
            mtu: reported,
            learned_at: now,
        };
        if self.entries.push(entry).is_ok() {
            return true;
        }
        let Some((index, _)) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, existing)| existing.learned_at)
        else {
            // A table of no entries at all, which holds nothing and
            // reports nothing.
            return false;
        };
        self.entries.remove(index);
        self.entries.push(entry).is_ok()
    }

    /// When an estimate is next old enough to be tried again, or `None`
    /// when there is none.
    #[must_use]
    pub fn poll_at(&self) -> Option<Instant> {
        self.entries
            .iter()
            .map(|entry| entry.learned_at.saturating_add(self.interval))
            .min()
    }

    /// Forgets every estimate older than the interval, and answers how
    /// many went.
    ///
    /// Forgetting is how an increase is detected: the next packet goes
    /// out at the link MTU, and either it arrives or a message brings the
    /// estimate back down.
    pub fn poll(&mut self, now: Instant) -> usize {
        let before = self.entries.len();
        let interval = self.interval;
        let mut index = 0;
        while index < self.entries.len() {
            let old = self
                .entries
                .get(index)
                .is_some_and(|entry| entry.learned_at.saturating_add(interval) <= now);
            if old {
                self.entries.remove(index);
            } else {
                index = index.saturating_add(1);
            }
        }
        before.saturating_sub(self.entries.len())
    }
}

/// The reported MTU as a length.
///
/// A `u32` is no wider than a `usize` on any target this system builds
/// for, so this is a widening cast and cannot lose a value.
#[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
const fn widen(mtu: u32) -> usize {
    mtu as usize
}
