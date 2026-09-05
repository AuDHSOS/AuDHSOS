// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The way out: one call that finds the route, resolves the neighbor, and
//! writes the frames.
//!
//! This is what a transport calls. It takes an [`IpAddr`] pair and a
//! protocol and knows nothing about which family will carry the datagram,
//! so a socket is written once (D-69). What it joins is already written:
//! the routing table of [`crate::route`], the neighbor cache of
//! `net-eth`, and the fragmenter of [`mod@crate::fragment`].
//!
//! Nothing is sent here. Each finished frame is handed to a closure the
//! caller supplied, which is what keeps this crate free of a device: the
//! driver decides what a frame costs and when it goes.

use audhsos_time::Instant;
use net_eth::{Frame, NeighborCache, Resolution};
use net_wire::{EtherType, IpAddr, MacAddr, Protocol};

use crate::error::IpError;
use crate::fragment::fragment;
use crate::header::Header;
use crate::route::RoutingTable;

/// What became of a datagram handed to [`Sender::send`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sent {
    /// It went out, in this many frames.
    Frames(usize),
    /// The neighbor is not known yet. The datagram is held in the cache
    /// and goes when the answer arrives; the caller emits whatever
    /// [`NeighborCache::poll`](net_eth::NeighborCache::poll) asks for.
    Resolving,
    /// The neighbor is not known and the datagram could not be held,
    /// because it is longer than the cache keeps or there was no room.
    Dropped,
}

/// What the interface is: its hardware address, its own addresses, and
/// what it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The hardware address frames leave with.
    pub hardware: MacAddr,
    /// The largest payload a frame carries.
    pub mtu: usize,
}

/// What is being sent, apart from the payload itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Outgoing {
    /// Which of this host's addresses it leaves from.
    pub source: IpAddr,
    /// Where it goes.
    pub destination: IpAddr,
    /// What the payload is.
    pub protocol: Protocol,
    /// Which datagram it is, for reassembly at the other end. The caller
    /// counts this up; nothing here holds state to count with.
    pub identification: u16,
    /// Whether it may be cut up on the way out.
    pub dont_fragment: bool,
}

/// The interface and the two tables a datagram leaves through.
///
/// It borrows rather than owns, so the routes and the neighbors live
/// wherever the caller keeps them and one send does not move them.
#[derive(Debug)]
pub struct Sender<'a, const ROUTES: usize, const ENTRIES: usize, const PENDING: usize> {
    /// What the interface is.
    pub interface: Interface,
    /// Where things go.
    pub routes: &'a RoutingTable<ROUTES>,
    /// Which hardware address holds which internet address.
    pub neighbors: &'a mut NeighborCache<ENTRIES, PENDING>,
}

