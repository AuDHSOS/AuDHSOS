// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Neighbor Discovery against the message and option formats of
//! RFC 4861, and the cache of `net-eth` it writes into.
//!
//! Every instant here is an argument, so the whole schedule of RFC 4861,
//! section 10 passes in microseconds of wall clock.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a message at known offsets"
)]

use audhsos_time::{Duration, Instant};
use net_eth::{Event, NeighborCache, NeighborState, Resolution, Timers, multicast_hardware};
use net_wire::{IpAddr, Ipv6Addr, MacAddr, Protocol, Writer};
use test_support::generators::{BoxGen, Generator, bool, bytes, pair, range};
use test_support::model::{ModelFailure, ModelTest, run_model_test, run_model_test_with};
use test_support::property::Config;
use test_support::property::check;

use super::{HARDWARE, HOST, PEER, PEER_HARDWARE, at, checksummed, packet, secs};
use crate::error::Ipv6Error;
use crate::header::{DISCOVERY_HOP_LIMIT, Packet};
use crate::ndp::{
    Discovery, NEIGHBOR_ADVERTISEMENT, NEIGHBOR_SOLICITATION, NdpOption, Options,
    ROUTER_SOLICITATION, answers, on_advertisement, on_solicitation, option_type, receive,
    write_neighbor_advertisement, write_neighbor_solicitation, write_router_solicitation,
};

/// A cache of four neighbors, each able to hold a packet of 64 bytes.
type Cache = NeighborCache<4, 64>;

/// A link-layer address option of `kind` for `hardware`.
fn link_layer(kind: u8, hardware: MacAddr) -> Vec<u8> {
    let mut option = vec![kind, 1];
    option.extend_from_slice(&hardware.octets());
    option
}

/// A neighbor advertisement for `target`, with the flags spelled out.
fn advertisement(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    hardware: Option<MacAddr>,
    solicited: bool,
    overriding: bool,
) -> Vec<u8> {
    let mut flags = 0u8;
    if solicited {
        flags |= 0x40;
    }
    if overriding {
        flags |= 0x20;
    }
    let mut message = vec![NEIGHBOR_ADVERTISEMENT, 0, 0, 0, flags, 0, 0, 0];
    message.extend_from_slice(&target.octets());
    if let Some(hardware) = hardware {
        message.extend_from_slice(&link_layer(option_type::TARGET_LINK_LAYER, hardware));
    }
    checksummed(source, destination, &mut message);
    message
}

/// A neighbor solicitation for `target`.
fn solicitation(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    hardware: Option<MacAddr>,
) -> Vec<u8> {
    let mut message = vec![NEIGHBOR_SOLICITATION, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&target.octets());
    if let Some(hardware) = hardware {
        message.extend_from_slice(&link_layer(option_type::SOURCE_LINK_LAYER, hardware));
    }
    checksummed(source, destination, &mut message);
    message
}

/// The packet a Neighbor Discovery message arrives in: hop limit 255, as
/// RFC 4861, section 7.1 requires.
fn discovery_packet(source: Ipv6Addr, destination: Ipv6Addr, message: &[u8]) -> Vec<u8> {
    packet(
        source,
        destination,
        Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        message,
    )
}

/// Reads the discovery message out of a packet built by the two above.
fn read(bytes: &[u8]) -> Result<Discovery<'_>, Ipv6Error> {
    let packet = Packet::parse(bytes)?;
    let upper = packet.upper_layer()?;
    receive(packet, upper.payload)
}

#[test]
fn a_solicitation_is_addressed_to_the_solicited_node_group_of_its_target() {
    let group = PEER.solicited_node();
    // RFC 4291, section 2.7.1: `ff02::1:ff` and the low twenty-four bits
    // of the target.
    assert_eq!(group.to_string(), "ff02::1:ff00:2");
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    write_neighbor_solicitation(&mut writer, HOST, group, PEER, Some(HARDWARE))
        .expect("room for a solicitation");
    let message = writer.written().to_vec();

    let bytes = discovery_packet(HOST, group, &message);
    let Ok(Discovery::NeighborSolicitation { target, options }) = read(&bytes) else {
        panic!("a solicitation reads as one");
    };
    assert_eq!(target, PEER);
    assert_eq!(options.source_link_layer(), Some(HARDWARE));
    // And on an Ethernet the frame goes to the group's own address, not
    // to a broadcast: a station whose address ends elsewhere never sees
    // it (RFC 2464, section 7).
    assert_eq!(
        multicast_hardware(group),
        MacAddr::new([0x33, 0x33, 0xFF, 0x00, 0x00, 0x02])
    );
}

