// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The neighbor cache: the five states of RFC 4861, the schedule of its
//! section 10, and what happens when nobody answers.
//!
//! Every instant here is an argument, so the thirty seconds of
//! `REACHABLE_TIME` pass in no time at all.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "a test computes a length or an instant in the open"
)]

use audhsos_time::{Duration, Instant};
use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, MacAddr};

use crate::neighbor::{Event, NeighborCache, NeighborState, Resolution, Timers};

/// A cache of three neighbors, each able to hold a packet of 64 bytes.
type Cache = NeighborCache<3, 64>;

/// The neighbor most tests ask about.
const NEIGHBOR: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

/// Its hardware address.
const HARDWARE: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB7]);

/// Somebody else's.
const IMPOSTOR: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xFF]);

/// A second and a third neighbor, for the tests about capacity.
const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2));
const THIRD: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 3));
const FOURTH: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 4));

/// `micros` microseconds after the origin.
fn at(micros: u64) -> Instant {
    Instant::from_micros(micros)
}

/// `seconds` seconds after the origin.
fn secs(seconds: u64) -> Instant {
    Instant::ZERO.saturating_add(Duration::from_secs(seconds))
}

#[test]
fn an_unknown_neighbor_is_asked_for_and_the_packet_waits() {
    let mut cache = Cache::new();
    assert!(cache.is_empty());
    assert_eq!(cache.poll_at(), None);

    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", at(0)),
        Resolution::Waiting
    );
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Incomplete));
    assert_eq!(cache.hardware(NEIGHBOR), None);
    // Nothing is sendable while nobody has answered.
    assert_eq!(cache.pending(NEIGHBOR), None);
    assert_eq!(cache.poll_at(), Some(at(0)));

    assert_eq!(
        cache.poll(at(0)),
        Some(Event::Solicit {
            address: NEIGHBOR,
            hardware: None
        })
    );
    assert_eq!(cache.poll(at(0)), None);
}

#[test]
fn an_answer_makes_the_entry_reachable_and_releases_the_packet() {
    let mut cache = Cache::new();
    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", at(0)),
        Resolution::Waiting
    );
    assert!(cache.poll(at(0)).is_some());

    cache.on_confirmed(NEIGHBOR, HARDWARE, at(10));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    assert_eq!(cache.pending(NEIGHBOR), Some((HARDWARE, &b"a packet"[..])));

    cache.clear_pending(NEIGHBOR);
    assert_eq!(cache.pending(NEIGHBOR), None);

    // The next packet goes straight out.
    assert_eq!(
        cache.resolve(NEIGHBOR, b"another", at(20)),
        Resolution::Deliver(HARDWARE)
    );
}

#[test]
fn a_confirmation_for_an_unknown_neighbor_creates_a_reachable_entry() {
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(0));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    assert_eq!(cache.pending(NEIGHBOR), None);
}

#[test]
fn a_reachable_entry_goes_stale_at_the_age_boundary() {
    let timers = Timers::DEFAULT;
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, Instant::ZERO);
    let boundary = Instant::ZERO.saturating_add(timers.reachable);
    assert_eq!(cache.poll_at(), Some(boundary));

    // One microsecond before the boundary nothing happens.
    assert_eq!(cache.poll(at(boundary.as_micros() - 1)), None);
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));

    // At it, the entry goes stale — with no event, because a stale entry
    // asks nothing until a packet is sent to it.
    assert_eq!(cache.poll(boundary), None);
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    assert_eq!(cache.poll_at(), None);
}

