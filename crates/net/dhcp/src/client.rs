// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client state machine of RFC 2131, figure 5, and the lease behind
//! it.
//!
//! Six states are here and two are not. Init, selecting, requesting,
//! bound, renewing and rebinding are the path from a host with no address
//! to a host with one and back again when the lease runs out. Init-reboot
//! and rebooting are not: they are the shortcut a client takes when it
//! remembers an address across a restart, and nothing in this system
//! remembers anything across one yet.
//!
//! Nothing is sent from here. A poll writes a complete UDP datagram into
//! the caller's buffer and names the two addresses it was written for
//! (D-84). Both ports are fixed by RFC 2131 — 68 here, 67 there — so the
//! only randomness this crate draws is the transaction id and the jitter
//! on the backoff.
//!
//! The BROADCAST flag is set while the client has no address and clear
//! once it has one. RFC 2131, section 4.1 is explicit about the deadlock
//! it exists for: a host that cannot take an IP datagram addressed to an
//! address it has not configured cannot be told the address it is being
//! given. From the bound state on the address stands, so a unicast reply
//! arrives and the flag comes off.
//!
//! The backoff before the lease is the one of section 4.1: four seconds,
//! doubled to a ceiling of sixty-four, each delay moved by a uniform value
//! between minus one and plus one second. After it, the rule of
//! section 4.4.5 takes over: half of what is left until the next deadline,
//! never below sixty seconds.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use crypto_rng::Rng;
use net_udp::{ChecksumPolicy, Datagram};
use net_wire::{IpAddr, Ipv4Addr, Ipv4Cidr, MacAddr, Port, Writer};

use crate::error::DhcpError;
use crate::message::{MIN_MESSAGE_LEN, Message, MessageType, Op};
use crate::option::{OptionCode, write_option};

/// Where a client listens (RFC 2131, section 4.1).
pub const CLIENT_PORT: Port = Port::new(68);

/// Where a server listens.
pub const SERVER_PORT: Port = Port::new(67);

/// How many name servers a lease carries.
pub const MAX_DNS_SERVERS: usize = 4;

/// How much room the options of one message take.
const OPTIONS_LEN: usize = 64;

/// The options this client asks to be told.
const PARAMETERS: [u8; 5] = [
    OptionCode::SUBNET_MASK.get(),
    OptionCode::ROUTER.get(),
    OptionCode::DOMAIN_NAME_SERVER.get(),
    OptionCode::RENEWAL_TIME.get(),
    OptionCode::REBINDING_TIME.get(),
];

/// The largest message this client can take, as the option carries it.
const MAX_MESSAGE_SIZE: u16 = 576;

/// Where the client stands (RFC 2131, figure 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// Nothing has been asked for.
    Init,
    /// A discover is out and an offer is awaited.
    Selecting,
    /// An offer was taken and a request is out.
    Requesting,
    /// The lease stands.
    Bound,
    /// T1 has passed; the server that granted the lease is being asked to
    /// extend it, by unicast.
    Renewing,
    /// T2 has passed; any server is being asked, by broadcast.
    Rebinding,
}

/// What a client is configured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The delay before the first retransmission.
    pub first_retry: Duration,
    /// The ceiling the doubling stops at.
    pub max_retry: Duration,
    /// How far a delay may be moved either way.
    pub jitter: Duration,
    /// The floor under the renewing and rebinding delays.
    pub min_renew_retry: Duration,
    /// How many requests are sent before the client starts over with a
    /// discover.
    pub request_attempts: u32,
}

impl Config {
    /// What RFC 2131 asks for: four seconds doubled to sixty-four, moved
    /// by a second either way, sixty seconds under a renewal, and four
    /// requests before starting over — the "total delay of 60 seconds" of
    /// section 3.1, note 5.
    pub const DEFAULT: Config = Config {
        first_retry: Duration::from_secs(4),
        max_retry: Duration::from_secs(64),
        jitter: Duration::from_secs(1),
        min_renew_retry: Duration::from_secs(60),
        request_attempts: 4,
    };
}

