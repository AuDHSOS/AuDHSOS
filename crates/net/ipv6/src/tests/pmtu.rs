// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Path MTU discovery against the three rules of RFC 8201, section 4: it
//! is lowered by a message, never raised by one, and never taken below
//! the minimum MTU of the format.

#![allow(
    clippy::arithmetic_side_effects,
    reason = "a test computes an instant in the open"
)]

use audhsos_time::Duration;

use super::{HOST, PEER, secs};
use crate::header::MIN_MTU;
use crate::pmtu::{INCREASE_INTERVAL, PathMtu};

/// Estimates for two paths.
type Paths = PathMtu<2>;

/// What the link carries in these tests.
const LINK: usize = 1500;

#[test]
fn a_path_nobody_reported_on_carries_what_the_link_does() {
    let paths = Paths::new();
    assert!(paths.is_empty());
    assert_eq!(paths.len(), 0);
    assert_eq!(paths.mtu(PEER, LINK), LINK);
    assert_eq!(paths.poll_at(), None);
}

#[test]
fn a_packet_too_big_lowers_the_estimate() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert_eq!(paths.mtu(PEER, LINK), 1400);
    assert_eq!(paths.len(), 1);
    // And only for the path it named.
    assert_eq!(paths.mtu(HOST, LINK), LINK);
}

#[test]
fn a_second_message_lowers_it_further() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert!(paths.on_packet_too_big(PEER, 1300, secs(1)));
    assert_eq!(paths.mtu(PEER, LINK), 1300);
    assert_eq!(paths.len(), 1, "one path, not two");
}

#[test]
fn the_estimate_is_never_raised_by_a_message() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1300, secs(0)));
    // A message claiming more is a stale packet, a forgery, or a second
    // path; none of the three is a reason to send larger packets.
    assert!(!paths.on_packet_too_big(PEER, 1400, secs(1)));
    assert!(!paths.on_packet_too_big(PEER, 1300, secs(2)));
    assert_eq!(paths.mtu(PEER, LINK), 1300);
}

#[test]
fn a_message_below_the_minimum_is_discarded() {
    let mut paths = Paths::new();
    assert!(!paths.on_packet_too_big(PEER, 1279, secs(0)));
    assert!(!paths.on_packet_too_big(PEER, 0, secs(0)));
    assert!(paths.is_empty());
    assert_eq!(paths.mtu(PEER, LINK), LINK);

    // Exactly the minimum is not below it.
    assert!(paths.on_packet_too_big(PEER, 1280, secs(0)));
    assert_eq!(paths.mtu(PEER, LINK), MIN_MTU);
}

#[test]
fn no_estimate_is_ever_below_the_minimum() {
    let mut paths = Paths::new();
    // The floor is kept where the message arrives, not where the answer
    // is given: nothing below the minimum ever enters the table.
    assert!(!paths.on_packet_too_big(PEER, 1279, secs(0)));
    assert!(paths.on_packet_too_big(PEER, 1280, secs(0)));
    assert_eq!(paths.mtu(PEER, LINK), MIN_MTU);
}

#[test]
fn a_link_narrower_than_the_minimum_is_reported_as_it_stands() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1280, secs(0)));
    // Such a link cannot carry IPv6, and the send path answers
    // `WouldFragment` for it. Raising the answer to 1280 here would turn
    // that into frames the driver could not send.
    assert_eq!(paths.mtu(PEER, 576), 576);
    assert_eq!(Paths::new().mtu(PEER, 576), 576);
}

#[test]
fn the_link_mtu_still_bounds_the_answer() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 9000, secs(0)));
    assert_eq!(paths.mtu(PEER, LINK), LINK, "the link is the narrower one");
}

#[test]
fn an_estimate_is_forgotten_after_the_interval_so_an_increase_is_found() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert_eq!(
        paths.poll_at(),
        Some(secs(0).saturating_add(INCREASE_INTERVAL))
    );
    assert_eq!(paths.poll(secs(599)), 0);
    assert_eq!(paths.mtu(PEER, LINK), 1400);

    assert_eq!(paths.poll(secs(600)), 1);
    assert!(paths.is_empty());
    // The next packet goes out at the link MTU, which is the only way to
    // find out that the path grew.
    assert_eq!(paths.mtu(PEER, LINK), LINK);
    assert_eq!(paths.poll_at(), None);
}

#[test]
fn a_schedule_of_the_callers_own_is_used_as_given() {
    let mut paths = PathMtu::<2>::with_interval(Duration::from_secs(60));
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert_eq!(paths.poll_at(), Some(secs(60)));
    assert_eq!(paths.poll(secs(60)), 1);
}

#[test]
fn the_estimate_evicted_at_capacity_is_the_oldest() {
    let mut paths = Paths::new();
    let third = crate::tests::HOST;
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert!(paths.on_packet_too_big(third, 1300, secs(10)));
    assert_eq!(paths.len(), 2);

    let fourth = net_wire::Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x04,
    ]);
    assert!(paths.on_packet_too_big(fourth, 1350, secs(20)));
    assert_eq!(paths.len(), 2);
    // The oldest went, and it is the one closest to being forgotten
    // anyway.
    assert_eq!(paths.mtu(PEER, LINK), LINK);
    assert_eq!(paths.mtu(third, LINK), 1300);
    assert_eq!(paths.mtu(fourth, LINK), 1350);
}

#[test]
fn a_table_of_no_paths_reports_nothing_and_holds_nothing() {
    let mut paths = PathMtu::<0>::new();
    assert!(!paths.on_packet_too_big(PEER, 1400, secs(0)));
    assert!(paths.is_empty());
    assert_eq!(paths.mtu(PEER, LINK), LINK);
}

#[test]
fn clearing_forgets_every_estimate() {
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1400, secs(0)));
    paths.clear();
    assert!(paths.is_empty());
    assert_eq!(paths.mtu(PEER, LINK), LINK);
}

#[test]
fn the_default_is_an_empty_table() {
    let paths = Paths::default();
    assert!(paths.is_empty());
}