#[test]
fn a_packet_to_a_stale_neighbor_starts_the_check_and_still_goes_out() {
    let timers = Timers::DEFAULT;
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, Instant::ZERO);
    let stale_at = Instant::ZERO.saturating_add(timers.reachable);
    assert_eq!(cache.poll(stale_at), None);

    // RFC 4861, section 7.3.3: the packet is sent, and the entry moves to
    // Delay with the first probe due `delay_first_probe` later.
    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", stale_at),
        Resolution::Deliver(HARDWARE)
    );
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Delay));
    let probe_at = stale_at.saturating_add(timers.delay_first_probe);
    assert_eq!(cache.poll_at(), Some(probe_at));

    // Nothing before the deadline.
    assert_eq!(cache.poll(at(probe_at.as_micros() - 1)), None);
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Delay));

    // Then a probe, to the address that answered last.
    assert_eq!(
        cache.poll(probe_at),
        Some(Event::Solicit {
            address: NEIGHBOR,
            hardware: Some(HARDWARE)
        })
    );
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Probe));

    // An answer puts it back.
    cache.on_confirmed(NEIGHBOR, HARDWARE, probe_at);
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));
}

#[test]
fn an_unanswered_request_is_repeated_and_then_given_up() {
    let timers = Timers::DEFAULT;
    let mut cache = Cache::new();
    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", Instant::ZERO),
        Resolution::Waiting
    );

    // Three solicitations at the retransmit interval, and not one more.
    for count in 0..u64::from(timers.max_multicast_solicit) {
        let due = Instant::ZERO.saturating_add(Duration::from_secs(count));
        assert_eq!(cache.poll_at(), Some(due), "solicitation {count}");
        assert_eq!(
            cache.poll(due),
            Some(Event::Solicit {
                address: NEIGHBOR,
                hardware: None
            }),
            "solicitation {count}"
        );
        assert_eq!(cache.poll(due), None, "solicitation {count}");
    }

    let give_up = secs(u64::from(timers.max_multicast_solicit));
    assert_eq!(cache.poll_at(), Some(give_up));
    assert_eq!(
        cache.poll(give_up),
        Some(Event::Unreachable { address: NEIGHBOR })
    );
    // The entry and the packet behind it are gone.
    assert_eq!(cache.state(NEIGHBOR), None);
    assert_eq!(cache.pending(NEIGHBOR), None);
    assert!(cache.is_empty());
    assert_eq!(cache.poll_at(), None);
}

#[test]
fn an_unanswered_probe_is_repeated_and_then_given_up() {
    let timers = Timers::DEFAULT;
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, Instant::ZERO);
    let stale_at = Instant::ZERO.saturating_add(timers.reachable);
    assert_eq!(cache.poll(stale_at), None);
    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", stale_at),
        Resolution::Deliver(HARDWARE)
    );
    let mut due = stale_at.saturating_add(timers.delay_first_probe);
    for count in 0..u64::from(timers.max_unicast_solicit) {
        assert_eq!(
            cache.poll(due),
            Some(Event::Solicit {
                address: NEIGHBOR,
                hardware: Some(HARDWARE)
            }),
            "probe {count}"
        );
        due = due.saturating_add(timers.retransmit);
    }
    assert_eq!(
        cache.poll(due),
        Some(Event::Unreachable { address: NEIGHBOR })
    );
    assert_eq!(cache.state(NEIGHBOR), None);
}

#[test]
fn a_second_packet_replaces_the_one_waiting_rather_than_queueing_behind_it() {
    let mut cache = Cache::new();
    assert_eq!(
        cache.resolve(NEIGHBOR, b"first", at(0)),
        Resolution::Waiting
    );
    assert_eq!(
        cache.resolve(NEIGHBOR, b"second", at(1)),
        Resolution::Waiting
    );
    assert_eq!(cache.len(), 1);

    cache.on_confirmed(NEIGHBOR, HARDWARE, at(2));
    assert_eq!(cache.pending(NEIGHBOR), Some((HARDWARE, &b"second"[..])));
}

