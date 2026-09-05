// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! IPv6 addresses: the canonical text of RFC 5952 and nothing else.

use test_support::generators::bytes;
use test_support::property::check;

use crate::addr::{IpAddr, IpCidr, IpVersion, Ipv4Addr, Ipv4Cidr, Ipv6Addr, Ipv6Cidr};
use crate::error::WireError;

#[test]
fn an_address_reads_and_writes_the_canonical_form() {
    // RFC 5952, section 4: the examples the recommendation itself gives.
    let cases = [
        ("::", Ipv6Addr::UNSPECIFIED),
        ("::1", Ipv6Addr::LOCALHOST),
        ("ff02::1", Ipv6Addr::ALL_NODES),
        ("ff02::2", Ipv6Addr::ALL_ROUTERS),
        (
            "2001:db8::1",
            Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]),
        ),
        (
            "2001:db8::2:1",
            Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 2, 1]),
        ),
        (
            "2001:db8:0:1:1:1:1:1",
            Ipv6Addr::new([0x2001, 0x0DB8, 0, 1, 1, 1, 1, 1]),
        ),
        (
            "2001:db8::1:0:0:1",
            Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 1, 0, 0, 1]),
        ),
        (
            "2001:0:0:1::1",
            Ipv6Addr::new([0x2001, 0, 0, 1, 0, 0, 0, 1]),
        ),
        (
            "fe80::1ff:fe23:4567:890a",
            Ipv6Addr::new([0xFE80, 0, 0, 0, 0x01FF, 0xFE23, 0x4567, 0x890A]),
        ),
        (
            "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
            Ipv6Addr::from_bits(u128::MAX),
        ),
    ];
    for (text, address) in cases {
        assert_eq!(Ipv6Addr::parse(text), Ok(address), "{text}");
        assert_eq!(address.to_string(), text, "{text}");
    }
}

#[test]
fn upper_case_is_read_and_never_written() {
    let address = Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 0x00AB]);
    assert_eq!(Ipv6Addr::parse("2001:DB8::AB"), Ok(address));
    assert_eq!(Ipv6Addr::parse("2001:Db8::aB"), Ok(address));
    assert_eq!(address.to_string(), "2001:db8::ab");
}

#[test]
fn a_text_that_is_not_the_canonical_form_is_refused() {
    for text in [
        // RFC 5952 4.1: leading zeros are suppressed.
        "2001:0db8::1",
        "::0001",
        "2001:db8::01",
        // 4.2.1: `::` is used to its maximum.
        "2001:db8:0:0:0:0:2:1",
        "2001:db8::0:1",
        "0:0:0:0:0:0:0:0",
        // 4.2.2: `::` never stands for one zero group.
        "2001:db8::1:1:1:1:1",
        "1:2:3:4:5:6:7::",
        "::2:3:4:5:6:7:8",
        // 4.2.3: the longest run, and the leftmost of two equal ones.
        "2001:0:0:1:0:0:0:1",
        "2001:db8:0:0:1::1",
        // Two compressions, or none where eight groups are needed.
        "2001::db8::1",
        "1:2:3:4:5:6:7",
        "1:2:3:4:5:6:7:8:9",
        // Groups that are not groups.
        "",
        ":",
        "1:2:3:4:5:6:7:",
        ":1:2:3:4:5:6:7",
        "2001:db8::12345",
        "2001:db8::g",
        "2001:db8::1 ",
        // The dotted form belongs to the mapped addresses, which this
        // system does not carry.
        "::ffff:1.2.3.4",
        "::1.2.3.4",
    ] {
        assert_eq!(Ipv6Addr::parse(text), Err(WireError::Address), "{text:?}");
    }
}

#[test]
fn the_predicates_name_the_prefixes_of_rfc_4291() {
    assert!(Ipv6Addr::UNSPECIFIED.is_unspecified());
    assert!(!Ipv6Addr::UNSPECIFIED.is_unicast());
    assert!(Ipv6Addr::LOCALHOST.is_loopback());
    assert!(Ipv6Addr::LOCALHOST.is_unicast());
    assert!(!Ipv6Addr::LOCALHOST.is_multicast());
    assert!(Ipv6Addr::ALL_NODES.is_multicast());
    assert!(!Ipv6Addr::ALL_NODES.is_unicast());
    assert!(Ipv6Addr::new([0xFE80, 0, 0, 0, 0, 0, 0, 1]).is_link_local());
    assert!(Ipv6Addr::new([0xFEBF, 0, 0, 0, 0, 0, 0, 1]).is_link_local());
    assert!(!Ipv6Addr::new([0xFEC0, 0, 0, 0, 0, 0, 0, 1]).is_link_local());
    assert!(!Ipv6Addr::new([0xFE7F, 0, 0, 0, 0, 0, 0, 1]).is_link_local());
    assert!(!Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]).is_link_local());
    assert!(Ipv6Addr::new([0xFC00, 0, 0, 0, 0, 0, 0, 1]).is_unique_local());
    assert!(Ipv6Addr::new([0xFDFF, 0, 0, 0, 0, 0, 0, 1]).is_unique_local());
    assert!(!Ipv6Addr::new([0xFE00, 0, 0, 0, 0, 0, 0, 1]).is_unique_local());
    assert_eq!(Ipv6Addr::LEN, 16);
    assert_eq!(Ipv6Addr::GROUPS, 8);
    assert_eq!(Ipv6Addr::default(), Ipv6Addr::UNSPECIFIED);
}