impl Default for Config {
    fn default() -> Config {
        Config::DEFAULT
    }
}

/// What a lease says.
#[derive(Debug)]
pub struct Lease {
    /// The address and the network it is on.
    pub network: Ipv4Cidr,
    /// The first router of the link, where the options named one.
    pub router: Option<Ipv4Addr>,
    /// The recursive name servers.
    pub servers: ArrayVec<Ipv4Addr, MAX_DNS_SERVERS>,
    /// Which server granted it.
    pub server: Ipv4Addr,
    /// When it was granted.
    pub granted: Instant,
    /// T1, when renewal begins.
    pub renew: Instant,
    /// T2, when rebinding begins.
    pub rebind: Instant,
    /// When it runs out.
    pub expires: Instant,
}

impl Lease {
    /// The lease an acknowledgment grants, as of `now`.
    ///
    /// The three options a lease cannot be made without are the subnet
    /// mask, the lease time, and the server identifier. RFC 2131,
    /// section 4.3.1 requires the last two of a server; the mask it only
    /// asks for, and this client requires it, because an address without
    /// one is an address with no network and therefore no route.
    ///
    /// # Errors
    ///
    /// [`DhcpError::MissingOption`] for each of those three, whatever the
    /// option walk found wrong on the way, [`DhcpError::Netmask`] for a
    /// mask whose bits do not run together, and [`DhcpError::Address`]
    /// for an address no host can be given.
    pub fn from_reply(message: &Message<'_>, now: Instant) -> Result<Lease, DhcpError> {
        check_address(message.yours)?;
        let mask = address_option(message, OptionCode::SUBNET_MASK)
            .ok_or(DhcpError::MissingOption(OptionCode::SUBNET_MASK))?;
        let server = address_option(message, OptionCode::SERVER_IDENTIFIER)
            .ok_or(DhcpError::MissingOption(OptionCode::SERVER_IDENTIFIER))?;
        let seconds = u32_option(message, OptionCode::LEASE_TIME)
            .ok_or(DhcpError::MissingOption(OptionCode::LEASE_TIME))?;
        let network = network_of(message.yours, mask)?;
        let whole = Duration::from_secs(u64::from(seconds));
        let (first, second) = timers(
            whole,
            u32_option(message, OptionCode::RENEWAL_TIME),
            u32_option(message, OptionCode::REBINDING_TIME),
        );
        Ok(Lease {
            network,
            router: first_address(message, OptionCode::ROUTER),
            servers: servers_of(message),
            server,
            granted: now,
            renew: now.saturating_add(first),
            rebind: now.saturating_add(second),
            expires: now.saturating_add(whole),
        })
    }

    /// The address it grants.
    #[must_use]
    pub const fn address(&self) -> Ipv4Addr {
        self.network.address()
    }
}

/// One datagram to send, and the two addresses it was written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outgoing<'a> {
    /// Where it goes out from, which is nothing at all until the address
    /// stands.
    pub source: IpAddr,
    /// Where it goes: the broadcast address, or the server that granted
    /// the lease.
    pub destination: IpAddr,
    /// The datagram itself, header and all.
    pub datagram: &'a [u8],
}

/// What one offer said, which the request repeats back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offer {
    /// The address offered.
    address: Ipv4Addr,
    /// Who offered it.
    server: Ipv4Addr,
}

/// The client.
#[derive(Debug)]
pub struct Client {
    /// What it was configured with.
    config: Config,
    /// This host's hardware address.
    hardware: MacAddr,
    /// Where it stands.
    state: State,
    /// The transaction id of the exchange it is in.
    xid: u32,
    /// Whether one has been drawn for this exchange.
    has_xid: bool,
    /// When the acquisition began, which the `secs` field counts from.
    started: Instant,
    /// The delay before the next retransmission, before jitter.
    retry: Duration,
    /// When the next message is due.
    due: Instant,
    /// How many requests are left before the client starts over.
    attempts: u32,
    /// The offer that was taken.
    offer: Option<Offer>,
    /// The lease, once there is one.
    lease: Option<Lease>,
}

