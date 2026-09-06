// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the connection tests share: the two ends, and the shuttling of
//! segments between them.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test counts segments and bytes by hand"
)]

use audhsos_time::Instant;
use crypto_rng::doubles::ScriptedRng;
use net_wire::{IpAddr, Ipv4Addr, Port, Writer};

use crate::connection::{Config, Connection, Endpoint};
use crate::segment::{Flags, Segment};
use crate::seq::SeqNumber;

/// The client's address.
pub(crate) const CLIENT: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The server's address.
pub(crate) const SERVER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// The client's port.
pub(crate) const CLIENT_PORT: Port = Port::new(49_152);

/// The server's port.
pub(crate) const SERVER_PORT: Port = Port::new(80);

/// The client's end.
pub(crate) const CLIENT_END: Endpoint = Endpoint::new(CLIENT, CLIENT_PORT);

/// The server's end.
pub(crate) const SERVER_END: Endpoint = Endpoint::new(SERVER, SERVER_PORT);

/// How many segments one `poll` loop may produce before a test calls it a
/// runaway.
const MAX_SEGMENTS: usize = 64;

/// The instant every test starts at.
pub(crate) fn start() -> Instant {
    Instant::from_micros(1_000_000)
}

/// A generator that hands out `value` as an initial sequence number, and
/// then refuses.
pub(crate) fn sequence(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

/// A configuration with the segment size a test wants and no waiting for
/// full segments.
pub(crate) fn config(max_segment: u16) -> Config {
    Config {
        max_segment,
        nagle: false,
        ..Config::DEFAULT
    }
}

/// Everything the connection has to say at `now`.
pub(crate) fn drain(connection: &mut Connection<'_>, now: Instant) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer = [0u8; 2048];
    while let Some(segment) = connection.poll(now, &mut buffer) {
        out.push(segment.to_vec());
        assert!(
            out.len() <= MAX_SEGMENTS,
            "a poll loop produced {MAX_SEGMENTS} segments and did not stop"
        );
    }
    out
}

/// The one segment the connection has to say, and a failure when it has
/// none or more than one.
pub(crate) fn one(connection: &mut Connection<'_>, now: Instant) -> Vec<u8> {
    let mut segments = drain(connection, now);
    assert_eq!(segments.len(), 1, "expected exactly one segment");
    segments.remove(0)
}

/// Reads the bytes as a segment sent from `from` to `to`.
pub(crate) fn read(bytes: &[u8], from: IpAddr, to: IpAddr) -> Segment<'_> {
    Segment::parse(bytes, from, to).expect("a segment")
}

/// Hands `bytes`, which came from `from`, to the connection.
pub(crate) fn feed(connection: &mut Connection<'_>, from: IpAddr, bytes: &[u8], now: Instant) {
    let to = connection.local().address;
    let segment = read(bytes, from, to);
    connection.on_segment(from, &segment, now);
}

/// Carries every segment one end has to say over to the other.
pub(crate) fn exchange(from: &mut Connection<'_>, to: &mut Connection<'_>, now: Instant) -> usize {
    let source = from.local().address;
    let segments = drain(from, now);
    let count = segments.len();
    for bytes in segments {
        feed(to, source, &bytes, now);
    }
    count
}

/// Opens a connection between the two, and leaves both established.
pub(crate) fn open(
    client: &mut Connection<'_>,
    server: &mut Connection<'_>,
    rng: &mut ScriptedRng<'_>,
    now: Instant,
) {
    client.connect(SERVER_END, rng).expect("an open");
    // SYN, then SYN with ACK, then ACK.
    exchange(client, server, now);
    exchange(server, client, now);
    exchange(client, server, now);
}

/// Two ends, opened, with the receive buffer of the client sized by the
/// caller. The generator, the buffers, and the script live as locals of
/// the test the macro is used in.
macro_rules! pair {
    ($client:ident, $server:ident, $now:ident, $recv:expr, $segment:expr) => {
        let mut client_send = vec![0u8; 512];
        let mut client_recv = vec![0u8; $recv];
        let mut server_send = vec![0u8; 512];
        let mut server_recv = vec![0u8; 512];
        let mut $client = crate::connection::Connection::new(
            crate::tests::harness::CLIENT_END,
            crate::tests::harness::config($segment),
            &mut client_send,
            &mut client_recv,
        );
        let mut $server = crate::connection::Connection::new(
            crate::tests::harness::SERVER_END,
            crate::tests::harness::config($segment),
            &mut server_send,
            &mut server_recv,
        );
        let script = [
            crate::tests::harness::sequence(1000),
            crate::tests::harness::sequence(5000),
        ]
        .concat();
        let mut rng = crypto_rng::doubles::ScriptedRng::new(&script);
        $server.listen(&mut rng).expect("a listen");
        crate::tests::harness::open(&mut $client, &mut $server, &mut rng, $now);
    };
}

pub(crate) use pair;

/// A segment from the server carrying `payload`, with the numbers and the
/// window a test wants.
pub(crate) fn from_server(
    seq: SeqNumber,
    ack: SeqNumber,
    window: u16,
    flags: Flags,
    payload: &[u8],
) -> Vec<u8> {
    let mut segment = Segment::new(SERVER_PORT, CLIENT_PORT, flags);
    segment.seq = seq;
    segment.ack = ack;
    segment.window = window;
    segment.payload = payload;
    let mut bytes = vec![0u8; 2048];
    let mut writer = Writer::new(&mut bytes);
    segment.write(&mut writer, SERVER, CLIENT).expect("room");
    let len = writer.position();
    bytes.truncate(len);
    bytes
}
