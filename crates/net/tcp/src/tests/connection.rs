// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The state machine: opening, carrying bytes, and closing.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test counts segments and bytes by hand"
)]

use audhsos_time::Duration;
use crypto_rng::doubles::ScriptedRng;
use net_wire::{Port, Writer};

use crate::connection::{Config, Connection, DEFAULT_MAX_SEGMENT, Endpoint, MAX_SEGMENT_LIFETIME};
use crate::error::TcpError;
use crate::segment::{Flags, Segment};
use crate::seq::SeqNumber;
use crate::state::State;
use crate::tests::harness::{
    CLIENT, CLIENT_END, CLIENT_PORT, SERVER, SERVER_END, SERVER_PORT, config, drain, exchange,
    feed, from_server, one, open, pair, read, sequence, start,
};

#[test]
fn the_three_way_handshake_opens_both_ends() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);

    server.listen(&mut rng).expect("a listen");
    assert_eq!(server.state(), State::Listen);
    client.connect(SERVER_END, &mut rng).expect("an open");
    assert_eq!(client.state(), State::SynSent);

    // The `SYN`, with the segment size this end will accept.
    let syn = one(&mut client, now);
    let parsed = read(&syn, CLIENT, SERVER);
    assert_eq!(parsed.flags, Flags::SYN);
    assert_eq!(parsed.seq, SeqNumber::new(5000));
    assert_eq!(parsed.max_segment_size, Some(1460));
    feed(&mut server, CLIENT, &syn, now);
    assert_eq!(server.state(), State::SynReceived);
    assert_eq!(server.remote(), CLIENT_END);

    // The `SYN` with `ACK`.
    let syn_ack = one(&mut server, now);
    let parsed = read(&syn_ack, SERVER, CLIENT);
    assert_eq!(parsed.flags, Flags::SYN.with(Flags::ACK));
    assert_eq!(parsed.seq, SeqNumber::new(1000));
    assert_eq!(parsed.ack, SeqNumber::new(5001));
    feed(&mut client, SERVER, &syn_ack, now);
    assert_eq!(client.state(), State::Established);

    // The acknowledgment that opens the server's half.
    let ack = one(&mut client, now);
    let parsed = read(&ack, CLIENT, SERVER);
    assert_eq!(parsed.flags, Flags::ACK);
    assert_eq!(parsed.seq, SeqNumber::new(5001));
    assert_eq!(parsed.ack, SeqNumber::new(1001));
    feed(&mut server, CLIENT, &ack, now);
    assert_eq!(server.state(), State::Established);
    assert!(drain(&mut server, now).is_empty());
}

