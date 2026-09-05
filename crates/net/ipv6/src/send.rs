// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The way out: one call that finds the route, resolves the neighbor,
//! cuts the packet to what the path carries, and writes the frames.
//!
//! What it joins is already written: the routing table of `net-ip`, the
//! neighbor cache of `net-eth`, the fragmentation arithmetic of
//! `net_ip::Fragments`, and the path MTU table of [`crate::pmtu`]. This
//! module is the IPv6 header and the fragment header, and nothing else.
//!
//! The path MTU is not a parameter a caller may forget. The sender holds
//! the table and asks it for every packet, so a packet larger than what
//! the path is known to carry is one this crate cannot write. That is
//! deliberate: no router will fragment for this host, so an MTU that is
//! not consulted is a packet that vanishes.
//!
//! A multicast destination is resolved without asking anything. RFC 2464,
//! section 7 maps the group onto an Ethernet address by arithmetic, so
//! there is no neighbor to look up and no route to find — which is what
//! lets a neighbor solicitation go out to a station this host knows
//! nothing about, including its own solicited-node group during duplicate
//! address detection.
//!
//! Nothing is sent here. Each finished frame is handed to a closure the
//! caller supplied, which is what keeps this crate free of a device.

use audhsos_time::Instant;
use net_eth::{Frame, NeighborCache, Resolution, multicast_hardware};
use net_ip::{Fragments, Interface, RoutingTable, Sent};
use net_wire::{EtherType, IpAddr, Ipv6Addr, MacAddr, Protocol, Writer};

use crate::error::Ipv6Error;
use crate::header::{FRAGMENT_HEADER_LEN, FragmentHeader, HEADER_LEN, Header};
use crate::pmtu::PathMtu;

/// What is being sent, apart from the payload itself.
///
/// The addresses are IPv6 addresses and not [`net_wire::IpAddr`], where
/// `net_ip::Outgoing` takes the wider type. The family is decided once,
/// by the facade of 12.6.12 that picks between the two senders, and
/// carrying it further down would only make an error arm here that
/// nothing can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Outgoing {
    /// Which of this host's addresses it leaves from.
    pub source: Ipv6Addr,
    /// Where it goes.
    pub destination: Ipv6Addr,
    /// What the payload is.
    pub protocol: Protocol,
    /// Which datagram it is, for reassembly at the other end. Thirty-two
    /// bits, where IPv4 has sixteen. The caller counts this up; nothing
    /// here holds state to count with.
    pub identification: u32,
}

/// The interface, the two tables, and the path MTU estimates a packet
/// leaves through.
///
/// It borrows rather than owns, so the routes, the neighbors, and the
/// estimates live wherever the caller keeps them.
#[derive(Debug)]
pub struct Sender<
    'a,
    const ROUTES: usize,
    const ENTRIES: usize,
    const PENDING: usize,
    const PATHS: usize,
> {
    /// What the interface is.
    pub interface: Interface,
    /// Where things go.
    pub routes: &'a RoutingTable<ROUTES>,
    /// Which hardware address holds which internet address.
    pub neighbors: &'a mut NeighborCache<ENTRIES, PENDING>,
    /// What each path is known to carry.
    pub path: &'a PathMtu<PATHS>,
}

impl<const ROUTES: usize, const ENTRIES: usize, const PENDING: usize, const PATHS: usize>
    Sender<'_, ROUTES, ENTRIES, PENDING, PATHS>
{
    /// Sends one packet.
    ///
    /// A multicast destination goes straight out at the address RFC 2464
    /// derives. For anything else the route decides which neighbor to
    /// resolve — the destination itself when it is on this link, the
    /// router otherwise — and when the cache does not know that neighbor,
    /// the packet waits in it and goes out when the advertisement
    /// arrives, which the caller drives through
    /// [`send_pending`](Self::send_pending).
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::Ip`] carrying `IpError::NoRoute` when nothing reaches
    /// the destination, [`Ipv6Error::WouldFragment`] when the path has no
    /// room for a header and eight bytes behind it, and whatever `emit`
    /// returns.
    pub fn send<E>(
        &mut self,
        outgoing: Outgoing,
        payload: &[u8],
        now: Instant,
        buffer: &mut [u8],
        mut emit: E,
    ) -> Result<Sent, Ipv6Error>
    where
        E: FnMut(&[u8]) -> Result<(), Ipv6Error>,
    {
        let mtu = self.path.mtu(outgoing.destination, self.interface.mtu);
        if outgoing.destination.is_multicast() {
            let hardware = multicast_hardware(outgoing.destination);
            let frames = emit_frames(
                self.interface,
                hardware,
                outgoing,
                payload,
                mtu,
                buffer,
                &mut emit,
            )?;
            return Ok(Sent::Frames(frames));
        }
        let next_hop = self.routes.lookup(IpAddr::V6(outgoing.destination))?;
        let neighbor = next_hop.address();
        let Some(hardware) = self.neighbors.hardware(neighbor) else {
            return Ok(hold(self.neighbors, neighbor, payload, now));
        };
        let frames = emit_frames(
            self.interface,
            hardware,
            outgoing,
            payload,
            mtu,
            buffer,
            &mut emit,
        )?;
        Ok(Sent::Frames(frames))
    }

    /// Sends what was waiting for `neighbor`, now that its hardware
    /// address is known, and answers how many frames went.
    ///
    /// A packet held in the cache is held whole, so this cuts it the way
    /// [`send`](Self::send) would have. Answering zero means there was
    /// nothing waiting.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::WouldFragment`] when the held packet does not fit the
    /// path, and whatever `emit` returns.
    pub fn send_pending<E>(
        &mut self,
        neighbor: Ipv6Addr,
        outgoing: Outgoing,
        buffer: &mut [u8],
        mut emit: E,
    ) -> Result<usize, Ipv6Error>
    where
        E: FnMut(&[u8]) -> Result<(), Ipv6Error>,
    {
        let mtu = self.path.mtu(outgoing.destination, self.interface.mtu);
        let address = IpAddr::V6(neighbor);
        let Some((hardware, payload)) = self.neighbors.pending(address) else {
            return Ok(0);
        };
        let frames = emit_frames(
            self.interface,
            hardware,
            outgoing,
            payload,
            mtu,
            buffer,
            &mut emit,
        )?;
        self.neighbors.clear_pending(address);
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
        // ago and was told there is none.
        Resolution::Dropped | Resolution::Deliver(_) => Sent::Dropped,
    }
}