#[test]
fn a_solicited_advertisement_makes_the_entry_reachable() {
    let mut cache = Cache::new();
    assert_eq!(
        cache.resolve(IpAddr::V6(PEER), b"waiting", at(0)),
        Resolution::Waiting
    );
    assert_eq!(
        cache.state(IpAddr::V6(PEER)),
        Some(NeighborState::Incomplete)
    );

    let bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(on_advertisement(&mut cache, &message, at(1)));
    assert_eq!(
        cache.state(IpAddr::V6(PEER)),
        Some(NeighborState::Reachable)
    );
    assert_eq!(cache.hardware(IpAddr::V6(PEER)), Some(PEER_HARDWARE));
    // The packet that waited is now sendable.
    assert_eq!(
        cache.pending(IpAddr::V6(PEER)),
        Some((PEER_HARDWARE, b"waiting".as_slice()))
    );
}

#[test]
fn an_unsolicited_advertisement_is_learned_but_does_not_confirm() {
    let mut cache = Cache::new();
    let bytes = advertisement(
        PEER,
        Ipv6Addr::ALL_NODES,
        PEER,
        Some(PEER_HARDWARE),
        false,
        true,
    );
    let packet_bytes = discovery_packet(PEER, Ipv6Addr::ALL_NODES, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(on_advertisement(&mut cache, &message, at(1)));
    // Stale, not reachable: nobody asked, so nothing was confirmed.
    assert_eq!(cache.state(IpAddr::V6(PEER)), Some(NeighborState::Stale));
    assert_eq!(cache.hardware(IpAddr::V6(PEER)), Some(PEER_HARDWARE));
}

#[test]
fn an_advertisement_without_the_override_bit_leaves_a_known_address_alone() {
    let mut cache = Cache::new();
    cache.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let impostor = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xFF]);
    let bytes = advertisement(PEER, HOST, PEER, Some(impostor), true, false);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    // RFC 4861, section 7.2.5 I (a): the address is not taken, and the
    // entry is demoted all the same, so the next packet to that neighbor
    // checks the mapping rather than trusting it for the rest of the
    // reachable time.
    assert!(on_advertisement(&mut cache, &message, at(1)));
    assert_eq!(cache.hardware(IpAddr::V6(PEER)), Some(PEER_HARDWARE));
    assert_eq!(cache.state(IpAddr::V6(PEER)), Some(NeighborState::Stale));
}

#[test]
fn an_advertisement_without_the_override_bit_is_ignored_by_an_entry_that_is_not_reachable() {
    // RFC 4861, section 7.2.5 I (b): there is nothing to demote, and the
    // address is still not taken.
    let mut cache = Cache::new();
    cache.on_observed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    assert_eq!(cache.state(IpAddr::V6(PEER)), Some(NeighborState::Stale));
    let impostor = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xFF]);
    let bytes = advertisement(PEER, HOST, PEER, Some(impostor), true, false);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(!on_advertisement(&mut cache, &message, at(1)));
    assert_eq!(cache.hardware(IpAddr::V6(PEER)), Some(PEER_HARDWARE));
    assert_eq!(cache.state(IpAddr::V6(PEER)), Some(NeighborState::Stale));
}

#[test]
fn an_advertisement_with_no_link_layer_option_teaches_nothing() {
    let mut cache = Cache::new();
    let bytes = advertisement(PEER, HOST, PEER, None, true, true);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(!on_advertisement(&mut cache, &message, at(1)));
    assert!(cache.is_empty());
}

#[test]
fn a_solicitation_teaches_the_cache_the_sender_it_names() {
    let mut cache = Cache::new();
    let group = HOST.solicited_node();
    let bytes = solicitation(PEER, group, HOST, Some(PEER_HARDWARE));
    let packet_bytes = discovery_packet(PEER, group, &bytes);
    let message = read(&packet_bytes).expect("a solicitation");
    assert!(on_solicitation(&mut cache, PEER, &message, at(1)));
    assert_eq!(cache.state(IpAddr::V6(PEER)), Some(NeighborState::Stale));
    assert_eq!(cache.hardware(IpAddr::V6(PEER)), Some(PEER_HARDWARE));
    // And this host owes an answer, because the target is its own
    // address and nobody else's.
    assert!(answers(&message, HOST));
    assert!(!answers(&message, PEER));
}

#[test]
fn a_solicitation_from_nowhere_teaches_nothing() {
    let mut cache = Cache::new();
    let group = HOST.solicited_node();
    let bytes = solicitation(Ipv6Addr::UNSPECIFIED, group, HOST, None);
    let packet_bytes = discovery_packet(Ipv6Addr::UNSPECIFIED, group, &bytes);
    let message = read(&packet_bytes).expect("a duplicate address detection probe");
    assert!(!on_solicitation(
        &mut cache,
        Ipv6Addr::UNSPECIFIED,
        &message,
        at(1)
    ));
    assert!(cache.is_empty());
}

#[test]
fn a_solicitation_with_no_link_layer_option_teaches_nothing() {
    let mut cache = Cache::new();
    let bytes = solicitation(PEER, HOST, HOST, None);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("a solicitation");
    assert!(!on_solicitation(&mut cache, PEER, &message, at(1)));
    assert!(cache.is_empty());
}

