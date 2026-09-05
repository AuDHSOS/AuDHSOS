// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The routing table: the longest prefix wins, and a family never reaches
//! into the other.

use net_wire::{IpAddr, IpCidr, Ipv4Addr, Ipv4Cidr, Ipv6Addr, Ipv6Cidr};

use crate::error::IpError;
use crate::route::{NextHop, Route, RoutingTable};

/// A table of six routes, which is more than a host with one interface
/// needs and few enough to fill in a test.
type Table = RoutingTable<6>;

/// The v4 network this host is on.
fn subnet() -> IpCidr {
    IpCidr::V4(Ipv4Cidr::parse("192.168.1.0/24").expect("a network"))
}

/// The default route's prefix.
fn default_v4() -> IpCidr {
    IpCidr::V4(Ipv4Cidr::parse("0.0.0.0/0").expect("a network"))
}

/// The router on that network.
fn gateway() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 254))
}

/// One host on it.
fn host() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7))
}

#[test]
fn an_empty_table_reaches_nothing() {
    let table = Table::new();
    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
    assert_eq!(table.lookup(host()), Err(IpError::NoRoute));
    assert!(!table.is_on_link(host()));
    assert_eq!(Table::default().len(), 0);
}

#[test]
fn the_longest_prefix_wins_over_the_subnet_and_the_default() {
    let mut table = Table::new();
    let other = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9));
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    table.add(Route::on_link(subnet())).expect("room");
    table
        .add(Route::via(
            IpCidr::V4(Ipv4Cidr::parse("192.168.1.9/32").expect("a host route")),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 200)),
        ))
        .expect("room");
    assert_eq!(table.len(), 3);

    // The host route beats the subnet.
    assert_eq!(
        table.lookup(other),
        Ok(NextHop::Gateway(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 200
        ))))
    );
    // The subnet beats the default, and it is on this link.
    assert_eq!(table.lookup(host()), Ok(NextHop::OnLink(host())));
    assert!(table.is_on_link(host()));
    // Everything else goes through the router.
    let far = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));
    assert_eq!(table.lookup(far), Ok(NextHop::Gateway(gateway())));
    assert!(!table.is_on_link(far));
}

#[test]
fn a_next_hop_names_the_address_to_resolve() {
    assert_eq!(NextHop::OnLink(host()).address(), host());
    assert_eq!(NextHop::Gateway(gateway()).address(), gateway());
    assert!(NextHop::OnLink(host()).is_on_link());
    assert!(!NextHop::Gateway(gateway()).is_on_link());
}

#[test]
fn a_route_of_one_family_never_reaches_a_destination_of_the_other() {
    let mut table = Table::new();
    table.add(Route::on_link(subnet())).expect("room");
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    let six = IpAddr::V6(Ipv6Addr::new([0x2001, 0x0DB8, 0, 0, 0, 0, 0, 1]));
    // A v4 default route is not a route to everything, only to every v4
    // destination.
    assert_eq!(table.lookup(six), Err(IpError::NoRoute));

    table
        .add(Route::on_link(IpCidr::V6(
            Ipv6Cidr::parse("2001:db8::/32").expect("a network"),
        )))
        .expect("room");
    assert_eq!(table.lookup(six), Ok(NextHop::OnLink(six)));
    // And the v4 destinations still go where they went.
    assert_eq!(table.lookup(host()), Ok(NextHop::OnLink(host())));
}

#[test]
fn adding_the_same_prefix_replaces_the_route_rather_than_shadowing_it() {
    let mut table = Table::new();
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    let second = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 253));
    table.add(Route::via(default_v4(), second)).expect("room");
    assert_eq!(table.len(), 1);
    assert_eq!(
        table.lookup(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
        Ok(NextHop::Gateway(second))
    );

    // Including a change of kind: a prefix that was reached through a
    // router becomes one on this link.
    table.add(Route::on_link(default_v4())).expect("room");
    assert_eq!(table.len(), 1);
    assert!(table.is_on_link(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))));
}

#[test]
fn a_route_can_be_taken_out_again() {
    let mut table = Table::new();
    table.add(Route::on_link(subnet())).expect("room");
    assert!(table.remove(subnet()));
    assert!(table.is_empty());
    assert!(!table.remove(subnet()));

    table.add(Route::on_link(subnet())).expect("room");
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    table.clear();
    assert!(table.is_empty());
    assert_eq!(table.lookup(host()), Err(IpError::NoRoute));
}

#[test]
fn a_full_table_refuses_rather_than_deciding_what_to_lose() {
    let mut table = RoutingTable::<1>::new();
    table.add(Route::on_link(subnet())).expect("room for one");
    assert_eq!(
        table.add(Route::via(default_v4(), gateway())),
        Err(IpError::NoRoute)
    );
    // The one that was there is untouched.
    assert_eq!(table.len(), 1);
    assert_eq!(table.lookup(host()), Ok(NextHop::OnLink(host())));
    // Replacing a prefix still works when there is no room for a new one.
    assert_eq!(table.add(Route::via(subnet(), gateway())), Ok(()));
    assert_eq!(table.lookup(host()), Ok(NextHop::Gateway(gateway())));
}

#[test]
fn the_routes_can_be_read_back_in_the_order_they_were_added() {
    let mut table = Table::new();
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    table.add(Route::on_link(subnet())).expect("room");
    let prefixes: Vec<IpCidr> = table.iter().map(|route| route.destination).collect();
    assert_eq!(prefixes, [default_v4(), subnet()]);
    let gateways: Vec<Option<IpAddr>> = table.iter().map(|route| route.gateway).collect();
    assert_eq!(gateways, [Some(gateway()), None]);
}

#[test]
fn a_default_route_of_each_family_covers_its_own() {
    let mut table = Table::new();
    table
        .add(Route::via(default_v4(), gateway()))
        .expect("room");
    let six_gateway = IpAddr::V6(Ipv6Addr::new([0xFE80, 0, 0, 0, 0, 0, 0, 1]));
    table
        .add(Route::via(
            IpCidr::V6(Ipv6Cidr::parse("::/0").expect("a network")),
            six_gateway,
        ))
        .expect("room");
    assert_eq!(
        table.lookup(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
        Ok(NextHop::Gateway(gateway()))
    );
    assert_eq!(
        table.lookup(IpAddr::V6(Ipv6Addr::new([0x2001, 0, 0, 0, 0, 0, 0, 1]))),
        Ok(NextHop::Gateway(six_gateway))
    );
}
