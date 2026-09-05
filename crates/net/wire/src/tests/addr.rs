// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Addresses: one text per value, in both directions.

use test_support::generators::{Generator, bytes, range};
use test_support::property::check;

use crate::addr::{Ipv4Addr, Ipv4Cidr, MacAddr, Port};
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
fn an_ipv4_address_round_trips_through_its_canonical_text() {
    let address = Ipv4Addr::new(192, 168, 1, 7);
    assert_eq!(address.to_string(), "192.168.1.7");
    assert_eq!(Ipv4Addr::parse("192.168.1.7"), Ok(address));
    assert_eq!(Ipv4Addr::parse("0.0.0.0"), Ok(Ipv4Addr::UNSPECIFIED));
    assert_eq!(Ipv4Addr::parse("255.255.255.255"), Ok(Ipv4Addr::BROADCAST));
    assert_eq!(Ipv4Addr::parse("127.0.0.1"), Ok(Ipv4Addr::LOCALHOST));
    assert_eq!(address.octets(), [192, 168, 1, 7]);
    assert_eq!(address.to_bits(), 0xC0A8_0107);
    assert_eq!(Ipv4Addr::from_bits(0xC0A8_0107), address);
    assert_eq!(Ipv4Addr::from_octets([192, 168, 1, 7]), address);
    assert_eq!(Ipv4Addr::LEN, 4);
    assert_eq!(Ipv4Addr::default(), Ipv4Addr::UNSPECIFIED);
}

#[test]
fn a_leading_zero_is_refused_rather_than_read_as_octal() {
    for text in ["010.0.0.1", "0.0.0.01", "00.0.0.0"] {
        assert_eq!(Ipv4Addr::parse(text), Err(WireError::Address), "{text:?}");
    }
}

#[test]
fn an_ipv4_text_that_is_not_four_octets_is_refused() {
    for text in [
        "",
        "1.2.3",
        "1.2.3.4.5",
        "1.2.3.",
        ".1.2.3",
        "1.2.3.256",
        "1.2.3.999",
        "1.2.3.4444",
        "1.2.3.x",
        "1.2.3.-4",
        "1.2. 3.4",
    ] {
        assert_eq!(Ipv4Addr::parse(text), Err(WireError::Address), "{text:?}");
    }
}

#[test]
fn the_ipv4_predicates_name_their_ranges() {
    assert!(Ipv4Addr::UNSPECIFIED.is_unspecified());
    assert!(Ipv4Addr::BROADCAST.is_broadcast());
    assert!(Ipv4Addr::LOCALHOST.is_loopback());
    assert!(Ipv4Addr::new(127, 255, 255, 254).is_loopback());
    assert!(!Ipv4Addr::new(128, 0, 0, 1).is_loopback());
    assert!(Ipv4Addr::new(224, 0, 0, 1).is_multicast());
    assert!(Ipv4Addr::new(239, 255, 255, 255).is_multicast());
    assert!(!Ipv4Addr::new(240, 0, 0, 1).is_multicast());
    assert!(!Ipv4Addr::new(223, 255, 255, 255).is_multicast());
    assert!(!Ipv4Addr::BROADCAST.is_unspecified());
    assert!(!Ipv4Addr::UNSPECIFIED.is_broadcast());
}

#[test]
fn a_prefix_of_zero_of_thirty_two_and_of_thirty_three() {
    let address = Ipv4Addr::new(10, 1, 2, 3);

    let everything = Ipv4Cidr::new(address, 0).expect("zero is a prefix length");
    assert_eq!(everything.netmask(), Ipv4Addr::UNSPECIFIED);
    assert_eq!(everything.network(), Ipv4Addr::UNSPECIFIED);
    assert_eq!(everything.broadcast(), Ipv4Addr::BROADCAST);
    assert!(everything.contains(Ipv4Addr::new(203, 0, 113, 9)));

    let host = Ipv4Cidr::new(address, 32).expect("32 is a prefix length");
    assert_eq!(host.netmask(), Ipv4Addr::BROADCAST);
    assert_eq!(host.network(), address);
    assert_eq!(host.broadcast(), address);
    assert!(host.contains(address));
    assert!(!host.contains(Ipv4Addr::new(10, 1, 2, 4)));

    assert_eq!(Ipv4Cidr::new(address, 33), Err(WireError::PrefixLength(33)));
    assert_eq!(Ipv4Cidr::MAX_PREFIX_LEN, 32);
}

