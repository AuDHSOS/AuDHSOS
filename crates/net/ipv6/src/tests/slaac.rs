// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Stateless address autoconfiguration against RFC 4862, the router
//! advertisement of RFC 4861 it reads that out of, the DNS server option
//! of RFC 8106, and the duplicate address detection that comes first.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds an advertisement at known offsets"
)]

use audhsos_time::{Duration, Instant};
use net_ip::{IpError, NextHop, RoutingTable};
use net_wire::{IpAddr, IpCidr, Ipv6Addr, Ipv6Cidr, MacAddr};

use super::{HARDWARE, HOST, PEER, PEER_HARDWARE, at, checksummed, packet, secs};
use crate::error::Ipv6Error;
use crate::header::{DISCOVERY_HOP_LIMIT, Packet};
use crate::ndp::{Discovery, ROUTER_ADVERTISEMENT, option_type, receive};
use crate::slaac::{
    Configuration, Dad, DadEvent, DadState, Expired, INTERFACE_ID_LEN, address_from, expiry,
    interface_identifier,
};

/// A configuration of two prefixes and three DNS servers.
type Config = Configuration<2, 3>;

/// The router's link-local address, which is where an advertisement comes
/// from.
const ROUTER: Ipv6Addr = Ipv6Addr::from_octets([
    0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x1b, 0x44, 0xff, 0xfe, 0x11, 0x3a, 0x01,
]);