impl Client {
    /// A client for the host at `hardware`, which has asked for nothing
    /// yet.
    #[must_use]
    pub const fn new(hardware: MacAddr, config: Config) -> Client {
        Client {
            config,
            hardware,
            state: State::Init,
            xid: 0,
            has_xid: false,
            started: Instant::from_micros(0),
            retry: config.first_retry,
            due: Instant::from_micros(0),
            attempts: 0,
            offer: None,
            lease: None,
        }
    }

    /// Where it stands.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// The lease, once there is one.
    #[must_use]
    pub const fn lease(&self) -> Option<&Lease> {
        self.lease.as_ref()
    }

    /// The address this host holds, which is none until the lease is
    /// bound and none again once it has run out.
    #[must_use]
    pub fn address(&self) -> Option<Ipv4Addr> {
        self.lease.as_ref().map(Lease::address)
    }

    /// Begins an acquisition.
    pub const fn start(&mut self, now: Instant) {
        self.lease = None;
        self.begin(now);
    }

    /// When the client next has something to say, or `None` before the
    /// first acquisition has been started.
    ///
    /// An instant that has passed means there is work now. In the bound
    /// state the answer is T1, and in the two states behind it the earlier
    /// of the next retransmission and the deadline that ends the state.
    #[must_use]
    pub const fn poll_at(&self) -> Option<Instant> {
        match self.state {
            State::Init => None,
            _ => Some(self.due),
        }
    }

