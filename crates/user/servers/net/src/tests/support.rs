// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two servers on one link, and what it takes to build one.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a frame at known offsets"
)]

use audhsos_abi::Handle;
use audhsos_time::Instant;
use crypto_rng::ChaChaRng;
use crypto_rng::doubles::CountingEntropy;
use net_stack::{Config, FRAME_LEN};
use net_wire::{IpAddr, Ipv4Addr, Ipv4Cidr, MacAddr};
use user_proto::ring::SocketPage;

use crate::memory::REGION_BYTES;
use crate::server::{Rings, Server};
use crate::sockets::MAX_SOCKETS;

/// The generator the tests draw from, which never runs out.
pub(crate) type Rng = ChaChaRng<CountingEntropy>;

/// A generator seeded with a number no test depends on.
pub(crate) fn rng(seed: u8) -> Rng {
    ChaChaRng::from_seed(&[seed; 32], CountingEntropy::new(seed))
}

/// The process handle an opening request carries, which the server
/// leaves to the program around it.
pub(crate) fn process() -> Handle {
    Handle::new(9, 1).expect("a handle")
}

/// The network both test hosts are on.
pub(crate) const LINK: Ipv4Cidr = match Ipv4Cidr::new(Ipv4Addr::new(10, 0, 0, 0), 24) {
    Ok(network) => network,
    Err(_) => panic!("a prefix of twenty-four bits"),
};

/// The port the listening host takes.
pub(crate) const PORT: u16 = 7;

/// One host: a server, and the rings of its sockets.
pub(crate) struct Host {
    /// The server.
    pub(crate) server: Server<'static>,
    /// The rings of its sockets, for a test that reads them.
    pub(crate) pages: [&'static SocketPage; MAX_SOCKETS],
}

/// A host with no address and no route, which is what a machine that has
/// just started is.
pub(crate) fn bare(mac: MacAddr) -> Host {
    let bytes: &'static mut [u8] = Box::leak(vec![0u8; REGION_BYTES].into_boxed_slice());
    let pages: [&'static SocketPage; MAX_SOCKETS] =
        core::array::from_fn(|_| &*Box::leak(Box::new(SocketPage::new())));
    let rings: [Rings<'static>; MAX_SOCKETS] = core::array::from_fn(|index| Rings {
        page: pages[index],
        object: Handle::new(u32::try_from(index).unwrap_or(0), 1).expect("a handle"),
    });
    let server = Server::new(Config::new(mac), bytes, rings).expect("the region is long enough");
    Host { server, pages }
}

/// A host at `address` with the hardware address `mac`, its route to the
/// link already in.
pub(crate) fn host(mac: MacAddr, address: Ipv4Addr) -> Host {
    let bytes: &'static mut [u8] = Box::leak(vec![0u8; REGION_BYTES].into_boxed_slice());
    let pages: [&'static SocketPage; MAX_SOCKETS] =
        core::array::from_fn(|_| &*Box::leak(Box::new(SocketPage::new())));
    let rings: [Rings<'static>; MAX_SOCKETS] = core::array::from_fn(|index| Rings {
        page: pages[index],
        object: Handle::new(u32::try_from(index).unwrap_or(0), 1).expect("a handle"),
    });
    let mut server =
        Server::new(Config::new(mac), bytes, rings).expect("the region is long enough");
    server
        .stack()
        .add_address(IpAddr::V4(address))
        .expect("an address");
    server
        .stack()
        .add_route(net_ip_route())
        .expect("a route to the link");
    Host { server, pages }
}

/// The route to the link both hosts are on.
fn net_ip_route() -> net_ip::Route {
    net_ip::Route::on_link(net_wire::IpCidr::V4(LINK))
}

/// How far the clock moves between two rounds of an exchange.
///
/// A round is a poll of each host, and the timers under them — the
/// delayed acknowledgment of RFC 9293 among them — are what a window
/// update waits on, so a clock that stands still is a link that stalls.
pub(crate) const STEP: u64 = 50_000;

/// Polls both hosts until neither has anything to say, carrying every
/// frame of one to the other, and answers the instant it reached.
///
/// The link neither delays nor drops: what this stands for is two stations
/// that reach each other, so that what a test sees is the doing of the
/// server and not of the link.
pub(crate) fn exchange(one: &mut Host, two: &mut Host, from: Instant, rng: &mut Rng) -> Instant {
    let mut turns = 0usize;
    let mut quiet = 0usize;
    let mut now = from;
    let mut pending: Vec<(usize, Vec<u8>)> = Vec::new();
    loop {
        turns += 1;
        assert!(turns <= 1024, "the two hosts never fell silent");
        now = Instant::from_micros(now.as_micros().saturating_add(STEP));
        let mut moved = false;
        for (index, host) in [&mut *one, &mut *two].into_iter().enumerate() {
            let taken = pending
                .iter()
                .position(|(to, _)| *to == index)
                .map(|at| pending.remove(at));
            let mut tx = [0u8; FRAME_LEN];
            let frame = taken.as_ref().map(|(_, bytes)| bytes.as_slice());
            let had = frame.is_some();
            let answer = host
                .server
                .poll(now, frame, &mut tx, rng)
                .expect("a poll of the server");
            if let Some(out) = answer {
                pending.push((1 - index, out.to_vec()));
                moved = true;
            }
            moved = moved || had;
        }
        // A poll that hands out nothing is not a host with nothing to do:
        // what one layer writes, the next poll is what carries out. So a
        // round that moved nothing is counted and the loop ends only when
        // several in a row do.
        if moved {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= QUIET_ROUNDS {
                return now;
            }
        }
    }
}

/// How many rounds that move nothing end an exchange.
const QUIET_ROUNDS: usize = 4;

/// Polls `host` against the link host of [`super::link`] until neither has
/// anything to say, and answers the instant it reached.
///
/// What this stands for is the built-in server of QEMU's user-mode
/// network: a station that answers a discover, a request, and one name,
/// and drops everything else.
pub(crate) fn against_the_link(host: &mut Host, from: Instant, rng: &mut Rng) -> Instant {
    let mut now = from;
    let mut quiet = 0usize;
    let mut turns = 0usize;
    let mut pending: Option<Vec<u8>> = None;
    loop {
        turns += 1;
        assert!(turns <= 2048, "the host never fell silent");
        now = Instant::from_micros(now.as_micros().saturating_add(STEP));
        let mut tx = [0u8; FRAME_LEN];
        let taken = pending.take();
        let frame = taken.as_deref();
        let had = frame.is_some();
        let answer = host
            .server
            .poll(now, frame, &mut tx, rng)
            .expect("a poll of the server");
        let mut moved = had;
        if let Some(out) = answer {
            let mut reply = [0u8; FRAME_LEN];
            if let Some(len) = super::link::answer(out, &mut reply) {
                pending = Some(reply.get(..len).unwrap_or(&[]).to_vec());
            }
            moved = true;
        }
        if moved {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet >= QUIET_ROUNDS {
                return now;
            }
        }
    }
}