#[test]
fn the_wrong_kind_of_message_changes_nothing() {
    let mut cache = Cache::new();
    let bytes = solicitation(PEER, HOST, HOST, Some(PEER_HARDWARE));
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("a solicitation");
    assert!(!on_advertisement(&mut cache, &message, at(1)));

    let bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(!on_solicitation(&mut cache, PEER, &message, at(1)));
    assert!(!answers(&message, HOST));
}

/// One thing that can happen to a neighbor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NdpOp {
    /// A packet is handed to the cache for the neighbor.
    Send,
    /// The packet that was waiting has gone out.
    Taken,
    /// A neighbor advertisement arrives.
    Advertise {
        /// Whether it answers a solicitation of this host's.
        solicited: bool,
        /// Whether it may replace a cached address.
        overriding: bool,
        /// Whether it names a different hardware address than the one
        /// that was cached.
        moved: bool,
    },
    /// A neighbor solicitation from the neighbor arrives.
    Solicit {
        /// The same.
        moved: bool,
    },
    /// Time moves on by this many seconds.
    Wait(u32),
    /// The cache is asked what it wants done.
    Poll,
}

/// Every operation, weighted so that a sequence reaches the far states.
///
/// Waiting and polling are drawn more often than the rest, because
/// `Probe` is four operations deep: a packet, an answer, a wait past the
/// reachable time, a second packet, a wait past the delay, and a poll.
fn any_ndp_op() -> BoxGen<NdpOp> {
    pair(
        range(0u8..=15),
        pair(range(1u32..=40), pair(bool(), pair(bool(), bool()))),
    )
    .map(
        |(choice, (seconds, (solicited, (overriding, moved))))| match choice {
            0 | 1 => NdpOp::Send,
            2 => NdpOp::Taken,
            3 | 4 => NdpOp::Advertise {
                solicited,
                overriding,
                moved,
            },
            5 => NdpOp::Solicit { moved },
            6..=10 => NdpOp::Wait(seconds),
            _ => NdpOp::Poll,
        },
    )
    .boxed()
}

/// The packet that waits behind an unresolved neighbor.
const HELD: &[u8] = b"a packet";

/// The hardware address a moved neighbor claims.
const IMPOSTOR: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xFF]);

/// The cache under test, with the clock the operations move.
struct Station {
    /// What is being tested.
    cache: Cache,
    /// What time it is.
    now: Instant,
}

/// One entry of the reference model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Believed {
    /// How sure this station is.
    state: NeighborState,
    /// The address, meaningless while `Incomplete`.
    hardware: MacAddr,
    /// When the current state runs out; `Instant::MAX` for never.
    deadline: Instant,
    /// How many solicitations have gone out in this state.
    solicits: u8,
    /// Whether a packet is waiting.
    waiting: bool,
}

/// The reference model: one neighbor, the five states of RFC 4861,
/// section 7.3.2, and the schedule of its section 10, written from the
/// document rather than from the cache.
struct Reference {
    /// What is believed about the neighbor, when anything is.
    entry: Option<Believed>,
    /// What time it is here.
    now: Instant,
    /// Which states and events this sequence went through. It is state of
    /// the model and of nothing else, and the runner reads it once the
    /// sequence has run.
    reached: u32,
}

/// What both sides are compared on after every operation.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    /// How many neighbors are known.
    len: usize,
    /// How sure this station is of the one under test.
    state: Option<NeighborState>,
    /// The address it would send to.
    hardware: Option<MacAddr>,
    /// The packet that is waiting, with the address it would go to.
    pending: Option<(MacAddr, Vec<u8>)>,
    /// When there is next work.
    poll_at: Option<Instant>,
}

/// What the cache shows.
fn seen(station: &Station) -> Seen {
    let peer = IpAddr::V6(PEER);
    Seen {
        len: station.cache.len(),
        state: station.cache.state(peer),
        hardware: station.cache.hardware(peer),
        pending: station
            .cache
            .pending(peer)
            .map(|(hardware, packet)| (hardware, packet.to_vec())),
        poll_at: station.cache.poll_at(),
    }
}

/// What the model says the cache should show.
fn believed(reference: &Reference) -> Seen {
    let Some(entry) = reference.entry else {
        return Seen {
            len: 0,
            state: None,
            hardware: None,
            pending: None,
            poll_at: None,
        };
    };
    let usable = entry.state.is_usable();
    Seen {
        len: 1,
        state: Some(entry.state),
        hardware: usable.then_some(entry.hardware),
        pending: (usable && entry.waiting).then(|| (entry.hardware, HELD.to_vec())),
        poll_at: (entry.deadline != Instant::MAX).then_some(entry.deadline),
    }
}

