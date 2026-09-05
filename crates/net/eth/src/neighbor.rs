// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The neighbor cache: which hardware address holds which internet
//! address, how sure of it this station is, and when to ask again.
//!
//! One cache serves both families (D-69). It is keyed by [`IpAddr`] and
//! holds the five states of RFC 4861, section 7.3.2; ARP fills its IPv4
//! half with the three of them it needs, and Neighbor Discovery fills the
//! other with all five. Neither protocol is in this module: the cache
//! answers *ask again for this address, now, at this hardware address or
//! at none*, and the caller writes whichever packet its family takes.
//!
//! Time is an argument. [`NeighborCache::poll_at`] says when there is
//! next work and [`NeighborCache::poll`] does it, so a test drives the
//! whole schedule of RFC 4861 in microseconds of wall clock.
//!
//! What this leaves out and why: RFC 4861, section 6.3.2 draws the
//! reachable time afresh from a random factor between one half and one
//! and a half, so that hosts which learned a neighbor together do not
//! re-probe together. This crate takes no randomness — it depends on
//! `audhsos-time` and `audhsos-collections` and on nothing that draws —
//! so the base time is used as it stands. What that costs is
//! synchronised probes among hosts that booted together; what it saves
//! is an `Rng` in the layer that is furthest from needing one.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use net_wire::{IpAddr, MacAddr};

/// How sure this station is of a mapping, per RFC 4861, section 7.3.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NeighborState {
    /// A request is out and no answer has come back. There is no hardware
    /// address yet.
    Incomplete,
    /// The neighbor answered within the reachable time and may be used.
    Reachable,
    /// The reachable time has passed. The address is still used; the next
    /// packet sent to it starts the check.
    Stale,
    /// A packet went to a stale neighbor, and the upper layer is being
    /// given `delay_first_probe` to confirm the neighbor itself.
    Delay,
    /// Nothing confirmed it, so solicitations are going out at the
    /// retransmit interval.
    Probe,
}

impl NeighborState {
    /// Whether an entry in this state has a hardware address to send to.
    #[must_use]
    pub const fn is_usable(self) -> bool {
        !matches!(self, NeighborState::Incomplete)
    }
}

/// The schedule of RFC 4861, section 10.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timers {
    /// How long a confirmation keeps an entry `Reachable`.
    pub reachable: Duration,
    /// How long between two solicitations.
    pub retransmit: Duration,
    /// How long a `Delay` entry waits for the upper layer to confirm the
    /// neighbor before it is probed.
    pub delay_first_probe: Duration,
    /// How many solicitations go to a neighbor that has never answered.
    pub max_multicast_solicit: u8,
    /// How many go to one that has answered before.
    pub max_unicast_solicit: u8,
}

impl Timers {
    /// The values RFC 4861, section 10 gives.
    pub const DEFAULT: Timers = Timers {
        reachable: Duration::from_secs(30),
        retransmit: Duration::from_secs(1),
        delay_first_probe: Duration::from_secs(5),
        max_multicast_solicit: 3,
        max_unicast_solicit: 3,
    };
}

impl Default for Timers {
    fn default() -> Timers {
        Timers::DEFAULT
    }
}

/// What the cache wants done, one thing at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// Ask for `address` again. `hardware` is `None` while nothing is
    /// known, which is the broadcast request of ARP or the solicited-node
    /// multicast of Neighbor Discovery, and `Some` for a probe, which
    /// goes to the address that answered last.
    Solicit {
        /// The address being asked for.
        address: IpAddr,
        /// Where to ask, or `None` to ask the link.
        hardware: Option<MacAddr>,
    },
    /// Resolution gave up. The entry is gone, and a packet waiting behind
    /// it went with it.
    Unreachable {
        /// The address nobody answered for.
        address: IpAddr,
    },
}

/// What the caller should do with a packet it wants to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Resolution {
    /// Send it to this hardware address.
    Deliver(MacAddr),
    /// The address is being resolved and the packet is held. It comes
    /// back from [`NeighborCache::pending`] when an answer arrives.
    Waiting,
    /// The packet is longer than a held packet may be, or the cache is
    /// full of entries that are all in use. Nothing was stored.
    Dropped,
}

