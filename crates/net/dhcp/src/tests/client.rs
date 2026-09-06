// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The four-message exchange, the timers behind it, and what the client
//! does when a server says no or says nothing.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a datagram at known offsets and builds a script at compile time"
)]

use audhsos_time::{Duration, Instant};
use crypto_rng::doubles::ScriptedRng;
use net_udp::{Datagram, HEADER_LEN as UDP_HEADER_LEN};
use net_wire::{IpAddr, Ipv4Addr, Port};

use crate::client::{CLIENT_PORT, Client, Config, Lease, Outgoing, SERVER_PORT, State};
use crate::error::DhcpError;
use crate::message::{Message, MessageType};
use crate::option::{OptionCode, Options};
use crate::tests::harness::{MAC, MASK, OFFERED, Reply, lease_time, opt};

/// The server, as an address of the layer above.
const SERVER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

/// A script of bytes that do not repeat, so that two transaction ids drawn
/// from it are two.
const SCRIPT: [u8; 256] = {
    let mut bytes = [0u8; 256];
    let mut at = 0usize;
    let mut value = 11u8;
    while at < 256 {
        bytes[at] = value;
        value = value.wrapping_add(37);
        at += 1;
    }
    bytes
};

/// A generator with enough bytes for a transaction id and many jitters.
fn rng() -> ScriptedRng<'static> {
    ScriptedRng::new(&SCRIPT)
}

/// What one call to `poll` produced.
struct Sent {
    /// Where it went out from.
    source: IpAddr,
    /// Where it went.
    destination: IpAddr,
    /// Which of the eight it is.
    kind: MessageType,
    /// The transaction id it carried.
    xid: u32,
    /// Whether it asked for a broadcast reply.
    broadcast: bool,
    /// The address it says it already holds.
    client: Ipv4Addr,
    /// How long the client says it has been at this.
    secs: u16,
    /// The options it carried.
    options: Vec<(OptionCode, Vec<u8>)>,
}

impl Sent {
    /// The body of the option `code`, where the message carried it.
    fn option(&self, code: OptionCode) -> Option<&[u8]> {
        self.options
            .iter()
            .find(|(seen, _)| *seen == code)
            .map(|(_, body)| body.as_slice())
    }
}

/// Polls once and reads back what went out.
fn send<R: crypto_rng::Rng + ?Sized>(
    client: &mut Client,
    rng: &mut R,
    now: Instant,
) -> Option<Sent> {
    let mut buffer = [0u8; 512];
    let Outgoing {
        source,
        destination,
        datagram,
    } = client.poll(now, rng, &mut buffer).expect("a message")?;
    let parsed = Datagram::parse(datagram, source, destination).expect("a datagram");
    assert_eq!(parsed.source_port, CLIENT_PORT);
    assert_eq!(parsed.destination_port, SERVER_PORT);
    assert_eq!(datagram.len(), UDP_HEADER_LEN + parsed.payload.len());
    let message = Message::parse(parsed.payload).expect("a message");
    let options = Options::new(message.options)
        .map(|option| {
            let (code, body) = option.expect("an option");
            (code, body.to_vec())
        })
        .collect();
    Some(Sent {
        source,
        destination,
        kind: message.message_type().expect("a message type"),
        xid: message.xid,
        broadcast: message.broadcast,
        client: message.client,
        secs: message.secs,
        options,
    })
}

/// Hands `reply` to `client` as if it had come from the server.
fn deliver(client: &mut Client, reply: &Reply, now: Instant) {
    client.on_datagram(SERVER, SERVER_PORT, &reply.bytes(), now);
}

/// A client that has just been started.
fn started(config: Config, now: Instant) -> Client {
    let mut client = Client::new(MAC, config);
    client.start(now);
    client
}

