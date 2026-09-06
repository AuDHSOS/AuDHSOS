// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The address selection of RFC 6724, as far as a host of this shape can
//! decide it.

use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::select::{common_prefix, order_destinations, policy, scope, source_for};

/// The IPv6 address `text` spells.
fn v6(text: &str) -> IpAddr {
    IpAddr::V6(Ipv6Addr::parse(text).expect("an address"))
}

/// The IPv4 address `text` spells.
fn v4(text: &str) -> IpAddr {
    IpAddr::V4(Ipv4Addr::parse(text).expect("an address"))
}

#[test]
fn the_policy_table_gives_every_prefix_the_numbers_rfc_6724_gives_it() {
    assert_eq!(policy(v6("::1")), (50, 0));
    assert_eq!(policy(v6("2606:2800::1")), (40, 1));
    assert_eq!(policy(v6("2002::1")), (30, 2));
    // The Teredo range of the table, which is `2001::/32` and not the
    // documentation prefix behind it.
    assert_eq!(policy(v6("2001:0:1::1")), (5, 5));
    assert_eq!(policy(v6("2001:db8::1")), (40, 1));
    assert_eq!(policy(v6("fc00::1")), (3, 13));
    assert_eq!(policy(v6("fd00::1")), (3, 13));
    assert_eq!(policy(v6("fec0::1")), (1, 11));
    assert_eq!(policy(v6("3ffe::1")), (1, 12));
    assert_eq!(policy(v6("::2")), (1, 3));
    // The one row that stands for the whole IPv4 family, because this
    // system has no mapped addresses to key it by (D-69).
    assert_eq!(policy(v4("192.0.2.1")), (35, 4));
    assert_eq!(policy(v4("127.0.0.1")), (35, 4));
}

#[test]
fn every_scope_is_the_one_rfc_6724_gives_it() {
    assert_eq!(scope(v6("::1")), 0x1);
    assert_eq!(scope(v6("fe80::1")), 0x2);
    assert_eq!(scope(v6("fec0::1")), 0x5);
    assert_eq!(scope(v6("2606:2800::1")), 0xE);
    assert_eq!(scope(v6("ff02::1")), 0x2);
    assert_eq!(scope(v6("ff0e::1")), 0xE);
    assert_eq!(scope(v4("127.0.0.1")), 0x2);
    assert_eq!(scope(v4("169.254.1.1")), 0x2);
    assert_eq!(scope(v4("224.0.0.1")), 0x2);
    // Only the first group of the multicast range is link-local; the rest
    // of it is global, and the administratively scoped block is site.
    assert_eq!(scope(v4("224.1.2.3")), 0xE);
    assert_eq!(scope(v4("225.0.0.1")), 0xE);
    assert_eq!(scope(v4("239.1.2.3")), 0x5);
    assert_eq!(scope(v4("169.1.1.1")), 0xE);
    assert_eq!(scope(v6("fe00::1")), 0xE);
    assert_eq!(scope(v6("fe40::1")), 0xE);
    // A private address is global, which is what the document says and
    // not what one might guess.
    assert_eq!(scope(v4("10.0.0.1")), 0xE);
    assert_eq!(scope(v4("192.0.2.1")), 0xE);
}

#[test]
fn two_addresses_share_the_bits_they_share() {
    assert_eq!(common_prefix(v4("192.0.2.1"), v4("192.0.2.129")), 24);
    assert_eq!(common_prefix(v4("10.0.0.1"), v4("10.0.0.1")), 32);
    assert_eq!(common_prefix(v6("2001:db8::1"), v6("2001:db8::2")), 126);
    // Across the two families nothing is shared, because nothing is
    // comparable.
    assert_eq!(common_prefix(v4("10.0.0.1"), v6("::1")), 0);
}

#[test]
fn a_source_of_the_other_family_is_never_one() {
    let candidates = [v4("192.0.2.5"), v6("2001:db8::5")];
    assert_eq!(
        source_for(v4("198.51.100.1"), &candidates),
        Some(v4("192.0.2.5"))
    );
    assert_eq!(
        source_for(v6("2001:db8::9"), &candidates),
        Some(v6("2001:db8::5"))
    );
    assert_eq!(source_for(v6("2001:db8::9"), &[v4("192.0.2.5")]), None);
    assert_eq!(source_for(v4("10.0.0.1"), &[]), None);
}

#[test]
fn the_destination_itself_is_the_source_where_this_host_holds_it() {
    let candidates = [v4("192.0.2.5"), v4("198.51.100.7")];
    assert_eq!(
        source_for(v4("198.51.100.7"), &candidates),
        Some(v4("198.51.100.7"))
    );
    // And the other way round, whichever order the list is in.
    let reversed = [v4("198.51.100.7"), v4("192.0.2.5")];
    assert_eq!(
        source_for(v4("192.0.2.5"), &reversed),
        Some(v4("192.0.2.5"))
    );
}