#[test]
fn a_packet_longer_than_the_cache_holds_is_dropped_and_nothing_is_kept() {
    let mut cache = Cache::new();
    let long = [0u8; 65];
    assert_eq!(cache.resolve(NEIGHBOR, &long, at(0)), Resolution::Dropped);
    assert!(cache.is_empty());

    // And on an entry that already exists, the packet waiting there is
    // dropped too: half a packet is worse than none.
    assert_eq!(
        cache.resolve(NEIGHBOR, b"short", at(1)),
        Resolution::Waiting
    );
    assert_eq!(cache.resolve(NEIGHBOR, &long, at(2)), Resolution::Dropped);
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(3));
    assert_eq!(cache.pending(NEIGHBOR), None);
}

#[test]
fn the_entry_evicted_at_capacity_is_the_least_recently_used() {
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(10));
    cache.on_confirmed(SECOND, HARDWARE, at(20));
    cache.on_confirmed(THIRD, HARDWARE, at(30));
    assert_eq!(cache.len(), 3);

    // Touching the oldest makes the second one the oldest instead.
    assert_eq!(
        cache.resolve(NEIGHBOR, b"x", at(40)),
        Resolution::Deliver(HARDWARE)
    );

    cache.on_confirmed(FOURTH, HARDWARE, at(50));
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.state(SECOND), None);
    assert!(cache.state(NEIGHBOR).is_some());
    assert!(cache.state(THIRD).is_some());
    assert!(cache.state(FOURTH).is_some());
}

#[test]
fn a_gratuitous_claim_refreshes_a_reachable_entry_that_agrees() {
    let timers = Timers::DEFAULT;
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, Instant::ZERO);
    let half = Instant::ZERO.saturating_add(Duration::from_secs(15));

    assert!(cache.on_observed(NEIGHBOR, HARDWARE, half));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));
    // The clock started again, so the entry outlives its first deadline.
    assert_eq!(cache.poll_at(), Some(half.saturating_add(timers.reachable)));
}

#[test]
fn a_gratuitous_claim_never_takes_a_reachable_entry_from_its_owner() {
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(0));
    assert!(!cache.on_observed(NEIGHBOR, IMPOSTOR, at(1)));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));
}

#[test]
fn an_observation_fills_an_entry_nobody_has_answered_for() {
    let mut cache = Cache::new();
    assert_eq!(
        cache.resolve(NEIGHBOR, b"a packet", at(0)),
        Resolution::Waiting
    );
    assert!(cache.on_observed(NEIGHBOR, HARDWARE, at(1)));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));
    // Stale is usable, so the packet that was waiting may go.
    assert_eq!(cache.pending(NEIGHBOR), Some((HARDWARE, &b"a packet"[..])));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
}

#[test]
fn an_observation_of_a_neighbor_nobody_asked_about_is_kept_as_stale() {
    let mut cache = Cache::new();
    assert!(cache.on_observed(NEIGHBOR, HARDWARE, at(0)));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));
    // A stale entry has no deadline of its own.
    assert_eq!(cache.poll_at(), None);

    // A station that is not reachable is taken over by whoever claims it
    // last, which is the limit of what this layer can tell.
    assert!(cache.on_observed(NEIGHBOR, IMPOSTOR, at(1)));
    assert_eq!(cache.hardware(NEIGHBOR), Some(IMPOSTOR));
}

#[test]
fn one_cache_holds_neighbors_of_both_families() {
    let six = IpAddr::V6(Ipv6Addr::new([0xFE80, 0, 0, 0, 0, 0, 0, 1]));
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(0));
    cache.on_confirmed(six, IMPOSTOR, at(1));
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    assert_eq!(cache.hardware(six), Some(IMPOSTOR));

    // A solicitation names the address, and the caller reads the family
    // off it to decide which packet to write.
    assert_eq!(
        cache.resolve(six, b"x", at(2)),
        Resolution::Deliver(IMPOSTOR)
    );
}