impl<const ROUTES: usize, const ENTRIES: usize, const PENDING: usize>
    Sender<'_, ROUTES, ENTRIES, PENDING>
{
    /// Sends one datagram.
    ///
    /// The route decides which neighbor to resolve — the destination
    /// itself when it is on this link, the router otherwise. When the
    /// cache knows that neighbor, the datagram is cut to the MTU and each
    /// piece is handed to `emit` as a complete frame. When it does not,
    /// the datagram waits in the cache and goes out when the answer
    /// arrives, which the caller drives through
    /// [`send_pending`](Self::send_pending).
    ///
    /// # Errors
    ///
    /// [`IpError::NoRoute`] when nothing reaches the destination or the
    /// pair is IPv6, which `net-ipv6` sends instead,
    /// [`IpError::WouldFragment`] when the datagram is too
    /// long for the MTU and may not be cut, and whatever `emit` returns.
    pub fn send<E>(
        &mut self,
        outgoing: Outgoing,
        payload: &[u8],
        now: Instant,
        buffer: &mut [u8],
        mut emit: E,
    ) -> Result<Sent, IpError>
    where
        E: FnMut(&[u8]) -> Result<(), IpError>,
    {
        let (IpAddr::V4(source), IpAddr::V4(destination)) = (outgoing.source, outgoing.destination)
        else {
            // IPv6 leaves through `net_ipv6::Sender`, which writes its own
            // header and then asks this same routing table and the same
            // neighbor cache. This one carries the family it has a header
            // for, and the facade of 12.6.12 picks between the two.
            return Err(IpError::NoRoute);
        };
        let next_hop = self.routes.lookup(outgoing.destination)?;
        let neighbor = next_hop.address();

        let mut header = Header::new(source, destination, outgoing.protocol, payload.len());
        header.identification = outgoing.identification;
        header.dont_fragment = outgoing.dont_fragment;

        let Some(hardware) = self.neighbors.hardware(neighbor) else {
            return Ok(hold(self.neighbors, neighbor, payload, now));
        };
        let frames = emit_frames(
            self.interface,
            hardware,
            &header,
            payload,
            buffer,
            &mut emit,
        )?;
        Ok(Sent::Frames(frames))
    }

    /// Sends what was waiting for `neighbor`, now that its hardware
    /// address is known, and answers how many frames went.
    ///
    /// A datagram held in the cache is held whole, so this cuts it the way
    /// [`send`](Self::send) would have. The held packet is cleared once it
    /// is out. Answering zero means there was nothing waiting.
    ///
    /// # Errors
    ///
    /// [`IpError::WouldFragment`] when the held datagram does not fit the
    /// MTU, and whatever `emit` returns.
    pub fn send_pending<E>(
        &mut self,
        neighbor: IpAddr,
        outgoing: Outgoing,
        buffer: &mut [u8],
        mut emit: E,
    ) -> Result<usize, IpError>
    where
        E: FnMut(&[u8]) -> Result<(), IpError>,
    {
        let (IpAddr::V4(source), IpAddr::V4(destination)) = (outgoing.source, outgoing.destination)
        else {
            return Ok(0);
        };
        let Some((hardware, payload)) = self.neighbors.pending(neighbor) else {
            return Ok(0);
        };
        let mut header = Header::new(source, destination, outgoing.protocol, payload.len());
        header.identification = outgoing.identification;
        header.dont_fragment = outgoing.dont_fragment;
        let frames = emit_frames(
            self.interface,
            hardware,
            &header,
            payload,
            buffer,
            &mut emit,
        )?;
        self.neighbors.clear_pending(neighbor);
        Ok(frames)
    }
}

/// Holds `payload` in the cache and answers what became of it.
fn hold<const ENTRIES: usize, const PENDING: usize>(
    neighbors: &mut NeighborCache<ENTRIES, PENDING>,
    neighbor: IpAddr,
    payload: &[u8],
    now: Instant,
) -> Sent {
    match neighbors.resolve(neighbor, payload, now) {
        Resolution::Waiting => Sent::Resolving,
        // `Dropped` is the cache saying no, and `Deliver` cannot happen:
        // the caller asked the same cache for a hardware address a moment
        // ago and was told there is none. Both leave nothing held.
        Resolution::Dropped | Resolution::Deliver(_) => Sent::Dropped,
    }
}

/// Writes the datagram as one frame or as several, and answers how many.
///
/// The datagram is built in the caller's buffer and then copied into the
/// frame behind its link header. That copy is deliberate: the alternative
/// is to write the link header into the front of the same buffer and let
/// the fragmenter write behind it, which would mean this module knowing
/// the layout of an Ethernet frame instead of asking `net-eth` for it.
/// One memcpy of at most an MTU is the cheaper of the two, on a path
/// where the driver will copy again anyway.
fn emit_frames<E>(
    interface: Interface,
    hardware: MacAddr,
    header: &Header,
    payload: &[u8],
    buffer: &mut [u8],
    emit: &mut E,
) -> Result<usize, IpError>
where
    E: FnMut(&[u8]) -> Result<(), IpError>,
{
    let datagram = buffer
        .get_mut(..interface.mtu)
        .ok_or(IpError::TooLarge(interface.mtu))?;
    let mut frame = [0u8; net_eth::MAX_FRAME_LEN];
    fragment(header, payload, interface.mtu, datagram, |piece| {
        let mut writer = net_wire::Writer::new(&mut frame);
        Frame::write(
            &mut writer,
            hardware,
            interface.hardware,
            EtherType::IPV4,
            piece,
        )
        .map_err(|_| IpError::TooLarge(piece.len()))?;
        emit(writer.written())
    })
}
