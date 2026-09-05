// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Neighbor Discovery of RFC 4861, and the options RFC 8106 adds to it.
//!
//! Four messages are read: router solicitation and advertisement,
//! neighbor solicitation and advertisement. Redirects are not: this host
//! has one interface and one default router, and a redirect is a router
//! telling it about a better one, which is a field an attacker writes for
//! a gain this host has no use for.
//!
//! What this protocol resolves goes into the neighbor cache of `net-eth`
//! and not into a second cache of its own (D-69). ARP fills that cache's
//! IPv4 half; the three functions at the end of this module fill the
//! other, and the states, the timers, and the storage stay where they
//! are.
//!
//! A solicitation is addressed to the solicited-node multicast group of
//! its target and not to a broadcast, which is the whole reason
//! `Ipv6Addr::solicited_node` exists. On an Ethernet that group is
//! reached at the address `net_eth::multicast_hardware` derives, so a
//! station whose address does not end in the same twenty-four bits never
//! sees the frame — where every station on the link sees every ARP
//! request.
//!
//! RFC 4861, section 7.1 requires three things of every message here, and
//! [`receive`] checks all three: the hop limit is 255, so nothing that
//! crossed a router is read as Neighbor Discovery; the code is zero; and
//! the checksum verifies over the pseudo-header.

use net_eth::NeighborCache;
use net_wire::{IpAddr, Ipv6Addr, MacAddr, Reader, Writer};

use audhsos_time::Instant;

use crate::error::Ipv6Error;
use crate::header::{DISCOVERY_HOP_LIMIT, Packet};
use crate::icmp::{self, HEADER_LEN as ICMP_HEADER_LEN};

/// Router solicitation.
pub const ROUTER_SOLICITATION: u8 = 133;
/// Router advertisement.
pub const ROUTER_ADVERTISEMENT: u8 = 134;
/// Neighbor solicitation.
pub const NEIGHBOR_SOLICITATION: u8 = 135;
/// Neighbor advertisement.
pub const NEIGHBOR_ADVERTISEMENT: u8 = 136;

/// The option type numbers of RFC 4861, section 4.6 and RFC 8106,
/// section 5.
pub mod option_type {
    /// The sender's link-layer address.
    pub const SOURCE_LINK_LAYER: u8 = 1;
    /// The link-layer address being advertised.
    pub const TARGET_LINK_LAYER: u8 = 2;
    /// A prefix, with the flags and lifetimes that go with it.
    pub const PREFIX: u8 = 3;
    /// The MTU of the link.
    pub const MTU: u8 = 5;
    /// The recursive DNS servers of RFC 8106.
    pub const RDNSS: u8 = 25;
}

/// How long one unit of an option's length field is.
const OPTION_UNIT: usize = 8;

/// The two bytes every option begins with: its type and its length.
const OPTION_HEADER_LEN: usize = 2;

/// The lifetime that stands for "for ever" in every field of RFC 4861
/// and RFC 8106 that has one.
pub const INFINITE_LIFETIME: u32 = u32::MAX;

/// One Neighbor Discovery message, borrowed from the bytes it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discovery<'a> {
    /// A host asking the routers on the link to advertise themselves.
    RouterSolicitation {
        /// What came with it.
        options: Options<'a>,
    },
    /// A router describing itself and the link.
    RouterAdvertisement {
        /// What the router recommends as a hop limit, or zero for no
        /// recommendation.
        hop_limit: u8,
        /// Whether addresses are available over `DHCPv6`, which this system
        /// does not speak (D-69).
        managed: bool,
        /// Whether other configuration is, likewise.
        other: bool,
        /// How many seconds this router may be a default router, or zero
        /// for none.
        router_lifetime: u16,
        /// What the router says a neighbor stays reachable for, in
        /// milliseconds, or zero for no statement.
        reachable_time: u32,
        /// What it says between two solicitations, likewise.
        retransmit_time: u32,
        /// What came with it: prefixes, an MTU, DNS servers.
        options: Options<'a>,
    },
    /// Who holds this address?
    NeighborSolicitation {
        /// The address being asked about.
        target: Ipv6Addr,
        /// What came with it.
        options: Options<'a>,
    },
    /// This link-layer address holds it.
    NeighborAdvertisement {
        /// The address being answered for.
        target: Ipv6Addr,
        /// Whether the sender is a router.
        router: bool,
        /// Whether this answers a solicitation of the receiver's, which
        /// is the only evidence that makes a neighbor reachable.
        solicited: bool,
        /// Whether it may replace a link-layer address already cached.
        overriding: bool,
        /// What came with it.
        options: Options<'a>,
    },
}