/// What the model says an advertisement or a solicitation that carries
/// `hardware` does, and whether it changes anything.
///
/// This is `on_observed` as RFC 4861, section 7.2.5 II has it, with the
/// one departure the crate documents: a `Reachable` entry is never moved
/// to a different address by a claim this host did not ask for, whatever
/// the override bit says.
fn observe(reference: &mut Reference, hardware: MacAddr) -> bool {
    let reachable = reference.now.saturating_add(Timers::DEFAULT.reachable);
    match reference.entry.as_mut() {
        Some(entry) if entry.state == NeighborState::Reachable => {
            if entry.hardware != hardware {
                return false;
            }
            entry.deadline = reachable;
            true
        }
        Some(entry) => {
            entry.hardware = hardware;
            entry.state = NeighborState::Stale;
            entry.deadline = Instant::MAX;
            entry.solicits = 0;
            reference.reached |= reached::STALE;
            true
        }
        None => {
            reference.entry = Some(Believed {
                state: NeighborState::Stale,
                hardware,
                deadline: Instant::MAX,
                solicits: 0,
                waiting: false,
            });
            reference.reached |= reached::STALE;
            true
        }
    }
}

/// What a sequence reached.
///
/// A model test that stopped reaching `Probe` would go on passing and
/// say nothing, so the model records what it touched and the runner
/// refuses a run that missed one of these. The bits are set from the
/// model, which is the side that is written from the document.
mod reached {
    /// An entry was created with nothing known about it.
    pub(super) const INCOMPLETE: u32 = 1 << 0;
    /// A neighbor answered a solicitation of this host's.
    pub(super) const REACHABLE: u32 = 1 << 1;
    /// A confirmation ran out, or an unsolicited claim arrived.
    pub(super) const STALE: u32 = 1 << 2;
    /// A packet went to a stale neighbor.
    pub(super) const DELAY: u32 = 1 << 3;
    /// Nothing confirmed it within the delay.
    pub(super) const PROBE: u32 = 1 << 4;
    /// A solicitation went to the link, nothing being known.
    pub(super) const SOLICIT_GROUP: u32 = 1 << 5;
    /// A probe went to the address that answered last.
    pub(super) const SOLICIT_UNICAST: u32 = 1 << 6;
    /// Resolution gave up.
    pub(super) const UNREACHABLE: u32 = 1 << 7;
    /// A packet was handed an address to go to.
    pub(super) const DELIVERED: u32 = 1 << 8;
    /// A claim that disagreed took a reachable entry to stale.
    pub(super) const DEMOTED: u32 = 1 << 9;
    /// A packet waited behind an unresolved neighbor.
    pub(super) const HELD: u32 = 1 << 10;

    /// What every state and event is called, for the report.
    pub(super) const NAMES: [(u32, &str); 11] = [
        (INCOMPLETE, "Incomplete"),
        (REACHABLE, "Reachable"),
        (STALE, "Stale"),
        (DELAY, "Delay"),
        (PROBE, "Probe"),
        (SOLICIT_GROUP, "a solicitation to the group"),
        (SOLICIT_UNICAST, "a probe to the cached address"),
        (UNREACHABLE, "giving up"),
        (DELIVERED, "a packet delivered"),
        (DEMOTED, "a demotion by a claim that disagreed"),
        (HELD, "a packet held"),
    ];

    /// Everything the run has to reach.
    pub(super) fn required() -> &'static [&'static str] {
        &[
            "Incomplete",
            "Reachable",
            "Stale",
            "Delay",
            "Probe",
            "a solicitation to the group",
            "a probe to the cached address",
            "giving up",
            "a packet delivered",
            "a demotion by a claim that disagreed",
            "a packet held",
        ]
    }
}

/// A packet is handed to the cache for the neighbor.
fn apply_send(
    model: &DiscoveryModel,
    station: &mut Station,
    reference: &mut Reference,
) -> Result<(), String> {
    let timers = Timers::DEFAULT;
    let answered = station.cache.resolve(IpAddr::V6(PEER), HELD, station.now);
    let expected = match reference.entry.as_mut() {
        None => {
            reference.entry = Some(Believed {
                state: NeighborState::Incomplete,
                hardware: MacAddr::UNSPECIFIED,
                // Due at once, so the first solicitation comes out of the
                // caller's next poll.
                deadline: reference.now,
                solicits: 0,
                waiting: true,
            });
            reference.reached |= reached::INCOMPLETE | reached::HELD;
            Resolution::Waiting
        }
        Some(entry) => {
            if entry.state == NeighborState::Stale && !model.faulty {
                // RFC 4861, section 7.3.3: the first packet to a stale
                // neighbor starts the check.
                entry.state = NeighborState::Delay;
                entry.deadline = reference.now.saturating_add(timers.delay_first_probe);
                reference.reached |= reached::DELAY;
            }
            if entry.state.is_usable() {
                reference.reached |= reached::DELIVERED;
                Resolution::Deliver(entry.hardware)
            } else {
                entry.waiting = true;
                reference.reached |= reached::HELD;
                Resolution::Waiting
            }
        }
    };
    if answered == expected {
        return Ok(());
    }
    Err(format!("resolve answered {answered:?}, not {expected:?}"))
}

