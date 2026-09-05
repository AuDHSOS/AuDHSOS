// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The routing table: where a datagram goes next.
//!
//! One table holds routes of both families (D-69), because a host has one
//! set of destinations and not two. A route of one family never matches a
//! destination of the other, so the two live together without reaching
//! into each other.
//!
//! The match is the longest prefix: a host route beats a subnet route and
//! a subnet route beats the default, which is a prefix of length zero and
//! nothing special otherwise. That is the whole of the algorithm — there
//! is no metric and no cost, because a host with one interface has
//! nothing to weigh.

use audhsos_collections::ArrayVec;
use net_wire::{IpAddr, IpCidr};

use crate::error::IpError;

/// Where a datagram goes to reach a destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NextHop {
    /// The destination is on this link. Resolve the destination itself.
    OnLink(IpAddr),
    /// It is elsewhere. Resolve this router and send to it.
    Gateway(IpAddr),
}

impl NextHop {
    /// The address to resolve, whichever kind of hop this is. It is the
    /// destination for an on-link hop and the router for a gateway, which
    /// is the one thing every caller of this module wants.
    #[must_use]
    pub const fn address(self) -> IpAddr {
        match self {
            NextHop::OnLink(address) | NextHop::Gateway(address) => address,
        }
    }

    /// Whether the destination is on this link.
    #[must_use]
    pub const fn is_on_link(self) -> bool {
        matches!(self, NextHop::OnLink(_))
    }
}

/// One route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Route {
    /// Which destinations it covers.
    pub destination: IpCidr,
    /// The router to send through, or `None` when the prefix is on this
    /// link and the destination is resolved directly.
    pub gateway: Option<IpAddr>,
}

impl Route {
    /// A prefix that is on this link.
    #[must_use]
    pub const fn on_link(destination: IpCidr) -> Route {
        Route {
            destination,
            gateway: None,
        }
    }

    /// A prefix reached through a router.
    #[must_use]
    pub const fn via(destination: IpCidr, gateway: IpAddr) -> Route {
        Route {
            destination,
            gateway: Some(gateway),
        }
    }
}

/// A table of at most `N` routes.
#[derive(Debug)]
pub struct RoutingTable<const N: usize> {
    /// The routes, in the order they were added.
    routes: ArrayVec<Route, N>,
}

impl<const N: usize> Default for RoutingTable<N> {
    fn default() -> RoutingTable<N> {
        RoutingTable::new()
    }
}

impl<const N: usize> RoutingTable<N> {
    /// An empty table.
    #[must_use]
    pub const fn new() -> RoutingTable<N> {
        RoutingTable {
            routes: ArrayVec::new(),
        }
    }

    /// How many routes there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether there are none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Adds `route`, replacing one that covers exactly the same prefix.
    ///
    /// Replacing rather than appending is what makes a second default
    /// route from a second router advertisement an update and not a
    /// duplicate that shadows the first.
    ///
    /// # Errors
    ///
    /// [`IpError::NoRoute`] when the table is full. There is no room and
    /// nothing to evict: which route to lose is not a thing this table can
    /// decide, so it refuses and the caller, which knows why it wanted the
    /// route, decides.
    pub fn add(&mut self, route: Route) -> Result<(), IpError> {
        if let Some(existing) = self
            .routes
            .iter_mut()
            .find(|existing| existing.destination == route.destination)
        {
            *existing = route;
            return Ok(());
        }
        self.routes.push(route).map_err(|_| IpError::NoRoute)
    }

    /// Removes the route for exactly this prefix, and answers whether
    /// there was one.
    pub fn remove(&mut self, destination: IpCidr) -> bool {
        let Some((index, _)) = self
            .routes
            .iter()
            .enumerate()
            .find(|(_, route)| route.destination == destination)
        else {
            return false;
        };
        self.routes.remove(index).is_some()
    }

    /// Forgets every route.
    pub fn clear(&mut self) {
        self.routes.clear();
    }

    /// The routes, in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = &Route> {
        self.routes.iter()
    }

    /// Where `destination` goes.
    ///
    /// The route with the longest matching prefix wins. Among two of the
    /// same length the first added wins, which cannot happen through
    /// [`add`](Self::add) — it replaces a prefix rather than repeating it.
    ///
    /// # Errors
    ///
    /// [`IpError::NoRoute`] when no route covers the destination.
    pub fn lookup(&self, destination: IpAddr) -> Result<NextHop, IpError> {
        let best = self
            .routes
            .iter()
            .filter(|route| route.destination.contains(destination))
            .max_by_key(|route| route.destination.prefix_len())
            .ok_or(IpError::NoRoute)?;
        Ok(match best.gateway {
            Some(gateway) => NextHop::Gateway(gateway),
            None => NextHop::OnLink(destination),
        })
    }

    /// Whether `destination` is on a link this host is attached to.
    #[must_use]
    pub fn is_on_link(&self, destination: IpAddr) -> bool {
        self.lookup(destination).is_ok_and(NextHop::is_on_link)
    }
}