/// The prefix a router advertises here.
const PREFIX: Ipv6Addr =
    Ipv6Addr::from_octets([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

/// A DNS server under it.
const SERVER: Ipv6Addr = Ipv6Addr::from_octets([
    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x35,
]);

/// A prefix information option (RFC 4861, section 4.6.2).
fn prefix_option(
    prefix: Ipv6Addr,
    prefix_len: u8,
    on_link: bool,
    autonomous: bool,
    valid: u32,
    preferred: u32,
) -> Vec<u8> {
    let mut flags = 0u8;
    if on_link {
        flags |= 0x80;
    }
    if autonomous {
        flags |= 0x40;
    }
    let mut option = vec![option_type::PREFIX, 4, prefix_len, flags];
    option.extend_from_slice(&valid.to_be_bytes());
    option.extend_from_slice(&preferred.to_be_bytes());
    option.extend_from_slice(&[0; 4]);
    option.extend_from_slice(&prefix.octets());
    option
}

/// An MTU option (RFC 4861, section 4.6.4).
fn mtu_option(mtu: u32) -> Vec<u8> {
    let mut option = vec![option_type::MTU, 1, 0, 0];
    option.extend_from_slice(&mtu.to_be_bytes());
    option
}

/// A recursive DNS server option (RFC 8106, section 5.1).
fn rdnss_option(lifetime: u32, servers: &[Ipv6Addr]) -> Vec<u8> {
    let units = u8::try_from(1 + 2 * servers.len()).expect("a short list");
    let mut option = vec![option_type::RDNSS, units, 0, 0];
    option.extend_from_slice(&lifetime.to_be_bytes());
    for server in servers {
        option.extend_from_slice(&server.octets());
    }
    option
}

/// A router advertisement carrying `options`.
fn advertisement(router_lifetime: u16, hop_limit: u8, options: &[u8]) -> Vec<u8> {
    let mut message = vec![ROUTER_ADVERTISEMENT, 0, 0, 0, hop_limit, 0];
    message.extend_from_slice(&router_lifetime.to_be_bytes());
    // The reachable time and the retransmit timer, which this host reads
    // and does not act on: the cache of `net-eth` runs on its own
    // schedule and takes no randomness to spread the probes with.
    message.extend_from_slice(&[0; 8]);
    message.extend_from_slice(options);
    checksummed(ROUTER, Ipv6Addr::ALL_NODES, &mut message);
    message
}

/// Reads an advertisement out of the packet it would arrive in.
///
/// The packet outlives the borrow because the test binary keeps it: a
/// `Discovery` borrows the bytes it was read from, and building the
/// packet into a temporary would hand back a view of something already
/// gone.
fn read(message: &[u8]) -> Discovery<'static> {
    let bytes = packet(
        ROUTER,
        Ipv6Addr::ALL_NODES,
        net_wire::Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        message,
    );
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let packet = Packet::parse(bytes).expect("a well-formed packet");
    let upper = packet.upper_layer().expect("no chain");
    receive(packet, upper.payload).expect("a router advertisement")
}

#[test]
fn the_interface_identifier_is_the_modified_eui_64_of_rfc_2464() {
    // RFC 2464, section 4 works the example 34-56-78-9A-BC-DE, whose
    // identifier is 36-56-78-FF-FE-9A-BC-DE.
    let hardware = MacAddr::new([0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE]);
    assert_eq!(
        interface_identifier(hardware),
        [0x36, 0x56, 0x78, 0xFF, 0xFE, 0x9A, 0xBC, 0xDE]
    );
    // The bit is complemented, not set: an address that already has it
    // set loses it.
    let local = MacAddr::new([0x36, 0x56, 0x78, 0x9A, 0xBC, 0xDE]);
    assert_eq!(interface_identifier(local)[0], 0x34);
}

#[test]
fn an_address_is_the_prefix_with_the_identifier_behind_it() {
    let identifier: [u8; INTERFACE_ID_LEN] = [0x36, 0x56, 0x78, 0xFF, 0xFE, 0x9A, 0xBC, 0xDE];
    let address = address_from(PREFIX, identifier);
    assert_eq!(address.to_string(), "2001:db8::3656:78ff:fe9a:bcde");
    // Whatever stood in the low half of the prefix is overwritten.
    assert_eq!(address_from(SERVER, identifier), address);
}

#[test]
fn a_lifetime_of_all_ones_never_runs_out() {
    assert_eq!(expiry(secs(1), u32::MAX), Instant::MAX);
    assert_eq!(expiry(secs(1), 10), secs(11));
}

#[test]
fn a_prefix_and_a_router_are_learned_and_an_address_is_formed() {
    let mut config = Config::new();
    assert_eq!(config.router(), None);
    assert_eq!(config.address(), None);
    assert_eq!(config.poll_at(), None);

    let options = prefix_option(PREFIX, 64, true, true, 2_592_000, 604_800);
    let message = read(&advertisement(1800, 64, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room for one prefix");

    let router = config.router().expect("a default router");
    assert_eq!(router.address, ROUTER);
    assert_eq!(router.expires_at, secs(1800));
    assert_eq!(config.hop_limit(), Some(64));

    let address = config.address().expect("an address formed by SLAAC");
    assert_eq!(
        address,
        address_from(PREFIX, interface_identifier(HARDWARE))
    );
    let configured = config.prefixes().next().expect("one prefix");
    assert_eq!(configured.prefix, Ipv6Cidr::new(PREFIX, 64).expect("a /64"));
    assert!(configured.on_link);
    assert_eq!(configured.valid_until, secs(2_592_000));
    assert_eq!(configured.preferred_until, secs(604_800));
}

#[test]
fn a_prefix_length_that_is_not_sixty_four_forms_no_address() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 48, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    // The prefix is still learned — it says what is on this link — and
    // no address comes out of it (RFC 4862, section 5.5.3 (d)).
    let configured = config.prefixes().next().expect("one prefix");
    assert_eq!(configured.address, None);
    assert_eq!(config.address(), None);
    assert!(configured.on_link);
    // And nothing was recommended as a hop limit.
    assert_eq!(config.hop_limit(), None);
}

#[test]
fn a_prefix_without_the_autonomous_flag_forms_no_address() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, false, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(config.address(), None);
}

#[test]
fn a_link_local_prefix_is_ignored() {
    let mut config = Config::new();
    let link_local = Ipv6Addr::from_octets([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let options = prefix_option(link_local, 64, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(config.prefixes().count(), 0);
}

#[test]
fn a_prefix_of_no_valid_lifetime_is_forgotten() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(config.prefixes().count(), 1);

    let options = prefix_option(PREFIX, 64, true, true, 0, 0);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(1))
        .expect("room");
    assert_eq!(config.prefixes().count(), 0);
}

#[test]
fn a_second_advertisement_updates_the_prefix_rather_than_repeating_it() {
    let mut config = Config::new();
    for (valid, now) in [(3600u32, secs(0)), (7200, secs(10))] {
        let options = prefix_option(PREFIX, 64, true, true, valid, valid);
        let message = read(&advertisement(1800, 0, &options));
        config
            .on_advertisement(ROUTER, &message, HARDWARE, now)
            .expect("room");
    }
    assert_eq!(config.prefixes().count(), 1);
    let configured = config.prefixes().next().expect("one prefix");
    assert_eq!(configured.valid_until, secs(7210));
}

#[test]
fn more_prefixes_than_the_configuration_holds_are_refused() {
    let mut config = Configuration::<1, 1>::new();
    let mut options = prefix_option(PREFIX, 64, true, true, 3600, 1800);
    options.extend_from_slice(&prefix_option(SERVER, 64, true, true, 3600, 1800));
    let message = read(&advertisement(1800, 0, &options));
    assert_eq!(
        config.on_advertisement(ROUTER, &message, HARDWARE, secs(0)),
        Err(Ipv6Error::Ip(IpError::NoRoute))
    );
}

#[test]
fn a_router_lifetime_of_zero_says_it_is_not_a_default_router() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert!(config.router().is_some());

    let message = read(&advertisement(0, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(1))
        .expect("room");
    assert_eq!(config.router(), None);
    // The prefixes it still carried were read all the same.
    assert_eq!(config.prefixes().count(), 1);
}

#[test]
fn the_recursive_dns_servers_of_rfc_8106_are_read_out() {
    let mut config = Config::new();
    let second = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x36,
    ]);
    let options = rdnss_option(600, &[SERVER, second]);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    let servers: Vec<_> = config.servers().map(|server| server.address).collect();
    assert_eq!(servers, vec![SERVER, second]);
    for server in config.servers() {
        assert_eq!(server.expires_at, secs(600));
    }
}