/// A neighbor advertisement arrives.
fn apply_advertisement(
    station: &mut Station,
    reference: &mut Reference,
    solicited: bool,
    overriding: bool,
    moved: bool,
) -> Result<(), String> {
    let timers = Timers::DEFAULT;
    let hardware = if moved { IMPOSTOR } else { PEER_HARDWARE };
    let bytes = advertisement(PEER, HOST, PEER, Some(hardware), solicited, overriding);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).map_err(|error| error.to_string())?;
    let changed = on_advertisement(&mut station.cache, &message, station.now);
    let known = believed(reference).hardware;
    let expected = if !overriding && known.is_some_and(|known| known != hardware) {
        // RFC 4861, section 7.2.5 I: the address is not taken, and a
        // reachable entry is demoted all the same.
        match reference.entry.as_mut() {
            Some(entry) if entry.state == NeighborState::Reachable => {
                entry.state = NeighborState::Stale;
                entry.deadline = Instant::MAX;
                reference.reached |= reached::DEMOTED | reached::STALE;
                true
            }
            _ => false,
        }
    } else if solicited {
        let deadline = reference.now.saturating_add(timers.reachable);
        match reference.entry.as_mut() {
            Some(entry) => {
                entry.hardware = hardware;
                entry.state = NeighborState::Reachable;
                entry.deadline = deadline;
                entry.solicits = 0;
            }
            None => {
                reference.entry = Some(Believed {
                    state: NeighborState::Reachable,
                    hardware,
                    deadline,
                    solicits: 0,
                    waiting: false,
                });
            }
        }
        reference.reached |= reached::REACHABLE;
        true
    } else {
        observe(reference, hardware)
    };
    if changed == expected {
        return Ok(());
    }
    Err(format!(
        "an advertisement reported {changed} and the model {expected}"
    ))
}

/// A neighbor solicitation from the neighbor arrives.
fn apply_solicitation(
    station: &mut Station,
    reference: &mut Reference,
    moved: bool,
) -> Result<(), String> {
    let hardware = if moved { IMPOSTOR } else { PEER_HARDWARE };
    let group = HOST.solicited_node();
    let bytes = solicitation(PEER, group, HOST, Some(hardware));
    let packet_bytes = discovery_packet(PEER, group, &bytes);
    let message = read(&packet_bytes).map_err(|error| error.to_string())?;
    let changed = on_solicitation(&mut station.cache, PEER, &message, station.now);
    let expected = observe(reference, hardware);
    if changed == expected {
        return Ok(());
    }
    Err(format!(
        "a solicitation reported {changed} and the model {expected}"
    ))
}

/// The cache is asked what it wants done.
fn apply_poll(station: &mut Station, reference: &mut Reference) -> Result<(), String> {
    let peer = IpAddr::V6(PEER);
    let timers = Timers::DEFAULT;
    let event = station.cache.poll(station.now);
    let now = reference.now;
    let expected = match reference.entry.as_mut() {
        Some(entry) if entry.deadline <= now => match entry.state {
            NeighborState::Reachable | NeighborState::Stale => {
                // A confirmation that ran out makes the entry stale, and a
                // stale entry then waits for traffic and not for a clock.
                entry.state = NeighborState::Stale;
                entry.deadline = Instant::MAX;
                reference.reached |= reached::STALE;
                None
            }
            NeighborState::Delay => {
                entry.state = NeighborState::Probe;
                entry.solicits = 1;
                entry.deadline = now.saturating_add(timers.retransmit);
                reference.reached |= reached::PROBE | reached::SOLICIT_UNICAST;
                Some(Event::Solicit {
                    address: peer,
                    hardware: Some(entry.hardware),
                })
            }
            NeighborState::Incomplete => {
                if entry.solicits >= timers.max_multicast_solicit {
                    reference.entry = None;
                    reference.reached |= reached::UNREACHABLE;
                    Some(Event::Unreachable { address: peer })
                } else {
                    entry.solicits = entry.solicits.saturating_add(1);
                    entry.deadline = now.saturating_add(timers.retransmit);
                    reference.reached |= reached::SOLICIT_GROUP;
                    Some(Event::Solicit {
                        address: peer,
                        hardware: None,
                    })
                }
            }
            NeighborState::Probe => {
                if entry.solicits >= timers.max_unicast_solicit {
                    reference.entry = None;
                    reference.reached |= reached::UNREACHABLE;
                    Some(Event::Unreachable { address: peer })
                } else {
                    entry.solicits = entry.solicits.saturating_add(1);
                    entry.deadline = now.saturating_add(timers.retransmit);
                    Some(Event::Solicit {
                        address: peer,
                        hardware: Some(entry.hardware),
                    })
                }
            }
        },
        Some(_) | None => None,
    };
    if event == expected {
        return Ok(());
    }
    Err(format!("poll answered {event:?}, not {expected:?}"))
}

/// The neighbor cache driven by Neighbor Discovery, against a model of
/// RFC 4861 written beside it.
struct DiscoveryModel {
    /// Whether the model is deliberately wrong, which is how the test
    /// below shows that a disagreement is found at all.
    faulty: bool,
}

