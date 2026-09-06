// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The resolver: what it sends, what it believes, and what it does when
//! nobody answers.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a datagram at known offsets"
)]

use audhsos_time::{Duration, Instant};
use crypto_rng::doubles::ScriptedRng;
use net_udp::{Datagram, HEADER_LEN as UDP_HEADER_LEN};
use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, Port};

use crate::error::DnsError;
use crate::message::{MAX_MESSAGE_LEN, Message, ResponseCode};
use crate::name::Name;
use crate::record::{Class, Record, RecordData, RecordType};
use crate::resolver::{Config, Query, Resolver, SERVER_PORT, Status};
use crate::tests::harness::{Response, a, aaaa, cname, name};

/// This host.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The first server, which DHCP handed over.
const FIRST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

/// The second.
const SECOND: IpAddr = IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9));

/// The port the caller's socket holds.
const LOCAL_PORT: Port = Port::new(49_876);

/// The address `example.com` has.
const V4: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);

/// The one it has over IPv6.
const V6: Ipv6Addr = Ipv6Addr::from_octets([
    0x26, 0x06, 0x28, 0x00, 0x02, 0x20, 0, 0x01, 0x02, 0x48, 0x18, 0x93, 0x25, 0xc8, 0x19, 0x46,
]);

/// A generator with enough bytes for any number of transaction ids.
fn rng() -> ScriptedRng<'static> {
    ScriptedRng::new(&[
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x0F,
        0x1F, 0x2F, 0x3F, 0x4F, 0x5F, 0x6F, 0x7F, 0x8F, 0x9F,
    ])
}

/// A resolver with two servers.
fn resolver(config: Config) -> Resolver {
    let mut resolver = Resolver::new(HERE, LOCAL_PORT, config);
    resolver.add_server(FIRST).expect("room");
    resolver.add_server(SECOND).expect("room");
    resolver
}

/// What one call to `poll` produced: where it went and what it asked.
struct Sent {
    /// Where it went.
    destination: IpAddr,
    /// The transaction id it carried.
    id: u16,
    /// The name it asked about.
    name: Name,
    /// The type it asked for.
    record_type: RecordType,
}

/// Polls once and reads back what went out.
fn send<R: crypto_rng::Rng + ?Sized>(
    resolver: &mut Resolver,
    rng: &mut R,
    now: Instant,
) -> Option<Sent> {
    let mut buffer = [0u8; 600];
    let Query {
        source,
        destination,
        datagram,
    } = resolver.poll(now, rng, &mut buffer).expect("a query")?;
    assert_eq!(source, HERE);
    let parsed = Datagram::parse(datagram, source, destination).expect("a datagram");
    assert_eq!(parsed.source_port, LOCAL_PORT);
    assert_eq!(parsed.destination_port, SERVER_PORT);
    assert_eq!(datagram.len(), UDP_HEADER_LEN + parsed.payload.len());
    let message = Message::parse(parsed.payload).expect("a message");
    assert!(!message.header.flags.is_response());
    assert!(message.header.flags.recursion_desired());
    let question = message.question().expect("a question").expect("one");
    assert_eq!(question.class, Class::IN);
    Some(Sent {
        destination,
        id: message.header.id,
        name: question.name,
        record_type: question.record_type,
    })
}

/// Everything that goes out at `now`, in the order it goes.
fn drain<R: crypto_rng::Rng + ?Sized>(
    resolver: &mut Resolver,
    rng: &mut R,
    now: Instant,
) -> Vec<Sent> {
    let mut out = Vec::new();
    while let Some(sent) = send(resolver, rng, now) {
        out.push(sent);
        assert!(out.len() <= 8, "one instant produced queries without end");
    }
    out
}

/// Hands `response` to `resolver` as if it had arrived from `from`.
fn deliver(resolver: &mut Resolver, from: IpAddr, response: &Response, now: Instant) {
    resolver.on_datagram(from, SERVER_PORT, &response.bytes(), now);
}

