// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two instances of the state machine, back to back over a network that
//! delays, duplicates, reorders, and drops.
//!
//! This is the test that makes the rest of the crate trustworthy. Every
//! other test says that one rule is followed; this one says that the rules
//! together carry a byte stream across a link that misbehaves, and that
//! both ends put the connection down afterwards.
//!
//! Nothing sleeps. Time is a number the loop advances to whatever the two
//! connections and the link say is next, so a sixty-second `TIME-WAIT` and
//! a one-second retransmission timeout both pass in microseconds of wall
//! clock.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a simulation counts steps and bytes by hand"
)]

use audhsos_time::{Duration, Instant};
use crypto_rng::doubles::ScriptedRng;
use net_wire::IpAddr;
use test_support::generators::bytes;
use test_support::property::check;

use crate::connection::Connection;
use crate::state::State;
use crate::tests::harness::{CLIENT, CLIENT_END, SERVER, SERVER_END, config, sequence, start};

/// What the client sends.
const FROM_CLIENT: &[u8] = b"GET /index.html HTTP/1.1\r\nHost: example\r\n\r\n";

/// What the server sends back.
const FROM_SERVER: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";

/// How long a segment takes to cross the link.
const LATENCY: Duration = Duration::from_millis(10);

/// How much longer a delayed one takes.
const EXTRA: Duration = Duration::from_millis(250);

/// The segment size both ends use, small enough that the streams above
/// take several segments each.
const SEGMENT: u16 = 8;

/// How many passes the loop may make before a run is called stuck.
const BUDGET: usize = 4000;

/// How many passes at one instant mean the two are spinning rather than
/// working.
const SAME_INSTANT: usize = 200;

/// What the link does with a segment it was handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handling {
    /// It arrives after the usual latency.
    Deliver,
    /// It arrives late, which is what lets the one behind it overtake it.
    Delay,
    /// It arrives twice.
    Duplicate,
    /// It never arrives.
    Drop,
}

/// The handling this code asks for. One in sixteen is a drop, two in
/// sixteen a delay, one a duplicate, and the rest arrive as sent.
const fn handling(code: u8) -> Handling {
    match code % 16 {
        0 => Handling::Drop,
        1 | 2 => Handling::Delay,
        3 => Handling::Duplicate,
        _ => Handling::Deliver,
    }
}

/// One segment on its way.
#[derive(Clone, Debug)]
struct InFlight {
    /// When it arrives.
    at: Instant,
    /// Which was handed over first, so that two segments due at one
    /// instant keep their order.
    order: usize,
    /// The segment.
    bytes: Vec<u8>,
}

/// How many segments must get through between two drops.
///
/// The bound is what keeps the link a link and not a wall. A link that
/// loses more segments in a row than a connection has attempts is one no
/// implementation of this protocol recovers from, and giving up on it is
/// the right answer — which is what
/// `a_connection_whose_segment_is_never_answered_is_given_up` tests. This
/// test is about the other thing: that a link which loses segments,
/// reorders them, and repeats them still carries the streams.
const BETWEEN_DROPS: usize = 4;

/// One direction of the link.
#[derive(Debug, Default)]
struct Link {
    /// What is on its way.
    queue: Vec<InFlight>,
    /// How many segments have got through since the last drop.
    since_drop: usize,
    /// How many segments have been handed over.
    handed: usize,
}

impl Link {
    /// A link that has just carried enough segments to be allowed a drop.
    fn new() -> Link {
        Link {
            since_drop: BETWEEN_DROPS,
            ..Link::default()
        }
    }

    /// Takes a segment, and decides what becomes of it.
    fn accept(&mut self, bytes: Vec<u8>, now: Instant, code: u8) {
        let asked = handling(code);
        let what = if asked == Handling::Drop && self.since_drop < BETWEEN_DROPS {
            Handling::Deliver
        } else {
            asked
        };
        self.since_drop = if what == Handling::Drop {
            0
        } else {
            self.since_drop.saturating_add(1)
        };
        let soon = now.saturating_add(LATENCY);
        let late = soon.saturating_add(EXTRA);
        match what {
            Handling::Drop => {}
            Handling::Deliver => self.push(bytes, soon),
            Handling::Delay => self.push(bytes, late),
            Handling::Duplicate => {
                self.push(bytes.clone(), soon);
                self.push(bytes, late);
            }
        }
    }

    /// Puts a segment in the queue, due at `at`.
    fn push(&mut self, bytes: Vec<u8>, at: Instant) {
        self.handed += 1;
        let order = self.handed;
        self.queue.push(InFlight { at, order, bytes });
    }

    /// Everything that has arrived by `now`, in the order it arrives.
    fn take(&mut self, now: Instant) -> Vec<Vec<u8>> {
        let mut due: Vec<InFlight> = Vec::new();
        let mut rest = Vec::new();
        for packet in self.queue.drain(..) {
            if packet.at <= now {
                due.push(packet);
            } else {
                rest.push(packet);
            }
        }
        self.queue = rest;
        due.sort_by_key(|packet| (packet.at, packet.order));
        due.into_iter().map(|packet| packet.bytes).collect()
    }

    /// When the next segment arrives.
    fn next_at(&self) -> Option<Instant> {
        self.queue.iter().map(|packet| packet.at).min()
    }
}

#[test]
fn two_instances_carry_their_streams_across_a_link_that_misbehaves() {
    check("tcp back to back", &bytes(1..=64), |schedule| run(schedule));
}

#[test]
fn a_link_that_behaves_carries_the_streams_and_both_ends_close() {
    // The schedule of all deliveries, which is the case every other one is
    // a departure from.
    run(&[0x0F; 8]).expect("a clean link carries both streams");
}