impl ModelTest for DiscoveryModel {
    type Op = NdpOp;
    type Sut = Station;
    type Model = Reference;

    fn generator(&self) -> BoxGen<NdpOp> {
        any_ndp_op()
    }

    fn required(&self) -> &'static [&'static str] {
        reached::required()
    }

    fn reached(&self, model: &Reference) -> Vec<&'static str> {
        reached::NAMES
            .iter()
            .filter(|(bit, _)| model.reached & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    fn new_sut(&self) -> Station {
        Station {
            cache: Cache::new(),
            now: Instant::ZERO,
        }
    }

    fn new_model(&self) -> Reference {
        Reference {
            entry: None,
            now: Instant::ZERO,
            reached: 0,
        }
    }

    fn step(
        &self,
        station: &mut Station,
        reference: &mut Reference,
        op: &NdpOp,
    ) -> Result<(), String> {
        match *op {
            NdpOp::Send => apply_send(self, station, reference)?,
            NdpOp::Taken => {
                station.cache.clear_pending(IpAddr::V6(PEER));
                if let Some(entry) = reference.entry.as_mut() {
                    entry.waiting = false;
                }
            }
            NdpOp::Advertise {
                solicited,
                overriding,
                moved,
            } => apply_advertisement(station, reference, solicited, overriding, moved)?,
            NdpOp::Solicit { moved } => apply_solicitation(station, reference, moved)?,
            NdpOp::Wait(seconds) => {
                let step = Duration::from_secs(u64::from(seconds));
                station.now = station.now.saturating_add(step);
                reference.now = reference.now.saturating_add(step);
            }
            NdpOp::Poll => apply_poll(station, reference)?,
        }
        if station.now != reference.now {
            return Err("the two clocks parted".to_owned());
        }
        let (left, right) = (seen(station), believed(reference));
        if left != right {
            return Err(format!("the cache shows {left:?} and the model {right:?}"));
        }
        Ok(())
    }
}

#[test]
fn the_cache_matches_a_model_of_rfc_4861_under_discovery() {
    // One neighbor, because the cache holds its entries "in no particular
    // order" and therefore does not say which of several due neighbors
    // `poll` picks; a model that fixed that would be testing an accident.
    // Capacity, eviction, and several entries falling due at once are
    // `net-eth`'s own tests.
    // The runner refuses a run that never reached one of the states
    // `DiscoveryModel::required` names, so a test that stopped reaching
    // `Probe` fails rather than passing and saying nothing.
    run_model_test("ndp_neighbor_cache", &DiscoveryModel { faulty: false }, 48);
}

#[test]
fn a_model_that_disagrees_is_found_and_shrunk_to_a_short_sequence() {
    // The same run against a model with one transition missing: the
    // packet to a stale neighbor that starts the check of RFC 4861,
    // section 7.3.3. It stands for any regression in that transition, and
    // it is what says the test above would notice one.
    let failure = run_model_test_with(
        &Config::default(),
        "ndp_neighbor_cache_faulty",
        &DiscoveryModel { faulty: true },
        24,
    )
    .expect_err("a model with a missing transition disagrees");
    let ModelFailure::Disagreed(failure) = failure else {
        panic!("a disagreement is reported as one");
    };
    // Reaching `Stale` and then sending to it takes a handful of
    // operations, and the shrunk sequence is not much more than that.
    let operations = failure.shrunk.split(',').count();
    assert!(
        operations <= 8,
        "shrunk to {operations} operations: {}",
        failure.shrunk
    );
    assert!(
        failure.message.contains("resolve answered") || failure.message.contains("the cache shows"),
        "{}",
        failure.message
    );
}