#[test]
fn the_four_messages_produce_a_lease() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    assert_eq!(client.state(), State::Selecting);

    let discover = send(&mut client, &mut rng, start).expect("a discover");
    assert_eq!(discover.kind, MessageType::DISCOVER);
    assert_eq!(discover.source, IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    assert_eq!(discover.destination, IpAddr::V4(Ipv4Addr::BROADCAST));
    assert!(discover.broadcast);
    assert_eq!(discover.client, Ipv4Addr::UNSPECIFIED);
    assert_eq!(
        discover.option(OptionCode::PARAMETER_LIST),
        Some([1u8, 3, 6, 58, 59].as_slice())
    );
    assert_eq!(
        discover.option(OptionCode::MAX_MESSAGE_SIZE),
        Some(576u16.to_be_bytes().as_slice())
    );
    assert!(send(&mut client, &mut rng, start).is_none());

    deliver(&mut client, &Reply::offer(discover.xid), start);
    assert_eq!(client.state(), State::Requesting);

    let request = send(&mut client, &mut rng, start).expect("a request");
    assert_eq!(request.kind, MessageType::REQUEST);
    assert_eq!(request.xid, discover.xid);
    assert!(request.broadcast);
    assert_eq!(request.client, Ipv4Addr::UNSPECIFIED);
    assert_eq!(
        request.option(OptionCode::REQUESTED_ADDRESS),
        Some(OFFERED.octets().as_slice())
    );
    assert_eq!(
        request.option(OptionCode::SERVER_IDENTIFIER),
        Some(Ipv4Addr::new(192, 168, 1, 1).octets().as_slice())
    );

    deliver(&mut client, &Reply::ack(discover.xid), start);
    assert_eq!(client.state(), State::Bound);
    let lease = client.lease().expect("a lease");
    assert_eq!(lease.address(), OFFERED);
    assert_eq!(lease.network.netmask(), MASK);
    assert_eq!(lease.network.prefix_len(), 24);
    assert_eq!(lease.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
    assert_eq!(
        lease.servers.iter().copied().collect::<Vec<_>>(),
        vec![Ipv4Addr::new(192, 168, 1, 1), Ipv4Addr::new(9, 9, 9, 9)]
    );
    assert_eq!(lease.server, Ipv4Addr::new(192, 168, 1, 1));
    assert_eq!(lease.granted, start);
    assert_eq!(lease.renew, Instant::from_micros(1_800_000_000));
    assert_eq!(lease.rebind, Instant::from_micros(3_150_000_000));
    assert_eq!(lease.expires, Instant::from_micros(3_600_000_000));
    assert_eq!(client.address(), Some(OFFERED));
    assert_eq!(client.poll_at(), Some(lease.renew));
    assert!(send(&mut client, &mut rng, start).is_none());
}

#[test]
fn a_discover_that_nobody_answers_backs_off_and_stays_inside_the_jitter() {
    let config = Config::DEFAULT;
    let mut rng = ScriptedRng::new(&[0x00u8; 256]);
    let start = Instant::from_micros(0);
    let mut client = started(config, start);
    let mut now = start;
    let mut delays = Vec::new();
    for _ in 0..6 {
        send(&mut client, &mut rng, now).expect("a discover");
        let next = client.poll_at().expect("a schedule");
        delays.push(next.saturating_duration_since(now));
        now = next;
    }
    // Four, eight, sixteen, thirty-two, sixty-four, and then the ceiling,
    // each moved by a second at most. This generator draws the lowest
    // value there is, so each delay is a second short.
    let unjittered = [4u64, 8, 16, 32, 64, 64];
    for (delay, seconds) in delays.iter().zip(unjittered) {
        let nominal = Duration::from_secs(seconds);
        assert!(
            *delay >= nominal.saturating_sub(config.jitter)
                && *delay <= nominal.saturating_add(config.jitter),
            "a delay of {delay:?} is outside the jitter around {nominal:?}"
        );
    }
    assert_eq!(delays[0], Duration::from_secs(3));

    // And the same again with the highest value the generator has.
    let mut high = ScriptedRng::new(&[0xFFu8; 256]);
    let mut client = started(config, start);
    send(&mut client, &mut high, start).expect("a discover");
    assert_eq!(
        client
            .poll_at()
            .expect("a schedule")
            .saturating_duration_since(start),
        Duration::from_secs(5)
    );
}

#[test]
fn the_first_offer_is_taken_and_the_second_is_ignored() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");

    let mut second = Reply::offer(discover.xid);
    second.yours = Ipv4Addr::new(192, 168, 1, 99);
    second.options = vec![
        opt(
            OptionCode::SERVER_IDENTIFIER,
            &Ipv4Addr::new(192, 168, 1, 2).octets(),
        ),
        opt(OptionCode::SUBNET_MASK, &MASK.octets()),
        lease_time(3600),
    ];
    deliver(&mut client, &Reply::offer(discover.xid), start);
    deliver(&mut client, &second, start);

    let request = send(&mut client, &mut rng, start).expect("a request");
    assert_eq!(
        request.option(OptionCode::REQUESTED_ADDRESS),
        Some(OFFERED.octets().as_slice())
    );
}

