// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One connection: the state machine of RFC 9293 with its two buffers,
//! its timers, and its congestion window.
//!
//! The shape is the one the whole track has. A segment that arrived goes
//! in through [`Connection::on_segment`] with the instant it arrived at;
//! whatever the connection owes in answer comes out through
//! [`Connection::poll`], one segment per call, into a buffer the caller
//! supplied; and [`Connection::poll_at`] says when there will be something
//! to fetch. Nothing here reads a clock, blocks, or allocates, so a
//! sixty-second backoff is exercised in microseconds of wall clock and two
//! of these can be joined back to back over a network double.
//!
//! What the connection owes is state and not a queue. Processing a segment
//! sets marks — an acknowledgment is due, a reset is due, the
//! retransmission timer expired — and `poll` reads the marks and the
//! windows and decides what one segment would best say. That is why an
//! acknowledgment that was owed twice is sent once, and why data that
//! became sendable while an acknowledgment was pending leaves as one
//! segment carrying both.

use audhsos_time::{Duration, Instant};
use crypto_rng::Rng;
use net_wire::{IpAddr, Port, Writer};

use crate::congestion::Congestion;
use crate::error::TcpError;
use crate::recv::RecvBuffer;
use crate::rto::Rto;
use crate::segment::{Flags, Segment};
use crate::send::SendBuffer;
use crate::seq::{SeqNumber, is_acceptable};
use crate::state::State;

/// The maximum segment size a connection assumes when the peer announced
/// none (RFC 9293, section 3.7.1).
pub const DEFAULT_MAX_SEGMENT: u16 = 536;

/// The smallest segment size a peer may ask for. RFC 9293, section 3.7.1
/// puts the floor at the smallest datagram every host must accept, less
/// the two headers.
pub const MIN_MAX_SEGMENT: u16 = 536;

/// How long a segment may live in the network, as this implementation
/// counts it. `TIME-WAIT` lasts twice this.
pub const MAX_SEGMENT_LIFETIME: Duration = Duration::from_secs(30);

/// One end of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// Which host.
    pub address: IpAddr,
    /// Which port on it.
    pub port: Port,
}

impl Endpoint {
    /// The endpoint at `address` on `port`.
    #[must_use]
    pub const fn new(address: IpAddr, port: Port) -> Endpoint {
        Endpoint { address, port }
    }
}

/// What a connection is configured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The largest segment this end will accept, which it announces, and
    /// the largest it will send once the peer has announced its own.
    pub max_segment: u16,
    /// How many retransmissions of one segment are attempted before the
    /// connection is given up.
    pub retransmit_limit: u32,
    /// How long an acknowledgment may be held back.
    pub delayed_ack: Duration,
    /// Whether small segments wait for outstanding data to be
    /// acknowledged.
    pub nagle: bool,
}

impl Config {
    /// What RFC 9293 and RFC 1122 ask for, with the segment size of a path
    /// whose MTU is unknown.
    pub const DEFAULT: Config = Config {
        max_segment: DEFAULT_MAX_SEGMENT,
        retransmit_limit: 8,
        // RFC 1122, section 4.2.3.2: an acknowledgment is delayed by at
        // most half a second.
        delayed_ack: Duration::from_millis(500),
        nagle: true,
    };
}

impl Default for Config {
    fn default() -> Config {
        Config::DEFAULT
    }
}

/// What one call to [`Connection::poll`] decided to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// A reset, with the end it goes to, the sequence number it carries,
    /// and the acknowledgment when there is one.
    Reset {
        /// Where it goes.
        to: Endpoint,
        /// What goes in the sequence field.
        seq: SeqNumber,
        /// What goes in the acknowledgment field, when there is one.
        ack: Option<SeqNumber>,
    },
    /// The `SYN` of an active open, or the `SYN` with `ACK` of a passive
    /// one.
    Syn {
        /// Whether it acknowledges the peer's `SYN`.
        acknowledges: bool,
    },
    /// Bytes from the send buffer, possibly with the closing `FIN`.
    Data {
        /// How far into the unacknowledged bytes they start.
        offset: usize,
        /// How many of them.
        len: usize,
        /// Whether the `FIN` rides on this segment.
        fin: bool,
        /// Whether the segment is the last of what there is to send.
        push: bool,
        /// Whether this is a retransmission, which yields no round trip
        /// measurement (Karn's rule).
        again: bool,
    },
    /// The closing `FIN` on its own.
    Fin,
    /// One byte past the peer's closed window, to find out whether it has
    /// opened.
    Probe {
        /// Where the byte is.
        offset: usize,
    },
    /// An acknowledgment and nothing else.
    Ack,
}