#[test]
fn two_ends_that_open_at_once_both_reach_established() {
    let now = start();
    let (mut ls, mut lr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut rs, mut rr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut left = Connection::new(CLIENT_END, config(1460), &mut ls, &mut lr);
    let mut right = Connection::new(SERVER_END, config(1460), &mut rs, &mut rr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    left.connect(SERVER_END, &mut rng).expect("an open");
    right.connect(CLIENT_END, &mut rng).expect("an open");

    // Each sends its own `SYN` before it sees the other's.
    let left_syn = one(&mut left, now);
    let right_syn = one(&mut right, now);
    feed(&mut left, SERVER, &right_syn, now);
    feed(&mut right, CLIENT, &left_syn, now);
    assert_eq!(left.state(), State::SynReceived);
    assert_eq!(right.state(), State::SynReceived);

    exchange(&mut left, &mut right, now);
    exchange(&mut right, &mut left, now);
    assert_eq!(left.state(), State::Established);
    assert_eq!(right.state(), State::Established);
}

#[test]
fn a_segment_that_acknowledges_what_was_never_sent_draws_a_reset() {
    let now = start();
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = sequence(1000);
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");

    // A bare acknowledgment reaching a connection that has sent nothing.
    let mut stray = Segment::new(CLIENT_PORT, SERVER_PORT, Flags::ACK);
    stray.ack = SeqNumber::new(777);
    let mut bytes = [0u8; 64];
    let mut writer = Writer::new(&mut bytes);
    stray.write(&mut writer, CLIENT, SERVER).expect("room");
    let len = writer.position();
    feed(&mut server, CLIENT, &bytes[..len], now);

    let reset = one(&mut server, now);
    let parsed = read(&reset, SERVER, CLIENT);
    assert!(parsed.flags.has(Flags::RST));
    assert_eq!(parsed.seq, SeqNumber::new(777));
    assert_eq!(server.state(), State::Listen);
}

#[test]
fn a_syn_answered_with_an_impossible_acknowledgment_draws_a_reset() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let script = sequence(5000);
    let mut rng = ScriptedRng::new(&script);
    client.connect(SERVER_END, &mut rng).expect("an open");
    drain(&mut client, now);

    let mut answer = Segment::new(SERVER_PORT, CLIENT_PORT, Flags::SYN.with(Flags::ACK));
    answer.seq = SeqNumber::new(1000);
    // The client's `SYN` occupies 5000, so 9999 was never sent.
    answer.ack = SeqNumber::new(9999);
    let mut bytes = [0u8; 64];
    let mut writer = Writer::new(&mut bytes);
    answer.write(&mut writer, SERVER, CLIENT).expect("room");
    let len = writer.position();
    feed(&mut client, SERVER, &bytes[..len], now);

    let reset = one(&mut client, now);
    assert!(read(&reset, CLIENT, SERVER).flags.has(Flags::RST));
    assert_eq!(client.state(), State::SynSent);
}

#[test]
fn a_syn_that_arrives_twice_is_answered_twice_and_changes_nothing() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    client.connect(SERVER_END, &mut rng).expect("an open");

    let syn = one(&mut client, now);
    feed(&mut server, CLIENT, &syn, now);
    let first = one(&mut server, now);
    // The same `SYN` again: the answer is the same and the state stands.
    feed(&mut server, CLIENT, &syn, now);
    let second = one(&mut server, now);
    assert_eq!(first, second);
    assert_eq!(server.state(), State::SynReceived);
}

#[test]
fn bytes_cross_in_both_directions() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);

    assert_eq!(client.write(b"GET / HTTP/1.1").expect("room"), 14);
    exchange(&mut client, &mut server, now);
    assert_eq!(server.readable(), 14);
    let mut out = [0u8; 32];
    let taken = server.read(&mut out);
    assert_eq!(&out[..taken], b"GET / HTTP/1.1");

    assert_eq!(server.write(b"200 OK").expect("room"), 6);
    exchange(&mut server, &mut client, now);
    assert_eq!(client.peek(), b"200 OK");
    client.consume(6);
    assert_eq!(client.readable(), 0);
}

#[test]
fn a_segment_beyond_the_window_is_refused_and_the_edge_is_taken() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 8]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);

    // The client advertised eight bytes. The last byte of the window is
    // taken and the first byte past it is not.
    let at_edge = data_segment(SeqNumber::new(1008), b"x", SeqNumber::new(5001));
    feed(&mut client, SERVER, &at_edge, now);
    assert_eq!(
        client.readable(),
        0,
        "a byte at the far edge is early, not ready"
    );

    let past_edge = data_segment(SeqNumber::new(1009), b"y", SeqNumber::new(5001));
    let before = client.readable();
    feed(&mut client, SERVER, &past_edge, now);
    assert_eq!(client.readable(), before);

    let in_order = data_segment(SeqNumber::new(1001), b"abcdefg", SeqNumber::new(5001));
    feed(&mut client, SERVER, &in_order, now);
    assert_eq!(client.readable(), 8);
}

#[test]
fn a_segment_out_of_order_is_kept_and_completed_by_the_one_before_it() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 64]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);

    let second = data_segment(SeqNumber::new(1004), b"def", SeqNumber::new(5001));
    feed(&mut client, SERVER, &second, now);
    assert_eq!(client.readable(), 0);
    // A gap is answered at once, and the acknowledgment still asks for the
    // byte that is missing.
    let answer = one(&mut client, now);
    assert_eq!(read(&answer, CLIENT, SERVER).ack, SeqNumber::new(1001));

    let first = data_segment(SeqNumber::new(1001), b"abc", SeqNumber::new(5001));
    feed(&mut client, SERVER, &first, now);
    assert_eq!(client.readable(), 6);
    let answer = one(&mut client, now);
    assert_eq!(read(&answer, CLIENT, SERVER).ack, SeqNumber::new(1007));
    let mut out = [0u8; 8];
    let taken = client.read(&mut out);
    assert_eq!(&out[..taken], b"abcdef");
}

