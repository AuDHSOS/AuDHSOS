// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The stub resolver: a state machine that asks a name in both address
//! families and moves no bytes of its own.
//!
//! What it produces is a complete UDP datagram in the caller's buffer and
//! the two addresses it was written for, which is what D-84 has a
//! transport do, one layer up. What it consumes is the payload of a
//! datagram together with the address and port it arrived from — the three
//! things a receive record of `net-udp` carries.
//!
//! A response is believed only when four things agree: the source address
//! is one of the servers this resolver was given, the source port is 53,
//! the transaction id is the one that went out, and the question section
//! is the question that was asked. The id is drawn from `Rng` (D-51) and
//! the source port is the port of the socket the caller bound, which
//! `net-udp` drew from the dynamic range. Three of the four are what an
//! attacker who cannot see the query has to guess; the fourth is what
//! keeps one answer from being taken for another. A fifth thing is
//! believed only up to 512 bytes: this resolver announces no buffer of its
//! own, so RFC 1035, section 4.2.1 is the whole of what a server may send
//! it.
//!
//! The two questions are in the air at once. Each carries its own id, its
//! own attempt counter and its own place in the server rotation, and one
//! deadline governs both. A family that never answers therefore costs its
//! own attempts and nothing else, where asking one after the other would
//! spend the whole deadline on the first and never reach the second.
//!
//! A `CNAME` chain is followed inside the answer it arrived in. Where it
//! ends at a name that answer carries no address for, the resolver asks
//! that name: one budget of eight links governs the chain however many
//! messages it is spread over, and a record the chain has already stepped
//! through ends it as a loop.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use crypto_rng::Rng;
use net_udp::{ChecksumPolicy, Datagram};
use net_wire::{IpAddr, Port, Writer};

use crate::error::DnsError;
use crate::message::{MAX_MESSAGE_LEN, MAX_QUERY_LEN, Message, ResponseCode, write_query};
use crate::name::Name;
use crate::record::{RecordData, RecordType};

/// Where a name server listens (RFC 1035, section 4.2).
pub const SERVER_PORT: Port = Port::new(53);

/// How many servers a resolver rotates through.
pub const MAX_SERVERS: usize = 4;

/// How many addresses one resolution collects, over both families
/// together.
pub const MAX_ADDRESSES: usize = 8;

/// How many `CNAME` links one resolution follows, over however many
/// messages the chain is spread.
pub const MAX_CNAME_LINKS: usize = 8;

/// How far into an answer section the chain is walked.
///
/// A response to a question about an address carries a handful of records;
/// thirty-two is past anything a server sends and is what keeps the walk,
/// which is one pass over the section per link, from being quadratic in a
/// number an attacker writes.
const MAX_ANSWERS_WALKED: usize = 32;

/// The two questions, which are asked together.
const QUESTIONS: [RecordType; 2] = [RecordType::A, RecordType::AAAA];

/// What a resolver is configured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// How long one question waits for an answer before it is asked
    /// again, at the next server.
    pub retry: Duration,
    /// How often one question is asked before it is given up.
    pub attempts: u32,
    /// How long the whole resolution may take, `CNAME` chains included.
    pub deadline: Duration,
}

impl Config {
    /// Three attempts two seconds apart, and ten seconds for the whole.
    ///
    /// The deadline is not the sum of the attempts: a chain of aliases
    /// starts a question's attempts over, and this is the bound on the
    /// whole that keeps that from running for ever.
    pub const DEFAULT: Config = Config {
        retry: Duration::from_secs(2),
        attempts: 3,
        deadline: Duration::from_secs(10),
    };
}

impl Default for Config {
    fn default() -> Config {
        Config::DEFAULT
    }
}

/// Where a resolution stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing has been asked.
    Idle,
    /// At least one question is still open.
    Asking,
    /// Both questions are settled. What came back is in
    /// [`addresses`](Resolver::addresses), which may be empty: a name that
    /// exists and has no address is an answer.
    Done,
    /// Neither question could be settled.
    Failed(DnsError),
}

/// One datagram to send, and the two addresses it was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query<'a> {
    /// Where it goes out from.
    pub source: IpAddr,
    /// Which server it goes to.
    pub destination: IpAddr,
    /// The datagram itself, header and all.
    pub datagram: &'a [u8],
}

/// Where one of the two questions stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AskState {
    /// A query is scheduled or in flight.
    Asking,
    /// The server answered, whether with an address or with the news that
    /// there is none.
    Settled,
    /// It could not be answered.
    Failed(DnsError),
}