    /// Writes at most one message into `buffer` and answers where it goes.
    ///
    /// # Errors
    ///
    /// [`DhcpError::Rng`] when the generator fails, and
    /// [`DhcpError::Wire`] or [`DhcpError::Udp`] when the buffer has no
    /// room for a datagram.
    pub fn poll<'b, R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rng: &mut R,
        buffer: &'b mut [u8],
    ) -> Result<Option<Outgoing<'b>>, DhcpError> {
        if self.state == State::Init || now < self.due {
            return Ok(None);
        }
        self.advance(now);
        if !self.has_xid {
            let mut seed = [0u8; 4];
            rng.fill(&mut seed)?;
            self.xid = u32::from_be_bytes(seed);
            self.has_xid = true;
        }
        // The jitter is drawn whether or not this state uses one, so that
        // a message costs the same four bytes of the generator in every
        // state and a caller can size an entropy budget by counting
        // messages.
        let mut word = [0u8; 4];
        rng.fill(&mut word)?;
        let plan = self.plan(now);
        let mut options = [0u8; OPTIONS_LEN];
        let mut block = Writer::new(&mut options);
        plan.options(&mut block)?;
        let block_len = block.position();
        let message = Message {
            options: options.get(..block_len).unwrap_or(&[]),
            ..plan.message
        };
        let mut scratch = [0u8; MIN_MESSAGE_LEN];
        let mut inner = Writer::new(&mut scratch);
        message.write(&mut inner)?;
        let len = inner.position();
        let payload = scratch.get(..len).unwrap_or(&[]);
        let mut writer = Writer::new(buffer);
        Datagram {
            source_port: CLIENT_PORT,
            destination_port: SERVER_PORT,
            payload,
        }
        .write(
            &mut writer,
            plan.source,
            plan.destination,
            ChecksumPolicy::Computed,
        )?;
        // The backoff moves and the attempt is counted only now, with the
        // datagram in the caller's buffer: a buffer that had no room for
        // one is a buffer nothing went out of.
        self.schedule(now, u32::from_be_bytes(word));
        Ok(Some(Outgoing {
            source: plan.source,
            destination: plan.destination,
            datagram: writer.finish(),
        }))
    }

    /// Takes in the payload of a datagram that arrived from `source` at
    /// `port`.
    ///
    /// Everything that does not match is ignored: a reply whose
    /// transaction id or hardware address is another client's is another
    /// client's reply.
    pub fn on_datagram(&mut self, source: IpAddr, port: Port, payload: &[u8], now: Instant) {
        if self.state == State::Init || port != SERVER_PORT || !source.is_v4() {
            return;
        }
        let Ok(message) = Message::parse(payload) else {
            return;
        };
        if message.op != Op::REPLY
            || !self.has_xid
            || message.xid != self.xid
            || message.hardware != self.hardware
        {
            return;
        }
        let Ok(kind) = message.message_type() else {
            return;
        };
        match (self.state, kind) {
            (State::Selecting, MessageType::OFFER) => self.take_offer(&message, now),
            (State::Requesting | State::Renewing | State::Rebinding, MessageType::ACK) => {
                self.take_lease(&message, now);
            }
            (State::Requesting | State::Renewing | State::Rebinding, MessageType::NAK) => {
                // RFC 2131, section 4.4.5: a refusal takes the client back
                // to the start, with the address it thought it had gone.
                self.lease = None;
                self.begin(now);
            }
            _ => {}
        }
    }

    /// Puts the client in the selecting state with a fresh exchange.
    const fn begin(&mut self, now: Instant) {
        self.state = State::Selecting;
        self.has_xid = false;
        self.started = now;
        self.retry = self.config.first_retry;
        self.due = now;
        self.attempts = self.config.request_attempts;
        self.offer = None;
    }

    /// The transitions that a passing instant makes on its own.
    fn advance(&mut self, now: Instant) {
        if self.state == State::Requesting && self.attempts == 0 {
            // Four requests and no answer: RFC 2131, section 3.1, note 5
            // has the client start over rather than wait longer.
            self.begin(now);
        }
        let Some(lease) = self.lease.as_ref() else {
            return;
        };
        let (renew, rebind, expires) = (lease.renew, lease.rebind, lease.expires);
        if self.state == State::Bound && now >= renew {
            self.state = State::Renewing;
            // RFC 2131, table 1: `secs` counts from the beginning of an
            // acquisition **or of a renewal**, so the renewal is where it
            // starts over.
            self.started = now;
        }
        if self.state == State::Renewing && now >= rebind {
            self.state = State::Rebinding;
        }
        if self.state == State::Rebinding && now >= expires {
            self.lease = None;
            self.begin(now);
        }
    }

    /// What the next message is and where it goes.
    fn plan(&self, now: Instant) -> Plan {
        let seconds = now.saturating_duration_since(self.started).as_secs();
        let mut message = Message::request(self.xid, self.hardware);
        // The field is sixteen bits; a client that has been at this for
        // more than eighteen hours reports the largest number there is.
        message.secs = u16::try_from(seconds).unwrap_or(u16::MAX);
        match self.state {
            State::Renewing | State::Rebinding => {
                let address = self
                    .lease
                    .as_ref()
                    .map_or(Ipv4Addr::UNSPECIFIED, Lease::address);
                let server = self
                    .lease
                    .as_ref()
                    .map_or(Ipv4Addr::UNSPECIFIED, |lease| lease.server);
                message.client = address;
                Plan {
                    source: IpAddr::V4(address),
                    destination: if self.state == State::Renewing {
                        IpAddr::V4(server)
                    } else {
                        IpAddr::V4(Ipv4Addr::BROADCAST)
                    },
                    message,
                    kind: MessageType::REQUEST,
                    offer: None,
                }
            }
            State::Requesting => {
                message.broadcast = true;
                Plan {
                    source: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    destination: IpAddr::V4(Ipv4Addr::BROADCAST),
                    message,
                    kind: MessageType::REQUEST,
                    offer: self.offer,
                }
            }
            _ => {
                message.broadcast = true;
                Plan {
                    source: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    destination: IpAddr::V4(Ipv4Addr::BROADCAST),
                    message,
                    kind: MessageType::DISCOVER,
                    offer: None,
                }
            }
        }
    }

    /// Sets the instant the next message is due at, and counts the
    /// attempt.
    fn schedule(&mut self, now: Instant, word: u32) {
        match self.state {
            State::Renewing | State::Rebinding => {
                // RFC 2131, section 4.4.5: half of what is left until the
                // next deadline, and never less than a minute.
                let renewing = self.state == State::Renewing;
                let deadline = self.lease.as_ref().map_or(now, |lease| {
                    if renewing {
                        lease.rebind
                    } else {
                        lease.expires
                    }
                });
                let until = deadline.saturating_duration_since(now);
                let half = Duration::from_micros(until.as_micros() >> 1);
                let delay = half.max(self.config.min_renew_retry);
                self.due = now.saturating_add(delay).min(deadline);
            }
            _ => {
                if self.state == State::Requesting {
                    self.attempts = self.attempts.saturating_sub(1);
                }
                self.due = now.saturating_add(jittered(self.retry, self.config.jitter, word));
                self.retry = self
                    .retry
                    .saturating_add(self.retry)
                    .min(self.config.max_retry);
            }
        }
    }

    /// Takes the first offer and asks for it. A second one arrives in the
    /// requesting state and is ignored there.
    fn take_offer(&mut self, message: &Message<'_>, now: Instant) {
        if check_address(message.yours).is_err() {
            return;
        }
        let Some(server) = address_option(message, OptionCode::SERVER_IDENTIFIER) else {
            return;
        };
        self.offer = Some(Offer {
            address: message.yours,
            server,
        });
        self.state = State::Requesting;
        self.retry = self.config.first_retry;
        self.attempts = self.config.request_attempts;
        self.due = now;
    }

    /// Turns an acknowledgment into a lease.
    ///
    /// An acknowledgment this client cannot make a lease out of is
    /// ignored: the request stands and is asked again. Which option was
    /// missing, or which mask was no prefix, is what
    /// [`Lease::from_reply`] says to a caller that wants to know.
    fn take_lease(&mut self, message: &Message<'_>, now: Instant) {
        let Ok(lease) = Lease::from_reply(message, now) else {
            return;
        };
        self.due = lease.renew;
        self.lease = Some(lease);
        self.state = State::Bound;
        self.has_xid = false;
        self.offer = None;
    }
}