#[test]
fn a_dns_server_of_no_lifetime_is_forgotten() {
    let mut config = Config::new();
    let options = rdnss_option(600, &[SERVER]);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(config.servers().count(), 1);

    let options = rdnss_option(0, &[SERVER]);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(1))
        .expect("room");
    assert_eq!(config.servers().count(), 0);
}

#[test]
fn a_server_that_is_advertised_twice_has_its_lifetime_refreshed() {
    let mut config = Config::new();
    for now in [secs(0), secs(100)] {
        let options = rdnss_option(600, &[SERVER]);
        let message = read(&advertisement(1800, 0, &options));
        config
            .on_advertisement(ROUTER, &message, HARDWARE, now)
            .expect("room");
    }
    assert_eq!(config.servers().count(), 1);
    let server = config.servers().next().expect("one server");
    assert_eq!(server.expires_at, secs(700));
}

#[test]
fn an_mtu_option_is_taken_and_one_below_the_minimum_is_not() {
    let mut config = Config::new();
    let message = read(&advertisement(1800, 0, &mtu_option(1492)));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(config.link_mtu(), Some(1492));

    // A link that cannot carry 1280 bytes cannot carry IPv6.
    let message = read(&advertisement(1800, 0, &mtu_option(576)));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(1))
        .expect("room");
    assert_eq!(config.link_mtu(), Some(1492));
}

#[test]
fn the_lifetimes_expire_one_at_a_time() {
    let mut config = Config::new();
    let mut options = prefix_option(PREFIX, 64, true, true, 100, 50);
    options.extend_from_slice(&rdnss_option(200, &[SERVER]));
    let message = read(&advertisement(300, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");

    assert_eq!(config.poll_at(), Some(secs(100)));
    assert_eq!(config.poll(secs(50)), None);

    let prefix = Ipv6Cidr::new(PREFIX, 64).expect("a /64");
    assert_eq!(config.poll(secs(100)), Some(Expired::Prefix(prefix)));
    assert_eq!(config.address(), None);
    assert_eq!(config.poll_at(), Some(secs(200)));

    assert_eq!(config.poll(secs(200)), Some(Expired::Server(SERVER)));
    assert_eq!(config.poll_at(), Some(secs(300)));

    assert_eq!(config.poll(secs(300)), Some(Expired::Router(ROUTER)));
    assert_eq!(config.router(), None);
    assert_eq!(config.poll(secs(300)), None);
    assert_eq!(config.poll_at(), None);
}

#[test]
fn an_infinite_lifetime_is_never_polled_for() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, true, u32::MAX, u32::MAX);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    assert_eq!(
        config.poll_at(),
        Some(secs(1800)),
        "only the router runs out"
    );
    assert_eq!(config.poll(secs(1800)), Some(Expired::Router(ROUTER)));
    assert_eq!(config.poll_at(), None);
}