/// One connection.
#[derive(Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "seven independent facts about one connection; a bitset would hide what each means"
)]
pub struct Connection<'a> {
    /// Where it stands.
    state: State,
    /// This end.
    local: Endpoint,
    /// The other end, once it is known.
    remote: Endpoint,
    /// What it was configured with.
    config: Config,

    /// The oldest byte this end has sent and the peer has not
    /// acknowledged.
    snd_una: SeqNumber,
    /// The next number this end will send.
    snd_nxt: SeqNumber,
    /// The highest number this end has ever sent. A segment below it is a
    /// retransmission, which yields no round trip measurement.
    snd_max: SeqNumber,
    /// What the peer says it has room for.
    snd_wnd: u32,
    /// The sequence number of the segment that last updated the window.
    snd_wl1: SeqNumber,
    /// Its acknowledgment number.
    snd_wl2: SeqNumber,
    /// The number this end started at.
    iss: SeqNumber,

    /// The next number this end expects.
    rcv_nxt: SeqNumber,
    /// The number the peer started at.
    irs: SeqNumber,
    /// The far edge of the window last advertised, which never moves
    /// back.
    rcv_edge: SeqNumber,

    /// Bytes waiting to go out.
    send: SendBuffer<'a>,
    /// The window and what has arrived in it.
    recv: RecvBuffer<'a>,

    /// The largest segment this end sends.
    max_segment: u32,
    /// What the path is believed to carry.
    congestion: Congestion,
    /// How long to wait for an acknowledgment.
    rto: Rto,

    /// When the oldest unacknowledged segment stops being waited for.
    retransmit_at: Option<Instant>,
    /// When a held-back acknowledgment must go.
    ack_at: Option<Instant>,
    /// When the next window probe is due.
    persist_at: Option<Instant>,
    /// When `TIME-WAIT` ends.
    time_wait_until: Option<Instant>,

    /// The number being timed for a round trip measurement, and when it
    /// was sent.
    rtt_sample: Option<(SeqNumber, Instant)>,
    /// Whether an acknowledgment is owed at once.
    ack_now: bool,
    /// Whether the peer's `SYN` arrived again and the answer to it is
    /// owed again.
    resend_syn: bool,
    /// Whether three duplicate acknowledgments asked for the oldest
    /// unacknowledged segment to go again.
    fast_retransmit: bool,
    /// How many full segments have arrived since the last acknowledgment.
    since_ack: u32,
    /// How often the segment at the front of the send buffer has been
    /// sent again. RFC 1122, section 4.2.3.5 counts attempts per segment
    /// and not per connection, so this begins again as soon as the peer
    /// acknowledges something new.
    retries: u32,
    /// A reset that is owed: where it goes and the numbers it carries.
    /// Where it goes is remembered because a connection in `LISTEN` sends
    /// one to an end it is not otherwise joined to.
    reset_pending: Option<(Endpoint, SeqNumber, Option<SeqNumber>)>,
    /// Whether the caller has closed this end.
    closing: bool,
    /// The number this end's `FIN` occupies, once it has been sent.
    fin_seq: Option<SeqNumber>,
    /// Whether the peer's `FIN` has been seen.
    peer_finished: bool,
    /// Whether the peer reset the connection.
    was_reset: bool,
    /// Whether the connection was given up because a segment went
    /// unanswered too often.
    timed_out: bool,
}

impl<'a> Connection<'a> {
    /// A connection with nothing in it, over the two buffers.
    #[must_use]
    pub fn new(
        local: Endpoint,
        config: Config,
        send: &'a mut [u8],
        recv: &'a mut [u8],
    ) -> Connection<'a> {
        let max_segment = u32::from(config.max_segment);
        Connection {
            state: State::Closed,
            local,
            remote: Endpoint::new(local.address, Port::UNSPECIFIED),
            config,
            snd_una: SeqNumber::new(0),
            snd_nxt: SeqNumber::new(0),
            snd_max: SeqNumber::new(0),
            snd_wnd: 0,
            snd_wl1: SeqNumber::new(0),
            snd_wl2: SeqNumber::new(0),
            iss: SeqNumber::new(0),
            rcv_nxt: SeqNumber::new(0),
            irs: SeqNumber::new(0),
            rcv_edge: SeqNumber::new(0),
            send: SendBuffer::new(send),
            recv: RecvBuffer::new(recv),
            max_segment,
            congestion: Congestion::new(max_segment),
            rto: Rto::new(),
            retransmit_at: None,
            ack_at: None,
            persist_at: None,
            time_wait_until: None,
            rtt_sample: None,
            ack_now: false,
            resend_syn: false,
            fast_retransmit: false,
            since_ack: 0,
            retries: 0,
            reset_pending: None,
            closing: false,
            fin_seq: None,
            peer_finished: false,
            was_reset: false,
            timed_out: false,
        }
    }

    /// Gives both buffers back.
    #[must_use]
    pub const fn into_buffers(self) -> (&'a mut [u8], &'a mut [u8]) {
        (self.send.into_bytes(), self.recv.into_bytes())
    }

    /// Where the connection stands.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// This end.
    #[must_use]
    pub const fn local(&self) -> Endpoint {
        self.local
    }

    /// The other end. Its port is zero while the connection is listening.
    #[must_use]
    pub const fn remote(&self) -> Endpoint {
        self.remote
    }

    /// Whether the peer reset the connection.
    #[must_use]
    pub const fn was_reset(&self) -> bool {
        self.was_reset
    }

    /// Whether the connection was given up because one segment went
    /// unanswered for the configured number of attempts.
    #[must_use]
    pub const fn timed_out(&self) -> bool {
        self.timed_out
    }

    /// Whether the peer has closed its half, so that no more bytes will
    /// arrive.
    #[must_use]
    pub const fn peer_finished(&self) -> bool {
        self.peer_finished
    }

    /// How many bytes have arrived and can be read.
    #[must_use]
    pub const fn readable(&self) -> usize {
        self.recv.ready()
    }

    /// How many more bytes the send buffer would take.
    #[must_use]
    pub const fn writable(&self) -> usize {
        self.send.free()
    }

    /// What the path is believed to carry.
    #[must_use]
    pub const fn congestion(&self) -> Congestion {
        self.congestion
    }

    /// How long an unanswered segment is waited for.
    #[must_use]
    pub const fn rto(&self) -> Rto {
        self.rto
    }

    /// The largest segment this end sends.
    #[must_use]
    pub const fn max_segment(&self) -> u32 {
        self.max_segment
    }

    /// Puts the connection in `LISTEN`, where the first `SYN` that reaches
    /// it makes it that connection.
    ///
    /// The number this end's stream will start at is drawn here rather
    /// than when the `SYN` arrives, because the arrival of a segment is
    /// not a place a generator belongs: a peer would then decide how often
    /// this host asks for randomness.
    ///
    /// # Errors
    ///
    /// [`TcpError::WrongState`] unless the connection is closed, and
    /// [`TcpError::Rng`] when the generator fails.
    pub fn listen<R: Rng + ?Sized>(&mut self, rng: &mut R) -> Result<(), TcpError> {
        if self.state != State::Closed {
            return Err(TcpError::WrongState(self.state));
        }
        self.set_initial_sequence(rng)?;
        self.state = State::Listen;
        Ok(())
    }

    /// Opens a connection to `remote`, drawing the initial sequence number
    /// from `rng` as RFC 6528 asks.
    ///
    /// # Errors
    ///
    /// [`TcpError::WrongState`] unless the connection is closed, and
    /// [`TcpError::Rng`] when the generator fails.
    pub fn connect<R: Rng + ?Sized>(
        &mut self,
        remote: Endpoint,
        rng: &mut R,
    ) -> Result<(), TcpError> {
        if self.state != State::Closed {
            return Err(TcpError::WrongState(self.state));
        }
        if remote.address.version() != self.local.address.version() {
            return Err(TcpError::MixedFamilies);
        }
        self.remote = remote;
        self.set_initial_sequence(rng)?;
        self.state = State::SynSent;
        Ok(())
    }

    /// Hands bytes over to be sent, and answers how many were taken.
    ///
    /// A partial write is the working of a stream and not a failure: what
    /// did not fit goes when the peer acknowledges what did.
    ///
    /// # Errors
    ///
    /// [`TcpError::WrongState`] when this end has closed or is not open
    /// yet.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, TcpError> {
        if self.closing || !self.state.can_send() {
            return Err(TcpError::WrongState(self.state));
        }
        Ok(self.send.write(data))
    }

    /// Copies out what has arrived, and answers how much that was.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        self.recv.read(out)
    }

    /// The bytes that have arrived, as far as they run without leaving the
    /// end of the buffer. [`consume`](Connection::consume) drops them.
    #[must_use]
    pub fn peek(&self) -> &[u8] {
        self.recv.contiguous()
    }

    /// Drops `count` bytes that have been read.
    pub fn consume(&mut self, count: usize) {
        self.recv.consume(count);
    }

    /// Closes this end, which sends a `FIN` once everything before it has
    /// gone.
    ///
    /// # Errors
    ///
    /// [`TcpError::WrongState`] when there is nothing to close.
    pub const fn close(&mut self) -> Result<(), TcpError> {
        match self.state {
            State::Closed => Err(TcpError::WrongState(self.state)),
            State::Listen | State::SynSent => {
                self.state = State::Closed;
                Ok(())
            }
            _ => {
                self.closing = true;
                Ok(())
            }
        }
    }

    /// Tears the connection down at once, with a reset to the peer.
    pub fn abort(&mut self) {
        if self.state.is_open() && self.state != State::Listen {
            self.reset_pending = Some((self.remote, self.snd_nxt, Some(self.rcv_nxt)));
        }
        self.state = State::Closed;
        self.closing = false;
    }
}