#[test]
fn both_families_are_asked_at_once_and_both_answers_come_back() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("a resolution");
    assert_eq!(resolver.status(), Status::Asking);

    let sent = drain(&mut resolver, &mut rng, start);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].record_type, RecordType::A);
    assert_eq!(sent[1].record_type, RecordType::AAAA);
    assert_ne!(sent[0].id, sent[1].id);
    // The two questions start at different servers.
    assert_eq!(sent[0].destination, FIRST);
    assert_eq!(sent[1].destination, SECOND);
    assert!(drain(&mut resolver, &mut rng, start).is_empty());

    deliver(
        &mut resolver,
        FIRST,
        &Response::new(sent[0].id, "example.com", RecordType::A).with(&a("example.com", V4)),
        start,
    );
    assert_eq!(resolver.status(), Status::Asking);
    deliver(
        &mut resolver,
        SECOND,
        &Response::new(sent[1].id, "example.com", RecordType::AAAA).with(&aaaa("example.com", V6)),
        start,
    );
    assert_eq!(resolver.status(), Status::Done);
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4), IpAddr::V6(V6)]
    );
    assert_eq!(resolver.poll_at(), None);
}

#[test]
fn a_response_that_fails_one_of_the_four_checks_is_ignored() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    let good = Response::new(sent[0].id, "example.com", RecordType::A).with(&a("example.com", V4));

    // A wrong transaction id.
    let mut wrong = Response::new(sent[0].id ^ 1, "example.com", RecordType::A);
    wrong = wrong.with(&a("example.com", V4));
    deliver(&mut resolver, FIRST, &wrong, start);
    assert_eq!(resolver.addresses().count(), 0);

    // A wrong question section.
    let other = Response::new(sent[0].id, "example.org", RecordType::A).with(&a("example.org", V4));
    deliver(&mut resolver, FIRST, &other, start);
    assert_eq!(resolver.addresses().count(), 0);

    // A source address that is no server of this resolver's.
    deliver(
        &mut resolver,
        IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
        &good,
        start,
    );
    assert_eq!(resolver.addresses().count(), 0);

    // A wrong source port.
    resolver.on_datagram(FIRST, Port::new(1024), &good.bytes(), start);
    assert_eq!(resolver.addresses().count(), 0);

    // And the one that passes all four.
    deliver(&mut resolver, FIRST, &good, start);
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4)]
    );
}

#[test]
fn a_message_that_is_not_a_response_is_ignored() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);

    // A query, not a response.
    let mut query = [0u8; 600];
    let mut writer = net_wire::Writer::new(&mut query);
    crate::message::write_query(&mut writer, sent[0].id, &name("example.com"), RecordType::A)
        .expect("room");
    let bytes = writer.finish().to_vec();
    resolver.on_datagram(FIRST, SERVER_PORT, &bytes, start);
    assert_eq!(resolver.addresses().count(), 0);

    // Bytes that are no message at all.
    resolver.on_datagram(FIRST, SERVER_PORT, &[0u8; 3], start);
    assert_eq!(resolver.addresses().count(), 0);

    // A response whose answer section cannot be walked: the header
    // promises a record and the message stops behind the question.
    let mut cut = Response::new(sent[0].id, "example.com", RecordType::A)
        .with(&a("example.com", V4))
        .bytes();
    cut.truncate(cut.len() - 3);
    resolver.on_datagram(FIRST, SERVER_PORT, &cut, start);
    assert_eq!(resolver.addresses().count(), 0);
    assert_eq!(resolver.status(), Status::Asking);
}