/// One neighbor.
#[derive(Clone, Copy, Debug)]
struct Entry<const PENDING: usize> {
    /// The address this entry is about.
    address: IpAddr,
    /// The hardware address, meaningless while `Incomplete`.
    hardware: MacAddr,
    /// How sure this station is.
    state: NeighborState,
    /// When the current state runs out.
    deadline: Instant,
    /// When a packet was last sent to this neighbor, which is what
    /// decides who is evicted when the cache is full.
    used: Instant,
    /// How many solicitations have gone out in this state.
    solicits: u8,
    /// The packet waiting for the answer, and how much of the buffer it
    /// fills. At most one: a second packet for the same neighbor replaces
    /// the first, because the first is the older news.
    pending: [u8; PENDING],
    /// How many bytes of `pending` are a packet, zero for none.
    pending_len: usize,
}

/// The cache: `ENTRIES` neighbors, each able to hold one packet of at
/// most `PENDING` bytes while it is being resolved.
///
/// A new neighbor evicts the least recently used one when the cache is
/// full, and a packet waiting behind the evicted entry goes with it
/// without an event. That is deliberate: the caller learns of it the way
/// it learns of any other loss on a link, by the upper layer not being
/// acknowledged, and a cache sized for the neighbors a host actually
/// talks to does not reach the case.
#[derive(Debug)]
pub struct NeighborCache<const ENTRIES: usize, const PENDING: usize> {
    /// The neighbors, in no particular order.
    entries: ArrayVec<Entry<PENDING>, ENTRIES>,
    /// The schedule.
    timers: Timers,
}

impl<const ENTRIES: usize, const PENDING: usize> Default for NeighborCache<ENTRIES, PENDING> {
    fn default() -> NeighborCache<ENTRIES, PENDING> {
        NeighborCache::new()
    }
}

impl<const ENTRIES: usize, const PENDING: usize> NeighborCache<ENTRIES, PENDING> {
    /// An empty cache on the schedule of RFC 4861.
    #[must_use]
    pub const fn new() -> NeighborCache<ENTRIES, PENDING> {
        NeighborCache {
            entries: ArrayVec::new(),
            timers: Timers::DEFAULT,
        }
    }

    /// An empty cache on a schedule of the caller's own.
    #[must_use]
    pub const fn with_timers(timers: Timers) -> NeighborCache<ENTRIES, PENDING> {
        NeighborCache {
            entries: ArrayVec::new(),
            timers,
        }
    }

    /// The schedule this cache runs on.
    #[must_use]
    pub const fn timers(&self) -> Timers {
        self.timers
    }

    /// How many neighbors are known, in any state.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is known.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The state of `address`, or `None` when it is not in the cache.
    #[must_use]
    pub fn state(&self, address: IpAddr) -> Option<NeighborState> {
        self.find(address).map(|entry| entry.state)
    }

    /// The hardware address known for `address`, if one is.
    #[must_use]
    pub fn hardware(&self, address: IpAddr) -> Option<MacAddr> {
        self.find(address)
            .filter(|entry| entry.state.is_usable())
            .map(|entry| entry.hardware)
    }

    /// Where to send `packet`, and what to do if the answer is nowhere
    /// yet.
    ///
    /// A packet for a neighbor that is already being resolved replaces
    /// the one waiting there rather than queueing behind it: the older
    /// packet is the one a retransmission will produce again, and the
    /// newer is the one the caller has just decided to send.
    pub fn resolve(&mut self, address: IpAddr, packet: &[u8], now: Instant) -> Resolution {
        let delay_deadline = now.saturating_add(self.timers.delay_first_probe);
        if let Some(entry) = self.find_mut(address) {
            entry.used = now;
            if entry.state == NeighborState::Stale {
                // RFC 4861, section 7.3.3: the first packet to a stale
                // neighbor starts the check, and the upper layer gets
                // `delay_first_probe` to confirm it before a probe goes
                // out.
                entry.state = NeighborState::Delay;
                entry.deadline = delay_deadline;
            }
            if entry.state.is_usable() {
                return Resolution::Deliver(entry.hardware);
            }
            return if entry.store(packet) {
                Resolution::Waiting
            } else {
                Resolution::Dropped
            };
        }
        let mut entry = Entry {
            address,
            hardware: MacAddr::UNSPECIFIED,
            state: NeighborState::Incomplete,
            // Due at once, so the first solicitation comes out of the
            // caller's next `poll` and not out of a second entry point.
            deadline: now,
            used: now,
            solicits: 0,
            pending: [0; PENDING],
            pending_len: 0,
        };
        if !entry.store(packet) {
            return Resolution::Dropped;
        }
        if self.insert(entry).is_none() {
            return Resolution::Dropped;
        }
        Resolution::Waiting
    }