#[test]
fn a_segment_that_repeats_acknowledged_bytes_is_taken_and_adds_nothing() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 64]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);

    let first = data_segment(SeqNumber::new(1001), b"abc", SeqNumber::new(5001));
    feed(&mut client, SERVER, &first, now);
    drain(&mut client, now);
    assert_eq!(client.readable(), 3);
    // The same segment again, and a segment that begins before what has
    // arrived and reaches past it.
    feed(&mut client, SERVER, &first, now);
    assert_eq!(client.readable(), 3);
    let overlapping = data_segment(SeqNumber::new(1002), b"bcde", SeqNumber::new(5001));
    feed(&mut client, SERVER, &overlapping, now);
    assert_eq!(client.readable(), 5);
    let mut out = [0u8; 8];
    let taken = client.read(&mut out);
    assert_eq!(&out[..taken], b"abcde");
}

/// A segment from the server carrying `payload` at `seq`.
fn data_segment(seq: SeqNumber, payload: &[u8], ack: SeqNumber) -> Vec<u8> {
    let mut segment = Segment::new(SERVER_PORT, CLIENT_PORT, Flags::ACK);
    segment.seq = seq;
    segment.ack = ack;
    segment.window = 4096;
    segment.payload = payload;
    let mut bytes = vec![0u8; 128];
    let mut writer = Writer::new(&mut bytes);
    segment.write(&mut writer, SERVER, CLIENT).expect("room");
    let len = writer.position();
    bytes.truncate(len);
    bytes
}

#[test]
fn a_connection_that_will_take_no_more_announces_a_segment_size_and_honours_the_peers() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    // This end would send 1460, the peer will accept only 700.
    let mut client = Connection::new(CLIENT_END, config(1460), &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, config(700), &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);
    assert_eq!(client.max_segment(), 700);
    // The other way round the peer's larger offer is clamped to what this
    // end was configured with.
    assert_eq!(server.max_segment(), 700);
}

#[test]
fn a_peer_that_announces_nothing_gets_the_default_of_the_memo() {
    let now = start();
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let mut server = Connection::new(SERVER_END, config(1460), &mut ss, &mut sr);
    let script = sequence(1000);
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");

    let mut syn = Segment::new(CLIENT_PORT, SERVER_PORT, Flags::SYN);
    syn.seq = SeqNumber::new(5000);
    syn.window = 4096;
    let mut bytes = [0u8; 64];
    let mut writer = Writer::new(&mut bytes);
    syn.write(&mut writer, CLIENT, SERVER).expect("room");
    let len = writer.position();
    feed(&mut server, CLIENT, &bytes[..len], now);
    assert_eq!(server.max_segment(), u32::from(DEFAULT_MAX_SEGMENT));
}

#[test]
fn a_connection_that_was_never_opened_refuses_every_operation() {
    let (mut s, mut r) = (vec![0u8; 64], vec![0u8; 64]);
    let mut connection = Connection::new(CLIENT_END, Config::DEFAULT, &mut s, &mut r);
    assert_eq!(connection.state(), State::Closed);
    assert_eq!(
        connection.write(b"a"),
        Err(TcpError::WrongState(State::Closed))
    );
    assert_eq!(connection.close(), Err(TcpError::WrongState(State::Closed)));
    assert_eq!(connection.poll_at(start()), None);
}