#[test]
fn a_query_is_asked_again_at_the_next_server_and_then_given_up() {
    let config = Config {
        retry: Duration::from_secs(1),
        attempts: 3,
        deadline: Duration::from_secs(30),
    };
    let mut rng = rng();
    let mut resolver = resolver(config);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");

    let first = drain(&mut resolver, &mut rng, start);
    assert_eq!(first[0].destination, FIRST);
    assert_eq!(first[1].destination, SECOND);
    let id = first[0].id;

    // Nothing is due before the retry.
    let second = Instant::from_micros(1_000_000);
    assert_eq!(resolver.poll_at(), Some(second));
    assert!(drain(&mut resolver, &mut rng, Instant::from_micros(999_999)).is_empty());

    let again = drain(&mut resolver, &mut rng, second);
    assert_eq!(again.len(), 2);
    // The id is kept across retries, so an answer that is merely late is
    // still an answer.
    assert_eq!(again[0].id, id);
    assert_eq!(again[0].destination, SECOND);
    assert_eq!(again[1].destination, FIRST);

    let third = Instant::from_micros(2_000_000);
    let last = drain(&mut resolver, &mut rng, third);
    assert_eq!(last.len(), 2);
    assert_eq!(last[0].destination, FIRST);

    // The attempts are spent; the next instant retires both questions.
    let fourth = Instant::from_micros(3_000_000);
    assert!(drain(&mut resolver, &mut rng, fourth).is_empty());
    assert_eq!(resolver.status(), Status::Failed(DnsError::Exhausted));
    assert_eq!(resolver.poll_at(), None);
}

#[test]
fn an_answer_that_is_merely_late_is_still_an_answer() {
    let config = Config {
        retry: Duration::from_secs(1),
        attempts: 3,
        deadline: Duration::from_secs(30),
    };
    let mut rng = rng();
    let mut resolver = resolver(config);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let first = drain(&mut resolver, &mut rng, start);
    let later = Instant::from_micros(1_500_000);
    drain(&mut resolver, &mut rng, Instant::from_micros(1_000_000));
    deliver(
        &mut resolver,
        FIRST,
        &Response::new(first[0].id, "example.com", RecordType::A).with(&a("example.com", V4)),
        later,
    );
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4)]
    );
}

#[test]
fn the_deadline_ends_a_resolution_that_nobody_answers() {
    let config = Config {
        retry: Duration::from_secs(1),
        attempts: 100,
        deadline: Duration::from_secs(2),
    };
    let mut rng = ScriptedRng::new(&[0xAB, 0xCD, 0xEF, 0x01]);
    let mut resolver = resolver(config);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    drain(&mut resolver, &mut rng, start);
    assert_eq!(resolver.poll_at(), Some(Instant::from_micros(1_000_000)));
    let over = Instant::from_micros(2_000_000);
    assert!(drain(&mut resolver, &mut rng, over).is_empty());
    assert_eq!(resolver.status(), Status::Failed(DnsError::Deadline));
}

#[test]
fn a_name_that_does_not_exist_is_an_answer_and_not_a_failure() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    for one in &sent {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "example.com", one.record_type).coded(ResponseCode::NAME_ERROR),
            start,
        );
    }
    assert_eq!(resolver.status(), Status::Done);
    assert_eq!(resolver.addresses().count(), 0);
}

#[test]
fn a_server_that_breaks_down_is_asked_again_at_once() {
    let config = Config {
        retry: Duration::from_secs(5),
        attempts: 2,
        deadline: Duration::from_secs(30),
    };
    let mut rng = rng();
    let mut resolver = Resolver::new(HERE, LOCAL_PORT, config);
    resolver.add_server(FIRST).expect("room");
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    assert_eq!(sent.len(), 2);
    for one in &sent {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "example.com", one.record_type)
                .coded(ResponseCode::SERVER_FAILURE),
            start,
        );
    }
    // Not after the retry — now, and the second attempt is the last.
    let again = drain(&mut resolver, &mut rng, start);
    assert_eq!(again.len(), 2);
    assert_eq!(resolver.status(), Status::Asking);
    for one in &again {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "example.com", one.record_type).coded(ResponseCode::REFUSED),
            start,
        );
    }
    assert_eq!(
        resolver.status(),
        Status::Failed(DnsError::Rcode(ResponseCode::REFUSED))
    );
}

#[test]
fn a_truncated_answer_is_a_failure_because_there_is_no_tcp_here() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    for one in &sent {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "example.com", one.record_type).cut_short(),
            start,
        );
    }
    assert_eq!(resolver.status(), Status::Failed(DnsError::Truncated));
}