impl<'a> Discovery<'a> {
    /// The message in `bytes`, which are an `ICMPv6` message.
    ///
    /// The checksum is not checked here and the hop limit cannot be:
    /// [`receive`] does both, and is what a receive path calls.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::NotDiscovery`] when the type is not one of the four
    /// or the code is not zero, [`Ipv6Error::BadOption`] when an option
    /// has no length or runs past the message, and [`Ipv6Error::Wire`]
    /// when the message ends inside a fixed field.
    pub fn parse(bytes: &'a [u8]) -> Result<Discovery<'a>, Ipv6Error> {
        let mut reader = Reader::new(bytes);
        let message_type = reader.read_u8()?;
        let code = reader.read_u8()?;
        if code != 0 {
            return Err(Ipv6Error::NotDiscovery(message_type));
        }
        let _checksum = reader.read_u16()?;
        let message = match message_type {
            ROUTER_SOLICITATION => {
                // Four reserved bytes, then the options.
                reader.skip(4)?;
                Discovery::RouterSolicitation {
                    options: Options::new(reader.rest())?,
                }
            }
            ROUTER_ADVERTISEMENT => {
                let hop_limit = reader.read_u8()?;
                let flags = reader.read_u8()?;
                let router_lifetime = reader.read_u16()?;
                let reachable_time = reader.read_u32()?;
                let retransmit_time = reader.read_u32()?;
                Discovery::RouterAdvertisement {
                    hop_limit,
                    managed: flags & 0x80 != 0,
                    other: flags & 0x40 != 0,
                    router_lifetime,
                    reachable_time,
                    retransmit_time,
                    options: Options::new(reader.rest())?,
                }
            }
            NEIGHBOR_SOLICITATION => {
                reader.skip(4)?;
                Discovery::NeighborSolicitation {
                    target: reader.read_ipv6()?,
                    options: Options::new(reader.rest())?,
                }
            }
            NEIGHBOR_ADVERTISEMENT => {
                let flags = reader.read_u8()?;
                reader.skip(3)?;
                Discovery::NeighborAdvertisement {
                    target: reader.read_ipv6()?,
                    router: flags & 0x80 != 0,
                    solicited: flags & 0x40 != 0,
                    overriding: flags & 0x20 != 0,
                    options: Options::new(reader.rest())?,
                }
            }
            other => return Err(Ipv6Error::NotDiscovery(other)),
        };
        Ok(message)
    }

    /// What came with the message.
    #[must_use]
    pub const fn options(&self) -> Options<'a> {
        match *self {
            Discovery::RouterSolicitation { options }
            | Discovery::RouterAdvertisement { options, .. }
            | Discovery::NeighborSolicitation { options, .. }
            | Discovery::NeighborAdvertisement { options, .. } => options,
        }
    }
}

/// Reads the Neighbor Discovery message `packet` carries.
///
/// `message` is the `ICMPv6` message the chain walk reached. All three
/// checks of RFC 4861, section 7.1 are made here: the hop limit is 255,
/// the checksum verifies over the pseudo-header, and the code is zero,
/// which [`Discovery::parse`] does.
///
/// The hop limit is the important one. Neighbor Discovery is a link-local
/// protocol whose messages carry no other proof of where they came from,
/// and 255 is the only value a packet cannot have if it crossed a router:
/// every router decrements it. A host that skips this check accepts
/// neighbor advertisements from the whole internet.
///
/// # Errors
///
/// [`Ipv6Error::NotDiscovery`] when the hop limit is not 255,
/// [`Ipv6Error::BadIcmp`] when the checksum does not verify, and whatever
/// [`Discovery::parse`] returns.
pub fn receive<'a>(packet: Packet<'a>, message: &'a [u8]) -> Result<Discovery<'a>, Ipv6Error> {
    if packet.hop_limit() != DISCOVERY_HOP_LIMIT {
        let message_type = message.first().copied().unwrap_or(0);
        return Err(Ipv6Error::NotDiscovery(message_type));
    }
    if !icmp::verify(packet.source(), packet.destination(), message) {
        return Err(Ipv6Error::BadIcmp);
    }
    Discovery::parse(message)
}