#[test]
fn an_open_connection_refuses_a_second_open() {
    let (mut s, mut r) = (vec![0u8; 64], vec![0u8; 64]);
    let mut connection = Connection::new(CLIENT_END, Config::DEFAULT, &mut s, &mut r);
    let script = [sequence(1), sequence(2)].concat();
    let mut rng = ScriptedRng::new(&script);
    connection.connect(SERVER_END, &mut rng).expect("an open");
    assert_eq!(
        connection.connect(SERVER_END, &mut rng),
        Err(TcpError::WrongState(State::SynSent))
    );
    assert_eq!(
        connection.listen(&mut rng),
        Err(TcpError::WrongState(State::SynSent))
    );
}

#[test]
fn an_end_of_one_family_does_not_open_to_an_end_of_the_other() {
    let (mut s, mut r) = (vec![0u8; 64], vec![0u8; 64]);
    let mut connection = Connection::new(CLIENT_END, Config::DEFAULT, &mut s, &mut r);
    let script = sequence(1);
    let mut rng = ScriptedRng::new(&script);
    let elsewhere = Endpoint::new(
        net_wire::IpAddr::V6(net_wire::Ipv6Addr::from_octets([0; 16])),
        Port::new(80),
    );
    assert_eq!(
        connection.connect(elsewhere, &mut rng),
        Err(TcpError::MixedFamilies)
    );
}

#[test]
fn the_buffers_come_back_when_the_connection_is_given_up() {
    let (mut s, mut r) = (vec![0u8; 32], vec![0u8; 64]);
    let connection = Connection::new(CLIENT_END, Config::DEFAULT, &mut s, &mut r);
    let (send, recv) = connection.into_buffers();
    assert_eq!(send.len(), 32);
    assert_eq!(recv.len(), 64);
}

#[test]
fn the_lifetime_of_a_segment_is_the_constant_the_design_names() {
    assert_eq!(MAX_SEGMENT_LIFETIME, Duration::from_secs(30));
}

#[test]
fn an_active_close_walks_through_fin_wait_one_two_and_time_wait() {
    let now = start();
    pair!(client, server, now, 512, 1460);

    client.close().expect("a close");
    let fin = one(&mut client, now);
    let parsed = read(&fin, CLIENT, SERVER);
    assert!(parsed.flags.has(Flags::FIN));
    assert!(parsed.flags.has(Flags::ACK));
    assert_eq!(client.state(), State::FinWait1);
    // Nothing more may be handed over once this end has closed.
    assert_eq!(
        client.write(b"late"),
        Err(TcpError::WrongState(State::FinWait1))
    );

    feed(&mut server, CLIENT, &fin, now);
    assert_eq!(server.state(), State::CloseWait);
    assert!(server.peer_finished());
    let ack = one(&mut server, now);
    feed(&mut client, SERVER, &ack, now);
    assert_eq!(client.state(), State::FinWait2);

    server.close().expect("a close");
    let fin = one(&mut server, now);
    assert_eq!(server.state(), State::LastAck);
    feed(&mut client, SERVER, &fin, now);
    assert_eq!(client.state(), State::TimeWait);
    let ack = one(&mut client, now);
    feed(&mut server, CLIENT, &ack, now);
    assert_eq!(server.state(), State::Closed);
    assert!(!server.was_reset());
}

#[test]
fn time_wait_ends_at_twice_the_segment_lifetime_and_not_before() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    client.close().expect("a close");
    exchange(&mut client, &mut server, now);
    exchange(&mut server, &mut client, now);
    server.close().expect("a close");
    exchange(&mut server, &mut client, now);
    assert_eq!(client.state(), State::TimeWait);
    drain(&mut client, now);

    let ends = now.saturating_add(MAX_SEGMENT_LIFETIME.saturating_add(MAX_SEGMENT_LIFETIME));
    assert_eq!(client.poll_at(now), Some(ends));
    let almost = ends
        .checked_sub(Duration::from_micros(1))
        .expect("an instant");
    assert!(drain(&mut client, almost).is_empty());
    assert_eq!(client.state(), State::TimeWait);
    assert!(drain(&mut client, ends).is_empty());
    assert_eq!(client.state(), State::Closed);
    assert_eq!(client.poll_at(ends), None);
}

