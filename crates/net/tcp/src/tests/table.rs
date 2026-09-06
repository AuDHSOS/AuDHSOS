// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The connection table: which connection a segment belongs to, and the
//! reset for a segment that belongs to none.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test counts segments and connections by hand"
)]

use crypto_rng::doubles::ScriptedRng;
use net_wire::{Port, Writer};

use crate::connection::{Config, Endpoint};
use crate::error::TcpError;
use crate::segment::{Flags, Segment};
use crate::seq::SeqNumber;
use crate::state::State;
use crate::table::{ConnectionId, Connections, Delivery};
use crate::tests::harness::{
    CLIENT, CLIENT_END, CLIENT_PORT, SERVER, SERVER_END, SERVER_PORT, config, sequence, start,
};

/// A segment from the client to the server, written into a vector.
fn from_client(flags: Flags, seq: SeqNumber, ack: SeqNumber) -> Vec<u8> {
    let mut segment = Segment::new(CLIENT_PORT, SERVER_PORT, flags);
    segment.seq = seq;
    segment.ack = ack;
    segment.window = 4096;
    let mut bytes = vec![0u8; 64];
    let mut writer = Writer::new(&mut bytes);
    segment.write(&mut writer, CLIENT, SERVER).expect("room");
    let len = writer.position();
    bytes.truncate(len);
    bytes
}

#[test]
fn a_syn_reaches_the_connection_that_is_listening_and_makes_it_that_one() {
    let now = start();
    let (mut s, mut r) = (vec![0u8; 256], vec![0u8; 256]);
    let mut table = Connections::<2>::new();
    assert!(table.is_empty());
    let script = sequence(1000);
    let mut rng = ScriptedRng::new(&script);
    let id = table
        .listen(SERVER_END, config(1460), &mut rng, &mut s, &mut r)
        .expect("a free port");
    assert_eq!(table.len(), 1);

    let syn = from_client(Flags::SYN, SeqNumber::new(5000), SeqNumber::new(0));
    assert_eq!(
        table.receive(CLIENT, SERVER, &syn, now),
        Delivery::Delivered(id)
    );
    let connection = table.get(id).expect("the connection");
    assert_eq!(connection.state(), State::SynReceived);
    assert_eq!(connection.remote(), CLIENT_END);
}

#[test]
fn a_segment_finds_the_connection_that_joins_its_two_ends() {
    let now = start();
    let (mut ls, mut lr) = (vec![0u8; 256], vec![0u8; 256]);
    let (mut cs, mut cr) = (vec![0u8; 256], vec![0u8; 256]);
    let mut table = Connections::<3>::new();
    let script = [sequence(1000), sequence(7000)].concat();
    let mut rng = ScriptedRng::new(&script);
    let listener = table
        .listen(SERVER_END, config(1460), &mut rng, &mut ls, &mut lr)
        .expect("a free port");
    // A connection of this host to the same peer, from another port.
    let other = Endpoint::new(SERVER, Port::new(40_000));
    let joined = table
        .connect(other, CLIENT_END, config(1460), &mut rng, &mut cs, &mut cr)
        .expect("a free tuple");
    assert_ne!(listener, joined);

    // A segment addressed to the second port goes to the second
    // connection, not to the one that is listening.
    let mut segment = Segment::new(CLIENT_PORT, Port::new(40_000), Flags::ACK);
    segment.seq = SeqNumber::new(1);
    segment.window = 4096;
    let mut bytes = vec![0u8; 64];
    let mut writer = Writer::new(&mut bytes);
    segment.write(&mut writer, CLIENT, SERVER).expect("room");
    let len = writer.position();
    assert_eq!(
        table.receive(CLIENT, SERVER, &bytes[..len], now),
        Delivery::Delivered(joined)
    );
}

#[test]
fn a_segment_for_a_port_nobody_holds_is_answered_with_a_reset() {
    let now = start();
    let mut table = Connections::<2>::new();
    // A `SYN` acknowledges nothing, so the reset acknowledges everything
    // the segment occupied instead.
    let syn = from_client(Flags::SYN, SeqNumber::new(5000), SeqNumber::new(0));
    let Delivery::Refused(reset) = table.receive(CLIENT, SERVER, &syn, now) else {
        panic!("a segment to a closed port draws a reset");
    };
    assert_eq!(reset.flags, Flags::RST.with(Flags::ACK));
    assert_eq!(reset.seq, SeqNumber::new(0));
    assert_eq!(reset.ack, SeqNumber::new(5001));
    assert_eq!(reset.source_port, SERVER_PORT);
    assert_eq!(reset.destination_port, CLIENT_PORT);

    // One that does acknowledge carries that number and nothing else.
    let ack = from_client(Flags::ACK, SeqNumber::new(5000), SeqNumber::new(999));
    let Delivery::Refused(reset) = table.receive(CLIENT, SERVER, &ack, now) else {
        panic!("a segment to a closed port draws a reset");
    };
    assert_eq!(reset.flags, Flags::RST);
    assert_eq!(reset.seq, SeqNumber::new(999));
}

#[test]
fn a_reset_to_a_closed_port_is_never_answered_with_another() {
    let now = start();
    let mut table = Connections::<2>::new();
    let reset = from_client(Flags::RST, SeqNumber::new(5000), SeqNumber::new(0));
    assert_eq!(
        table.receive(CLIENT, SERVER, &reset, now),
        Delivery::Dropped
    );
}