#[test]
fn a_reply_of_another_exchange_is_ignored() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");

    // A foreign transaction id.
    deliver(&mut client, &Reply::offer(discover.xid ^ 1), start);
    assert_eq!(client.state(), State::Selecting);

    // A foreign hardware address.
    let mut other = Reply::offer(discover.xid);
    other.hardware = net_wire::MacAddr::new([2, 2, 2, 2, 2, 2]);
    deliver(&mut client, &other, start);
    assert_eq!(client.state(), State::Selecting);

    // A wrong source port.
    client.on_datagram(
        SERVER,
        Port::new(1024),
        &Reply::offer(discover.xid).bytes(),
        start,
    );
    assert_eq!(client.state(), State::Selecting);

    // Bytes that are no message.
    client.on_datagram(SERVER, SERVER_PORT, &[0u8; 4], start);
    assert_eq!(client.state(), State::Selecting);

    // A message from a server over IPv6, which DHCP is not.
    client.on_datagram(
        IpAddr::V6(net_wire::Ipv6Addr::from_octets([0; 16])),
        SERVER_PORT,
        &Reply::offer(discover.xid).bytes(),
        start,
    );
    assert_eq!(client.state(), State::Selecting);

    // A request from another client, which is not a reply.
    let mut request = Reply::offer(discover.xid);
    request.kind = MessageType::DISCOVER;
    deliver(&mut client, &request, start);
    assert_eq!(client.state(), State::Selecting);

    // And the one that matches.
    deliver(&mut client, &Reply::offer(discover.xid), start);
    assert_eq!(client.state(), State::Requesting);
}

#[test]
fn a_refusal_sends_the_client_back_to_the_start() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(&mut client, &Reply::offer(discover.xid), start);
    send(&mut client, &mut rng, start).expect("a request");
    deliver(&mut client, &Reply::nak(discover.xid), start);
    assert_eq!(client.state(), State::Selecting);
    assert!(client.lease().is_none());
    let again = send(&mut client, &mut rng, start).expect("a discover");
    assert_eq!(again.kind, MessageType::DISCOVER);
    assert_ne!(again.xid, discover.xid);
}

#[test]
fn four_requests_without_an_answer_start_the_exchange_over() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(&mut client, &Reply::offer(discover.xid), start);
    let mut now = start;
    for attempt in 0..4 {
        let request = send(&mut client, &mut rng, now).expect("a request");
        assert_eq!(request.kind, MessageType::REQUEST, "attempt {attempt}");
        now = client.poll_at().expect("a schedule");
    }
    let over = send(&mut client, &mut rng, now).expect("a discover");
    assert_eq!(over.kind, MessageType::DISCOVER);
    assert_eq!(client.state(), State::Selecting);
    assert_ne!(over.xid, discover.xid);
}

/// A client with a bound lease, and the instant it was granted at.
fn bound(config: Config) -> (Client, Instant, ScriptedRng<'static>) {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(config, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(&mut client, &Reply::offer(discover.xid), start);
    send(&mut client, &mut rng, start).expect("a request");
    deliver(&mut client, &Reply::ack(discover.xid), start);
    (client, start, rng)
}

