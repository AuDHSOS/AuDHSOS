// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The hardware address, the port, and the two types that carry an
//! address of either family. What belongs to one family is in
//! [`ipv4`](super::ipv4) and [`ipv6`](super::ipv6).

use test_support::generators::bytes;
use test_support::property::check;

use crate::addr::{
    IpAddr, IpCidr, IpVersion, Ipv4Addr, Ipv4Cidr, Ipv6Addr, Ipv6Cidr, MacAddr, Port,
};
use crate::error::WireError;

#[test]
fn a_mac_address_writes_lower_case_and_reads_either() {
    let address = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB7]);
    assert_eq!(address.to_string(), "00:1b:44:11:3a:b7");
    for text in [
        "00:1b:44:11:3a:b7",
        "00:1B:44:11:3A:B7",
        "00:1b:44:11:3A:b7",
    ] {
        assert_eq!(MacAddr::parse(text), Ok(address), "{text:?}");
    }
}

#[test]
fn a_mac_address_that_is_not_six_pairs_is_refused() {
    for text in [
        "",
        "00:1b:44:11:3a",
        "00:1b:44:11:3a:b7:c8",
        "00:1b:44:11:3a:b",
        "00:1b:44:11:3a:bb7",
        "00-1b-44-11-3a-b7",
        "00:1b:44:11:3a:bg",
        "00:1b:44:11:3a::",
    ] {
        assert_eq!(MacAddr::parse(text), Err(WireError::Address), "{text:?}");
    }
}

#[test]
fn the_mac_predicates_follow_the_group_bit() {
    assert!(MacAddr::BROADCAST.is_broadcast());
    assert!(MacAddr::BROADCAST.is_multicast());
    assert!(!MacAddr::BROADCAST.is_unicast());
    assert!(MacAddr::UNSPECIFIED.is_unspecified());
    assert!(MacAddr::UNSPECIFIED.is_unicast());
    assert!(!MacAddr::UNSPECIFIED.is_broadcast());
    assert!(!MacAddr::UNSPECIFIED.is_multicast());
    let multicast = MacAddr::new([0x01, 0x00, 0x5E, 0x00, 0x00, 0x01]);
    assert!(multicast.is_multicast());
    assert!(!multicast.is_broadcast());
    assert!(!multicast.is_unspecified());
    let unicast = MacAddr::new([0x02, 0x00, 0x5E, 0x00, 0x00, 0x01]);
    assert!(unicast.is_unicast());
    assert_eq!(unicast.octets(), [0x02, 0x00, 0x5E, 0x00, 0x00, 0x01]);
    assert_eq!(MacAddr::LEN, 6);
    assert_eq!(MacAddr::default(), MacAddr::UNSPECIFIED);
}

#[test]
fn every_mac_address_reads_back_from_what_it_wrote() {
    check("mac text round trip", &bytes(6..=6), |octets| {
        let mut value = [0u8; 6];
        value.copy_from_slice(octets.get(..6).unwrap_or(&[0; 6]));
        let address = MacAddr::new(value);
        let text = address.to_string();
        match MacAddr::parse(&text) {
            Ok(back) if back == address => Ok(()),
            other => Err(format!("{text} came back as {other:?}")),
        }
    });
}

#[test]
fn a_port_knows_the_dynamic_range() {
    assert!(Port::UNSPECIFIED.is_unspecified());
    assert!(!Port::UNSPECIFIED.is_ephemeral());
    assert_eq!(Port::new(53).get(), 53);
    assert!(!Port::new(53).is_ephemeral());
    assert!(!Port::new(49151).is_ephemeral());
    assert!(Port::EPHEMERAL_FIRST.is_ephemeral());
    assert!(Port::EPHEMERAL_LAST.is_ephemeral());
    assert_eq!(Port::EPHEMERAL_LAST.get(), u16::MAX);
    assert_eq!(Port::new(443).to_string(), "443");
    assert_eq!(Port::default(), Port::UNSPECIFIED);
}

#[test]
fn an_address_of_either_family_carries_its_own_version() {
    let four = IpAddr::from(Ipv4Addr::new(192, 168, 1, 1));
    let six = IpAddr::from(Ipv6Addr::LOCALHOST);
    assert_eq!(four.version(), IpVersion::V4);
    assert_eq!(six.version(), IpVersion::V6);
    assert!(four.is_v4() && !four.is_v6());
    assert!(six.is_v6() && !six.is_v4());
    assert_eq!(four.to_string(), "192.168.1.1");
    assert_eq!(six.to_string(), "::1");
    assert_eq!(IpVersion::V4.to_string(), "IPv4");
    assert_eq!(IpVersion::V6.to_string(), "IPv6");
}

#[test]
fn a_text_with_a_colon_is_read_as_the_second_family() {
    assert_eq!(
        IpAddr::parse("10.0.0.1"),
        Ok(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))
    );
    assert_eq!(IpAddr::parse("::1"), Ok(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert_eq!(IpAddr::parse("10.0.0.256"), Err(WireError::Address));
    assert_eq!(IpAddr::parse("::x"), Err(WireError::Address));
}

#[test]
fn the_predicates_of_either_family_answer_through_the_enum() {
    assert!(IpAddr::V4(Ipv4Addr::UNSPECIFIED).is_unspecified());
    assert!(IpAddr::V6(Ipv6Addr::UNSPECIFIED).is_unspecified());
    assert!(IpAddr::V4(Ipv4Addr::LOCALHOST).is_loopback());
    assert!(IpAddr::V6(Ipv6Addr::LOCALHOST).is_loopback());
    assert!(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1)).is_multicast());
    assert!(IpAddr::V6(Ipv6Addr::ALL_NODES).is_multicast());
    assert!(!IpAddr::V4(Ipv4Addr::LOCALHOST).is_multicast());
    assert!(!IpAddr::V6(Ipv6Addr::LOCALHOST).is_multicast());
    assert!(!IpAddr::V4(Ipv4Addr::LOCALHOST).is_unspecified());
    assert!(!IpAddr::V6(Ipv6Addr::LOCALHOST).is_unspecified());
}

#[test]
fn a_route_of_one_family_never_matches_a_destination_of_the_other() {
    let four = IpCidr::from(Ipv4Cidr::parse("10.0.0.0/8").expect("a v4 network"));
    let six = IpCidr::from(Ipv6Cidr::parse("2001:db8::/32").expect("a v6 network"));
    assert!(four.contains(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
    assert!(!four.contains(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    assert!(six.contains(IpAddr::V6(Ipv6Addr::new([
        0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1
    ]))));
    assert!(!six.contains(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
    assert_eq!(four.version(), IpVersion::V4);
    assert_eq!(six.version(), IpVersion::V6);
    assert_eq!(four.prefix_len(), 8);
    assert_eq!(six.prefix_len(), 32);
    assert_eq!(four.address(), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)));
    assert_eq!(
        six.address(),
        IpAddr::V6(Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 0]))
    );
    assert_eq!(four.to_string(), "10.0.0.0/8");
    assert_eq!(six.to_string(), "2001:db8::/32");
    assert_eq!(IpCidr::parse("10.0.0.0/8"), Ok(four));
    assert_eq!(IpCidr::parse("2001:db8::/32"), Ok(six));
    assert_eq!(
        IpCidr::parse("10.0.0.0/33"),
        Err(WireError::PrefixLength(33))
    );
}