#[test]
fn a_network_contains_both_of_its_ends_and_nothing_beyond_them() {
    let network = Ipv4Cidr::parse("192.168.1.7/24").expect("a network in canonical form");
    assert_eq!(network.address(), Ipv4Addr::new(192, 168, 1, 7));
    assert_eq!(network.prefix_len(), 24);
    assert_eq!(network.netmask(), Ipv4Addr::new(255, 255, 255, 0));
    assert_eq!(network.network(), Ipv4Addr::new(192, 168, 1, 0));
    assert_eq!(network.broadcast(), Ipv4Addr::new(192, 168, 1, 255));
    assert!(network.contains(Ipv4Addr::new(192, 168, 1, 0)));
    assert!(network.contains(Ipv4Addr::new(192, 168, 1, 255)));
    assert!(!network.contains(Ipv4Addr::new(192, 168, 0, 255)));
    assert!(!network.contains(Ipv4Addr::new(192, 168, 2, 0)));
    assert_eq!(network.to_string(), "192.168.1.7/24");

    let odd = Ipv4Cidr::parse("10.0.0.130/26").expect("a prefix inside an octet");
    assert_eq!(odd.netmask(), Ipv4Addr::new(255, 255, 255, 192));
    assert_eq!(odd.network(), Ipv4Addr::new(10, 0, 0, 128));
    assert_eq!(odd.broadcast(), Ipv4Addr::new(10, 0, 0, 191));
    assert!(odd.contains(Ipv4Addr::new(10, 0, 0, 191)));
    assert!(!odd.contains(Ipv4Addr::new(10, 0, 0, 192)));
}

#[test]
fn a_network_text_that_is_not_one_is_refused() {
    assert_eq!(Ipv4Cidr::parse("192.168.1.0"), Err(WireError::Address));
    assert_eq!(Ipv4Cidr::parse("192.168.1.0/"), Err(WireError::Address));
    assert_eq!(Ipv4Cidr::parse("192.168.1.0/x"), Err(WireError::Address));
    assert_eq!(Ipv4Cidr::parse("192.168.1.0/024"), Err(WireError::Address));
    assert_eq!(Ipv4Cidr::parse("192.168.1.0/300"), Err(WireError::Address));
    assert_eq!(Ipv4Cidr::parse("192.168.1/24"), Err(WireError::Address));
    assert_eq!(
        Ipv4Cidr::parse("192.168.1.0/33"),
        Err(WireError::PrefixLength(33))
    );
    assert_eq!(
        Ipv4Cidr::parse("192.168.1.0/0"),
        Ipv4Cidr::new(Ipv4Addr::new(192, 168, 1, 0), 0)
    );
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
fn every_ipv4_address_reads_back_from_what_it_wrote() {
    check("ipv4 text round trip", &bytes(4..=4), |octets| {
        let mut value = [0u8; 4];
        value.copy_from_slice(octets.get(..4).unwrap_or(&[0; 4]));
        let address = Ipv4Addr::from_octets(value);
        let text = address.to_string();
        match Ipv4Addr::parse(&text) {
            Ok(back) if back == address => Ok(()),
            other => Err(format!("{text} came back as {other:?}")),
        }
    });
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
fn a_network_contains_exactly_the_addresses_between_its_ends() {
    let generator = range(0u8..=32).map(|prefix_len| (prefix_len, 0xC0A8_2A07u32));
    check("cidr bounds", &generator, |(prefix_len, bits)| {
        let network = Ipv4Cidr::new(Ipv4Addr::from_bits(*bits), *prefix_len)
            .map_err(|error| error.to_string())?;
        let first = network.network().to_bits();
        let last = network.broadcast().to_bits();
        if first > last {
            return Err(format!("{network} runs backwards"));
        }
        if !network.contains(Ipv4Addr::from_bits(first))
            || !network.contains(Ipv4Addr::from_bits(last))
        {
            return Err(format!("{network} excludes one of its ends"));
        }
        let outside = last.checked_add(1).map(Ipv4Addr::from_bits);
        match outside {
            Some(address) if network.contains(address) && *prefix_len != 0 => {
                Err(format!("{network} reaches {address}"))
            }
            _ => Ok(()),
        }
    });
}
