// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! How a host configures itself: stateless address autoconfiguration of
//! RFC 4862, the router advertisement it reads that out of, and the
//! duplicate address detection that has to pass before an address is
//! used.
//!
//! There is no `DHCPv6` (D-69). An advertisement carries the prefix, the
//! router, the lifetimes, the link MTU, and — with the option of
//! RFC 8106 — the recursive DNS servers, which is everything `net-dhcp`
//! gets over IPv4 from a protocol of its own size. The managed and other
//! flags of the advertisement are read and carried, so a caller can see
//! that a network wanted `DHCPv6`, and nothing here acts on them.
//!
//! The interface identifier is the modified EUI-64 of RFC 4291,
//! appendix A, formed from the Ethernet address as RFC 2464, section 4
//! prescribes. It is stable, which means an address formed here follows
//! the hardware from network to network and is a name a distant server
//! can keep. The temporary addresses of RFC 4941 are what answer that,
//! and they are not in the first version: they need a generator in a
//! crate that has none, and a policy about when to rotate one, and both
//! belong with the address selection of 12.6.12.
//!
//! Every lifetime is an [`Instant`] the moment it is read, so nothing
//! here counts down and no test sleeps.

use audhsos_collections::ArrayVec;
use audhsos_time::{Duration, Instant};
use net_ip::{IpError, Route, RoutingTable};
use net_wire::{IpCidr, Ipv6Addr, Ipv6Cidr, MacAddr};

use crate::error::Ipv6Error;
use crate::header::MIN_MTU;
use crate::ndp::{Discovery, INFINITE_LIFETIME, NdpOption};

/// The prefix length SLAAC forms an address under.
///
/// RFC 4862, section 5.5.3 (d) ignores a prefix whose length plus the
/// interface identifier's does not make 128, and the identifier of an
/// Ethernet interface is 64 bits (RFC 2464, section 4). So it is this
/// number and no other, and an advertisement that says otherwise forms no
/// address.
pub const SLAAC_PREFIX_LEN: u8 = 64;

/// How long an interface identifier is.
pub const INTERFACE_ID_LEN: usize = 8;

/// The modified EUI-64 interface identifier of an Ethernet address.
///
/// RFC 2464, section 4: the three bytes of the OUI, then `FF FE`, then the
/// last three bytes, with the universal/local bit of the first byte
/// complemented. That bit is the second-lowest, and complementing it is
/// what turns a universally administered hardware address into a globally
/// unique interface identifier.
#[must_use]
pub const fn interface_identifier(hardware: MacAddr) -> [u8; INTERFACE_ID_LEN] {
    let [first, second, third, fourth, fifth, sixth] = hardware.octets();
    [
        first ^ 0x02,
        second,
        third,
        0xFF,
        0xFE,
        fourth,
        fifth,
        sixth,
    ]
}

/// The address `prefix` and `identifier` make.
///
/// The prefix is taken to be 64 bits, which is the only length this forms
/// an address under; whatever the advertisement left in the low half is
/// overwritten by the identifier.
#[must_use]
pub const fn address_from(prefix: Ipv6Addr, identifier: [u8; INTERFACE_ID_LEN]) -> Ipv6Addr {
    let [p0, p1, p2, p3, p4, p5, p6, p7, ..] = prefix.octets();
    let [i0, i1, i2, i3, i4, i5, i6, i7] = identifier;
    Ipv6Addr::from_octets([
        p0, p1, p2, p3, p4, p5, p6, p7, i0, i1, i2, i3, i4, i5, i6, i7,
    ])
}

/// A lifetime in seconds as the instant it runs out at.
///
/// The all-ones value is infinity in every field of RFC 4861 and RFC 8106
/// that carries one, and it becomes [`Instant::MAX`], which no clock
/// reaches.
#[must_use]
pub fn expiry(now: Instant, seconds: u32) -> Instant {
    if seconds == INFINITE_LIFETIME {
        return Instant::MAX;
    }
    now.saturating_add(Duration::from_secs(u64::from(seconds)))
}

/// The default router, while it is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Router {
    /// Its address, which is the link-local address the advertisement
    /// came from.
    pub address: Ipv6Addr,
    /// When it stops being a default router.
    pub expires_at: Instant,
}