/// Writes the packet as one frame or as several, and answers how many.
///
/// A packet that fits carries no fragment header at all, which is where
/// this differs from IPv4: the fields are in an extension header, so a
/// datagram that was never cut up costs none of them. One that does not
/// fit is cut into pieces that each carry the header, a fragment header,
/// and a multiple of eight bytes.
fn emit_frames<E>(
    interface: Interface,
    hardware: MacAddr,
    outgoing: Outgoing,
    payload: &[u8],
    mtu: usize,
    buffer: &mut [u8],
    emit: &mut E,
) -> Result<usize, Ipv6Error>
where
    E: FnMut(&[u8]) -> Result<(), Ipv6Error>,
{
    let room = buffer.get_mut(..mtu).ok_or(Ipv6Error::WouldFragment {
        length: payload.len(),
        mtu,
    })?;
    let mut frame = [0u8; net_eth::MAX_FRAME_LEN];
    let whole = HEADER_LEN.saturating_add(payload.len());
    if whole <= mtu {
        let header = Header::new(
            outgoing.source,
            outgoing.destination,
            outgoing.protocol,
            payload.len(),
        );
        let mut writer = Writer::new(room);
        header.write(&mut writer)?;
        writer.write_bytes(payload)?;
        let packet = writer.written();
        send_frame(&mut frame, hardware, interface.hardware, packet, emit)?;
        return Ok(1);
    }
    let carried = HEADER_LEN.saturating_add(FRAGMENT_HEADER_LEN);
    let mut pieces = Fragments::with_header(payload.len(), mtu, carried)?;
    let mut written = 0usize;
    while let Some((at, length, more)) = pieces.next() {
        let piece = payload
            .get(at..at.saturating_add(length))
            .ok_or(Ipv6Error::PayloadTooLong(payload.len()))?;
        let header = Header::new(
            outgoing.source,
            outgoing.destination,
            // The base header names the fragment header, and the fragment
            // header names the upper layer: that is the whole of what an
            // extension header is.
            Protocol::FRAGMENT,
            FRAGMENT_HEADER_LEN.saturating_add(length),
        );
        let fragment = FragmentHeader {
            next_header: outgoing.protocol,
            offset: at,
            more,
            identification: outgoing.identification,
        };
        let mut writer = Writer::new(room);
        header.write(&mut writer)?;
        fragment.write(&mut writer)?;
        writer.write_bytes(piece)?;
        let packet = writer.written();
        send_frame(&mut frame, hardware, interface.hardware, packet, emit)?;
        written = written.saturating_add(1);
    }
    Ok(written)
}

/// Wraps one packet in a frame and hands it over.
///
/// The packet is copied into the frame behind its link header, for the
/// reason `net-ip` gives for the same copy: writing the link header into
/// the front of the same buffer would mean this crate knowing the layout
/// of an Ethernet frame instead of asking `net-eth` for it.
fn send_frame<E>(
    frame: &mut [u8; net_eth::MAX_FRAME_LEN],
    destination: MacAddr,
    source: MacAddr,
    packet: &[u8],
    emit: &mut E,
) -> Result<(), Ipv6Error>
where
    E: FnMut(&[u8]) -> Result<(), Ipv6Error>,
{
    let mut writer = Writer::new(frame);
    Frame::write(&mut writer, destination, source, EtherType::IPV6, packet)
        .map_err(|_| Ipv6Error::PayloadTooLong(packet.len()))?;
    emit(writer.written())
}
