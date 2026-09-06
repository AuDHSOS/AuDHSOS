// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The eleven states of RFC 9293, section 3.3.2.
//!
//! The names are the memo's, and so is the meaning of each: `Closed` is
//! not a state a connection is in but the absence of one, `Listen` is a
//! connection waiting for a peer to begin, and the nine between them are
//! the ones a connection passes through while it opens, carries bytes,
//! and closes.
//!
//! Two questions are asked of a state often enough to be written here
//! rather than at each site. Whether it is synchronized — whether the two
//! ends have agreed on sequence numbers — decides how an unacceptable
//! segment is answered, because an unsynchronized connection answers with
//! a reset and a synchronized one with an acknowledgment (RFC 9293,
//! section 3.10.7.4). And whether a state still carries data in either
//! direction is what a caller asks before it offers bytes or waits for
//! them.

use core::fmt;

/// Where a connection stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// No connection at all.
    #[default]
    Closed,
    /// Waiting for a peer to open one.
    Listen,
    /// A `SYN` has been sent and not yet answered.
    SynSent,
    /// A `SYN` has been received and answered, and its acknowledgment is
    /// outstanding.
    SynReceived,
    /// Open in both directions.
    Established,
    /// This end closed first and is waiting for the acknowledgment of its
    /// `FIN`, for the peer's `FIN`, or for both.
    FinWait1,
    /// This end's `FIN` is acknowledged; the peer has not closed yet.
    FinWait2,
    /// The peer closed first; this end may still send.
    CloseWait,
    /// Both ends closed at once and neither `FIN` is acknowledged yet.
    Closing,
    /// The peer closed first, this end followed, and its `FIN` is
    /// outstanding.
    LastAck,
    /// Both are closed. The connection waits out twice the maximum segment
    /// lifetime, so that a delayed segment of it cannot be taken for a
    /// segment of the next connection between the same two ports.
    TimeWait,
}

impl State {
    /// Whether the two ends have agreed on sequence numbers.
    ///
    /// RFC 9293, section 3.10.7.4 turns on this: an unacceptable segment
    /// reaching a synchronized connection is answered with an
    /// acknowledgment, and one reaching an unsynchronized connection with
    /// a reset.
    #[must_use]
    pub const fn is_synchronized(self) -> bool {
        !matches!(
            self,
            State::Closed | State::Listen | State::SynSent | State::SynReceived
        )
    }

    /// Whether a connection in this state has been opened at all, whatever
    /// has happened to it since.
    #[must_use]
    pub const fn is_open(self) -> bool {
        !matches!(self, State::Closed)
    }

    /// Whether this end may still hand bytes to the peer. It may until it
    /// has sent its own `FIN`.
    #[must_use]
    pub const fn can_send(self) -> bool {
        matches!(self, State::Established | State::CloseWait)
    }

    /// Whether more bytes may still arrive. They may until the peer's
    /// `FIN` has been seen.
    #[must_use]
    pub const fn can_receive(self) -> bool {
        matches!(self, State::Established | State::FinWait1 | State::FinWait2)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::Closed => "CLOSED",
            State::Listen => "LISTEN",
            State::SynSent => "SYN-SENT",
            State::SynReceived => "SYN-RECEIVED",
            State::Established => "ESTABLISHED",
            State::FinWait1 => "FIN-WAIT-1",
            State::FinWait2 => "FIN-WAIT-2",
            State::CloseWait => "CLOSE-WAIT",
            State::Closing => "CLOSING",
            State::LastAck => "LAST-ACK",
            State::TimeWait => "TIME-WAIT",
        })
    }
}