#[test]
fn a_schedule_of_the_callers_own_is_used_as_given() {
    let timers = Timers {
        reachable: Duration::from_millis(100),
        retransmit: Duration::from_millis(10),
        delay_first_probe: Duration::from_millis(50),
        max_multicast_solicit: 1,
        max_unicast_solicit: 1,
    };
    let mut cache = NeighborCache::<2, 16>::with_timers(timers);
    assert_eq!(cache.timers(), timers);
    assert_eq!(
        cache.resolve(NEIGHBOR, b"x", Instant::ZERO),
        Resolution::Waiting
    );
    assert!(cache.poll(Instant::ZERO).is_some());
    let give_up = Instant::ZERO.saturating_add(timers.retransmit);
    assert_eq!(
        cache.poll(give_up),
        Some(Event::Unreachable { address: NEIGHBOR })
    );
}

#[test]
fn the_states_say_which_of_them_can_be_sent_to() {
    assert!(!NeighborState::Incomplete.is_usable());
    for state in [
        NeighborState::Reachable,
        NeighborState::Stale,
        NeighborState::Delay,
        NeighborState::Probe,
    ] {
        assert!(state.is_usable(), "{state:?}");
    }
    assert_eq!(Timers::default(), Timers::DEFAULT);
    assert_eq!(Cache::default().len(), 0);
}

#[test]
fn a_cache_that_holds_nothing_drops_what_it_is_given() {
    let mut cache = NeighborCache::<0, 16>::new();
    assert_eq!(cache.resolve(NEIGHBOR, b"x", at(0)), Resolution::Dropped);
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(1));
    assert_eq!(cache.state(NEIGHBOR), None);
    assert!(!cache.on_observed(NEIGHBOR, HARDWARE, at(2)));
    assert_eq!(cache.poll(at(3)), None);
    assert_eq!(cache.poll_at(), None);
    cache.clear_pending(NEIGHBOR);
}

#[test]
fn one_instant_can_fall_due_for_several_neighbors() {
    let mut cache = Cache::new();
    assert_eq!(cache.resolve(NEIGHBOR, b"a", at(0)), Resolution::Waiting);
    assert_eq!(cache.resolve(SECOND, b"b", at(0)), Resolution::Waiting);
    let mut asked = Vec::new();
    while let Some(event) = cache.poll(at(0)) {
        asked.push(event);
    }
    assert_eq!(
        asked,
        [
            Event::Solicit {
                address: NEIGHBOR,
                hardware: None
            },
            Event::Solicit {
                address: SECOND,
                hardware: None
            }
        ]
    );
}

#[test]
fn a_claim_that_disagrees_takes_a_reachable_entry_to_stale_and_no_further() {
    // RFC 4861, section 7.2.5 I (a). The address is not touched — that is
    // what the override bit would have said — but the entry stops being
    // trusted for the rest of the reachable time, so the next packet to
    // it starts the check.
    let mut cache = Cache::new();
    cache.on_confirmed(NEIGHBOR, HARDWARE, at(0));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Reachable));

    assert!(cache.on_conflict(NEIGHBOR));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));
    assert_eq!(cache.hardware(NEIGHBOR), Some(HARDWARE));
    // A stale entry waits for traffic and not for a clock.
    assert_eq!(cache.poll_at(), None);
}

#[test]
fn a_claim_that_disagrees_moves_nothing_else() {
    // Section 7.2.5 I (b): an entry in any other state ignores it, and so
    // does an address the cache has never heard of.
    let mut cache = Cache::new();
    assert!(!cache.on_conflict(NEIGHBOR));
    assert!(cache.is_empty());

    cache.on_observed(NEIGHBOR, HARDWARE, at(0));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));
    assert!(!cache.on_conflict(NEIGHBOR));
    assert_eq!(cache.state(NEIGHBOR), Some(NeighborState::Stale));

    assert_eq!(
        cache.resolve(SECOND, b"waiting", at(0)),
        Resolution::Waiting
    );
    assert_eq!(cache.state(SECOND), Some(NeighborState::Incomplete));
    assert!(!cache.on_conflict(SECOND));
    assert_eq!(cache.state(SECOND), Some(NeighborState::Incomplete));
}