/// What one poll decided to say.
struct Plan {
    /// Where the datagram goes out from.
    source: IpAddr,
    /// Where it goes.
    destination: IpAddr,
    /// The message, without its options.
    message: Message<'static>,
    /// Which of the eight it is.
    kind: MessageType,
    /// The offer it repeats back, where there is one.
    offer: Option<Offer>,
}

impl Plan {
    /// Writes the option block, end marker and all.
    fn options(&self, writer: &mut Writer<'_>) -> Result<(), DhcpError> {
        write_option(writer, OptionCode::MESSAGE_TYPE, &[self.kind.get()])?;
        if let Some(offer) = self.offer {
            write_option(
                writer,
                OptionCode::REQUESTED_ADDRESS,
                &offer.address.octets(),
            )?;
            write_option(
                writer,
                OptionCode::SERVER_IDENTIFIER,
                &offer.server.octets(),
            )?;
        }
        write_option(
            writer,
            OptionCode::MAX_MESSAGE_SIZE,
            &MAX_MESSAGE_SIZE.to_be_bytes(),
        )?;
        write_option(writer, OptionCode::PARAMETER_LIST, &PARAMETERS)?;
        writer.write_u8(OptionCode::END.get())?;
        Ok(())
    }
}

/// The four bytes of `code` as an address, where the option is there and
/// is four bytes.
fn address_option(message: &Message<'_>, code: OptionCode) -> Option<Ipv4Addr> {
    let body = body_of(message, code)?;
    let octets = body.first_chunk::<4>()?;
    (body.len() == 4).then(|| Ipv4Addr::from_octets(*octets))
}