/// One of the two questions.
#[derive(Clone, Copy, Debug)]
struct Ask {
    /// Which kind of record it asks for.
    record_type: RecordType,
    /// Which name it currently asks about, which a `CNAME` moves.
    name: Name,
    /// The transaction id, drawn once per name and kept across retries: a
    /// fresh id per retry would make every answer that is merely late
    /// unusable, and the window it leaves an attacker is the same either
    /// way.
    id: u16,
    /// Whether an id has been drawn for the name it now asks about.
    has_id: bool,
    /// Which server the next attempt goes to.
    server: usize,
    /// How many attempts are left for this name.
    attempts: u32,
    /// When the next attempt is due.
    due: Instant,
    /// How many `CNAME` links the chain has taken so far.
    links: usize,
    /// Where it stands.
    state: AskState,
}

impl Ask {
    /// A question of `record_type` that has not been asked.
    const fn new(record_type: RecordType) -> Ask {
        Ask {
            record_type,
            name: Name::ROOT,
            id: 0,
            has_id: false,
            server: 0,
            attempts: 0,
            due: Instant::from_micros(0),
            links: 0,
            state: AskState::Settled,
        }
    }

    /// Points the question at `name` with a full budget of attempts and a
    /// transaction id still to draw.
    const fn ask(&mut self, name: &Name, attempts: u32, now: Instant) {
        self.name = *name;
        self.has_id = false;
        self.attempts = attempts;
        self.due = now;
        self.state = AskState::Asking;
    }
}

/// The resolver.
#[derive(Debug)]
pub struct Resolver {
    /// Which of this host's addresses the queries go out from.
    local: IpAddr,
    /// Which port they go out from, which is the port of the socket the
    /// caller bound.
    port: Port,
    /// What it was configured with.
    config: Config,
    /// The servers, in the order they are asked.
    servers: ArrayVec<IpAddr, MAX_SERVERS>,
    /// The two questions.
    asks: [Ask; 2],
    /// What has come back.
    addresses: ArrayVec<IpAddr, MAX_ADDRESSES>,
    /// When the whole resolution runs out.
    deadline: Instant,
    /// Whether a resolution is running or finished; `false` before the
    /// first one.
    started: bool,
}

impl Resolver {
    /// A resolver that sends from `local` on `port` and has no servers
    /// yet.
    ///
    /// `local` is one of this host's addresses and `port` is the port of a
    /// socket the caller bound through `net-udp`, which is where the
    /// randomness of the source port comes from.
    #[must_use]
    pub fn new(local: IpAddr, port: Port, config: Config) -> Resolver {
        Resolver {
            local,
            port,
            config,
            servers: ArrayVec::new(),
            asks: QUESTIONS.map(Ask::new),
            addresses: ArrayVec::new(),
            deadline: Instant::from_micros(0),
            started: false,
        }
    }

    /// Adds a server.
    ///
    /// Which of a host's addresses a packet leaves with is the question of
    /// RFC 6724 and belongs to the facade above this crate, so a resolver
    /// carries one address and asks servers of that address's family. A
    /// host that wants to ask over the other family builds a second
    /// resolver; the records that come back are of both families either
    /// way.
    ///
    /// # Errors
    ///
    /// [`DnsError::MixedFamilies`] for a server of the other family, and
    /// [`DnsError::TooManyServers`] when the table is full.
    pub fn add_server(&mut self, address: IpAddr) -> Result<(), DnsError> {
        if address.version() != self.local.version() {
            return Err(DnsError::MixedFamilies);
        }
        self.servers
            .push(address)
            .map_err(|_| DnsError::TooManyServers)
    }

    /// The servers, in the order they are asked.
    pub fn servers(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.servers.iter().copied()
    }

    /// Where the resolution stands.
    #[must_use]
    pub fn status(&self) -> Status {
        if !self.started {
            return Status::Idle;
        }
        if self.asks.iter().any(|ask| ask.state == AskState::Asking) {
            return Status::Asking;
        }
        let failure = self.asks.iter().find_map(|ask| match ask.state {
            AskState::Failed(error) => Some(error),
            _ => None,
        });
        match failure {
            Some(error) if !self.asks.iter().any(|ask| ask.state == AskState::Settled) => {
                Status::Failed(error)
            }
            _ => Status::Done,
        }
    }

    /// The addresses that came back, in the order they arrived.
    pub fn addresses(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.addresses.iter().copied()
    }