#[test]
fn what_was_learned_becomes_a_default_route_and_an_on_link_route() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");

    let mut table = RoutingTable::<4>::new();
    config.install(&mut table).expect("room for two routes");
    assert_eq!(table.len(), 2);
    let prefix = Ipv6Cidr::new(PREFIX, 64).expect("a /64");
    let on_link = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x99,
    ]);
    assert!(prefix.contains(on_link));
    assert_eq!(
        table.lookup(IpAddr::V6(on_link)),
        Ok(NextHop::OnLink(IpAddr::V6(on_link)))
    );
    // Anything outside it goes through the router.
    let far = Ipv6Addr::from_octets([
        0x20, 0x01, 0x48, 0x60, 0x48, 0x60, 0, 0, 0, 0, 0, 0, 0, 0, 0x88, 0x88,
    ]);
    assert!(!prefix.contains(far));
    assert_eq!(
        table.lookup(IpAddr::V6(far)),
        Ok(NextHop::Gateway(IpAddr::V6(ROUTER)))
    );

    // Installing twice replaces rather than repeats.
    config.install(&mut table).expect("room");
    assert_eq!(table.len(), 2);

    // And when the router stops being one, the default route goes.
    let message = read(&advertisement(0, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(1))
        .expect("room");
    config.install(&mut table).expect("room");
    assert_eq!(table.len(), 1);
    assert_eq!(
        table.lookup(IpAddr::V6(far)),
        Err(IpError::NoRoute),
        "no default route is left"
    );
    assert!(!table.is_empty());
    assert_eq!(
        table.iter().next().map(|route| route.destination),
        Some(IpCidr::V6(prefix))
    );
}

#[test]
fn a_table_with_no_room_refuses_the_route() {
    let mut config = Config::new();
    let options = prefix_option(PREFIX, 64, true, true, 3600, 1800);
    let message = read(&advertisement(1800, 0, &options));
    config
        .on_advertisement(ROUTER, &message, HARDWARE, secs(0))
        .expect("room");
    let mut table = RoutingTable::<1>::new();
    assert_eq!(config.install(&mut table), Err(IpError::NoRoute));
}

#[test]
fn a_message_that_is_not_an_advertisement_configures_nothing() {
    let mut config = Config::new();
    let mut message = vec![133u8, 0, 0, 0, 0, 0, 0, 0];
    checksummed(ROUTER, Ipv6Addr::ALL_NODES, &mut message);
    let solicitation = read(&message);
    assert!(matches!(
        config.on_advertisement(ROUTER, &solicitation, HARDWARE, secs(0)),
        Err(Ipv6Error::NotDiscovery(0))
    ));
}

#[test]
fn duplicate_address_detection_asks_the_solicited_node_group_and_gives_up() {
    let mut dad = Dad::new(HOST, at(0));
    assert_eq!(dad.address(), HOST);
    assert_eq!(dad.state(), DadState::Tentative);
    assert_eq!(dad.poll_at(), Some(at(0)));

    assert_eq!(
        dad.poll(at(0)),
        Some(DadEvent::Solicit {
            target: HOST,
            group: HOST.solicited_node(),
        })
    );
    // Nothing more until the retransmit timer.
    assert_eq!(dad.poll(at(1)), None);
    assert_eq!(dad.poll_at(), Some(secs(1)));

    assert_eq!(dad.poll(secs(1)), Some(DadEvent::Unique(HOST)));
    assert_eq!(dad.state(), DadState::Unique);
    assert_eq!(dad.poll_at(), None);
    assert_eq!(dad.poll(secs(2)), None);
}