/// The options of one message, as a walk over the bytes behind it.
///
/// Every option is well formed by the time this exists: [`Options::new`]
/// walks the list once and refuses a length of zero, which RFC 4861,
/// section 4.6 requires the packet be discarded over, and one that runs
/// past the message. That is what lets
/// [`next_option`](Options::next_option) be infallible.
///
/// The walk is [`next_option`](Options::next_option) and not an
/// [`Iterator`], for the reason `net_ip::Fragments` gives: the type is
/// `Copy`, and an iterator that is copied silently starts again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options<'a> {
    /// What is left to read.
    rest: &'a [u8],
}

impl<'a> Options<'a> {
    /// The options in `bytes`, checked for shape.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::BadOption`] for an option of length zero or one that
    /// runs past the end.
    pub fn new(bytes: &'a [u8]) -> Result<Options<'a>, Ipv6Error> {
        let mut rest = bytes;
        while let Some((head, _)) = rest.split_first_chunk::<OPTION_HEADER_LEN>() {
            let [kind, units] = *head;
            if units == 0 {
                return Err(Ipv6Error::BadOption(kind));
            }
            let length = usize::from(units).saturating_mul(OPTION_UNIT);
            let (_, tail) = rest
                .split_at_checked(length)
                .ok_or(Ipv6Error::BadOption(kind))?;
            rest = tail;
        }
        if !rest.is_empty() {
            // One byte is left over, which is not an option and cannot
            // become one.
            return Err(Ipv6Error::BadOption(rest.first().copied().unwrap_or(0)));
        }
        Ok(Options { rest: bytes })
    }

    /// The bytes the options occupy.
    #[must_use]
    pub const fn bytes(self) -> &'a [u8] {
        self.rest
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.rest.is_empty()
    }

    /// The link-layer address of whoever sent the message, if it gave
    /// one.
    #[must_use]
    pub fn source_link_layer(self) -> Option<MacAddr> {
        let mut options = self;
        while let Some(option) = options.next_option() {
            if let NdpOption::SourceLinkLayer(hardware) = option {
                return Some(hardware);
            }
        }
        None
    }

    /// The link-layer address the message advertises, if it gave one.
    #[must_use]
    pub fn target_link_layer(self) -> Option<MacAddr> {
        let mut options = self;
        while let Some(option) = options.next_option() {
            if let NdpOption::TargetLinkLayer(hardware) = option {
                return Some(hardware);
            }
        }
        None
    }

    /// The MTU the router recommends for the link, if it gave one.
    #[must_use]
    pub fn link_mtu(self) -> Option<u32> {
        let mut options = self;
        while let Some(option) = options.next_option() {
            if let NdpOption::Mtu(mtu) = option {
                return Some(mtu);
            }
        }
        None
    }
}

impl<'a> Options<'a> {
    /// The next option, or `None` at the end.
    ///
    /// It is not called `next`, and it is not an [`Iterator`], for the
    /// reason the type's own documentation gives.
    pub fn next_option(&mut self) -> Option<NdpOption<'a>> {
        let (head, _) = self.rest.split_first_chunk::<OPTION_HEADER_LEN>()?;
        let [kind, units] = *head;
        let length = usize::from(units).saturating_mul(OPTION_UNIT);
        let (option, tail) = self.rest.split_at_checked(length)?;
        self.rest = tail;
        Some(NdpOption::read(kind, option))
    }
}