#[test]
fn two_ends_that_close_at_once_go_through_closing() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    client.close().expect("a close");
    server.close().expect("a close");
    let client_fin = one(&mut client, now);
    let server_fin = one(&mut server, now);
    feed(&mut server, CLIENT, &client_fin, now);
    feed(&mut client, SERVER, &server_fin, now);
    assert_eq!(client.state(), State::Closing);
    assert_eq!(server.state(), State::Closing);

    exchange(&mut client, &mut server, now);
    exchange(&mut server, &mut client, now);
    assert_eq!(client.state(), State::TimeWait);
    assert_eq!(server.state(), State::TimeWait);
}

#[test]
fn a_closed_window_stops_transmission_and_the_probe_finds_out_when_it_opens() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    // The peer says it has no room.
    let shut = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        0,
        Flags::ACK,
        b"",
    );
    feed(&mut client, SERVER, &shut, now);
    client.write(b"waiting").expect("room");

    // The first probe goes at once, carrying one byte and no more.
    let probe = one(&mut client, now);
    let parsed = read(&probe, CLIENT, SERVER);
    assert_eq!(parsed.payload, b"w");
    // Nothing follows until the persist timer says so.
    assert!(drain(&mut client, now).is_empty());
    let due = client.poll_at(now).expect("a persist timer");
    assert!(due > now);
    assert!(
        drain(
            &mut client,
            due.checked_sub(Duration::from_micros(1))
                .expect("an instant")
        )
        .is_empty()
    );

    // The next probe carries the same byte again, because the peer never
    // said it took the first.
    let again = one(&mut client, due);
    assert_eq!(read(&again, CLIENT, SERVER).payload, b"w");
    let next = client.poll_at(due).expect("a persist timer");
    assert!(next > due, "the probes come further apart, not closer");

    // A peer that takes the probe byte after all acknowledges it, and
    // that acknowledgment is of something that went out — not of
    // something that was never sent.
    let took_it = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5002),
        0,
        Flags::ACK,
        b"",
    );
    feed(&mut client, SERVER, &took_it, due);
    assert!(!client.was_reset());

    // A window update lets everything go.
    let opened = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5002),
        4096,
        Flags::ACK,
        b"",
    );
    feed(&mut client, SERVER, &opened, due);
    let sent = one(&mut client, due);
    assert_eq!(read(&sent, CLIENT, SERVER).payload, b"aiting");
}

#[test]
fn the_window_this_end_advertises_never_moves_its_far_edge_back() {
    let now = start();
    pair!(client, server, now, 64, 1460);
    let mut edge = None;
    let mut at = now;
    for (index, chunk) in [b"abcd".as_slice(), b"efgh", b"ijkl"]
        .into_iter()
        .enumerate()
    {
        let offset = u32::try_from(index * 4).expect("a small number");
        let segment = from_server(
            SeqNumber::new(1001 + offset),
            SeqNumber::new(5001),
            4096,
            Flags::ACK,
            chunk,
        );
        feed(&mut client, SERVER, &segment, at);
        at = client.poll_at(at).expect("a delayed acknowledgment");
        let answer = one(&mut client, at);
        let parsed = read(&answer, CLIENT, SERVER);
        let far = parsed.ack.add(u32::from(parsed.window));
        if let Some(previous) = edge {
            assert!(
                !far.before(previous),
                "the window shrank from {previous} to {far}"
            );
        }
        edge = Some(far);
    }
}

#[test]
fn a_segment_that_goes_unanswered_is_sent_again_when_the_timer_says_so() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    client.write(b"hello").expect("room");
    let first = one(&mut client, now);
    assert_eq!(client.rto().attempts(), 0);

    let due = client.poll_at(now).expect("a retransmission timer");
    assert_eq!(due, now.saturating_add(client.rto().get()));
    assert!(drain(&mut client, now).is_empty());

    let again = one(&mut client, due);
    assert_eq!(first, again, "the same bytes go again");
    assert_eq!(client.rto().attempts(), 1);
    // And the wait doubles.
    let next = client.poll_at(due).expect("a retransmission timer");
    assert_eq!(
        next.saturating_duration_since(due),
        due.saturating_duration_since(now)
            .saturating_add(due.saturating_duration_since(now))
    );
}