    /// Starts a resolution of `name`.
    ///
    /// # Errors
    ///
    /// [`DnsError::NoServer`] when no server was added, and
    /// [`DnsError::Busy`] when a resolution is still running.
    pub fn start(&mut self, name: &Name, now: Instant) -> Result<(), DnsError> {
        if self.servers.is_empty() {
            return Err(DnsError::NoServer);
        }
        if self.status() == Status::Asking {
            return Err(DnsError::Busy);
        }
        self.addresses.clear();
        self.deadline = now.saturating_add(self.config.deadline);
        self.started = true;
        let count = self.servers.len();
        let attempts = self.config.attempts;
        for (index, ask) in self.asks.iter_mut().enumerate() {
            ask.links = 0;
            // The two questions start at different servers, so that one
            // server that is down costs one of them and not both.
            ask.server = if index < count { index } else { 0 };
            ask.ask(name, attempts, now);
        }
        Ok(())
    }

    /// Forgets the resolution, so that the next one starts clean.
    pub fn reset(&mut self) {
        self.started = false;
        self.addresses.clear();
        for ask in &mut self.asks {
            ask.state = AskState::Settled;
        }
    }

    /// When the resolver next has something to say, or `None` when it has
    /// nothing outstanding.
    ///
    /// An instant that has passed means there is work now, and the
    /// deadline is one of the instants this can name: reaching it is what
    /// gives up on the questions that are still open.
    #[must_use]
    pub fn poll_at(&self) -> Option<Instant> {
        if self.status() != Status::Asking {
            return None;
        }
        let next = self
            .asks
            .iter()
            .filter(|ask| ask.state == AskState::Asking)
            .map(|ask| ask.due)
            .min()?;
        Some(next.min(self.deadline))
    }

