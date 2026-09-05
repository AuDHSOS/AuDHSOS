// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two demultiplexing tables.

use crate::protocol::{EtherType, Protocol};

#[test]
fn the_ether_types_this_system_reads_are_registered_and_the_others_are_not() {
    assert_eq!(EtherType::IPV4.get(), 0x0800);
    assert_eq!(EtherType::ARP.get(), 0x0806);
    assert_eq!(EtherType::IPV6.get(), 0x86DD);
    assert!(EtherType::IPV4.is_registered());
    assert!(EtherType::ARP.is_registered());
    assert!(EtherType::IPV6.is_registered());
    assert!(!EtherType::new(0x8100).is_registered());
    assert_eq!(EtherType::new(0x0800), EtherType::IPV4);
    assert_eq!(EtherType::default().get(), 0);
}

#[test]
fn a_field_below_the_smallest_type_is_a_length() {
    assert!(!EtherType::new(0x05DC).is_type());
    assert!(EtherType::MIN.is_type());
    assert!(EtherType::IPV4.is_type());
    assert!(!EtherType::default().is_type());
}

#[test]
fn an_ether_type_writes_its_name_or_its_number() {
    assert_eq!(EtherType::IPV4.to_string(), "IPv4");
    assert_eq!(EtherType::ARP.to_string(), "ARP");
    assert_eq!(EtherType::IPV6.to_string(), "IPv6");
    assert_eq!(EtherType::new(0x8100).to_string(), "0x8100");
    assert_eq!(EtherType::new(0x8100).name(), None);
    assert_eq!(EtherType::IPV4.name(), Some("IPv4"));
}

#[test]
fn the_protocols_this_system_reads_are_registered_and_the_others_are_not() {
    assert_eq!(Protocol::ICMP.get(), 1);
    assert_eq!(Protocol::TCP.get(), 6);
    assert_eq!(Protocol::UDP.get(), 17);
    assert_eq!(Protocol::ICMPV6.get(), 58);
    assert!(Protocol::ICMP.is_registered());
    assert!(Protocol::ICMPV6.is_registered());
    assert!(Protocol::TCP.is_registered());
    assert!(Protocol::UDP.is_registered());
    assert!(!Protocol::new(47).is_registered());
    assert!(!Protocol::FRAGMENT.is_registered());
    assert_eq!(Protocol::new(6), Protocol::TCP);
    assert_eq!(Protocol::default().get(), 0);
}

#[test]
fn the_transport_protocols_and_icmpv6_carry_a_pseudo_header() {
    assert!(Protocol::TCP.has_pseudo_header());
    assert!(Protocol::UDP.has_pseudo_header());
    // RFC 4443, section 2.3: ICMPv6 sums over the pseudo-header, which is
    // the one place the two ICMPs differ for a checksum routine.
    assert!(Protocol::ICMPV6.has_pseudo_header());
    assert!(!Protocol::ICMP.has_pseudo_header());
    assert!(!Protocol::new(47).has_pseudo_header());
}

#[test]
fn the_extension_headers_are_the_four_this_system_steps_over() {
    for header in [
        Protocol::HOP_BY_HOP,
        Protocol::ROUTING,
        Protocol::FRAGMENT,
        Protocol::DESTINATION_OPTIONS,
    ] {
        assert!(header.is_extension_header(), "{header}");
        assert!(!header.has_pseudo_header(), "{header}");
    }
    for upper in [
        Protocol::TCP,
        Protocol::UDP,
        Protocol::ICMPV6,
        Protocol::NO_NEXT_HEADER,
        Protocol::new(47),
    ] {
        assert!(!upper.is_extension_header(), "{upper}");
    }
    assert_eq!(Protocol::HOP_BY_HOP.get(), 0);
    assert_eq!(Protocol::ROUTING.get(), 43);
    assert_eq!(Protocol::FRAGMENT.get(), 44);
    assert_eq!(Protocol::NO_NEXT_HEADER.get(), 59);
    assert_eq!(Protocol::DESTINATION_OPTIONS.get(), 60);
}

#[test]
fn a_protocol_writes_its_name_or_its_number() {
    assert_eq!(Protocol::ICMP.to_string(), "ICMP");
    assert_eq!(Protocol::TCP.to_string(), "TCP");
    assert_eq!(Protocol::UDP.to_string(), "UDP");
    assert_eq!(Protocol::ICMPV6.to_string(), "ICMPv6");
    assert_eq!(Protocol::HOP_BY_HOP.to_string(), "hop-by-hop options");
    assert_eq!(Protocol::ROUTING.to_string(), "routing header");
    assert_eq!(Protocol::FRAGMENT.to_string(), "fragment header");
    assert_eq!(Protocol::NO_NEXT_HEADER.to_string(), "no next header");
    assert_eq!(
        Protocol::DESTINATION_OPTIONS.to_string(),
        "destination options"
    );
    assert_eq!(Protocol::new(47).to_string(), "47");
    assert_eq!(Protocol::new(47).name(), None);
}