/// One option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdpOption<'a> {
    /// The sender's link-layer address.
    SourceLinkLayer(MacAddr),
    /// The link-layer address being advertised.
    TargetLinkLayer(MacAddr),
    /// A prefix and what may be done with it.
    Prefix(PrefixInformation),
    /// The MTU of the link.
    Mtu(u32),
    /// The recursive DNS servers of RFC 8106.
    Rdnss(Rdnss<'a>),
    /// An option this crate does not read, or one whose body is not the
    /// length its type requires. RFC 4861, section 4.6 has an
    /// unrecognized option ignored, and a malformed body is read as
    /// unrecognized rather than as a reason to drop the message: the
    /// length field was right, so the walk is not lost.
    Other {
        /// The type number.
        kind: u8,
        /// The body, without the two bytes of type and length.
        body: &'a [u8],
    },
}

impl<'a> NdpOption<'a> {
    /// Reads one option, whose length field has already been checked.
    fn read(kind: u8, option: &'a [u8]) -> NdpOption<'a> {
        let body = option.get(OPTION_HEADER_LEN..).unwrap_or(&[]);
        let unknown = NdpOption::Other { kind, body };
        match kind {
            option_type::SOURCE_LINK_LAYER => {
                hardware(body).map_or(unknown, NdpOption::SourceLinkLayer)
            }
            option_type::TARGET_LINK_LAYER => {
                hardware(body).map_or(unknown, NdpOption::TargetLinkLayer)
            }
            option_type::PREFIX => PrefixInformation::read(body).map_or(unknown, NdpOption::Prefix),
            option_type::MTU => mtu(body).map_or(unknown, NdpOption::Mtu),
            option_type::RDNSS => Rdnss::read(body).map_or(unknown, NdpOption::Rdnss),
            _ => unknown,
        }
    }
}

/// The Ethernet address in a link-layer option body.
///
/// The length has to be exactly six. RFC 4861, section 4.6.1 gives the
/// option a length of one unit for a link whose addresses are six bytes,
/// and a body of any other length belongs to a link this crate does not
/// carry frames over — so it is read as unrecognized rather than as the
/// first six bytes of something else.
fn hardware(body: &[u8]) -> Option<MacAddr> {
    <[u8; MacAddr::LEN]>::try_from(body).ok().map(MacAddr::new)
}

/// The MTU in an MTU option body: two reserved bytes and four of number,
/// and no other length.
fn mtu(body: &[u8]) -> Option<u32> {
    let (_reserved, number) = body.split_at_checked(2)?;
    Some(u32::from_be_bytes(<[u8; 4]>::try_from(number).ok()?))
}

/// A prefix, as RFC 4861, section 4.6.2 carries one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PrefixInformation {
    /// The prefix itself.
    pub prefix: Ipv6Addr,
    /// How many of its leading bits are the prefix.
    pub prefix_len: u8,
    /// Whether an address under it is on this link.
    pub on_link: bool,
    /// Whether a host may form an address from it, which is what SLAAC
    /// reads.
    pub autonomous: bool,
    /// How many seconds the prefix stays valid.
    pub valid_lifetime: u32,
    /// How many of those it is preferred for.
    pub preferred_lifetime: u32,
}

/// How long the body of a prefix option is: the length field says four
/// units, and thirty of the thirty-two bytes are body.
const PREFIX_BODY_LEN: usize = 30;

impl PrefixInformation {
    /// Reads the option body, or `None` when it is not the one length
    /// this option has. RFC 4861, section 4.6.2 gives it a length of four
    /// units and no other.
    fn read(body: &[u8]) -> Option<PrefixInformation> {
        if body.len() != PREFIX_BODY_LEN {
            return None;
        }
        let mut reader = Reader::new(body);
        let prefix_len = reader.read_u8().ok()?;
        let flags = reader.read_u8().ok()?;
        let valid_lifetime = reader.read_u32().ok()?;
        let preferred_lifetime = reader.read_u32().ok()?;
        // Four reserved bytes, which carried a router address in an
        // earlier revision and carry nothing now.
        reader.skip(4).ok()?;
        let prefix = reader.read_ipv6().ok()?;
        Some(PrefixInformation {
            prefix,
            prefix_len,
            on_link: flags & 0x80 != 0,
            autonomous: flags & 0x40 != 0,
            valid_lifetime,
            preferred_lifetime,
        })
    }
}