impl Connection<'_> {
    /// Takes in a segment that arrived from `source` at `now`.
    ///
    /// Nothing goes out here. What the segment made due is marked, and
    /// [`poll`](Connection::poll) is what says it.
    pub fn on_segment(&mut self, source: IpAddr, segment: &Segment<'_>, now: Instant) {
        match self.state {
            // A closed connection answers nothing; the table above it
            // sends the reset, because it is the one that knows no
            // connection holds the port.
            State::Closed => {}
            State::Listen => self.on_segment_listen(source, segment),
            State::SynSent => self.on_segment_syn_sent(segment, now),
            _ => self.on_segment_open(segment, now),
        }
    }

    /// When the connection next has something to say, or `None` when it
    /// has nothing and no timer running.
    ///
    /// An instant that has passed means there is work now. The answer is
    /// never an instant at which [`poll`](Connection::poll) would produce
    /// nothing.
    #[must_use]
    pub fn poll_at(&self, now: Instant) -> Option<Instant> {
        if self.reset_pending.is_some() || self.ack_now || self.resend_syn || self.fast_retransmit {
            return Some(now);
        }
        if !self.state.is_open() {
            return None;
        }
        if self.state == State::TimeWait {
            return self.time_wait_until;
        }
        if self.syn_outstanding() && self.retransmit_at.is_none() {
            return Some(now);
        }
        if self.can_send_now() {
            return Some(now);
        }
        [self.retransmit_at, self.ack_at, self.persist_at]
            .into_iter()
            .flatten()
            .min()
    }

    /// Writes at most one segment into `buffer` and answers what to send.
    ///
    /// A caller loops until this answers `None`, which is when the
    /// connection has nothing further to say at this instant.
    pub fn poll<'b>(&mut self, now: Instant, buffer: &'b mut [u8]) -> Option<&'b [u8]> {
        let plan = self.decide(now)?;
        self.transmit(plan, now, buffer)
    }

    /// The `SYN` of a passive open, and everything the peer's `SYN` told
    /// this end.
    fn on_segment_listen(&mut self, source: IpAddr, segment: &Segment<'_>) {
        if segment.flags.has(Flags::RST) {
            return;
        }
        if segment.flags.has(Flags::ACK) {
            // Nothing was sent, so nothing can be acknowledged
            // (RFC 9293, section 3.10.7.2).
            self.reset_pending = Some((
                Endpoint::new(source, segment.source_port),
                segment.ack,
                None,
            ));
            return;
        }
        if !segment.flags.has(Flags::SYN) || source.version() != self.local.address.version() {
            return;
        }
        self.remote = Endpoint::new(source, segment.source_port);
        self.accept_peer(segment);
        self.state = State::SynReceived;
    }

    /// A segment answering this end's `SYN`.
    fn on_segment_syn_sent(&mut self, segment: &Segment<'_>, now: Instant) {
        let acknowledges = segment.flags.has(Flags::ACK);
        if acknowledges
            && (segment.ack.before_or_equal(self.iss) || segment.ack.after(self.snd_max))
        {
            if !segment.flags.has(Flags::RST) {
                self.reset_pending = Some((self.remote, segment.ack, None));
            }
            return;
        }
        if segment.flags.has(Flags::RST) {
            if acknowledges {
                self.tear_down(true);
            }
            return;
        }
        if !segment.flags.has(Flags::SYN) {
            return;
        }
        self.accept_peer(segment);
        if acknowledges {
            self.snd_una = segment.ack;
            self.snd_wl1 = segment.seq;
            self.snd_wl2 = segment.ack;
        }
        if self.snd_una.after(self.iss) {
            self.state = State::Established;
            self.retries = 0;
            self.measure(segment.ack, now);
            self.ack_now = true;
            self.arm_retransmit(now);
        } else {
            // Both ends opened at once. This end answers the peer's `SYN`
            // and waits for its own to be acknowledged.
            self.state = State::SynReceived;
            self.retransmit_at = None;
        }
    }

    /// Everything RFC 9293, section 3.10.7.4 does, in its order.
    fn on_segment_open(&mut self, segment: &Segment<'_>, now: Instant) {
        if self.state == State::SynReceived
            && segment.flags.has(Flags::SYN)
            && segment.seq == self.irs
        {
            self.on_repeated_syn(segment, now);
            return;
        }
        let window_before = self.recv.window();
        let window = u32::try_from(window_before).unwrap_or(u32::MAX);
        let length = segment.sequence_len();
        if !is_acceptable(segment.seq, length, self.rcv_nxt, window) {
            // An acknowledgment says where this end stands, which is what
            // a peer that has drifted needs; a reset would tear down a
            // connection on a segment that may be an old duplicate.
            if !segment.flags.has(Flags::RST) {
                self.ack_now = true;
            }
            return;
        }
        if segment.flags.has(Flags::RST) {
            // RFC 5961, section 3.2: a reset is believed only at the exact
            // number expected. One merely inside the window draws an
            // acknowledgment, which costs a blind attacker the sequence
            // number as well as the window.
            if segment.seq == self.rcv_nxt {
                self.tear_down(true);
            } else {
                self.ack_now = true;
            }
            return;
        }
        if segment.flags.has(Flags::SYN) {
            // RFC 5961, section 4.2: a `SYN` inside the window is answered
            // and never acted on. In `SYN-RECEIVED` that answer is the
            // `SYN` with `ACK` again, which is how a retransmitted `SYN`
            // is absorbed.
            self.ack_now = true;
            return;
        }
        if !segment.flags.has(Flags::ACK) {
            return;
        }
        if !self.check_ack(segment, now) {
            return;
        }
        self.update_window(segment);
        self.accept_text(segment, window_before, now);
        self.check_fin(segment, now);
    }

    /// A `SYN` that arrives again at a connection already in
    /// `SYN-RECEIVED`, at the number the peer began with.
    ///
    /// It is one of two things. When it carries an acknowledgment of this
    /// end's own `SYN`, the two ends opened at once and this is the
    /// answer that completes both — the acceptance test of section 3.4
    /// would call it an old duplicate, because every number it occupies
    /// has been received, so it is taken here instead. Otherwise the peer
    /// did not hear the answer, and the answer goes again.
    fn on_repeated_syn(&mut self, segment: &Segment<'_>, now: Instant) {
        let acknowledges = segment.flags.has(Flags::ACK)
            && self.snd_una.before(segment.ack)
            && segment.ack.before_or_equal(self.snd_max);
        if !acknowledges {
            self.resend_syn = true;
            return;
        }
        self.snd_una = segment.ack;
        self.state = State::Established;
        self.retries = 0;
        self.update_window(segment);
        self.measure(segment.ack, now);
        self.arm_retransmit(now);
        self.ack_now = true;
    }

    /// The acknowledgment field. Answers whether processing goes on.
    fn check_ack(&mut self, segment: &Segment<'_>, now: Instant) -> bool {
        let ack = segment.ack;
        if self.state == State::SynReceived {
            if !(self.snd_una.before(ack) && ack.before_or_equal(self.snd_max)) {
                self.reset_pending = Some((self.remote, ack, None));
                return false;
            }
            self.state = State::Established;
            self.retransmit_at = None;
        }
        if ack.after(self.snd_max) {
            // The peer acknowledges what was never sent. The comparison is
            // against the highest number ever sent and not against
            // `SND.NXT`: a retransmission timeout winds `SND.NXT` back to
            // the oldest unacknowledged number, and an acknowledgment of
            // everything that went out before it is then perfectly good
            // news rather than a segment to argue with.
            self.ack_now = true;
            return false;
        }
        if ack.after(self.snd_una) {
            self.on_new_ack(ack, now);
        } else if ack == self.snd_una
            && segment.payload.is_empty()
            && !segment.flags.has(Flags::FIN)
            && u32::from(segment.window) == self.snd_wnd
            && self.in_flight() > 0
        {
            self.on_duplicate_ack();
        }
        true
    }

    /// An acknowledgment of numbers that were outstanding.
    fn on_new_ack(&mut self, ack: SeqNumber, now: Instant) {
        let mut counted = ack.distance_from(self.snd_una);
        if self.snd_una == self.iss {
            // The first number a connection sends is its `SYN`, and it is
            // not a byte of the stream.
            counted = counted.saturating_sub(1);
        }
        if let Some(fin) = self.fin_seq
            && ack.after(fin)
        {
            counted = counted.saturating_sub(1);
        }
        let bytes = usize::try_from(counted).unwrap_or(usize::MAX);
        self.send.acknowledge(bytes);
        self.snd_una = ack;
        if self.snd_nxt.before(self.snd_una) {
            self.snd_nxt = self.snd_una;
        }
        // The segment at the front got through, so the next one begins
        // with its full allowance of attempts. The backoff of the timeout
        // itself is not thrown away here: only a measurement does that,
        // which is Karn's rule.
        self.retries = 0;
        self.measure(ack, now);
        self.congestion.on_ack(counted);
        self.persist_at = None;
        self.arm_retransmit(now);
        self.advance_after_ack(now);
    }

    /// Takes a round trip measurement when `ack` covers the number being
    /// timed. Karn's rule has already cleared the sample if the segment
    /// that carried that number was sent more than once.
    fn measure(&mut self, ack: SeqNumber, now: Instant) {
        if let Some((timed, sent_at)) = self.rtt_sample
            && ack.after(timed)
        {
            self.rto.sample(now.saturating_duration_since(sent_at));
            self.rtt_sample = None;
        }
    }

    /// The three duplicate acknowledgments of RFC 5681, section 3.2.
    const fn on_duplicate_ack(&mut self) {
        if self
            .congestion
            .on_duplicate_ack(self.in_flight(), self.snd_max())
        {
            // The oldest unacknowledged segment goes again without waiting
            // for the timer — that one segment and no more. The duplicates
            // are proof that everything behind it arrived, so sending it
            // all again would be sending what the peer already has.
            self.fast_retransmit = true;
            self.rtt_sample = None;
        }
    }

    /// What an acknowledgment of this end's `FIN` moves the state to.
    fn advance_after_ack(&mut self, now: Instant) {
        let acknowledged = self.fin_seq.is_some_and(|fin| self.snd_una.after(fin));
        match self.state {
            State::FinWait1 if acknowledged => self.state = State::FinWait2,
            State::Closing if acknowledged => self.enter_time_wait(now),
            State::LastAck if acknowledged => self.tear_down(false),
            _ => {}
        }
    }

    /// The window the peer advertises, under the rule of RFC 9293,
    /// section 3.10.7.4 that an older segment may not undo a newer one.
    fn update_window(&mut self, segment: &Segment<'_>) {
        let newer = self.snd_wl1.before(segment.seq)
            || (self.snd_wl1 == segment.seq && self.snd_wl2.before_or_equal(segment.ack));
        if !newer {
            return;
        }
        self.snd_wnd = u32::from(segment.window);
        self.snd_wl1 = segment.seq;
        self.snd_wl2 = segment.ack;
        if self.snd_wnd > 0 {
            self.persist_at = None;
        }
    }

    /// The bytes the segment carries.
    fn accept_text(&mut self, segment: &Segment<'_>, window_before: usize, now: Instant) {
        if segment.payload.is_empty() || !self.state.can_receive() {
            return;
        }
        let base = self.receive_base();
        let (offset, data) = if segment.seq.before(base) {
            // Part of it has been read already; that part is skipped and
            // the rest goes where it belongs.
            let skip = usize::try_from(base.distance_from(segment.seq)).unwrap_or(usize::MAX);
            (0, segment.payload.get(skip..).unwrap_or(&[]))
        } else {
            let offset = usize::try_from(segment.seq.distance_from(base)).unwrap_or(usize::MAX);
            (offset, segment.payload)
        };
        let in_order = segment.seq.before_or_equal(self.rcv_nxt);
        let had_gap = self.recv.has_early_bytes();
        let ready = self.recv.accept(offset, data);
        self.rcv_nxt = base.add(u32::try_from(ready).unwrap_or(u32::MAX));
        if !in_order || had_gap || window_before == 0 {
            // A segment that opens a gap, one that fills a gap
            // (RFC 5681, section 4.2), and one that probed a closed
            // window are all answered at once: in each the peer is
            // waiting to learn something it cannot guess.
            self.ack_now = true;
            return;
        }
        if data.len() < usize::from(self.config.max_segment) {
            // Only a stream of full segments is acknowledged every second
            // one (RFC 1122, section 4.2.3.2); a short segment waits for
            // the timer, which is what lets a request and its answer share
            // one segment.
            self.ack_at = Some(now.saturating_add(self.config.delayed_ack));
            return;
        }
        self.since_ack = self.since_ack.saturating_add(1);
        if self.since_ack >= 2 {
            self.ack_now = true;
        } else {
            self.ack_at = Some(now.saturating_add(self.config.delayed_ack));
        }
    }

    /// The peer's `FIN`, once everything before it has arrived.
    fn check_fin(&mut self, segment: &Segment<'_>, now: Instant) {
        if !segment.flags.has(Flags::FIN) {
            return;
        }
        let payload = u32::try_from(segment.payload.len()).unwrap_or(u32::MAX);
        if segment.seq.add(payload) != self.rcv_nxt {
            // Bytes before it are still missing, so the `FIN` is not yet
            // in order and the peer will send it again.
            return;
        }
        self.rcv_nxt = self.rcv_nxt.add(1);
        self.peer_finished = true;
        self.ack_now = true;
        match self.state {
            State::Established => self.state = State::CloseWait,
            State::FinWait1 => {
                if self.fin_seq.is_some_and(|fin| self.snd_una.after(fin)) {
                    self.enter_time_wait(now);
                } else {
                    self.state = State::Closing;
                }
            }
            State::FinWait2 => self.enter_time_wait(now),
            State::TimeWait => self.time_wait_until = Some(time_wait_end(now)),
            _ => {}
        }
    }
}