/// A prefix that was advertised, and the address this host formed under
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Configured {
    /// The prefix, as it was advertised.
    pub prefix: Ipv6Cidr,
    /// The address formed from it, or `None` when the prefix was
    /// on-link-only: an advertisement may say a prefix is on this link
    /// and still forbid forming an address from it.
    pub address: Option<Ipv6Addr>,
    /// Whether an address under the prefix is on this link.
    pub on_link: bool,
    /// When the address stops being usable at all.
    pub valid_until: Instant,
    /// When it stops being the one to send from.
    pub preferred_until: Instant,
}

/// A recursive DNS server, while its lifetime lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Server {
    /// Where it is.
    pub address: Ipv6Addr,
    /// When it stops being one.
    pub expires_at: Instant,
}

/// What ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expired {
    /// The default router. The caller removes the default route.
    Router(Ipv6Addr),
    /// A prefix, and with it whatever address was formed under it. The
    /// caller removes the on-link route.
    Prefix(Ipv6Cidr),
    /// A DNS server.
    Server(Ipv6Addr),
}

/// What a host learned about the link it is on.
///
/// `PREFIXES` prefixes and `SERVERS` DNS servers at a time. Both are
/// small: a link advertises one prefix and two or three servers, and a
/// router that advertised more than this holds would be describing a
/// network this host has no way to be on.
#[derive(Debug)]
pub struct Configuration<const PREFIXES: usize, const SERVERS: usize> {
    /// The default router, while there is one.
    router: Option<Router>,
    /// The prefixes and the addresses under them.
    prefixes: ArrayVec<Configured, PREFIXES>,
    /// The recursive DNS servers.
    servers: ArrayVec<Server, SERVERS>,
    /// What the router recommends as a hop limit, zero for nothing.
    hop_limit: u8,
    /// What it says the link carries, when it said.
    link_mtu: Option<usize>,
}

impl<const PREFIXES: usize, const SERVERS: usize> Default for Configuration<PREFIXES, SERVERS> {
    fn default() -> Configuration<PREFIXES, SERVERS> {
        Configuration::new()
    }
}

impl<const PREFIXES: usize, const SERVERS: usize> Configuration<PREFIXES, SERVERS> {
    /// A host that has heard nothing yet.
    #[must_use]
    pub const fn new() -> Configuration<PREFIXES, SERVERS> {
        Configuration {
            router: None,
            prefixes: ArrayVec::new(),
            servers: ArrayVec::new(),
            hop_limit: 0,
            link_mtu: None,
        }
    }

    /// The default router, while there is one.
    #[must_use]
    pub const fn router(&self) -> Option<Router> {
        self.router
    }

    /// The prefixes that are current, with the addresses under them.
    pub fn prefixes(&self) -> impl Iterator<Item = &Configured> {
        self.prefixes.iter()
    }

    /// The first address this host formed, which is the one it sends
    /// from until the selection of 12.6.12 has more to choose between.
    #[must_use]
    pub fn address(&self) -> Option<Ipv6Addr> {
        self.prefixes.iter().find_map(|entry| entry.address)
    }

    /// The recursive DNS servers.
    pub fn servers(&self) -> impl Iterator<Item = &Server> {
        self.servers.iter()
    }

    /// The hop limit the router recommends, or `None` when it recommended
    /// none.
    #[must_use]
    pub const fn hop_limit(&self) -> Option<u8> {
        if self.hop_limit == 0 {
            return None;
        }
        Some(self.hop_limit)
    }

    /// The MTU the router gave for the link, when it gave one.
    #[must_use]
    pub const fn link_mtu(&self) -> Option<usize> {
        self.link_mtu
    }