/// The recursive DNS servers of RFC 8106, section 5.1.
///
/// The addresses are borrowed rather than copied: how many there are is a
/// function of the option's length, and a host that reads two of them
/// should not pay for a fixed array of eight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rdnss<'a> {
    /// How many seconds the addresses may be used for, `u32::MAX` for
    /// ever and zero to stop using them.
    pub lifetime: u32,
    /// The addresses, sixteen bytes each.
    addresses: &'a [u8],
}

impl<'a> Rdnss<'a> {
    /// Reads the option body, or `None` when its length is not one this
    /// option has: RFC 8106 requires at least one address and a whole
    /// number of them.
    fn read(body: &'a [u8]) -> Option<Rdnss<'a>> {
        // Two reserved bytes and the lifetime, then the addresses.
        let (head, addresses) = body.split_at_checked(6)?;
        let lifetime = u32::from_be_bytes(*head.last_chunk::<4>()?);
        if addresses.is_empty() || addresses.len().checked_rem(Ipv6Addr::LEN) != Some(0) {
            return None;
        }
        Some(Rdnss {
            lifetime,
            addresses,
        })
    }

    /// How many servers the option names.
    #[must_use]
    pub const fn len(self) -> usize {
        self.addresses.len().saturating_div(Ipv6Addr::LEN)
    }

    /// Whether it names none, which it never does: an option that named
    /// none was not read as this option at all.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.addresses.is_empty()
    }

    /// The addresses, in the order the option gives them.
    pub fn iter(self) -> impl Iterator<Item = Ipv6Addr> + 'a {
        self.addresses
            .as_chunks::<{ Ipv6Addr::LEN }>()
            .0
            .iter()
            .copied()
            .map(Ipv6Addr::from_octets)
    }
}

/// Writes a neighbor solicitation asking who holds `target`.
///
/// The destination is the solicited-node group of the target, which is
/// what [`Ipv6Addr::solicited_node`] derives. `link_layer` is this host's
/// hardware address, which RFC 4861, section 4.3 forbids including when
/// the source is unspecified — that is the duplicate address detection
/// case, where this host has no address to be answered at yet.
///
/// # Errors
///
/// [`Ipv6Error::Wire`] when the buffer is too small.
pub fn write_neighbor_solicitation(
    writer: &mut Writer<'_>,
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    link_layer: Option<MacAddr>,
) -> Result<(), Ipv6Error> {
    let at = writer.position();
    writer.write_u8(NEIGHBOR_SOLICITATION)?;
    writer.write_u8(0)?;
    writer.write_u16(0)?;
    writer.write_u32(0)?;
    writer.write_ipv6(target)?;
    if let Some(hardware) = link_layer {
        write_link_layer(writer, option_type::SOURCE_LINK_LAYER, hardware)?;
    }
    icmp::finish(writer, at, source, destination)
}

/// Writes a neighbor advertisement saying that `hardware` holds `target`.
///
/// `solicited` says this answers a solicitation, which is what makes the
/// receiver's cache entry reachable; `overriding` says it may replace a
/// link-layer address the receiver already has.
///
/// # Errors
///
/// [`Ipv6Error::Wire`] when the buffer is too small.
pub fn write_neighbor_advertisement(
    writer: &mut Writer<'_>,
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    hardware: MacAddr,
    solicited: bool,
    overriding: bool,
) -> Result<(), Ipv6Error> {
    let mut flags = 0u8;
    if solicited {
        flags |= 0x40;
    }
    if overriding {
        flags |= 0x20;
    }
    let at = writer.position();
    writer.write_u8(NEIGHBOR_ADVERTISEMENT)?;
    writer.write_u8(0)?;
    writer.write_u16(0)?;
    writer.write_u8(flags)?;
    writer.write_u8(0)?;
    writer.write_u16(0)?;
    writer.write_ipv6(target)?;
    write_link_layer(writer, option_type::TARGET_LINK_LAYER, hardware)?;
    icmp::finish(writer, at, source, destination)
}