#[test]
fn an_alias_chain_inside_one_answer_is_followed() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("www.example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    deliver(
        &mut resolver,
        FIRST,
        &Response::new(sent[0].id, "www.example.com", RecordType::A)
            .with(&cname("www.example.com", "web.example.net"))
            .with(&cname("web.example.net", "host.example.net"))
            .with(&a("host.example.net", V4)),
        start,
    );
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4)]
    );
}

#[test]
fn an_alias_the_answer_does_not_resolve_is_asked_on_its_own() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("www.example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    deliver(
        &mut resolver,
        FIRST,
        &Response::new(sent[0].id, "www.example.com", RecordType::A)
            .with(&cname("www.example.com", "host.example.net")),
        start,
    );
    // A new question, about the target, with a new id.
    let again = drain(&mut resolver, &mut rng, start);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].name, name("host.example.net"));
    assert_ne!(again[0].id, sent[0].id);
    deliver(
        &mut resolver,
        again[0].destination,
        &Response::new(again[0].id, "host.example.net", RecordType::A)
            .with(&a("host.example.net", V4)),
        start,
    );
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4)]
    );
}

#[test]
fn an_alias_chain_that_returns_to_a_record_it_has_used_is_a_loop() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver.start(&name("a.example"), start).expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    for one in &sent {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "a.example", one.record_type)
                .with(&cname("a.example", "b.example"))
                .with(&cname("b.example", "a.example")),
            start,
        );
    }
    assert_eq!(resolver.status(), Status::Failed(DnsError::CnameLoop));
}

#[test]
fn an_alias_chain_longer_than_eight_links_is_refused() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver.start(&name("l0.example"), start).expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    for one in &sent {
        let mut response = Response::new(one.id, "l0.example", one.record_type);
        for link in 0..10u32 {
            let owner = format!("l{link}.example");
            let target = format!("l{}.example", link + 1);
            response = response.with(&cname_owned(&owner, &target));
        }
        deliver(&mut resolver, FIRST, &response, start);
    }
    assert_eq!(resolver.status(), Status::Failed(DnsError::CnameChain));
}

/// A `CNAME` built from two owned texts.
fn cname_owned(owner: &str, target: &str) -> Record<'static> {
    Record {
        name: name(owner),
        record_type: RecordType::CNAME,
        class: Class::IN,
        ttl: 60,
        data: RecordData::Cname(name(target)),
    }
}

#[test]
fn a_resolver_with_no_server_asks_nobody() {
    let mut resolver = Resolver::new(HERE, LOCAL_PORT, Config::DEFAULT);
    assert_eq!(resolver.status(), Status::Idle);
    assert_eq!(
        resolver.start(&name("example.com"), Instant::from_micros(0)),
        Err(DnsError::NoServer)
    );
    assert_eq!(resolver.poll_at(), None);
}

#[test]
fn a_server_of_the_other_family_is_not_one_this_resolver_can_reach() {
    let mut resolver = Resolver::new(HERE, LOCAL_PORT, Config::DEFAULT);
    assert_eq!(
        resolver.add_server(IpAddr::V6(V6)),
        Err(DnsError::MixedFamilies)
    );
    for _ in 0..4 {
        resolver.add_server(FIRST).expect("room");
    }
    assert_eq!(resolver.add_server(SECOND), Err(DnsError::TooManyServers));
    assert_eq!(resolver.servers().count(), 4);
}

#[test]
fn a_second_resolution_waits_for_the_first_and_then_starts_clean() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    assert_eq!(
        resolver.start(&name("example.org"), start),
        Err(DnsError::Busy)
    );
    let sent = drain(&mut resolver, &mut rng, start);
    for one in &sent {
        deliver(
            &mut resolver,
            FIRST,
            &Response::new(one.id, "example.com", one.record_type).with(&a("example.com", V4)),
            start,
        );
    }
    assert_eq!(resolver.status(), Status::Done);
    resolver.reset();
    assert_eq!(resolver.status(), Status::Idle);
    assert_eq!(resolver.addresses().count(), 0);
    resolver
        .start(&name("example.org"), start)
        .expect("started");
    assert_eq!(resolver.status(), Status::Asking);
}