#[test]
fn a_link_local_destination_is_reached_from_a_link_local_source() {
    let candidates = [v6("2001:db8::5"), v6("fe80::5")];
    assert_eq!(source_for(v6("fe80::9"), &candidates), Some(v6("fe80::5")));
    // And a global destination from the global address, because a
    // link-local scope is smaller than the destination's.
    assert_eq!(
        source_for(v6("2606:2800::1"), &candidates),
        Some(v6("2001:db8::5"))
    );
}

#[test]
fn the_longest_matching_prefix_decides_between_two_of_one_scope() {
    let candidates = [v4("10.0.0.1"), v4("192.0.2.5")];
    assert_eq!(
        source_for(v4("192.0.2.200"), &candidates),
        Some(v4("192.0.2.5"))
    );
}

#[test]
fn a_name_of_both_families_is_tried_in_the_order_the_document_gives() {
    // A host with a global address of each family reaches the IPv6
    // destination first: the labels match on both sides, and IPv6's
    // precedence of 40 beats the IPv4 row's 35.
    let mine = [v4("192.0.2.5"), v6("2001:db8::5")];
    let source_of = |destination: IpAddr| source_for(destination, &mine);
    let mut addresses = [v4("198.51.100.1"), v6("2606:2800::1")];
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, [v6("2606:2800::1"), v4("198.51.100.1")]);
    // The same list the other way round comes out the same way.
    let mut reversed = [v6("2606:2800::1"), v4("198.51.100.1")];
    order_destinations(&mut reversed, &source_of);
    assert_eq!(reversed, [v6("2606:2800::1"), v4("198.51.100.1")]);
}

#[test]
fn a_host_of_one_family_puts_what_it_cannot_reach_last() {
    let mine = [v4("192.0.2.5")];
    let source_of = |destination: IpAddr| source_for(destination, &mine);
    let mut addresses = [v6("2606:2800::1"), v4("198.51.100.1")];
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, [v4("198.51.100.1"), v6("2606:2800::1")]);
}

#[test]
fn two_destinations_nothing_can_reach_stay_as_they_were() {
    let source_of = |_: IpAddr| None;
    let mut addresses = [v6("2606:2800::1"), v4("198.51.100.1"), v4("203.0.113.1")];
    let before = addresses;
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, before);
}

#[test]
fn a_smaller_scope_is_preferred_where_everything_else_agrees() {
    let mine = [v6("fe80::5"), v6("2001:db8::5")];
    let source_of = |destination: IpAddr| source_for(destination, &mine);
    let mut addresses = [v6("2001:db8::9"), v6("fe80::9")];
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, [v6("fe80::9"), v6("2001:db8::9")]);
}

#[test]
fn ordering_a_list_of_one_or_of_none_does_nothing() {
    let source_of = |_: IpAddr| Some(v4("192.0.2.5"));
    let mut none: [IpAddr; 0] = [];
    order_destinations(&mut none, &source_of);
    let mut one = [v4("198.51.100.1")];
    order_destinations(&mut one, &source_of);
    assert_eq!(one, [v4("198.51.100.1")]);
}

#[test]
fn the_destination_itself_wins_whichever_of_the_two_it_is() {
    // The held candidate is the destination and the one being weighed
    // against it is not.
    let candidates = [v4("192.0.2.5"), v4("198.51.100.7")];
    assert_eq!(
        source_for(v4("192.0.2.5"), &candidates),
        Some(v4("192.0.2.5"))
    );
}

#[test]
fn a_source_whose_label_is_the_destinations_is_preferred() {
    // Both are global and neither is the destination, so the label of the
    // policy table decides: `2002::/16` carries label 2 and the
    // destination carries 1.
    let candidates = [v6("2002::5"), v6("2001:db8::5")];
    assert_eq!(
        source_for(v6("2606:2800::1"), &candidates),
        Some(v6("2001:db8::5"))
    );
}

#[test]
fn two_destinations_that_agree_on_everything_else_are_ordered_by_prefix() {
    let mine = [v4("192.0.2.5")];
    let source_of = |destination: IpAddr| source_for(destination, &mine);
    let mut addresses = [v4("198.51.100.1"), v4("192.0.2.200")];
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, [v4("192.0.2.200"), v4("198.51.100.1")]);
}

#[test]
fn a_destination_whose_scope_its_source_does_not_share_comes_second() {
    // A host with only a global address reaches a link-local destination
    // through nothing, so rule 1 already puts it last; give it a
    // link-local source too and rule 2 is what decides.
    let mine = [v6("2001:db8::5"), v6("fe80::5")];
    let source_of = |destination: IpAddr| source_for(destination, &mine);
    let mut addresses = [v6("ff0e::1"), v6("2606:2800::1")];
    order_destinations(&mut addresses, &source_of);
    assert_eq!(addresses, [v6("2606:2800::1"), v6("ff0e::1")]);
}