#[test]
fn renewal_at_t1_goes_by_unicast_to_the_server_that_granted_the_lease() {
    let (mut client, _, mut rng) = bound(Config::DEFAULT);
    let renew = client.lease().expect("a lease").renew;
    assert!(send(&mut client, &mut rng, Instant::from_micros(1)).is_none());
    let request = send(&mut client, &mut rng, renew).expect("a renewal");
    assert_eq!(client.state(), State::Renewing);
    assert_eq!(request.kind, MessageType::REQUEST);
    assert_eq!(request.destination, SERVER);
    assert_eq!(request.source, IpAddr::V4(OFFERED));
    assert_eq!(request.client, OFFERED);
    assert!(!request.broadcast);
    // A renewal repeats neither the address nor the server: the lease is
    // already this client's and the message goes to the one server.
    assert_eq!(request.option(OptionCode::REQUESTED_ADDRESS), None);
    assert_eq!(request.option(OptionCode::SERVER_IDENTIFIER), None);
    // A renewal is where `secs` starts over (RFC 2131, table 1), so the
    // first one carries nothing and the one behind it carries the wait.
    assert_eq!(request.secs, 0);
    let next = client.poll_at().expect("a schedule");
    let again = send(&mut client, &mut rng, next).expect("a renewal");
    assert_eq!(
        again.secs,
        u16::try_from(next.saturating_duration_since(renew).as_secs()).unwrap_or(u16::MAX)
    );
}

#[test]
fn a_client_that_has_been_at_this_for_a_day_reports_the_largest_number() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let much_later = start.saturating_add(Duration::from_secs(100_000));
    let discover = send(&mut client, &mut rng, much_later).expect("a discover");
    assert_eq!(discover.secs, u16::MAX);
}

#[test]
fn rebinding_at_t2_goes_by_broadcast_to_anyone() {
    let (mut client, _, mut rng) = bound(Config::DEFAULT);
    let rebind = client.lease().expect("a lease").rebind;
    let request = send(&mut client, &mut rng, rebind).expect("a rebinding");
    assert_eq!(client.state(), State::Rebinding);
    assert_eq!(request.destination, IpAddr::V4(Ipv4Addr::BROADCAST));
    assert_eq!(request.source, IpAddr::V4(OFFERED));
    assert_eq!(request.client, OFFERED);
    assert!(!request.broadcast);
}

#[test]
fn a_renewal_answered_late_keeps_the_lease() {
    let (mut client, _, mut rng) = bound(Config::DEFAULT);
    let rebind = client.lease().expect("a lease").rebind;
    let request = send(&mut client, &mut rng, rebind).expect("a rebinding");
    assert_eq!(client.state(), State::Rebinding);
    let late = rebind.saturating_add(Duration::from_secs(120));
    deliver(&mut client, &Reply::ack(request.xid), late);
    assert_eq!(client.state(), State::Bound);
    let lease = client.lease().expect("a lease");
    assert_eq!(lease.granted, late);
    assert_eq!(lease.address(), OFFERED);
    assert_eq!(
        lease.expires,
        late.saturating_add(Duration::from_secs(3600))
    );
}

#[test]
fn an_expired_lease_takes_the_address_away_and_starts_over() {
    let (mut client, _, mut rng) = bound(Config::DEFAULT);
    let lease = client.lease().expect("a lease");
    let (renew, expires) = (lease.renew, lease.expires);
    send(&mut client, &mut rng, renew).expect("a renewal");
    let over = send(&mut client, &mut rng, expires).expect("a discover");
    assert_eq!(over.kind, MessageType::DISCOVER);
    assert_eq!(client.state(), State::Selecting);
    assert_eq!(client.address(), None);
}

#[test]
fn a_renewal_waits_half_of_what_is_left_and_never_less_than_a_minute() {
    let config = Config::DEFAULT;
    let (mut client, _, mut rng) = bound(config);
    let lease = client.lease().expect("a lease");
    let (renew, rebind) = (lease.renew, lease.rebind);
    send(&mut client, &mut rng, renew).expect("a renewal");
    let next = client.poll_at().expect("a schedule");
    assert_eq!(
        next.saturating_duration_since(renew),
        Duration::from_micros(rebind.saturating_duration_since(renew).as_micros() >> 1)
    );
    // Close to the deadline the floor takes over, and the deadline itself
    // is never overshot.
    let late = rebind
        .checked_sub(Duration::from_secs(10))
        .expect("before T2");
    send(&mut client, &mut rng, late).expect("a renewal");
    assert_eq!(client.poll_at(), Some(rebind));
}