    /// A neighbor answered a solicitation: it holds `hardware` and is
    /// reachable.
    ///
    /// A packet waiting behind it is now sendable; the caller reads it
    /// with [`pending`](Self::pending) and drops it with
    /// [`clear_pending`](Self::clear_pending) once it is on the wire.
    pub fn on_confirmed(&mut self, address: IpAddr, hardware: MacAddr, now: Instant) {
        let deadline = now.saturating_add(self.timers.reachable);
        if let Some(entry) = self.find_mut(address) {
            entry.hardware = hardware;
            entry.state = NeighborState::Reachable;
            entry.deadline = deadline;
            entry.solicits = 0;
            return;
        }
        let entry = Entry {
            address,
            hardware,
            state: NeighborState::Reachable,
            deadline,
            used: now,
            solicits: 0,
            pending: [0; PENDING],
            pending_len: 0,
        };
        self.insert(entry);
    }

    /// A mapping seen without having asked for it: a gratuitous ARP, or
    /// the sender's own address on a request meant for someone else.
    ///
    /// It creates an entry that does not exist, refreshes a `Reachable`
    /// one that agrees, and takes the new address into any other state,
    /// leaving the entry `Stale`. What it never does is move a
    /// `Reachable` entry to a different hardware address: that station
    /// answered a solicitation of this host's, which is evidence, and an
    /// unsolicited claim is not.
    ///
    /// That one comparison is the cheap half of resistance to ARP
    /// spoofing, and it is worth being plain about how thin the
    /// protection is. An entry that is not `Reachable` is taken over by
    /// whoever claims it last, and the expensive half — knowing which
    /// station is entitled to an address — is not something a link layer
    /// can know. What the comparison buys is that a neighbor this host is
    /// actively talking to cannot be stolen mid-conversation.
    ///
    /// The answer says whether the observation changed anything.
    pub fn on_observed(&mut self, address: IpAddr, hardware: MacAddr, now: Instant) -> bool {
        let reachable_deadline = now.saturating_add(self.timers.reachable);
        if let Some(entry) = self.find_mut(address) {
            if entry.state == NeighborState::Reachable {
                if entry.hardware != hardware {
                    return false;
                }
                entry.deadline = reachable_deadline;
                return true;
            }
            entry.hardware = hardware;
            entry.state = NeighborState::Stale;
            entry.deadline = Instant::MAX;
            entry.solicits = 0;
            return true;
        }
        let entry = Entry {
            address,
            hardware,
            state: NeighborState::Stale,
            // A stale entry waits for traffic and not for a clock.
            deadline: Instant::MAX,
            used: now,
            solicits: 0,
            pending: [0; PENDING],
            pending_len: 0,
        };
        self.insert(entry).is_some()
    }

    /// A claim that disagrees with what this entry holds, from a station
    /// this host has a reason to listen to.
    ///
    /// RFC 4861, section 7.2.5 I (a): a neighbor advertisement without
    /// the override bit whose link-layer address differs from the cached
    /// one takes a `Reachable` entry to `Stale` and changes nothing else;
    /// an entry in any other state ignores it altogether.
    ///
    /// The address is not taken — that is what the override bit would
    /// have said — but the disagreement is not nothing either: this host
    /// has just heard something that contradicts what it believes, and a
    /// `Stale` entry is checked before the next packet is trusted to it.
    /// The difference from [`on_observed`](Self::on_observed) is exactly
    /// that: this changes the state and never the address, that one
    /// changes the address of an entry that is not `Reachable`.
    ///
    /// The answer says whether the entry moved.
    pub fn on_conflict(&mut self, address: IpAddr) -> bool {
        let Some(entry) = self.find_mut(address) else {
            return false;
        };
        if entry.state != NeighborState::Reachable {
            return false;
        }
        entry.state = NeighborState::Stale;
        // A stale entry waits for traffic and not for a clock.
        entry.deadline = Instant::MAX;
        true
    }

    /// The packet waiting for `address`, if one is.
    #[must_use]
    pub fn pending(&self, address: IpAddr) -> Option<(MacAddr, &[u8])> {
        let entry = self.find(address)?;
        if !entry.state.is_usable() {
            return None;
        }
        let packet = entry.pending.get(..entry.pending_len)?;
        if packet.is_empty() {
            return None;
        }
        Some((entry.hardware, packet))
    }

