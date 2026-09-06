// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The TCP segment against arbitrary bytes, and the state machine behind
//! it: no input may panic, and whatever a parser hands back stays inside
//! the bytes it came from.
//!
//! The checksum stands between a mutator and everything else in this
//! format. It covers a pseudo-header of two addresses the bytes do not
//! carry, so a random stream verifies with probability `2^-16` and the
//! fuzzer would spend its whole budget on one branch. The target therefore
//! writes the right sum into a copy of the input before it parses it,
//! which puts the option list, the data offset, and the header fields
//! where the mutator can reach them. What is lost is coverage of the
//! checksum branch itself, which is one comparison with a test of its own.
//!
//! Two doors are driven. The parser is the front one, with the option list
//! behind it — a length field an attacker writes, walked by a loop that
//! only the two bounds and the end of the header stop. The state machine
//! is the other: an established connection is handed the segment and then
//! polled, which is the path a segment takes on a real host, and the one
//! where an unacceptable segment meets eleven states and four timers.

use audhsos_time::Instant;
use crypto_rng::doubles::ScriptedRng;
use net_tcp::connection::{Config, Connection, Endpoint};
use net_tcp::segment::{HEADER_LEN, Segment, reset_for};
use net_wire::{IpAddr, Ipv4Addr, Port, Protocol, Writer, transport};

/// This host.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// The other one.
const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// Where the checksum sits in the header.
const CHECKSUM_OFFSET: usize = 16;

/// The largest segment these buffers hold.
const BUFFER_LEN: usize = 512;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Some(segment) = summed(bytes) else {
        return;
    };
    parsing(&segment);
    machine(&segment);
});

/// The input with the checksum this pair of addresses gives it, or `None`
/// when it is too short to hold one.
fn summed(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < HEADER_LEN || bytes.len() > BUFFER_LEN {
        return None;
    }
    let mut copy = bytes.to_vec();
    let field = copy.get_mut(CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2)?;
    field.copy_from_slice(&[0, 0]);
    let sum = transport(HERE, PEER, Protocol::TCP, &copy).ok()?;
    let field = copy.get_mut(CHECKSUM_OFFSET..CHECKSUM_OFFSET + 2)?;
    field.copy_from_slice(&sum.to_be_bytes());
    Some(copy)
}

/// The parser, the option walk, and what a segment they accept must
/// satisfy.
fn parsing(bytes: &[u8]) {
    let Ok(segment) = Segment::parse(bytes, PEER, HERE) else {
        return;
    };
    assert!(
        segment.payload.len() <= bytes.len(),
        "a payload longer than the bytes it was read from"
    );
    assert!(
        segment.wire_len() <= bytes.len(),
        "a segment longer than the bytes it was read from"
    );
    // The two are equal only when every option the header carried is one
    // this crate writes back. An option list of no-operations, or one
    // holding an option this crate steps over, is read and not written, so
    // what goes out is shorter than what came in — which is the whole of
    // what "the options are one" means (D-50).
    assert!(
        segment.sequence_len() >= u32::try_from(segment.payload.len()).unwrap_or(u32::MAX),
        "a segment that occupies fewer numbers than it carries bytes"
    );

    // What was read, written back, reads as itself again. The option list
    // is not carried through — only the maximum segment size is — so the
    // comparison is of the fields and not of the bytes.
    let mut buffer = [0u8; BUFFER_LEN + HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    let Ok(()) = segment.write(&mut writer, PEER, HERE) else {
        return;
    };
    let written = writer.written();
    assert_eq!(
        Segment::parse(written, PEER, HERE),
        Ok(segment),
        "a segment that did not read back as itself"
    );

    // A reset answers everything but a reset, and never occupies numbers.
    if let Some(reset) = reset_for(&segment) {
        assert_eq!(
            reset.sequence_len(),
            0,
            "a reset that occupies a sequence number"
        );
        assert_eq!(
            reset.source_port, segment.destination_port,
            "a reset that goes to the wrong port"
        );
    }
}

/// The state machine, handed the segment and then polled.
fn machine(bytes: &[u8]) {
    let Ok(segment) = Segment::parse(bytes, PEER, HERE) else {
        return;
    };
    let mut send = [0u8; BUFFER_LEN];
    let mut recv = [0u8; BUFFER_LEN];
    let local = Endpoint::new(HERE, Port::new(49_152));
    let remote = Endpoint::new(PEER, segment.source_port);
    let mut connection = Connection::new(local, Config::DEFAULT, &mut send, &mut recv);
    let script = 0x1234_5678u32.to_be_bytes();
    let mut rng = ScriptedRng::new(&script);
    let Ok(()) = connection.connect(remote, &mut rng) else {
        return;
    };
    let mut out = [0u8; BUFFER_LEN + HEADER_LEN];
    for step in 0..4u64 {
        let now = Instant::from_micros(step);
        connection.on_segment(PEER, &segment, now);
        let mut sent = 0usize;
        while let Some(answer) = connection.poll(now, &mut out) {
            assert!(
                answer.len() >= HEADER_LEN,
                "a segment shorter than a header went out"
            );
            sent += 1;
            assert!(sent <= 8, "one instant produced a segment without end");
        }
        // A connection with nothing to do says so.
        if connection.poll_at(now).is_none() {
            assert!(
                !connection.state().is_open(),
                "an open connection with no work and no timer"
            );
        }
    }
}