impl Connection<'_> {
    /// What one segment would best say now, or `None` when nothing is due.
    fn decide(&mut self, now: Instant) -> Option<Plan> {
        if let Some((to, seq, ack)) = self.reset_pending.take() {
            return Some(Plan::Reset { to, seq, ack });
        }
        if self.state == State::TimeWait && self.time_wait_until.is_some_and(|until| now >= until) {
            self.tear_down(false);
        }
        if !self.state.is_open() || self.state == State::Listen {
            return None;
        }
        if self.syn_outstanding() {
            return self.decide_syn(now);
        }
        if self.retransmit_at.is_some_and(|at| now >= at) {
            if self.give_up() {
                return None;
            }
            self.rto.back_off();
            self.congestion.on_timeout(self.in_flight());
            // Everything from the oldest unacknowledged number goes again,
            // and the timer starts over at the doubled timeout
            // (RFC 6298, section 5.5). It has to be restarted here and not
            // where the segment is written: a timer left standing in the
            // past fires again on the next poll, and every firing counts
            // an attempt against the connection.
            self.snd_nxt = self.snd_una;
            self.rtt_sample = None;
            self.retransmit_at = Some(now.saturating_add(self.rto.get()));
        }
        if let Some(plan) = self.decide_probe(now) {
            return Some(plan);
        }
        if let Some(plan) = self.decide_data() {
            return Some(plan);
        }
        if self.fin_due() {
            return Some(Plan::Fin);
        }
        if self.ack_due(now) {
            return Some(Plan::Ack);
        }
        // A timer that fired and found nothing to send would fire again at
        // once, and every firing counts an attempt against the connection.
        // Nothing outstanding means no timer.
        if self.retransmit_at.is_some_and(|at| now >= at) {
            self.arm_retransmit(now);
        }
        None
    }

    /// The `SYN` of a connection that is still opening.
    ///
    /// An acknowledgment that is merely owed is a bare acknowledgment and
    /// not the `SYN` again: the two are answers to different things, and
    /// sending the `SYN` for both would have two ends that opened at once
    /// trade opening segments for ever.
    fn decide_syn(&mut self, now: Instant) -> Option<Plan> {
        let acknowledges = self.state == State::SynReceived;
        if self.resend_syn || self.retransmit_at.is_none() {
            self.resend_syn = false;
            return Some(Plan::Syn { acknowledges });
        }
        if self.retransmit_at.is_some_and(|at| now >= at) {
            if self.give_up() {
                return None;
            }
            self.rto.back_off();
            self.rtt_sample = None;
            self.retransmit_at = Some(now.saturating_add(self.rto.get()));
            return Some(Plan::Syn { acknowledges });
        }
        if self.ack_due(now) {
            return Some(Plan::Ack);
        }
        None
    }

    /// One byte into a window the peer closed, to find out whether it has
    /// opened again. Without it a lost window update would stall the
    /// connection for good.
    fn decide_probe(&self, now: Instant) -> Option<Plan> {
        if self.snd_wnd != 0 {
            return None;
        }
        let offset = self.sent_offset();
        if self.send.len() <= offset {
            return None;
        }
        match self.persist_at {
            None => Some(Plan::Probe { offset }),
            Some(at) if now >= at => Some(Plan::Probe { offset }),
            Some(_) => None,
        }
    }

    /// Bytes that the two windows and the segment size allow to go now.
    fn decide_data(&mut self) -> Option<Plan> {
        let segment_size = usize::try_from(self.max_segment).unwrap_or(usize::MAX);
        if self.fast_retransmit {
            self.fast_retransmit = false;
            let len = self.send.contiguous(0, segment_size).len();
            if len > 0 {
                return Some(Plan::Data {
                    offset: 0,
                    len,
                    fin: false,
                    push: false,
                    again: true,
                });
            }
        }
        let offset = self.sent_offset();
        let held = self.send.len().saturating_sub(offset);
        if held == 0 {
            return None;
        }
        let allowed = self.snd_wnd.min(self.congestion.window());
        let usable = allowed.saturating_sub(self.in_flight());
        let limit = usize::try_from(usable)
            .unwrap_or(usize::MAX)
            .min(segment_size);
        if limit == 0 {
            return None;
        }
        let len = self.send.contiguous(offset, limit).len();
        if len == 0 {
            return None;
        }
        let last = offset.saturating_add(len) == self.send.len();
        // Nagle: a segment that is not full waits while something is
        // outstanding, so that a stream of single bytes becomes a stream
        // of segments. A close does not wait, because there is nothing
        // behind it to fill the segment with.
        if self.config.nagle
            && len < segment_size
            && self.in_flight() > 0
            && !(self.closing && last)
        {
            return None;
        }
        Some(Plan::Data {
            offset,
            len,
            fin: self.closing && last && self.fin_seq.is_none() && self.state.can_send(),
            push: last,
            again: self.snd_nxt.before(self.snd_max),
        })
    }

    /// Whether the closing `FIN` is to go now: because it has not gone at
    /// all and has nothing left to ride on, or because it went, was not
    /// acknowledged, and the numbers have been rewound to it.
    ///
    /// The second case is what makes a lost `FIN` recoverable. It carries
    /// no bytes, so the send buffer does not hold it and nothing else
    /// would send it again.
    fn fin_due(&self) -> bool {
        if !self.closing || self.sent_offset() < self.send.len() {
            return false;
        }
        match self.fin_seq {
            None => self.state.can_send(),
            Some(fin) => self.snd_una.before_or_equal(fin) && self.snd_nxt.before_or_equal(fin),
        }
    }

    /// Whether an acknowledgment is owed now.
    fn ack_due(&self, now: Instant) -> bool {
        self.ack_now || self.ack_at.is_some_and(|at| now >= at)
    }

    /// Writes the segment the plan asks for and records that it went.
    fn transmit<'b>(&mut self, plan: Plan, now: Instant, buffer: &'b mut [u8]) -> Option<&'b [u8]> {
        let window = self.advertise();
        let source = self.local.address;
        let to = match plan {
            Plan::Reset { to, .. } => to,
            _ => self.remote,
        };
        let destination = to.address;
        let mut segment = Segment::new(self.local.port, to.port, Flags::ACK);
        segment.window = window;
        segment.ack = self.rcv_nxt;
        segment.seq = self.snd_nxt;
        let mut first = self.snd_nxt;
        let mut length = 0u32;
        let mut fin_at = None;
        let mut again = false;
        match plan {
            Plan::Reset { seq, ack, .. } => {
                segment.seq = seq;
                segment.window = 0;
                segment.flags = match ack {
                    Some(number) => {
                        segment.ack = number;
                        Flags::RST.with(Flags::ACK)
                    }
                    None => Flags::RST,
                };
            }
            Plan::Syn { acknowledges } => {
                segment.seq = self.iss;
                first = self.iss;
                segment.flags = if acknowledges {
                    Flags::SYN.with(Flags::ACK)
                } else {
                    Flags::SYN
                };
                segment.max_segment_size = Some(self.config.max_segment);
                length = 1;
                again = self.snd_max.after(self.iss);
            }
            Plan::Data {
                offset,
                len,
                fin,
                push,
                again: retransmission,
            } => {
                // The bytes at `offset` are the ones at that distance from
                // the oldest unacknowledged number, which is where they
                // are in the buffer as well. A retransmission therefore
                // names the number it named the first time.
                segment.seq = self.snd_una.add(u32::try_from(offset).unwrap_or(0));
                first = segment.seq;
                if push {
                    segment.flags = segment.flags.with(Flags::PSH);
                }
                if fin {
                    segment.flags = segment.flags.with(Flags::FIN);
                    fin_at = Some(segment.seq.add(u32::try_from(len).unwrap_or(0)));
                }
                segment.payload = self.send.contiguous(offset, len);
                length = u32::try_from(segment.payload.len())
                    .unwrap_or(0)
                    .saturating_add(u32::from(fin));
                again = retransmission;
            }
            Plan::Fin => {
                segment.flags = segment.flags.with(Flags::FIN);
                fin_at = Some(self.snd_nxt);
                length = 1;
                again = self.snd_max.after(self.snd_nxt);
            }
            Plan::Probe { offset } => {
                segment.seq = self.snd_una.add(u32::try_from(offset).unwrap_or(0));
                segment.payload = self.send.contiguous(offset, 1);
                length = u32::try_from(segment.payload.len()).unwrap_or(0);
            }
            Plan::Ack => {
                // A segment that carries nothing carries the highest
                // number ever sent and not `SND.NXT`. After a
                // retransmission timeout the two differ, and a bare
                // acknowledgment at the older number is one the peer's
                // acceptance test throws away as an old duplicate — which
                // leaves both ends answering each other for ever.
                segment.seq = self.snd_max;
                first = self.snd_max;
            }
        }
        let mut writer = Writer::new(buffer);
        segment.write(&mut writer, source, destination).ok()?;
        let written = writer.finish();
        self.sent(plan, now, first, length, fin_at, again);
        Some(written)
    }

    /// Everything that follows from a segment having gone out.
    fn sent(
        &mut self,
        plan: Plan,
        now: Instant,
        first: SeqNumber,
        length: u32,
        fin_at: Option<SeqNumber>,
        again: bool,
    ) {
        if matches!(plan, Plan::Reset { .. }) {
            return;
        }
        // Every segment but a reset carries an acknowledgment, so whatever
        // was owed has now been said.
        self.ack_now = false;
        self.ack_at = None;
        self.since_ack = 0;
        if matches!(plan, Plan::Probe { .. }) {
            // The byte is one the peer said it has no room for, so it does
            // not move `SND.NXT`: the next probe carries the same byte
            // again, and the persist timer and not the retransmission
            // timer is what schedules it. It does move `SND.MAX`, because
            // the byte did go out — a peer that takes it after all and
            // acknowledges it must not be told it acknowledged something
            // that was never sent.
            let end = first.add(length);
            if end.after(self.snd_max) {
                self.snd_max = end;
            }
            self.rto.back_off();
            self.persist_at = Some(now.saturating_add(self.rto.get()));
            return;
        }
        let end = first.add(length);
        if end.after(self.snd_max) {
            self.snd_max = end;
        }
        if self.snd_nxt.before(end) {
            self.snd_nxt = end;
        }
        if let Some(number) = fin_at {
            self.fin_seq = Some(number);
            self.state = match self.state {
                State::Established => State::FinWait1,
                State::CloseWait => State::LastAck,
                other => other,
            };
        }
        if length > 0 {
            if again {
                // Karn's rule: a number that was sent twice tells nothing
                // about the round trip.
                self.rtt_sample = None;
            } else if self.rtt_sample.is_none() {
                self.rtt_sample = Some((end.sub(1), now));
            }
            if self.retransmit_at.is_none() {
                // RFC 6298, section 5.1: a segment that occupies numbers
                // starts the timer when it is not already running. An
                // expiry restarts it where the expiry is handled.
                self.retransmit_at = Some(now.saturating_add(self.rto.get()));
            }
        }
    }

    /// What this end announces it has room for, never moving the far edge
    /// of the window back (RFC 9293, section 3.8.6.2.2).
    fn advertise(&mut self) -> u16 {
        if self.rcv_edge.before(self.rcv_nxt) {
            self.rcv_edge = self.rcv_nxt;
        }
        let room = u32::try_from(self.recv.window())
            .unwrap_or(u32::MAX)
            .min(u32::from(u16::MAX));
        let edge = self.rcv_nxt.add(room);
        if edge.after(self.rcv_edge) || !self.state.is_synchronized() {
            self.rcv_edge = edge;
        }
        u16::try_from(self.rcv_edge.distance_from(self.rcv_nxt)).unwrap_or(u16::MAX)
    }

    /// Takes what the peer's `SYN` said: where its numbers start, how much
    /// room it has, and how large a segment it will accept.
    fn accept_peer(&mut self, segment: &Segment<'_>) {
        self.irs = segment.seq;
        self.rcv_nxt = segment.seq.add(1);
        self.rcv_edge = self.rcv_nxt;
        self.snd_wnd = u32::from(segment.window);
        self.snd_wl1 = segment.seq;
        let announced = segment
            .max_segment_size
            .unwrap_or(DEFAULT_MAX_SEGMENT)
            .max(MIN_MAX_SEGMENT)
            .min(self.config.max_segment);
        self.max_segment = u32::from(announced);
        self.congestion.set_max_segment(self.max_segment);
    }

    /// Draws the number this end's stream starts at (D-51, RFC 6528).
    fn set_initial_sequence<R: Rng + ?Sized>(&mut self, rng: &mut R) -> Result<(), TcpError> {
        let mut seed = [0u8; 4];
        rng.fill(&mut seed)?;
        let iss = SeqNumber::new(u32::from_be_bytes(seed));
        self.iss = iss;
        self.snd_una = iss;
        self.snd_nxt = iss;
        self.snd_max = iss;
        Ok(())
    }

    /// Closes the connection, whether the peer asked for it or a timer
    /// did.
    const fn tear_down(&mut self, by_peer: bool) {
        self.state = State::Closed;
        self.was_reset = self.was_reset || by_peer;
        self.closing = false;
        self.retransmit_at = None;
        self.ack_at = None;
        self.persist_at = None;
        self.time_wait_until = None;
        self.ack_now = false;
    }

    /// Enters `TIME-WAIT` and starts its timer.
    const fn enter_time_wait(&mut self, now: Instant) {
        self.state = State::TimeWait;
        self.retransmit_at = None;
        self.persist_at = None;
        self.time_wait_until = Some(time_wait_end(now));
    }

    /// Whether the segment at the front has used up its allowance of
    /// attempts. When it has not, this counts the attempt that is about to
    /// be made.
    const fn give_up(&mut self) -> bool {
        if self.retries < self.config.retransmit_limit {
            self.retries = self.retries.saturating_add(1);
            return false;
        }
        self.tear_down(false);
        self.timed_out = true;
        true
    }

    /// Restarts or clears the retransmission timer, depending on whether
    /// anything is still outstanding.
    fn arm_retransmit(&mut self, now: Instant) {
        self.retransmit_at = if self.in_flight() == 0 {
            None
        } else {
            Some(now.saturating_add(self.rto.get()))
        };
    }

    /// Whether this end's `SYN` is still unacknowledged.
    fn syn_outstanding(&self) -> bool {
        matches!(self.state, State::SynSent | State::SynReceived) && self.snd_una == self.iss
    }

    /// How many numbers are outstanding.
    const fn in_flight(&self) -> u32 {
        self.snd_nxt.distance_from(self.snd_una)
    }

    /// How far into the send buffer the numbers already sent reach.
    fn sent_offset(&self) -> usize {
        let mut sent = self.in_flight();
        if self.snd_una == self.iss && self.state.is_open() {
            // The `SYN` takes a number and no byte of the buffer.
            sent = sent.saturating_sub(1);
        }
        usize::try_from(sent).unwrap_or(usize::MAX)
    }

    /// The number of the oldest byte the caller has not read, which is
    /// where the receive buffer's offsets are counted from.
    fn receive_base(&self) -> SeqNumber {
        self.rcv_nxt
            .sub(u32::try_from(self.recv.ready()).unwrap_or(0))
    }

    /// The highest number ever sent.
    const fn snd_max(&self) -> SeqNumber {
        self.snd_max
    }

    /// Whether there is something to send at this instant that no timer
    /// stands in the way of.
    fn can_send_now(&self) -> bool {
        if self.snd_wnd == 0 && self.send.len() > self.sent_offset() && self.persist_at.is_none() {
            return true;
        }
        if self.fin_due() {
            return true;
        }
        let held = self.send.len().saturating_sub(self.sent_offset());
        held > 0 && self.snd_wnd.min(self.congestion.window()) > self.in_flight()
    }
}

/// When a `TIME-WAIT` that began at `now` ends: twice the maximum segment
/// lifetime, so that no segment of this connection can still be in the
/// network when the next one between the same two ports begins.
const fn time_wait_end(now: Instant) -> Instant {
    now.saturating_add(MAX_SEGMENT_LIFETIME.saturating_add(MAX_SEGMENT_LIFETIME))
}