#[test]
fn the_cache_runs_the_five_states_of_rfc_4861_under_discovery() {
    // The reference the model is checked against is the document's own
    // sequence: a neighbor is asked for, answers, ages out, is used
    // again, is delayed, is probed, and is given up on.
    let mut cache = Cache::new();
    let peer = IpAddr::V6(PEER);
    let timers = cache.timers();

    // Incomplete: asked for, nothing known.
    assert_eq!(cache.resolve(peer, b"first", secs(0)), Resolution::Waiting);
    assert_eq!(cache.state(peer), Some(NeighborState::Incomplete));
    assert_eq!(
        cache.poll(secs(0)),
        Some(Event::Solicit {
            address: peer,
            hardware: None,
        }),
        "the first solicitation goes to the solicited-node group"
    );

    // Reachable: the neighbor answered what this host asked.
    let bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    let message = read(&packet_bytes).expect("an advertisement");
    assert!(on_advertisement(&mut cache, &message, secs(1)));
    assert_eq!(cache.state(peer), Some(NeighborState::Reachable));

    // Stale: the reachable time ran out and nothing was said since.
    let stale_at = secs(1).saturating_add(timers.reachable);
    assert_eq!(cache.poll(stale_at), None);
    assert_eq!(cache.state(peer), Some(NeighborState::Stale));

    // Delay: a packet went to a stale neighbor.
    cache.clear_pending(peer);
    assert_eq!(
        cache.resolve(peer, b"second", stale_at),
        Resolution::Deliver(PEER_HARDWARE)
    );
    assert_eq!(cache.state(peer), Some(NeighborState::Delay));

    // Probe: nothing confirmed it within the delay.
    let probe_at = stale_at.saturating_add(timers.delay_first_probe);
    assert_eq!(
        cache.poll(probe_at),
        Some(Event::Solicit {
            address: peer,
            hardware: Some(PEER_HARDWARE),
        }),
        "a probe goes to the address that answered last"
    );
    assert_eq!(cache.state(peer), Some(NeighborState::Probe));

    // And given up on after the solicitations RFC 4861, section 10
    // allows.
    let mut now = probe_at;
    for _ in 1..timers.max_unicast_solicit {
        now = now.saturating_add(timers.retransmit);
        assert!(matches!(cache.poll(now), Some(Event::Solicit { .. })));
    }
    now = now.saturating_add(timers.retransmit);
    assert_eq!(cache.poll(now), Some(Event::Unreachable { address: peer }));
    assert_eq!(cache.state(peer), None);
}

#[test]
fn a_message_that_crossed_a_router_is_not_read_as_discovery() {
    let bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    // Everything is right but the hop limit, which is the one field a
    // router cannot leave alone.
    let packet_bytes = packet(PEER, HOST, Protocol::ICMPV6, 64, &bytes);
    assert_eq!(
        read(&packet_bytes),
        Err(Ipv6Error::NotDiscovery(NEIGHBOR_ADVERTISEMENT))
    );
}

#[test]
fn a_message_whose_checksum_does_not_verify_is_not_read() {
    let mut bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    bytes[8] ^= 0xFF;
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(read(&packet_bytes), Err(Ipv6Error::BadIcmp));
}

#[test]
fn a_code_that_is_not_zero_is_not_read() {
    let mut bytes = advertisement(PEER, HOST, PEER, Some(PEER_HARDWARE), true, true);
    bytes[1] = 1;
    checksummed(PEER, HOST, &mut bytes);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(
        read(&packet_bytes),
        Err(Ipv6Error::NotDiscovery(NEIGHBOR_ADVERTISEMENT))
    );
}