#[test]
fn the_timers_the_server_sends_are_used_where_they_make_sense() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(&mut client, &Reply::offer(discover.xid), start);
    send(&mut client, &mut rng, start).expect("a request");
    deliver(
        &mut client,
        &Reply::ack(discover.xid)
            .with(opt(OptionCode::RENEWAL_TIME, &600u32.to_be_bytes()))
            .with(opt(OptionCode::REBINDING_TIME, &1200u32.to_be_bytes())),
        start,
    );
    let lease = client.lease().expect("a lease");
    assert_eq!(lease.renew, Instant::from_micros(600_000_000));
    assert_eq!(lease.rebind, Instant::from_micros(1_200_000_000));
}

#[test]
fn timers_that_do_not_make_sense_fall_back_to_the_defaults() {
    for (first, second) in [(2000u32, 1000u32), (100, 7200), (3600, 3600)] {
        let mut rng = rng();
        let start = Instant::from_micros(0);
        let mut client = started(Config::DEFAULT, start);
        let discover = send(&mut client, &mut rng, start).expect("a discover");
        deliver(&mut client, &Reply::offer(discover.xid), start);
        send(&mut client, &mut rng, start).expect("a request");
        deliver(
            &mut client,
            &Reply::ack(discover.xid)
                .with(opt(OptionCode::RENEWAL_TIME, &first.to_be_bytes()))
                .with(opt(OptionCode::REBINDING_TIME, &second.to_be_bytes())),
            start,
        );
        let lease = client.lease().expect("a lease");
        assert_eq!(
            (lease.renew, lease.rebind),
            (
                Instant::from_micros(1_800_000_000),
                Instant::from_micros(3_150_000_000)
            ),
            "T1 of {first} and T2 of {second}"
        );
    }
}

#[test]
fn an_acknowledgment_this_client_cannot_make_a_lease_out_of_is_ignored() {
    for missing in [
        OptionCode::SUBNET_MASK,
        OptionCode::SERVER_IDENTIFIER,
        OptionCode::LEASE_TIME,
    ] {
        let mut rng = rng();
        let start = Instant::from_micros(0);
        let mut client = started(Config::DEFAULT, start);
        let discover = send(&mut client, &mut rng, start).expect("a discover");
        deliver(&mut client, &Reply::offer(discover.xid), start);
        send(&mut client, &mut rng, start).expect("a request");
        deliver(
            &mut client,
            &Reply::ack(discover.xid).without(missing),
            start,
        );
        assert_eq!(client.state(), State::Requesting, "without {missing:?}");
        assert!(client.lease().is_none());
    }
}

#[test]
fn an_offer_without_a_server_identifier_is_no_offer() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(
        &mut client,
        &Reply::offer(discover.xid).without(OptionCode::SERVER_IDENTIFIER),
        start,
    );
    assert_eq!(client.state(), State::Selecting);
}

#[test]
fn a_mask_whose_bits_do_not_run_together_is_no_network() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let discover = send(&mut client, &mut rng, start).expect("a discover");
    deliver(&mut client, &Reply::offer(discover.xid), start);
    send(&mut client, &mut rng, start).expect("a request");
    deliver(
        &mut client,
        &Reply::ack(discover.xid)
            .without(OptionCode::SUBNET_MASK)
            .with(opt(OptionCode::SUBNET_MASK, &[255, 0, 255, 0])),
        start,
    );
    assert_eq!(client.state(), State::Requesting);
    // A mask of the wrong length is not one either.
    deliver(
        &mut client,
        &Reply::ack(discover.xid)
            .without(OptionCode::SUBNET_MASK)
            .with(opt(OptionCode::SUBNET_MASK, &[255, 255, 255])),
        start,
    );
    assert_eq!(client.state(), State::Requesting);
}

#[test]
fn a_client_that_has_not_started_says_nothing() {
    let mut rng = rng();
    let now = Instant::from_micros(0);
    let mut client = Client::new(MAC, Config::DEFAULT);
    assert_eq!(client.state(), State::Init);
    assert_eq!(client.poll_at(), None);
    assert!(send(&mut client, &mut rng, now).is_none());
    client.on_datagram(SERVER, SERVER_PORT, &Reply::offer(1).bytes(), now);
    assert_eq!(client.state(), State::Init);
    assert_eq!(client.address(), None);
}