    /// Takes a router advertisement in.
    ///
    /// `from` is where it came from, which is the router; `hardware` is
    /// this interface's address, which the identifier is formed from.
    ///
    /// A prefix forms an address when three things hold: the advertisement
    /// set the autonomous flag, the prefix is 64 bits long, and its valid
    /// lifetime is not zero. A link-local prefix is ignored, as
    /// RFC 4862, section 5.5.3 (a) requires — this host has its own
    /// link-local address and does not take one from a router.
    ///
    /// An MTU below the minimum of 1280 is ignored rather than taken: a
    /// link that cannot carry 1280 bytes cannot carry IPv6, and a router
    /// that says so is either wrong or hostile.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::NotDiscovery`] when the message is not a router
    /// advertisement, and [`Ipv6Error::Ip`] carrying
    /// [`IpError::NoRoute`] when there is no room for another prefix or
    /// another server.
    pub fn on_advertisement(
        &mut self,
        from: Ipv6Addr,
        message: &Discovery<'_>,
        hardware: MacAddr,
        now: Instant,
    ) -> Result<(), Ipv6Error> {
        let Discovery::RouterAdvertisement {
            hop_limit,
            router_lifetime,
            options,
            ..
        } = *message
        else {
            return Err(Ipv6Error::NotDiscovery(0));
        };
        if router_lifetime == 0 {
            // RFC 4861, section 6.3.4: a lifetime of zero says this
            // router is not a default router. It may still carry
            // prefixes, so the rest of the message is read.
            self.router = None;
        } else {
            self.router = Some(Router {
                address: from,
                expires_at: expiry(now, u32::from(router_lifetime)),
            });
        }
        if hop_limit != 0 {
            self.hop_limit = hop_limit;
        }
        let identifier = interface_identifier(hardware);
        let mut options = options;
        while let Some(option) = options.next_option() {
            match option {
                NdpOption::Mtu(mtu) => {
                    let mtu = usize::try_from(mtu).unwrap_or(usize::MAX);
                    if mtu >= MIN_MTU {
                        self.link_mtu = Some(mtu);
                    }
                }
                NdpOption::Prefix(information) => {
                    self.learn_prefix(information, identifier, now)?;
                }
                NdpOption::Rdnss(servers) => {
                    let expires_at = expiry(now, servers.lifetime);
                    for address in servers.iter() {
                        self.learn_server(address, servers.lifetime, expires_at)?;
                    }
                }
                NdpOption::SourceLinkLayer(_)
                | NdpOption::TargetLinkLayer(_)
                | NdpOption::Other { .. } => {}
            }
        }
        Ok(())
    }

    /// Takes one prefix option in.
    fn learn_prefix(
        &mut self,
        information: crate::ndp::PrefixInformation,
        identifier: [u8; INTERFACE_ID_LEN],
        now: Instant,
    ) -> Result<(), Ipv6Error> {
        if information.prefix.is_link_local() {
            return Ok(());
        }
        let Ok(prefix) = Ipv6Cidr::new(information.prefix, information.prefix_len) else {
            return Ok(());
        };
        if information.valid_lifetime == 0 {
            self.forget_prefix(prefix);
            return Ok(());
        }
        let forms_address = information.autonomous && information.prefix_len == SLAAC_PREFIX_LEN;
        let entry = Configured {
            prefix,
            address: forms_address.then(|| address_from(information.prefix, identifier)),
            on_link: information.on_link,
            valid_until: expiry(now, information.valid_lifetime),
            preferred_until: expiry(now, information.preferred_lifetime),
        };
        if let Some(existing) = self
            .prefixes
            .iter_mut()
            .find(|existing| existing.prefix == prefix)
        {
            *existing = entry;
            return Ok(());
        }
        self.prefixes
            .push(entry)
            .map_err(|_| Ipv6Error::Ip(IpError::NoRoute))
    }

    /// Takes one DNS server in.
    fn learn_server(
        &mut self,
        address: Ipv6Addr,
        lifetime: u32,
        expires_at: Instant,
    ) -> Result<(), Ipv6Error> {
        if lifetime == 0 {
            // RFC 8106, section 5.1: a lifetime of zero says stop using
            // these addresses.
            self.forget_server(address);
            return Ok(());
        }
        if let Some(existing) = self
            .servers
            .iter_mut()
            .find(|existing| existing.address == address)
        {
            existing.expires_at = expires_at;
            return Ok(());
        }
        self.servers
            .push(Server {
                address,
                expires_at,
            })
            .map_err(|_| Ipv6Error::Ip(IpError::NoRoute))
    }