/// The four bytes of `code` as a number.
fn u32_option(message: &Message<'_>, code: OptionCode) -> Option<u32> {
    let body = body_of(message, code)?;
    let octets = body.first_chunk::<4>()?;
    (body.len() == 4).then(|| u32::from_be_bytes(*octets))
}

/// The body of the last option with `code`, where the whole block reads.
fn body_of<'a>(message: &Message<'a>, code: OptionCode) -> Option<&'a [u8]> {
    let mut found = None;
    for option in message.options() {
        let (seen, body) = option.ok()?;
        if seen == code {
            found = Some(body);
        }
    }
    found
}

/// The first address in the body of `code`. RFC 2132, section 3.5 puts the
/// routers in order of preference, so the first is the one to use.
fn first_address(message: &Message<'_>, code: OptionCode) -> Option<Ipv4Addr> {
    let octets = body_of(message, code)?.first_chunk::<4>()?;
    Some(Ipv4Addr::from_octets(*octets))
}

/// The name servers of the lease.
fn servers_of(message: &Message<'_>) -> ArrayVec<Ipv4Addr, MAX_DNS_SERVERS> {
    let mut out = ArrayVec::new();
    let Some(body) = body_of(message, OptionCode::DOMAIN_NAME_SERVER) else {
        return out;
    };
    for octets in body.as_chunks::<4>().0.iter().take(MAX_DNS_SERVERS) {
        let _ = out.push(Ipv4Addr::from_octets(*octets));
    }
    out
}

/// Whether `address` is one a host can be given.
///
/// What is refused is what cannot name one host: the unspecified address,
/// the limited broadcast, a multicast group, and a loopback address. What
/// is not refused is the network address or the directed broadcast of the
/// prefix that comes with it — RFC 3021 gives a point-to-point link a /31
/// on which both are hosts, and a /32 lease is one address that is all
/// three at once, so the rule would need more exceptions than it has
/// cases.
const fn check_address(address: Ipv4Addr) -> Result<(), DhcpError> {
    if address.is_unspecified()
        || address.is_broadcast()
        || address.is_multicast()
        || address.is_loopback()
    {
        Err(DhcpError::Address(address))
    } else {
        Ok(())
    }
}

/// The network `address` is on, where `mask` is a prefix.
fn network_of(address: Ipv4Addr, mask: Ipv4Addr) -> Result<Ipv4Cidr, DhcpError> {
    let prefix = mask.to_bits().leading_ones();
    let Ok(prefix) = u8::try_from(prefix) else {
        return Err(DhcpError::Netmask(mask));
    };
    let network = Ipv4Cidr::new(address, prefix)?;
    if network.netmask() == mask {
        Ok(network)
    } else {
        Err(DhcpError::Netmask(mask))
    }
}

/// T1 and T2, from the options where the server sent usable ones and from
/// the defaults of RFC 2131, section 4.4.5 where it did not.
fn timers(whole: Duration, first: Option<u32>, second: Option<u32>) -> (Duration, Duration) {
    let micros = whole.as_micros();
    let default = (
        Duration::from_micros(micros >> 1),
        Duration::from_micros(micros.saturating_sub(micros >> 3)),
    );
    let (Some(first), Some(second)) = (first, second) else {
        return default;
    };
    let first = Duration::from_secs(u64::from(first));
    let second = Duration::from_secs(u64::from(second));
    if first < second && second < whole {
        (first, second)
    } else {
        default
    }
}

/// `base`, moved by a uniform value between minus and plus `jitter`.
///
/// `word` is four bytes of the generator read as a number, and the
/// multiply-and-shift maps its whole range onto the span without the bias
/// a remainder over a span that is not a power of two would introduce.
fn jittered(base: Duration, jitter: Duration, word: u32) -> Duration {
    let span = jitter.as_micros().saturating_mul(2).saturating_add(1);
    let offset = u64::from(word).saturating_mul(span) >> 32;
    base.saturating_add(Duration::from_micros(offset))
        .saturating_sub(jitter)
}