#[test]
fn a_connection_whose_segment_is_never_answered_is_given_up() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    client.write(b"hello").expect("room");
    let mut at = now;
    let mut sent = 0;
    for _ in 0..32 {
        sent += drain(&mut client, at).len();
        match client.poll_at(at) {
            Some(next) if next > at => at = next,
            _ => break,
        }
    }
    assert!(client.timed_out());
    assert_eq!(client.state(), State::Closed);
    assert!(!client.was_reset());
    // The first sending and then one attempt per allowance.
    let limit = usize::try_from(Config::DEFAULT.retransmit_limit).expect("a small number");
    assert_eq!(sent, limit + 1);
}

#[test]
fn a_segment_that_was_sent_twice_yields_no_round_trip_measurement() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    let measured = client.rto().smoothed().expect("the handshake was measured");
    client.write(b"hello").expect("room");
    drain(&mut client, now);
    let due = client.poll_at(now).expect("a retransmission timer");
    let again = one(&mut client, due);

    // The acknowledgment arrives a long time after the first copy went and
    // a moment after the second. Karn's rule says neither is a
    // measurement, because there is no telling which copy it answers.
    let later = due.saturating_add(Duration::from_millis(1));
    feed(&mut server, CLIENT, &again, later);
    let held = server.poll_at(later).expect("a delayed acknowledgment");
    let ack = one(&mut server, held);
    feed(&mut client, SERVER, &ack, held);
    assert_eq!(client.rto().smoothed(), Some(measured));
    // And the backoff stands, because only a measurement takes it away and
    // this exchange yielded none. That is Karn's algorithm and not an
    // oversight: a path that lost a segment is a path to keep waiting on.
    assert_eq!(client.rto().attempts(), 1);
}

#[test]
fn three_duplicate_acknowledgments_send_the_lost_segment_again_at_once() {
    let now = start();
    pair!(client, server, now, 512, 4);
    client.write(b"abcdefghijkl").expect("room");
    let segments = drain(&mut client, now);
    assert_eq!(segments.len(), 3, "three segments of four bytes");
    let first = segments.first().cloned().expect("a segment");

    // The peer acknowledges the first and nothing more, three times over.
    let duplicate = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"",
    );
    for _ in 0..2 {
        feed(&mut client, SERVER, &duplicate, now);
        assert!(drain(&mut client, now).is_empty());
    }
    feed(&mut client, SERVER, &duplicate, now);
    assert!(client.congestion().is_recovering());
    let resent = one(&mut client, now);
    assert_eq!(resent, first, "the segment the duplicates point at");
}

#[test]
fn an_acknowledgment_waits_for_a_second_full_segment_or_for_the_timer() {
    let now = start();
    pair!(client, server, now, 512, 4);
    let full = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"abcd",
    );
    feed(&mut client, SERVER, &full, now);
    assert!(drain(&mut client, now).is_empty(), "the first one waits");
    let due = client.poll_at(now).expect("a delayed acknowledgment");
    assert_eq!(due, now.saturating_add(Config::DEFAULT.delayed_ack));
    let answer = one(&mut client, due);
    assert_eq!(read(&answer, CLIENT, SERVER).ack, SeqNumber::new(1005));

    // The second full segment is acknowledged at once.
    let second = from_server(
        SeqNumber::new(1005),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"efgh",
    );
    feed(&mut client, SERVER, &second, due);
    assert!(drain(&mut client, due).is_empty(), "one of two waits");
    let third = from_server(
        SeqNumber::new(1009),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"ijkl",
    );
    feed(&mut client, SERVER, &third, due);
    let answer = one(&mut client, due);
    assert_eq!(read(&answer, CLIENT, SERVER).ack, SeqNumber::new(1013));
}