#[test]
fn the_solicited_node_address_is_the_low_bits_under_the_multicast_prefix() {
    // RFC 4291, section 2.7.1: the address 4037::1:800:200e:8c6c has the
    // solicited-node address ff02::1:ff0e:8c6c.
    let address = Ipv6Addr::new([0x4037, 0, 0, 0, 1, 0x0800, 0x200E, 0x8C6C]);
    let solicited = address.solicited_node();
    assert_eq!(solicited.to_string(), "ff02::1:ff0e:8c6c");
    assert!(solicited.is_multicast());
    // Addresses that differ only above the low 24 bits share the group.
    let other = Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0x200E, 0x8C6C]);
    assert_eq!(other.solicited_node(), solicited);
}

#[test]
fn the_octets_the_groups_and_the_bits_are_the_same_address() {
    let address = Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]);
    let octets = [0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    assert_eq!(address.octets(), octets);
    assert_eq!(Ipv6Addr::from_octets(octets), address);
    assert_eq!(address.groups(), [0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]);
    assert_eq!(address.to_bits(), 0x2001_0DB8_0000_0000_0000_0000_0000_0001);
    assert_eq!(Ipv6Addr::from_bits(address.to_bits()), address);
}

#[test]
fn a_prefix_of_zero_of_a_hundred_and_twenty_eight_and_of_one_more() {
    let address = Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]);

    let everything = Ipv6Cidr::new(address, 0).expect("zero is a prefix length");
    assert_eq!(everything.network(), Ipv6Addr::UNSPECIFIED);
    assert!(everything.contains(Ipv6Addr::ALL_NODES));

    let host = Ipv6Cidr::new(address, 128).expect("128 is a prefix length");
    assert_eq!(host.network(), address);
    assert!(host.contains(address));
    assert!(!host.contains(Ipv6Addr::LOCALHOST));

    assert_eq!(
        Ipv6Cidr::new(address, 129),
        Err(WireError::PrefixLength(129))
    );
    assert_eq!(Ipv6Cidr::MAX_PREFIX_LEN, 128);
}

#[test]
fn a_network_holds_the_addresses_that_share_its_prefix() {
    let network = Ipv6Cidr::parse("2001:db8::1/64").expect("a network in canonical form");
    assert_eq!(
        network.address(),
        Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1])
    );
    assert_eq!(network.prefix_len(), 64);
    assert_eq!(
        network.network(),
        Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 0])
    );
    assert!(network.contains(Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0xFFFF, 0, 0, 1])));
    assert!(!network.contains(Ipv6Addr::new([0x2001, 0x0DB8, 0, 1, 0, 0, 0, 1])));
    assert_eq!(network.to_string(), "2001:db8::1/64");

    let odd = Ipv6Cidr::parse("2001:db8::/33").expect("a prefix inside a group");
    assert!(odd.contains(Ipv6Addr::new([0x2001, 0x0DB8, 0x7FFF, 0, 0, 0, 0, 1])));
    assert!(!odd.contains(Ipv6Addr::new([0x2001, 0x0DB8, 0x8000, 0, 0, 0, 0, 1])));
}

#[test]
fn a_network_text_that_is_not_one_is_refused() {
    assert_eq!(Ipv6Cidr::parse("2001:db8::1"), Err(WireError::Address));
    assert_eq!(Ipv6Cidr::parse("2001:db8::1/"), Err(WireError::Address));
    assert_eq!(Ipv6Cidr::parse("2001:db8::1/x"), Err(WireError::Address));
    assert_eq!(Ipv6Cidr::parse("2001:db8::1/064"), Err(WireError::Address));
    assert_eq!(Ipv6Cidr::parse("2001:db8::1/300"), Err(WireError::Address));
    assert_eq!(Ipv6Cidr::parse("2001:0db8::1/64"), Err(WireError::Address));
    assert_eq!(
        Ipv6Cidr::parse("2001:db8::1/129"),
        Err(WireError::PrefixLength(129))
    );
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

#[test]
fn every_address_reads_back_from_what_it_wrote() {
    check("ipv6 text round trip", &bytes(16..=16), |octets| {
        let mut value = [0u8; 16];
        value.copy_from_slice(octets.get(..16).unwrap_or(&[0; 16]));
        let address = Ipv6Addr::from_octets(value);
        let text = address.to_string();
        match Ipv6Addr::parse(&text) {
            Ok(back) if back == address => Ok(()),
            other => Err(format!("{text} came back as {other:?}")),
        }
    });
}

#[test]
fn no_text_this_parser_accepts_has_a_second_spelling() {
    check("ipv6 text is canonical", &bytes(16..=16), |octets| {
        let mut value = [0u8; 16];
        value.copy_from_slice(octets.get(..16).unwrap_or(&[0; 16]));
        let address = Ipv6Addr::from_octets(value);
        // The uncompressed form of the same address is a second spelling
        // whenever it holds a run of zeros, and must be refused.
        let groups = address.groups();
        let written = groups
            .into_iter()
            .map(|group| format!("{group:x}"))
            .collect::<Vec<_>>()
            .join(":");
        let compressed = address.to_string();
        let accepted = Ipv6Addr::parse(&written).is_ok();
        if accepted == (written == compressed) {
            Ok(())
        } else {
            Err(format!("{written} and {compressed} disagree"))
        }
    });
}