/// Runs one schedule and answers what went wrong, if anything.
fn run(schedule: &[u8]) -> Result<(), String> {
    let mut client_send = vec![0u8; 64];
    let mut client_recv = vec![0u8; 64];
    let mut server_send = vec![0u8; 64];
    let mut server_recv = vec![0u8; 64];
    let mut client = Connection::new(
        CLIENT_END,
        config(SEGMENT),
        &mut client_send,
        &mut client_recv,
    );
    let mut server = Connection::new(
        SERVER_END,
        config(SEGMENT),
        &mut server_send,
        &mut server_recv,
    );
    let script = [sequence(1000), sequence(5000)].concat();
    let mut rng = ScriptedRng::new(&script);
    server.listen(&mut rng).map_err(|error| error.to_string())?;
    client
        .connect(SERVER_END, &mut rng)
        .map_err(|error| error.to_string())?;

    let mut to_server = Link::new();
    let mut to_client = Link::new();
    let mut now = start();
    let mut written = (0usize, 0usize);
    let mut got = (Vec::new(), Vec::new());
    let mut code = 0usize;
    let mut still = 0usize;
    let mut buffer = [0u8; 256];

    for _ in 0..BUDGET {
        offer(&mut client, FROM_CLIENT, &mut written.0);
        offer(&mut server, FROM_SERVER, &mut written.1);

        emit(
            &mut client,
            "client",
            &mut to_server,
            now,
            schedule,
            &mut code,
            &mut buffer,
        )?;
        emit(
            &mut server,
            "server",
            &mut to_client,
            now,
            schedule,
            &mut code,
            &mut buffer,
        )?;

        let mut moved = false;
        for bytes in to_server.take(now) {
            deliver(&mut server, CLIENT, &bytes, now);
            moved = true;
        }
        for bytes in to_client.take(now) {
            deliver(&mut client, SERVER, &bytes, now);
            moved = true;
        }
        collect(&mut server, &mut got.1);
        collect(&mut client, &mut got.0);

        if client.timed_out() || server.timed_out() {
            return Err(given_up(&client, &server, got.0.len(), got.1.len()));
        }
        if client.state() == State::Closed && server.state() == State::Closed {
            break;
        }

        let next = [
            to_server.next_at(),
            to_client.next_at(),
            client.poll_at(now),
            server.poll_at(now),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(next) = next else {
            break;
        };
        if next > now {
            now = next;
            still = 0;
        } else {
            still += usize::from(!moved);
            if still > SAME_INSTANT {
                return Err(format!("the two spun at {now:?} without working"));
            }
        }
    }

    verdict(&client, &server, &got.0, &got.1)
}

/// Everything one pass of a poll loop does for one end: take what it has
/// to say, check that it said beforehand that it had something, and hand
/// it to the link.
fn emit(
    connection: &mut Connection<'_>,
    who: &str,
    link: &mut Link,
    now: Instant,
    schedule: &[u8],
    code: &mut usize,
    buffer: &mut [u8],
) -> Result<(), String> {
    let due = connection.poll_at(now);
    while let Some(segment) = connection.poll(now, buffer) {
        if due.is_none_or(|at| at > now) {
            return Err(format!("the {who} sent at {now:?} and said it had nothing"));
        }
        link.accept(segment.to_vec(), now, next_code(schedule, code));
    }
    Ok(())
}

/// What must hold when the run is over: both streams arrived whole and in
/// order, and both ends put the connection down.
fn verdict(
    client: &Connection<'_>,
    server: &Connection<'_>,
    at_client: &[u8],
    at_server: &[u8],
) -> Result<(), String> {
    if at_server != FROM_CLIENT {
        return Err(format!(
            "the server read {:?}",
            String::from_utf8_lossy(at_server)
        ));
    }
    if at_client != FROM_SERVER {
        return Err(format!(
            "the client read {:?}",
            String::from_utf8_lossy(at_client)
        ));
    }
    if client.state() != State::Closed || server.state() != State::Closed {
        return Err(format!(
            "they ended at {} and {}",
            client.state(),
            server.state()
        ));
    }
    Ok(())
}

/// What to say about a run in which one end gave a segment up.
fn given_up(
    client: &Connection<'_>,
    server: &Connection<'_>,
    at_client: usize,
    at_server: usize,
) -> String {
    format!(
        "one end gave up: the client is {} and read {at_client} bytes, \
         the server is {} and read {at_server}",
        client.state(),
        server.state()
    )
}

/// The next code of the schedule, which repeats.
fn next_code(schedule: &[u8], at: &mut usize) -> u8 {
    let code = schedule.get(*at % schedule.len()).copied().unwrap_or(0x0F);
    *at += 1;
    code
}

/// Hands over as much of `data` as the connection will take, and closes
/// this end once all of it is over.
fn offer(connection: &mut Connection<'_>, data: &[u8], written: &mut usize) {
    if *written < data.len()
        && let Ok(taken) = connection.write(&data[*written..])
    {
        *written += taken;
    }
    if *written == data.len() && connection.state().can_send() {
        let _ = connection.close();
    }
}

/// Reads out everything that has arrived.
fn collect(connection: &mut Connection<'_>, into: &mut Vec<u8>) {
    let ready = connection.readable();
    if ready == 0 {
        return;
    }
    let mut out = vec![0u8; ready];
    let taken = connection.read(&mut out);
    out.truncate(taken);
    into.extend_from_slice(&out);
}

/// Hands a segment that came from `from` to the connection.
fn deliver(connection: &mut Connection<'_>, from: IpAddr, bytes: &[u8], now: Instant) {
    let to = connection.local().address;
    if let Ok(segment) = crate::segment::Segment::parse(bytes, from, to) {
        connection.on_segment(from, &segment, now);
    }
}