#[test]
fn one_address_that_came_back_twice_is_one_address() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    deliver(
        &mut resolver,
        FIRST,
        &Response::new(sent[0].id, "example.com", RecordType::A)
            .with(&a("example.com", V4))
            .with(&a("example.com", V4))
            .with(&a("example.com", Ipv4Addr::new(93, 184, 216, 35))),
        start,
    );
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4), IpAddr::V4(Ipv4Addr::new(93, 184, 216, 35))]
    );
}

#[test]
fn a_record_of_the_other_family_does_not_answer_this_question() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    // An `AAAA` record in the answer to the `A` question settles it with
    // nothing, because the two questions are kept apart.
    deliver(
        &mut resolver,
        FIRST,
        &Response::new(sent[0].id, "example.com", RecordType::A).with(&aaaa("example.com", V6)),
        start,
    );
    assert_eq!(resolver.addresses().count(), 0);
    assert_eq!(resolver.status(), Status::Asking);
    deliver(
        &mut resolver,
        SECOND,
        &Response::new(sent[1].id, "example.com", RecordType::AAAA).with(&aaaa("example.com", V6)),
        start,
    );
    assert_eq!(resolver.status(), Status::Done);
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V6(V6)]
    );
}

#[test]
fn a_generator_that_fails_stops_the_query_and_not_the_resolver() {
    let mut rng = ScriptedRng::new(&[0x01]);
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let mut buffer = [0u8; 600];
    assert!(matches!(
        resolver.poll(start, &mut rng, &mut buffer),
        Err(DnsError::Rng(_))
    ));
}

#[test]
fn a_buffer_too_small_for_a_datagram_says_so() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let mut buffer = [0u8; 8];
    assert!(matches!(
        resolver.poll(start, &mut rng, &mut buffer),
        Err(DnsError::Udp(_))
    ));
}

#[test]
fn an_idle_resolver_polls_to_nothing() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let now = Instant::from_micros(0);
    let mut buffer = [0u8; 600];
    assert_eq!(resolver.poll(now, &mut rng, &mut buffer), Ok(None));
    resolver.on_datagram(FIRST, SERVER_PORT, &[0u8; 12], now);
    assert_eq!(resolver.status(), Status::Idle);
}

#[test]
fn a_response_larger_than_a_datagram_of_this_format_is_ignored() {
    let mut rng = rng();
    let mut resolver = resolver(Config::DEFAULT);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let sent = drain(&mut resolver, &mut rng, start);
    // The same answer with padding behind it, past what a server may send
    // a resolver that announced no buffer of its own.
    let mut swollen = Response::new(sent[0].id, "example.com", RecordType::A)
        .with(&a("example.com", V4))
        .bytes();
    swollen.resize(MAX_MESSAGE_LEN + 1, 0);
    resolver.on_datagram(FIRST, SERVER_PORT, &swollen, start);
    assert_eq!(resolver.addresses().count(), 0);
    swollen.truncate(MAX_MESSAGE_LEN);
    resolver.on_datagram(FIRST, SERVER_PORT, &swollen, start);
    assert_eq!(
        resolver.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(V4)]
    );
}

#[test]
fn a_buffer_with_no_room_costs_no_attempt() {
    let config = Config {
        retry: Duration::from_secs(1),
        attempts: 1,
        deadline: Duration::from_secs(30),
    };
    let mut rng = rng();
    let mut resolver = resolver(config);
    let start = Instant::from_micros(0);
    resolver
        .start(&name("example.com"), start)
        .expect("started");
    let mut small = [0u8; 8];
    assert!(matches!(
        resolver.poll(start, &mut rng, &mut small),
        Err(DnsError::Udp(_))
    ));
    // The one attempt each question had is still there.
    let sent = drain(&mut resolver, &mut rng, start);
    assert_eq!(sent.len(), 2);
    assert_eq!(resolver.status(), Status::Asking);
}