#[test]
fn a_segment_that_fills_a_closed_window_is_acknowledged_at_once() {
    let now = start();
    pair!(client, server, now, 4, 4);
    // Four bytes fill the client's window exactly.
    let full = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"abcd",
    );
    feed(&mut client, SERVER, &full, now);
    drain(&mut client, now);
    assert_eq!(client.readable(), 4);
    // The window is closed now. The peer's probe is answered at once, and
    // not after half a second.
    let probe = from_server(
        SeqNumber::new(1005),
        SeqNumber::new(5001),
        512,
        Flags::ACK,
        b"e",
    );
    feed(&mut client, SERVER, &probe, now);
    assert_eq!(client.poll_at(now), Some(now));
    let answer = one(&mut client, now);
    assert_eq!(read(&answer, CLIENT, SERVER).window, 0);
}

#[test]
fn a_reset_at_the_number_expected_tears_the_connection_down() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    let reset = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        512,
        Flags::RST,
        b"",
    );
    feed(&mut client, SERVER, &reset, now);
    assert_eq!(client.state(), State::Closed);
    assert!(client.was_reset());
    assert!(drain(&mut client, now).is_empty());
}

#[test]
fn a_reset_merely_inside_the_window_draws_an_acknowledgment_and_nothing_else() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    let reset = from_server(
        SeqNumber::new(1005),
        SeqNumber::new(5001),
        512,
        Flags::RST,
        b"",
    );
    feed(&mut client, SERVER, &reset, now);
    assert_eq!(client.state(), State::Established);
    assert!(!client.was_reset());
    let challenge = one(&mut client, now);
    let parsed = read(&challenge, CLIENT, SERVER);
    assert_eq!(parsed.flags, Flags::ACK);
    assert_eq!(parsed.ack, SeqNumber::new(1001));
}

#[test]
fn a_reset_outside_the_window_is_ignored_without_a_word() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    let reset = from_server(
        SeqNumber::new(9999),
        SeqNumber::new(5001),
        512,
        Flags::RST,
        b"",
    );
    feed(&mut client, SERVER, &reset, now);
    assert_eq!(client.state(), State::Established);
    assert!(drain(&mut client, now).is_empty());
}

#[test]
fn a_syn_inside_the_window_of_an_open_connection_is_answered_and_not_acted_on() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    let intruder = from_server(
        SeqNumber::new(1001),
        SeqNumber::new(5001),
        512,
        Flags::SYN,
        b"",
    );
    feed(&mut client, SERVER, &intruder, now);
    assert_eq!(client.state(), State::Established);
    let challenge = one(&mut client, now);
    assert_eq!(read(&challenge, CLIENT, SERVER).flags, Flags::ACK);
}

#[test]
fn an_abort_sends_a_reset_and_closes_at_once() {
    let now = start();
    pair!(client, server, now, 512, 1460);
    client.abort();
    assert_eq!(client.state(), State::Closed);
    let reset = one(&mut client, now);
    let parsed = read(&reset, CLIENT, SERVER);
    assert!(parsed.flags.has(Flags::RST));
    feed(&mut server, CLIENT, &reset, now);
    assert_eq!(server.state(), State::Closed);
    assert!(server.was_reset());
}

#[test]
fn nagle_holds_a_short_segment_back_while_something_is_outstanding() {
    let now = start();
    let (mut cs, mut cr) = (vec![0u8; 512], vec![0u8; 512]);
    let (mut ss, mut sr) = (vec![0u8; 512], vec![0u8; 512]);
    let waiting = Config {
        max_segment: 8,
        ..Config::DEFAULT
    };
    let mut client = Connection::new(CLIENT_END, waiting, &mut cs, &mut cr);
    let mut server = Connection::new(SERVER_END, waiting, &mut ss, &mut sr);
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).expect("a listen");
    open(&mut client, &mut server, &mut rng, now);

    // A full segment goes at once.
    client.write(b"12345678").expect("room");
    assert_eq!(exchange(&mut client, &mut server, now), 1);
    // A short one waits while the first is outstanding.
    client.write(b"9").expect("room");
    assert!(drain(&mut client, now).is_empty());
    // The acknowledgment lets it go.
    let held = server.poll_at(now).expect("a delayed acknowledgment");
    exchange(&mut server, &mut client, held);
    let segments = drain(&mut client, held);
    assert_eq!(segments.len(), 1);
    assert_eq!(read(&segments[0], CLIENT, SERVER).payload, b"9");
}