    /// Forgets the packet waiting for `address`, which a caller does once
    /// it has sent it.
    pub fn clear_pending(&mut self, address: IpAddr) {
        if let Some(entry) = self.find_mut(address) {
            entry.pending_len = 0;
        }
    }

    /// When this cache next has work, or `None` when it has none.
    #[must_use]
    pub fn poll_at(&self) -> Option<Instant> {
        self.entries
            .iter()
            .map(|entry| entry.deadline)
            .filter(|deadline| *deadline != Instant::MAX)
            .min()
    }

    /// The next thing to do at `now`, or `None` when there is nothing.
    ///
    /// A caller drives this in a loop until it answers `None`, because
    /// one instant can fall due for several neighbors.
    pub fn poll(&mut self, now: Instant) -> Option<Event> {
        let timers = self.timers;
        let mut expired = None;
        for (index, entry) in self.entries.iter_mut().enumerate() {
            if entry.deadline > now {
                continue;
            }
            match entry.state {
                NeighborState::Reachable | NeighborState::Stale => {
                    // A confirmation that has run out makes the entry
                    // stale, and a stale entry then waits for traffic and
                    // not for a clock — which is why nothing is asked
                    // here and why this is the cheapest of the five
                    // states. The second half of this arm is what a stale
                    // entry would do if it were ever due, and it never
                    // is: its deadline is `Instant::MAX`.
                    entry.state = NeighborState::Stale;
                    entry.deadline = Instant::MAX;
                }
                NeighborState::Delay => {
                    entry.state = NeighborState::Probe;
                    entry.solicits = 1;
                    entry.deadline = now.saturating_add(timers.retransmit);
                    return Some(Event::Solicit {
                        address: entry.address,
                        hardware: Some(entry.hardware),
                    });
                }
                NeighborState::Incomplete => {
                    if entry.solicits >= timers.max_multicast_solicit {
                        expired = Some((index, entry.address));
                        break;
                    }
                    entry.solicits = entry.solicits.saturating_add(1);
                    entry.deadline = now.saturating_add(timers.retransmit);
                    return Some(Event::Solicit {
                        address: entry.address,
                        hardware: None,
                    });
                }
                NeighborState::Probe => {
                    if entry.solicits >= timers.max_unicast_solicit {
                        expired = Some((index, entry.address));
                        break;
                    }
                    entry.solicits = entry.solicits.saturating_add(1);
                    entry.deadline = now.saturating_add(timers.retransmit);
                    return Some(Event::Solicit {
                        address: entry.address,
                        hardware: Some(entry.hardware),
                    });
                }
            }
        }
        let (index, address) = expired?;
        self.entries.remove(index);
        Some(Event::Unreachable { address })
    }

    /// The entry for `address`.
    fn find(&self, address: IpAddr) -> Option<&Entry<PENDING>> {
        self.entries.iter().find(|entry| entry.address == address)
    }

    /// The entry for `address`, to be changed.
    fn find_mut(&mut self, address: IpAddr) -> Option<&mut Entry<PENDING>> {
        self.entries
            .iter_mut()
            .find(|entry| entry.address == address)
    }

    /// Puts `entry` in, evicting the least recently used neighbor when
    /// there is no room. Answers `None` only when the cache holds no
    /// entries at all, which is a cache of capacity zero.
    fn insert(&mut self, entry: Entry<PENDING>) -> Option<()> {
        if self.entries.push(entry).is_ok() {
            return Some(());
        }
        let (index, _) = self
            .entries
            .iter()
            .enumerate()
            .min_by_key(|(_, existing)| existing.used)?;
        self.entries.remove(index);
        self.entries.push(entry).ok()
    }
}

impl<const PENDING: usize> Entry<PENDING> {
    /// Holds `packet`, replacing whatever was held. Answers whether it
    /// fits; a packet that does not is dropped and nothing is kept, since
    /// half a packet is worse than none.
    fn store(&mut self, packet: &[u8]) -> bool {
        let Some(room) = self.pending.get_mut(..packet.len()) else {
            self.pending_len = 0;
            return false;
        };
        room.copy_from_slice(packet);
        self.pending_len = packet.len();
        true
    }
}