    /// Writes at most one query into `buffer` and answers where it goes.
    ///
    /// A caller loops until this answers `None`, which is when nothing
    /// further is due at this instant.
    ///
    /// # Errors
    ///
    /// [`DnsError::Rng`] when the generator fails, and [`DnsError::Wire`]
    /// or [`DnsError::Udp`] when the buffer has no room for a datagram.
    pub fn poll<'b, R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rng: &mut R,
        buffer: &'b mut [u8],
    ) -> Result<Option<Query<'b>>, DnsError> {
        if self.status() != Status::Asking {
            return Ok(None);
        }
        if now >= self.deadline {
            self.give_up(DnsError::Deadline);
            return Ok(None);
        }
        let count = self.servers.len();
        // A question whose attempts have run out is retired here rather
        // than at the instant it ran out, because that instant is one
        // where nothing was polled.
        let (index, name, record_type, id, at) = loop {
            let Some(index) = self.next_due(now) else {
                return Ok(None);
            };
            let Some(ask) = self.asks.get_mut(index) else {
                return Ok(None);
            };
            if ask.attempts == 0 {
                ask.state = AskState::Failed(DnsError::Exhausted);
                continue;
            }
            if !ask.has_id {
                let mut seed = [0u8; 2];
                rng.fill(&mut seed)?;
                ask.id = u16::from_be_bytes(seed);
                ask.has_id = true;
            }
            break (index, ask.name, ask.record_type, ask.id, ask.server);
        };
        let Some(server) = self.servers.get(at).copied() else {
            return Ok(None);
        };
        let local = self.local;
        let mut scratch = [0u8; MAX_QUERY_LEN];
        let mut inner = Writer::new(&mut scratch);
        write_query(&mut inner, id, &name, record_type)?;
        let len = inner.position();
        let payload = scratch.get(..len).unwrap_or(&[]);
        let mut writer = Writer::new(buffer);
        Datagram {
            source_port: self.port,
            destination_port: SERVER_PORT,
            payload,
        }
        .write(&mut writer, local, server, ChecksumPolicy::Computed)?;
        // The attempt is counted only now, with the datagram in the
        // caller's buffer: a buffer that had no room for one is a buffer
        // nothing went out of.
        let retry = self.config.retry;
        if let Some(ask) = self.asks.get_mut(index) {
            ask.attempts = ask.attempts.saturating_sub(1);
            ask.due = now.saturating_add(retry);
            let next = at.saturating_add(1);
            ask.server = if next < count { next } else { 0 };
        }
        Ok(Some(Query {
            source: local,
            destination: server,
            datagram: writer.finish(),
        }))
    }

    /// Takes in the payload of a datagram that arrived from `source` at
    /// `port`.
    ///
    /// Everything that does not match is ignored and nothing is said about
    /// it: a response that fails one of the four checks is a response to
    /// somebody else's question, or to nobody's.
    pub fn on_datagram(&mut self, source: IpAddr, port: Port, payload: &[u8], now: Instant) {
        if self.status() != Status::Asking || port != SERVER_PORT {
            return;
        }
        // This resolver announces no larger buffer, so RFC 1035,
        // section 4.2.1 caps what a server may send it at 512 bytes. More
        // than that is not an answer to a question this resolver asked.
        if payload.len() > MAX_MESSAGE_LEN {
            return;
        }
        if !self.servers.iter().any(|server| *server == source) {
            return;
        }
        let Ok(message) = Message::parse(payload) else {
            return;
        };
        if !message.header.flags.is_response() {
            return;
        }
        let Ok(Some(question)) = message.question() else {
            return;
        };
        let Some(index) = self.asks.iter().position(|ask| {
            ask.state == AskState::Asking
                && ask.has_id
                && ask.id == message.header.id
                && ask.record_type == question.record_type
                && ask.name == question.name
        }) else {
            return;
        };
        // A section that cannot be walked is not an answer to anything.
        if message.answers().any(|record| record.is_err()) {
            return;
        }
        self.accept(index, &message, now);
    }

    /// Applies a response that has passed all four checks.
    fn accept(&mut self, index: usize, message: &Message<'_>, now: Instant) {
        let Some(ask) = self.asks.get_mut(index) else {
            return;
        };
        if message.header.flags.is_truncated() {
            ask.state = AskState::Failed(DnsError::Truncated);
            return;
        }
        match message.header.flags.response_code() {
            ResponseCode::NO_ERROR => {}
            ResponseCode::NAME_ERROR => {
                // The name does not exist, which is an answer.
                ask.state = AskState::Settled;
                return;
            }
            other => {
                // The server could not answer. The next attempt goes to
                // the next server, at once rather than after the retry.
                ask.due = now;
                if ask.attempts == 0 {
                    ask.state = AskState::Failed(DnsError::Rcode(other));
                }
                return;
            }
        }
        self.walk(index, message, now);
    }

    /// Follows the chain through the answer section and settles the
    /// question, moves it to a new name, or fails it.
    fn walk(&mut self, index: usize, message: &Message<'_>, now: Instant) {
        let Some(ask) = self.asks.get(index).copied() else {
            return;
        };
        let attempts = self.config.attempts;
        let mut current = ask.name;
        let mut links = ask.links;
        let mut used: ArrayVec<usize, MAX_CNAME_LINKS> = ArrayVec::new();
        loop {
            let mut found = false;
            let mut alias: Option<(usize, Name)> = None;
            for (position, record) in message
                .answers()
                .flatten()
                .take(MAX_ANSWERS_WALKED)
                .enumerate()
            {
                if record.name != current {
                    continue;
                }
                match record.data {
                    RecordData::A(address) if ask.record_type == RecordType::A => {
                        self.remember(IpAddr::V4(address));
                        found = true;
                    }
                    RecordData::Aaaa(address) if ask.record_type == RecordType::AAAA => {
                        self.remember(IpAddr::V6(address));
                        found = true;
                    }
                    RecordData::Cname(target) if alias.is_none() => {
                        alias = Some((position, target));
                    }
                    _ => {}
                }
            }
            if found {
                self.settle(index, AskState::Settled, links);
                return;
            }
            let Some((mark, target)) = alias else {
                // The chain ends here. Where it has moved, the answer this
                // server sent does not carry the address and the new name
                // is asked for on its own; where it has not, the name
                // exists and has no record of this type.
                if current == ask.name {
                    self.settle(index, AskState::Settled, links);
                } else if let Some(ask) = self.asks.get_mut(index) {
                    ask.links = links;
                    ask.ask(&current, attempts, now);
                }
                return;
            };
            if used.iter().any(|seen| *seen == mark) {
                self.settle(index, AskState::Failed(DnsError::CnameLoop), links);
                return;
            }
            let _ = used.push(mark);
            links = links.saturating_add(1);
            if links > MAX_CNAME_LINKS {
                self.settle(index, AskState::Failed(DnsError::CnameChain), links);
                return;
            }
            current = target;
        }
    }

    /// Puts one question to rest.
    fn settle(&mut self, index: usize, state: AskState, links: usize) {
        if let Some(ask) = self.asks.get_mut(index) {
            ask.links = links;
            ask.state = state;
        }
    }

    /// Keeps an address, unless it is one that already came back or there
    /// is no room for it.
    fn remember(&mut self, address: IpAddr) {
        if self.addresses.iter().any(|held| *held == address) {
            return;
        }
        let _ = self.addresses.push(address);
    }

    /// Which question is due at `now`, the earliest first.
    fn next_due(&self, now: Instant) -> Option<usize> {
        self.asks
            .iter()
            .enumerate()
            .filter(|(_, ask)| ask.state == AskState::Asking && ask.due <= now)
            .min_by_key(|(_, ask)| ask.due)
            .map(|(index, _)| index)
    }

    /// Fails every question that is still open.
    fn give_up(&mut self, error: DnsError) {
        for ask in &mut self.asks {
            if ask.state == AskState::Asking {
                ask.state = AskState::Failed(error);
            }
        }
    }
}