#[test]
fn a_generator_that_fails_stops_the_message_and_not_the_client() {
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let mut buffer = [0u8; 512];
    let mut empty = ScriptedRng::new(&[]);
    assert!(matches!(
        client.poll(start, &mut empty, &mut buffer),
        Err(DhcpError::Rng(_))
    ));
    // The id is drawn first, the jitter behind it.
    let mut half = ScriptedRng::new(&[1, 2, 3, 4]);
    assert!(matches!(
        client.poll(start, &mut half, &mut buffer),
        Err(DhcpError::Rng(_))
    ));
}

#[test]
fn a_buffer_too_small_for_a_datagram_says_so() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let mut buffer = [0u8; 16];
    assert!(matches!(
        client.poll(start, &mut rng, &mut buffer),
        Err(DhcpError::Udp(_))
    ));
}

#[test]
fn a_lease_says_why_it_could_not_be_made() {
    let now = Instant::from_micros(0);
    for (missing, code) in [
        (OptionCode::SUBNET_MASK, OptionCode::SUBNET_MASK),
        (OptionCode::SERVER_IDENTIFIER, OptionCode::SERVER_IDENTIFIER),
        (OptionCode::LEASE_TIME, OptionCode::LEASE_TIME),
    ] {
        let bytes = Reply::ack(1).without(missing).bytes();
        let message = Message::parse(&bytes).expect("a message");
        assert_eq!(
            Lease::from_reply(&message, now).err(),
            Some(DhcpError::MissingOption(code))
        );
    }
    let bytes = Reply::ack(1)
        .without(OptionCode::SUBNET_MASK)
        .with(opt(OptionCode::SUBNET_MASK, &[255, 0, 255, 0]))
        .bytes();
    let message = Message::parse(&bytes).expect("a message");
    assert_eq!(
        Lease::from_reply(&message, now).err(),
        Some(DhcpError::Netmask(Ipv4Addr::new(255, 0, 255, 0)))
    );
    let bytes = Reply::ack(1).bytes();
    let message = Message::parse(&bytes).expect("a message");
    let lease = Lease::from_reply(&message, now).expect("a lease");
    assert_eq!(lease.address(), OFFERED);
    assert_eq!(lease.granted, now);
}

#[test]
fn a_lease_of_the_longest_kind_does_not_wrap_the_clock() {
    let now = Instant::from_micros(0);
    let bytes = Reply::ack(1)
        .without(OptionCode::LEASE_TIME)
        .with(lease_time(u32::MAX))
        .bytes();
    let message = Message::parse(&bytes).expect("a message");
    let lease = Lease::from_reply(&message, now).expect("a lease");
    assert!(lease.renew < lease.rebind);
    assert!(lease.rebind < lease.expires);
    assert_eq!(
        lease.expires,
        Instant::from_micros(u64::from(u32::MAX) * 1_000_000)
    );
}

#[test]
fn an_address_no_host_can_be_given_is_no_offer_and_no_lease() {
    let now = Instant::from_micros(0);
    for address in [
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(224, 0, 0, 1),
        Ipv4Addr::new(127, 0, 0, 1),
    ] {
        let mut reply = Reply::ack(1);
        reply.yours = address;
        let bytes = reply.bytes();
        let message = Message::parse(&bytes).expect("a message");
        assert_eq!(
            Lease::from_reply(&message, now).err(),
            Some(DhcpError::Address(address))
        );

        let mut rng = rng();
        let mut client = started(Config::DEFAULT, now);
        let discover = send(&mut client, &mut rng, now).expect("a discover");
        let mut offer = Reply::offer(discover.xid);
        offer.yours = address;
        deliver(&mut client, &offer, now);
        assert_eq!(client.state(), State::Selecting, "an offer of {address}");
    }
}

#[test]
fn a_buffer_with_no_room_costs_no_attempt() {
    let mut rng = rng();
    let start = Instant::from_micros(0);
    let mut client = started(Config::DEFAULT, start);
    let mut small = [0u8; 16];
    assert!(matches!(
        client.poll(start, &mut rng, &mut small),
        Err(DhcpError::Udp(_))
    ));
    // The delay is still the first one and not the doubled one.
    send(&mut client, &mut rng, start).expect("a discover");
    let next = client.poll_at().expect("a schedule");
    assert!(
        next.saturating_duration_since(start)
            <= Config::DEFAULT
                .first_retry
                .saturating_add(Config::DEFAULT.jitter)
    );
}