#[test]
fn a_type_that_is_not_one_of_the_four_is_not_read() {
    // 137 is a redirect, which this host does not read (see the module).
    let mut bytes = vec![137u8, 0, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(&PEER.octets());
    bytes.extend_from_slice(&HOST.octets());
    checksummed(PEER, HOST, &mut bytes);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(read(&packet_bytes), Err(Ipv6Error::NotDiscovery(137)));
}

#[test]
fn an_option_of_length_zero_makes_the_message_a_drop() {
    let mut bytes = solicitation(PEER, HOST, HOST, None);
    // RFC 4861, section 4.6: a receiver discards the packet.
    bytes.extend_from_slice(&[option_type::SOURCE_LINK_LAYER, 0, 0, 0, 0, 0, 0, 0]);
    checksummed(PEER, HOST, &mut bytes);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(
        read(&packet_bytes),
        Err(Ipv6Error::BadOption(option_type::SOURCE_LINK_LAYER))
    );
}

#[test]
fn an_option_that_runs_past_the_message_makes_it_a_drop() {
    let mut bytes = solicitation(PEER, HOST, HOST, None);
    bytes.extend_from_slice(&[option_type::SOURCE_LINK_LAYER, 4, 0, 0, 0, 0, 0, 0]);
    checksummed(PEER, HOST, &mut bytes);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(
        read(&packet_bytes),
        Err(Ipv6Error::BadOption(option_type::SOURCE_LINK_LAYER))
    );
}

#[test]
fn a_trailing_byte_that_cannot_be_an_option_makes_the_message_a_drop() {
    let mut bytes = solicitation(PEER, HOST, HOST, None);
    bytes.push(option_type::SOURCE_LINK_LAYER);
    checksummed(PEER, HOST, &mut bytes);
    let packet_bytes = discovery_packet(PEER, HOST, &bytes);
    assert_eq!(
        read(&packet_bytes),
        Err(Ipv6Error::BadOption(option_type::SOURCE_LINK_LAYER))
    );
}

#[test]
fn an_option_this_crate_does_not_read_is_carried_and_stepped_over() {
    let mut bytes = vec![option_type::SOURCE_LINK_LAYER, 1];
    bytes.extend_from_slice(&HARDWARE.octets());
    // Type 31 is the DNS search list of RFC 8106, which this system does
    // not read: a search list is a resolver policy and `net-dns` asks
    // for names as they are given.
    bytes.extend_from_slice(&[31, 1, 0, 0, 0, 0, 0, 0]);
    let options = Options::new(&bytes).expect("two well-formed options");
    assert_eq!(options.source_link_layer(), Some(HARDWARE));
    let mut walk = options;
    assert!(matches!(
        walk.next_option(),
        Some(NdpOption::SourceLinkLayer(_))
    ));
    assert_eq!(
        walk.next_option(),
        Some(NdpOption::Other {
            kind: 31,
            body: &[0, 0, 0, 0, 0, 0],
        })
    );
    assert_eq!(walk.next_option(), None);
}

#[test]
fn a_link_layer_option_whose_body_is_the_wrong_length_reads_as_unknown() {
    // Two units, so eight bytes of body where six make an address.
    let mut bytes = vec![option_type::TARGET_LINK_LAYER, 2];
    bytes.extend_from_slice(&[0; 14]);
    let options = Options::new(&bytes).expect("a well-formed length");
    // The length was right, so the walk is not lost; the body was not,
    // so nothing is learned from it.
    assert_eq!(options.target_link_layer(), None);
    assert!(!options.is_empty());
    assert_eq!(options.bytes().len(), 16);
}

#[test]
fn no_options_at_all_is_a_message_with_no_options() {
    let options = Options::new(&[]).expect("nothing is well formed");
    assert!(options.is_empty());
    assert_eq!(options.source_link_layer(), None);
    assert_eq!(options.target_link_layer(), None);
    assert_eq!(options.link_mtu(), None);
}

#[test]
fn a_router_solicitation_is_written_and_read_back() {
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    write_router_solicitation(&mut writer, HOST, Ipv6Addr::ALL_ROUTERS, Some(HARDWARE))
        .expect("room");
    let message = writer.written().to_vec();
    assert_eq!(message[0], ROUTER_SOLICITATION);
    let bytes = discovery_packet(HOST, Ipv6Addr::ALL_ROUTERS, &message);
    let Ok(Discovery::RouterSolicitation { options }) = read(&bytes) else {
        panic!("a router solicitation reads as one");
    };
    assert_eq!(options.source_link_layer(), Some(HARDWARE));
}

#[test]
fn an_advertisement_this_host_writes_is_one_it_reads() {
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    write_neighbor_advertisement(&mut writer, HOST, PEER, HOST, HARDWARE, true, true)
        .expect("room");
    let message = writer.written().to_vec();
    let bytes = discovery_packet(HOST, PEER, &message);
    let Ok(Discovery::NeighborAdvertisement {
        target,
        router,
        solicited,
        overriding,
        options,
    }) = read(&bytes)
    else {
        panic!("an advertisement reads as one");
    };
    assert_eq!(target, HOST);
    assert!(!router, "this host is not one");
    assert!(solicited);
    assert!(overriding);
    assert_eq!(options.target_link_layer(), Some(HARDWARE));
}

#[test]
fn a_solicitation_for_duplicate_address_detection_carries_no_address_option() {
    // RFC 4861, section 4.3 forbids the source link-layer option when the
    // source is unspecified: there is no address to answer it at.
    let mut buffer = [0u8; 64];
    let mut writer = Writer::new(&mut buffer);
    write_neighbor_solicitation(
        &mut writer,
        Ipv6Addr::UNSPECIFIED,
        HOST.solicited_node(),
        HOST,
        None,
    )
    .expect("room");
    let message = writer.written().to_vec();
    assert_eq!(message.len(), 24, "eight fixed bytes and the target");
    let bytes = discovery_packet(Ipv6Addr::UNSPECIFIED, HOST.solicited_node(), &message);
    let Ok(Discovery::NeighborSolicitation { target, options }) = read(&bytes) else {
        panic!("a probe reads as a solicitation");
    };
    assert_eq!(target, HOST);
    assert!(options.is_empty());
}

#[test]
fn a_buffer_too_small_for_a_message_is_refused() {
    let mut buffer = [0u8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        write_neighbor_solicitation(&mut writer, HOST, PEER, PEER, Some(HARDWARE)),
        Err(Ipv6Error::Wire(_))
    ));
    let mut buffer = [0u8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        write_neighbor_advertisement(&mut writer, HOST, PEER, HOST, HARDWARE, true, true),
        Err(Ipv6Error::Wire(_))
    ));
    let mut buffer = [0u8; 4];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(
        write_router_solicitation(&mut writer, HOST, PEER, None),
        Err(Ipv6Error::Wire(_))
    ));
}

#[test]
fn no_byte_stream_makes_the_discovery_parser_panic() {
    check("ndp_discovery_parse", &bytes(0..=96), |input| {
        let Ok(message) = Discovery::parse(input) else {
            return Ok(());
        };
        let mut options = message.options();
        let mut seen = 0usize;
        while let Some(option) = options.next_option() {
            seen += 1;
            if seen > input.len() {
                return Err("more options than there are bytes".into());
            }
            if let NdpOption::Rdnss(servers) = option
                && servers.len() != servers.iter().count()
            {
                return Err("a server count that does not match the addresses".into());
            }
        }
        Ok(())
    });
}