    /// Drops the prefix and the address under it.
    fn forget_prefix(&mut self, prefix: Ipv6Cidr) {
        if let Some((index, _)) = self
            .prefixes
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.prefix == prefix)
        {
            self.prefixes.remove(index);
        }
    }

    /// Drops the server.
    fn forget_server(&mut self, address: Ipv6Addr) {
        if let Some((index, _)) = self
            .servers
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.address == address)
        {
            self.servers.remove(index);
        }
    }

    /// When something next runs out, or `None` when nothing does.
    #[must_use]
    pub fn poll_at(&self) -> Option<Instant> {
        let router = self.router.iter().map(|router| router.expires_at);
        let prefixes = self.prefixes.iter().map(|entry| entry.valid_until);
        let servers = self.servers.iter().map(|entry| entry.expires_at);
        router
            .chain(prefixes)
            .chain(servers)
            .filter(|deadline| *deadline != Instant::MAX)
            .min()
    }

    /// The next thing that has run out at `now`, or `None` when nothing
    /// has.
    ///
    /// A caller drives this in a loop until it answers `None`, the way it
    /// drives the neighbor cache, because one instant can be the end of
    /// several things at once. What comes back is what to undo: a route
    /// to remove, an address to stop using, a server to stop asking.
    pub fn poll(&mut self, now: Instant) -> Option<Expired> {
        if let Some(router) = self.router
            && router.expires_at <= now
        {
            self.router = None;
            return Some(Expired::Router(router.address));
        }
        if let Some((index, entry)) = self
            .prefixes
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.valid_until <= now)
        {
            let prefix = entry.prefix;
            self.prefixes.remove(index);
            return Some(Expired::Prefix(prefix));
        }
        let (index, entry) = self
            .servers
            .iter()
            .enumerate()
            .find(|(_, entry)| entry.expires_at <= now)?;
        let address = entry.address;
        self.servers.remove(index);
        Some(Expired::Server(address))
    }

    /// Writes what this host learned into a routing table: a default
    /// route through the router, and an on-link route for every prefix
    /// that said it is on this link.
    ///
    /// Adding replaces the route for the same prefix rather than
    /// appending, so calling this after every advertisement is an update
    /// and not a table that grows. What it does not do is remove: a
    /// prefix that has run out is gone from this configuration by the
    /// time the caller hears about it, so the caller removes that route
    /// from what [`poll`](Self::poll) handed it.
    ///
    /// # Errors
    ///
    /// [`IpError::NoRoute`] when the table is full.
    pub fn install<const ROUTES: usize>(
        &self,
        table: &mut RoutingTable<ROUTES>,
    ) -> Result<(), IpError> {
        let default = IpCidr::V6(Ipv6Cidr::new(Ipv6Addr::UNSPECIFIED, 0)?);
        match self.router {
            Some(router) => table.add(Route::via(default, router.address.into()))?,
            None => {
                table.remove(default);
            }
        }
        for entry in self.prefixes.iter().filter(|entry| entry.on_link) {
            table.add(Route::on_link(IpCidr::V6(entry.prefix)))?;
        }
        Ok(())
    }
}

/// Where duplicate address detection has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DadState {
    /// Solicitations are still going out, and the address is not assigned
    /// to the interface: RFC 4862, section 5.4 has a tentative address
    /// answer nothing and be the source of nothing.
    Tentative,
    /// Nobody answered, and the address may be used.
    Unique,
    /// Somebody else holds it.
    Duplicate,
}

/// The number of solicitations RFC 4862, section 5.1 sends by default.
pub const DUP_ADDR_DETECT_TRANSMITS: u8 = 1;

/// Duplicate address detection for one address, per RFC 4862, section 5.4.
///
/// The check is a neighbor solicitation for the address this host wants,
/// sent from the unspecified address to the address's own solicited-node
/// group. Whoever holds it answers, and that answer is the whole of the
/// evidence: nobody answering is not proof that nobody holds it, which
/// the document says plainly, and this implementation says so too rather
/// than implying more.
#[derive(Clone, Copy, Debug)]
pub struct Dad {
    /// The address being checked.
    address: Ipv6Addr,
    /// Where the check has got to.
    state: DadState,
    /// How many solicitations have gone out.
    sent: u8,
    /// How many go out in all.
    transmits: u8,
    /// How long between two of them, and how long the last one is waited
    /// out.
    retransmit: Duration,
    /// When the next thing is due.
    deadline: Instant,
}

