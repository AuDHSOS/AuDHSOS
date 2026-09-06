// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The DNS message against arbitrary bytes: no input may panic, whatever a
//! parser hands back stays inside the bytes it came from, and no name
//! costs unbounded work to read.
//!
//! This format has no checksum, so a mutator reaches the parser with
//! nothing in the way. What it does have is the compression pointer, which
//! is the one field of an internet format that lets a sender aim a reader
//! at any earlier byte of the message — including, if the reader is
//! careless, at the pointer itself. That is the loop this target exists
//! for, and it is why the seeds hold a pointer that points forwards, a
//! ladder of pointers, and a chain of aliases that returns to its start.
//!
//! Three doors are driven. The message is the front one, with the name
//! walk and the record walk behind it. `Name::read` is the second, called
//! at offsets the message parser would never choose, because a name is
//! read wherever a record body says one stands. The resolver is the third,
//! and reaching it takes one edit: a response is believed only when its
//! transaction id is the one that went out, so the target asks for the
//! name the input itself carries and then writes the id of that query into
//! the input. What is lost is coverage of the id comparison, which is one
//! test with a test of its own; what is gained is the whole of the answer
//! walk, the alias chain, and the six states behind them.

use audhsos_time::Instant;
use crypto_rng::doubles::ScriptedRng;
use net_dns::message::{HEADER_LEN, MAX_MESSAGE_LEN, Message};
use net_dns::name::Name;
use net_dns::record::RecordData;
use net_dns::resolver::{Config, Resolver, SERVER_PORT};
use net_wire::{IpAddr, Ipv4Addr, Port};

/// This host.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The server the resolver was given.
const SERVER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));

/// The port the caller's socket holds.
const LOCAL_PORT: Port = Port::new(49_876);

/// The script the resolver draws its transaction ids from.
const SCRIPT: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

/// Where the query bit sits in the flags field.
const FLAG_RESPONSE: u8 = 0x80;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_MESSAGE_LEN {
        return;
    }
    parsing(bytes);
    names(bytes);
    resolving(bytes);
});

/// The message, its question section, and its answer section.
fn parsing(bytes: &[u8]) {
    let Ok(message) = Message::parse(bytes) else {
        return;
    };
    for question in message.questions() {
        let Ok(question) = question else { break };
        assert!(
            question.name.as_bytes().len() <= 255,
            "a name longer than a name may be"
        );
    }
    for record in message.answers() {
        let Ok(record) = record else { break };
        assert!(
            record.name.as_bytes().len() <= 255,
            "a name longer than a name may be"
        );
        match record.data {
            RecordData::Other(body) => assert!(
                body.len() <= bytes.len(),
                "a body longer than the message it was read from"
            ),
            RecordData::Cname(name) => assert!(
                name.as_bytes().len() <= 255,
                "an alias longer than a name may be"
            ),
            RecordData::A(_) | RecordData::Aaaa(_) => {}
        }
    }
}

/// `Name::read`, at offsets a record body might name.
fn names(bytes: &[u8]) {
    for at in [0usize, HEADER_LEN, bytes.len() / 2, bytes.len()] {
        let Ok((name, after)) = Name::read(bytes, at) else {
            continue;
        };
        assert!(
            name.as_bytes().len() <= 255,
            "a name longer than a name may be"
        );
        assert!(after > at, "a name that took no bytes of the message");
        assert!(after <= bytes.len(), "a name that ended past the message");
    }
}

/// The resolver, asked the question the input carries and then handed the
/// input as the answer.
fn resolving(bytes: &[u8]) {
    let Ok(message) = Message::parse(bytes) else {
        return;
    };
    let Ok(Some(question)) = message.question() else {
        return;
    };
    let mut rng = ScriptedRng::new(&SCRIPT);
    let mut resolver = Resolver::new(HERE, LOCAL_PORT, Config::DEFAULT);
    let Ok(()) = resolver.add_server(SERVER) else {
        return;
    };
    let now = Instant::from_micros(0);
    let Ok(()) = resolver.start(&question.name, now) else {
        return;
    };
    let mut out = [0u8; 600];
    let mut id = None;
    while let Ok(Some(query)) = resolver.poll(now, &mut rng, &mut out) {
        let Some(payload) = query.datagram.get(8..) else {
            break;
        };
        let Ok(sent) = Message::parse(payload) else {
            break;
        };
        if sent
            .question()
            .ok()
            .flatten()
            .map(|asked| asked.record_type)
            == Some(question.record_type)
        {
            id = Some(sent.header.id);
        }
    }
    let Some(id) = id else {
        return;
    };
    // The response has to carry the id that went out and the response bit,
    // or it is a response to nobody and the walk behind those two tests is
    // never reached.
    let mut answer = bytes.to_vec();
    let Some(head) = answer.get_mut(..3) else {
        return;
    };
    let [high, low] = id.to_be_bytes();
    head.copy_from_slice(&[high, low, head.get(2).copied().unwrap_or(0) | FLAG_RESPONSE]);
    resolver.on_datagram(SERVER, SERVER_PORT, &answer, now);
    for address in resolver.addresses() {
        assert!(
            address.is_v4() || address.is_v6(),
            "an address of no family"
        );
    }
    // Whatever it made of the answer, the resolver still answers what it
    // will do next without looping.
    let mut steps = 0u32;
    while let Ok(Some(_)) = resolver.poll(now, &mut rng, &mut out) {
        steps = steps.saturating_add(1);
        assert!(steps <= 8, "one instant produced queries without end");
    }
}