#[test]
fn bytes_that_are_not_a_segment_deliver_nothing() {
    let now = start();
    let mut table = Connections::<2>::new();
    assert_eq!(
        table.receive(CLIENT, SERVER, &[0u8; 4], now),
        Delivery::Malformed(TcpError::Short(4))
    );
}

#[test]
fn a_port_is_listened_on_once_and_a_tuple_is_joined_once() {
    let (mut first_send, mut first_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let (mut second_send, mut second_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let (mut third_send, mut third_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let (mut fourth_send, mut fourth_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let mut table = Connections::<4>::new();
    let script = [sequence(1), sequence(2), sequence(3), sequence(4)].concat();
    let mut rng = ScriptedRng::new(&script);
    table
        .listen(
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut first_send,
            &mut first_recv,
        )
        .expect("a free port");
    assert_eq!(
        table.listen(
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut second_send,
            &mut second_recv
        ),
        Err(TcpError::PortInUse(SERVER_PORT))
    );
    table
        .connect(
            CLIENT_END,
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut third_send,
            &mut third_recv,
        )
        .expect("a free tuple");
    assert_eq!(
        table.connect(
            CLIENT_END,
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut fourth_send,
            &mut fourth_recv
        ),
        Err(TcpError::PortInUse(CLIENT_PORT))
    );
}

#[test]
fn a_full_table_takes_no_further_connection() {
    let (mut listen_send, mut listen_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let (mut open_send, mut open_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let mut table = Connections::<1>::new();
    let script = [sequence(1), sequence(2)].concat();
    let mut rng = ScriptedRng::new(&script);
    table
        .listen(
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut listen_send,
            &mut listen_recv,
        )
        .expect("a free slot");
    assert_eq!(
        table.connect(
            CLIENT_END,
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut open_send,
            &mut open_recv
        ),
        Err(TcpError::NoConnection)
    );
}

#[test]
fn a_closed_connection_gives_its_buffers_and_its_slot_back() {
    let (mut send, mut recv) = (vec![0u8; 32], vec![0u8; 64]);
    let mut table = Connections::<1>::new();
    let script = sequence(1);
    let mut rng = ScriptedRng::new(&script);
    let id = table
        .listen(SERVER_END, Config::DEFAULT, &mut rng, &mut send, &mut recv)
        .expect("a free slot");
    let (back_send, back_recv) = table.close(id).expect("an open connection");
    assert_eq!(back_send.len(), 32);
    assert_eq!(back_recv.len(), 64);
    assert!(table.is_empty());
    assert_eq!(table.close(id), Err(TcpError::UnknownConnection));
    assert!(table.get(id).is_none());
    assert_eq!(
        table.close(ConnectionId::new(9)),
        Err(TcpError::UnknownConnection)
    );
}

#[test]
fn the_table_answers_for_the_connection_that_has_work_first() {
    let now = start();
    let (mut listen_send, mut listen_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let (mut open_send, mut open_recv) = (vec![0u8; 64], vec![0u8; 64]);
    let mut table = Connections::<2>::new();
    assert_eq!(table.poll_at(now), None);
    let script = [sequence(1), sequence(2)].concat();
    let mut rng = ScriptedRng::new(&script);
    table
        .listen(
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut listen_send,
            &mut listen_recv,
        )
        .expect("a free slot");
    // A listening connection has nothing to do until a peer arrives.
    assert_eq!(table.poll_at(now), None);
    table
        .connect(
            CLIENT_END,
            SERVER_END,
            Config::DEFAULT,
            &mut rng,
            &mut open_send,
            &mut open_recv,
        )
        .expect("a free slot");
    // The one that just opened has its `SYN` to send, now.
    assert_eq!(table.poll_at(now), Some(now));
    assert_eq!(table.iter_mut().count(), 2);
    let mut buffer = [0u8; 128];
    let sent = table
        .iter_mut()
        .filter_map(|(_, connection)| connection.poll(now, &mut buffer).map(<[u8]>::len))
        .count();
    assert_eq!(sent, 1);
}

#[test]
fn a_table_made_by_default_is_the_empty_one() {
    let table = Connections::<2>::default();
    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
}

#[test]
fn a_segment_for_a_connection_that_was_closed_draws_a_reset() {
    let now = start();
    let (mut send, mut recv) = (vec![0u8; 256], vec![0u8; 256]);
    let mut table = Connections::<2>::new();
    let script = sequence(1000);
    let mut rng = ScriptedRng::new(&script);
    let id = table
        .connect(
            SERVER_END,
            CLIENT_END,
            config(1460),
            &mut rng,
            &mut send,
            &mut recv,
        )
        .expect("a free tuple");
    let connection = table.get_mut(id).expect("the connection");
    connection.abort();
    assert_eq!(connection.state(), State::Closed);

    // The table still holds the entry, but the connection is gone, so a
    // segment for it is a segment for nothing.
    let stray = from_client(Flags::ACK, SeqNumber::new(5000), SeqNumber::new(1001));
    let Delivery::Refused(reset) = table.receive(CLIENT, SERVER, &stray, now) else {
        panic!("a segment for a closed connection draws a reset");
    };
    assert_eq!(reset.flags, Flags::RST);
    assert_eq!(reset.seq, SeqNumber::new(1001));
}