/// What duplicate address detection wants done, one thing at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DadEvent {
    /// Send a solicitation for `target` to `group`, from the unspecified
    /// address and with no source link-layer option, as RFC 4862,
    /// section 5.4.2 requires.
    Solicit {
        /// The address being checked.
        target: Ipv6Addr,
        /// The solicited-node group it is asked for at.
        group: Ipv6Addr,
    },
    /// Nobody answered. The address may be assigned.
    Unique(Ipv6Addr),
    /// Somebody answered. The address may not be used, and RFC 4862,
    /// section 5.4.5 has the interface left without one rather than
    /// guessing another.
    Duplicate(Ipv6Addr),
}

impl Dad {
    /// The check for `address`, on the defaults of RFC 4862 and RFC 4861:
    /// one solicitation, one second to answer it.
    #[must_use]
    pub const fn new(address: Ipv6Addr, now: Instant) -> Dad {
        Dad::with(
            address,
            DUP_ADDR_DETECT_TRANSMITS,
            Duration::from_secs(1),
            now,
        )
    }

    /// The check on a schedule of the caller's own.
    ///
    /// `transmits` of zero is RFC 4862's way of switching the check off,
    /// and it makes the address unique at once.
    #[must_use]
    pub const fn with(address: Ipv6Addr, transmits: u8, retransmit: Duration, now: Instant) -> Dad {
        Dad {
            address,
            state: DadState::Tentative,
            sent: 0,
            transmits,
            retransmit,
            // Due at once, so the first solicitation comes out of the
            // caller's next poll and not out of a second entry point.
            deadline: now,
        }
    }

    /// The address being checked.
    #[must_use]
    pub const fn address(&self) -> Ipv6Addr {
        self.address
    }

    /// Where the check has got to.
    #[must_use]
    pub const fn state(&self) -> DadState {
        self.state
    }

    /// When there is next work, or `None` when the check is over.
    #[must_use]
    pub const fn poll_at(&self) -> Option<Instant> {
        match self.state {
            DadState::Tentative => Some(self.deadline),
            DadState::Unique | DadState::Duplicate => None,
        }
    }

    /// The next thing to do at `now`, or `None` when there is nothing.
    pub fn poll(&mut self, now: Instant) -> Option<DadEvent> {
        if self.state != DadState::Tentative || self.deadline > now {
            return None;
        }
        if self.sent >= self.transmits {
            self.state = DadState::Unique;
            return Some(DadEvent::Unique(self.address));
        }
        self.sent = self.sent.saturating_add(1);
        self.deadline = now.saturating_add(self.retransmit);
        Some(DadEvent::Solicit {
            target: self.address,
            group: self.address.solicited_node(),
        })
    }

    /// A neighbor advertisement arrived. If it answers for the address
    /// being checked, that address is somebody else's.
    ///
    /// The answer says whether this ended the check.
    pub fn on_advertisement(&mut self, message: &Discovery<'_>) -> bool {
        let Discovery::NeighborAdvertisement { target, .. } = *message else {
            return false;
        };
        self.on_claim(target)
    }

    /// A neighbor solicitation arrived. One from the unspecified address
    /// for the address being checked is another node running the same
    /// check at the same time, and RFC 4862, section 5.4.3 has both of
    /// them give the address up.
    ///
    /// The answer says whether this ended the check.
    pub fn on_solicitation(&mut self, source: Ipv6Addr, message: &Discovery<'_>) -> bool {
        let Discovery::NeighborSolicitation { target, .. } = *message else {
            return false;
        };
        if !source.is_unspecified() {
            // A solicitation from a node that has an address is that node
            // asking who holds this one, which is not a claim on it.
            return false;
        }
        self.on_claim(target)
    }

    /// Somebody claimed `target`.
    fn on_claim(&mut self, target: Ipv6Addr) -> bool {
        if self.state != DadState::Tentative || target != self.address {
            return false;
        }
        self.state = DadState::Duplicate;
        true
    }
}