/// Writes a router solicitation, which a host sends to
/// [`Ipv6Addr::ALL_ROUTERS`] to be advertised to without waiting for the
/// next periodic advertisement.
///
/// # Errors
///
/// [`Ipv6Error::Wire`] when the buffer is too small.
pub fn write_router_solicitation(
    writer: &mut Writer<'_>,
    source: Ipv6Addr,
    destination: Ipv6Addr,
    link_layer: Option<MacAddr>,
) -> Result<(), Ipv6Error> {
    let at = writer.position();
    writer.write_u8(ROUTER_SOLICITATION)?;
    writer.write_u8(0)?;
    writer.write_u16(0)?;
    writer.write_u32(0)?;
    if let Some(hardware) = link_layer {
        write_link_layer(writer, option_type::SOURCE_LINK_LAYER, hardware)?;
    }
    icmp::finish(writer, at, source, destination)
}

/// Writes a link-layer address option, which is eight bytes for an
/// Ethernet: the type, the length of one unit, and the six of the
/// address.
fn write_link_layer(writer: &mut Writer<'_>, kind: u8, hardware: MacAddr) -> Result<(), Ipv6Error> {
    writer.write_u8(kind)?;
    writer.write_u8(1)?;
    writer.write_mac(hardware)?;
    Ok(())
}

/// How long the shortest message of each kind is, which is what a
/// solicitation and an advertisement take with one link-layer option.
pub const NEIGHBOR_SOLICITATION_LEN: usize = ICMP_HEADER_LEN + Ipv6Addr::LEN + 8;

/// Whether this host owes an advertisement for `address` in answer to
/// `message`.
///
/// A node answers for the addresses it holds and for nothing else. There
/// is no proxy here: RFC 4861, section 7.2.8 allows a node to answer for
/// another, and a host that did would be handing out a mapping it cannot
/// vouch for.
#[must_use]
pub fn answers(message: &Discovery<'_>, address: Ipv6Addr) -> bool {
    matches!(
        message,
        Discovery::NeighborSolicitation { target, .. } if *target == address
    )
}

/// Takes what a neighbor advertisement said into the cache, and answers
/// whether it changed anything.
///
/// A solicited advertisement is evidence: this host asked, and the node
/// that holds the address answered, so the entry becomes reachable. An
/// unsolicited one is not, and goes in the way a gratuitous ARP does —
/// which is to say it never replaces the address of an entry this host is
/// actively using.
///
/// The override bit is honored on top of that. RFC 4861, section 7.2.5
/// has an advertisement without it leave a cached link-layer address
/// alone when the two disagree, whatever state the entry is in, and that
/// is a check the cache cannot make for itself: it does not know the bit
/// exists.
pub fn on_advertisement<const ENTRIES: usize, const PENDING: usize>(
    cache: &mut NeighborCache<ENTRIES, PENDING>,
    message: &Discovery<'_>,
    now: Instant,
) -> bool {
    let Discovery::NeighborAdvertisement {
        target,
        solicited,
        overriding,
        options,
        ..
    } = *message
    else {
        return false;
    };
    let Some(hardware) = options.target_link_layer() else {
        // Nothing to learn. RFC 4861 allows the option to be left out
        // when the advertisement only confirms what the receiver has,
        // and confirming a mapping this host does not hold says nothing.
        return false;
    };
    let address = IpAddr::V6(target);
    if !overriding
        && cache
            .hardware(address)
            .is_some_and(|known| known != hardware)
    {
        return false;
    }
    if solicited {
        cache.on_confirmed(address, hardware, now);
        return true;
    }
    cache.on_observed(address, hardware, now)
}

/// Takes the source link-layer address of a neighbor solicitation into
/// the cache, and answers whether it changed anything.
///
/// A solicitation whose source is unspecified is a duplicate address
/// detection probe (RFC 4862, section 5.4.2). It teaches nothing: the
/// sender has no address yet, and an entry under the unspecified address
/// is an entry nothing can be sent to.
pub fn on_solicitation<const ENTRIES: usize, const PENDING: usize>(
    cache: &mut NeighborCache<ENTRIES, PENDING>,
    source: Ipv6Addr,
    message: &Discovery<'_>,
    now: Instant,
) -> bool {
    let Discovery::NeighborSolicitation { options, .. } = *message else {
        return false;
    };
    if source.is_unspecified() {
        return false;
    }
    let Some(hardware) = options.source_link_layer() else {
        return false;
    };
    cache.on_observed(IpAddr::V6(source), hardware, now)
}
