// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Key re-exchange (RFC 4253, section 9): when this client asks for one,
//! what it does when the peer asks, and what may be sent while one runs.
//!
//! A re-exchange is the same exchange again over the keys in effect. The
//! roles do not change, the session identifier does not change, and the
//! contexts are reset when `SSH_MSG_NEWKEYS` takes the new keys into use.
//!
//! Time enters as a parameter, as D-46 requires of every logic crate:
//! [`Rekey::due`] is given the moment it is asked about and reads no
//! clock.

use crate::error::SshError;

/// The first threshold of section 9: a gigabyte carried since the last
/// exchange.
pub const BYTES: u64 = 1 << 30;

/// The second: an hour of connection time, in microseconds, which is the
/// unit D-120 gives the clock.
pub const MICROSECONDS: u64 = 3_600 * 1_000_000;

/// The third, which is this crate's and not the document's: the sequence
/// number of section 6.4 wraps at 2^32 and a re-exchange must happen
/// before it does, so one is due at half of that.
pub const PACKETS: u64 = 1 << 31;

/// Where one connection stands between two key exchanges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Exchange {
    /// No key exchange is running, so anything may be sent.
    Settled,
    /// This side sent `SSH_MSG_KEXINIT` and waits for the peer's.
    Asked,
    /// Both sides have sent one, so only transport messages may be sent
    /// until the keys change.
    Running,
}

/// What the peer's `SSH_MSG_KEXINIT` asks of this side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Answer {
    /// The peer started the re-exchange, so this side must send its own
    /// `SSH_MSG_KEXINIT`.
    SendKexInit,
    /// The peer's message was the reply to this side's, so nothing is
    /// owed.
    Nothing,
}

/// What one connection has carried since its last key exchange, and what
/// it is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rekey {
    /// Bytes carried since the last exchange.
    bytes: u64,
    /// The moment of the last exchange, as the caller counts time.
    since: u64,
    /// Packets framed since the connection began, which the sequence
    /// number counts and a re-exchange does not reset.
    packets: u64,
    /// Where the connection stands.
    state: Exchange,
}

impl Rekey {
    /// A connection whose first key exchange is running, begun at `now`.
    ///
    /// The first exchange is the one that makes the session identifier,
    /// and it starts with both sides sending `SSH_MSG_KEXINIT`, which is
    /// why a new connection stands in [`Exchange::Running`].
    #[must_use]
    pub const fn new(now: u64) -> Rekey {
        Rekey {
            bytes: 0,
            since: now,
            packets: 0,
            state: Exchange::Running,
        }
    }

    /// Where the connection stands.
    #[must_use]
    pub const fn state(&self) -> Exchange {
        self.state
    }

    /// Whether a key exchange is running, which is what keeps everything
    /// but a transport message off the wire.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        !matches!(self.state, Exchange::Settled)
    }

    /// Bytes carried since the last exchange.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Packets framed since the connection began.
    #[must_use]
    pub const fn packets(&self) -> u64 {
        self.packets
    }

    /// Counts one packet of `bytes` in either direction.
    pub fn note(&mut self, bytes: usize) {
        let carried = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.bytes = self.bytes.saturating_add(carried);
        self.packets = self.packets.saturating_add(1);
    }

    /// Whether this side should ask for a re-exchange at `now`: a
    /// gigabyte, an hour, or half the sequence number space, whichever
    /// comes first.
    ///
    /// A moment before the last exchange counts as no time passed, so a
    /// clock that stands still asks for nothing.
    #[must_use]
    pub const fn due(&self, now: u64) -> bool {
        if self.is_running() {
            return false;
        }
        self.bytes >= BYTES
            || self.packets >= PACKETS
            || now.saturating_sub(self.since) >= MICROSECONDS
    }

    /// This side sends `SSH_MSG_KEXINIT`.
    ///
    /// # Errors
    ///
    /// [`SshError::Exchange`] when one is already running, which section
    /// 9 makes the one condition for starting a re-exchange.
    pub const fn ask(&mut self) -> Result<(), SshError> {
        if self.is_running() {
            return Err(SshError::Exchange);
        }
        self.state = Exchange::Asked;
        Ok(())
    }

    /// The peer's `SSH_MSG_KEXINIT` arrived, and this is what it asks
    /// for: a `SSH_MSG_KEXINIT` back unless this side's was already sent.
    ///
    /// # Errors
    ///
    /// [`SshError::Exchange`] when both sides have already sent one,
    /// which is a third `SSH_MSG_KEXINIT` in one exchange.
    pub const fn peer_asked(&mut self) -> Result<Answer, SshError> {
        match self.state {
            Exchange::Settled => {
                self.state = Exchange::Running;
                Ok(Answer::SendKexInit)
            }
            Exchange::Asked => {
                self.state = Exchange::Running;
                Ok(Answer::Nothing)
            }
            Exchange::Running => Err(SshError::Exchange),
        }
    }

    /// This side answered the peer's `SSH_MSG_KEXINIT` with its own.
    ///
    /// # Errors
    ///
    /// [`SshError::Exchange`] when no exchange is running.
    pub const fn answered(&mut self) -> Result<(), SshError> {
        if matches!(self.state, Exchange::Settled) {
            return Err(SshError::Exchange);
        }
        self.state = Exchange::Running;
        Ok(())
    }

    /// The new keys are in use: the exchange is over, and what it counted
    /// starts again at `now`.
    ///
    /// The packets are not reset, because the sequence number they stand
    /// for is not (section 6.4).
    ///
    /// # Errors
    ///
    /// [`SshError::Exchange`] when no exchange is running, which a
    /// `SSH_MSG_NEWKEYS` out of order is.
    pub const fn settled(&mut self, now: u64) -> Result<(), SshError> {
        if !self.is_running() {
            return Err(SshError::Exchange);
        }
        self.state = Exchange::Settled;
        self.bytes = 0;
        self.since = now;
        Ok(())
    }

    /// Whether a message of this number may be sent now.
    ///
    /// While a key exchange runs, section 9 leaves only the transport
    /// layer on the wire: everything above it — the authentication of
    /// RFC 4252 and the channels of RFC 4254, which are 50 and above —
    /// waits for the new keys.
    #[must_use]
    pub const fn may_send(&self, number: u8) -> bool {
        !self.is_running() || number < 50
    }
}