#[test]
fn an_advertisement_for_the_tentative_address_makes_it_a_duplicate() {
    let mut dad = Dad::new(HOST, at(0));
    dad.poll(at(0)).expect("a solicitation");

    let mut message = vec![136u8, 0, 0, 0, 0x60, 0, 0, 0];
    message.extend_from_slice(&HOST.octets());
    message.extend_from_slice(&[option_type::TARGET_LINK_LAYER, 1]);
    message.extend_from_slice(&PEER_HARDWARE.octets());
    checksummed(PEER, HOST.solicited_node(), &mut message);
    let bytes = packet(
        PEER,
        HOST.solicited_node(),
        net_wire::Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    let discovery = receive(parsed, upper.payload).expect("an advertisement");

    assert!(dad.on_advertisement(&discovery));
    assert_eq!(dad.state(), DadState::Duplicate);
    assert_eq!(dad.poll_at(), None);
    assert_eq!(dad.poll(secs(1)), None);
    // A second claim changes nothing: the address was already given up.
    assert!(!dad.on_advertisement(&discovery));
}

#[test]
fn an_advertisement_for_another_address_leaves_the_check_alone() {
    let mut dad = Dad::new(HOST, at(0));
    let mut message = vec![136u8, 0, 0, 0, 0x60, 0, 0, 0];
    message.extend_from_slice(&PEER.octets());
    checksummed(PEER, HOST, &mut message);
    let bytes = packet(
        PEER,
        HOST,
        net_wire::Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    let discovery = receive(parsed, upper.payload).expect("an advertisement");
    assert!(!dad.on_advertisement(&discovery));
    assert_eq!(dad.state(), DadState::Tentative);
}

#[test]
fn two_nodes_checking_the_same_address_at_once_both_give_it_up() {
    let mut dad = Dad::new(HOST, at(0));
    // The other node's probe: an unspecified source, no options, the same
    // target (RFC 4862, section 5.4.3).
    let mut message = vec![135u8, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&HOST.octets());
    checksummed(Ipv6Addr::UNSPECIFIED, HOST.solicited_node(), &mut message);
    let bytes = packet(
        Ipv6Addr::UNSPECIFIED,
        HOST.solicited_node(),
        net_wire::Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    let discovery = receive(parsed, upper.payload).expect("a solicitation");

    assert!(dad.on_solicitation(Ipv6Addr::UNSPECIFIED, &discovery));
    assert_eq!(dad.state(), DadState::Duplicate);
}

#[test]
fn a_solicitation_from_a_node_that_has_an_address_is_not_a_claim() {
    let mut dad = Dad::new(HOST, at(0));
    let mut message = vec![135u8, 0, 0, 0, 0, 0, 0, 0];
    message.extend_from_slice(&HOST.octets());
    checksummed(PEER, HOST.solicited_node(), &mut message);
    let bytes = packet(
        PEER,
        HOST.solicited_node(),
        net_wire::Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    let parsed = Packet::parse(&bytes).expect("a well-formed packet");
    let upper = parsed.upper_layer().expect("no chain");
    let discovery = receive(parsed, upper.payload).expect("a solicitation");
    assert!(!dad.on_solicitation(PEER, &discovery));
    assert_eq!(dad.state(), DadState::Tentative);
    // Nor does an advertisement change a solicitation-shaped check, or
    // the other way round.
    assert!(!dad.on_advertisement(&discovery));
}

#[test]
fn a_check_of_no_transmits_is_over_before_it_began() {
    let mut dad = Dad::with(HOST, 0, Duration::from_secs(1), at(0));
    assert_eq!(dad.poll(at(0)), Some(DadEvent::Unique(HOST)));
    assert_eq!(dad.state(), DadState::Unique);
}

#[test]
fn a_check_of_several_transmits_repeats_on_the_schedule_it_was_given() {
    let mut dad = Dad::with(HOST, 3, Duration::from_millis(500), at(0));
    let mut now = at(0);
    for _ in 0..3 {
        assert!(matches!(dad.poll(now), Some(DadEvent::Solicit { .. })));
        now = now.saturating_add(Duration::from_millis(500));
    }
    assert_eq!(dad.poll(now), Some(DadEvent::Unique(HOST)));
}
