// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The binary packet against arbitrary bytes: the length field a peer
//! chooses, the padding it names, and the round trip.
//!
//! A packet that is read must lie inside the bytes it was read from, and
//! its payload must be shorter than the packet that carried it. The round
//! trip runs the other way: what this crate frames, this crate reads back,
//! and the payload that comes out is the payload that went in.

use audhsos_ssh::packet::{Decoded, Decoder, Encoder, MAX_PAYLOAD};
use crypto_rng::doubles::CountingEntropy;
use crypto_rng::ChaChaRng;

/// The key both directions use here. The cipher has a target of its own in
/// `crypto-aead`; what is steered by here is the framing around it.
const KEY: [u8; 64] = [0x5c; 64];

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    read(bytes, false);
    read(bytes, true);
    round_trip(bytes, false);
    round_trip(bytes, true);
});

/// A generator that never runs out, for the padding.
fn generator() -> ChaChaRng<CountingEntropy> {
    ChaChaRng::new(CountingEntropy::new(0x11)).expect("the source delivers")
}

/// Reads the input as a packet, with the cipher in use or without it.
fn read(bytes: &[u8], sealed: bool) {
    let mut decoder = Decoder::new();
    if sealed {
        decoder.set_cipher(&KEY);
    }
    let mut copy = bytes.to_vec();
    let before = decoder.sequence();
    match decoder.decode(&mut copy) {
        Ok(Decoded::Packet { payload, length }) => {
            assert!(length <= bytes.len(), "a packet longer than the input");
            assert!(
                payload.len() < length,
                "a payload no shorter than the packet that carried it"
            );
            assert!(payload.len() <= MAX_PAYLOAD, "a payload above the maximum");
            assert_eq!(
                decoder.sequence(),
                before.wrapping_add(1),
                "a packet that was read and not counted"
            );
        }
        Ok(Decoded::Incomplete { needed }) => {
            assert!(
                needed > bytes.len(),
                "a packet that is incomplete and fits in what arrived"
            );
            assert_eq!(
                decoder.sequence(),
                before,
                "a packet that was not whole and was counted"
            );
        }
        Err(_) => assert_eq!(
            decoder.sequence(),
            before,
            "a packet that was refused and counted"
        ),
    }
}

/// Frames the input as a payload and reads it back.
fn round_trip(bytes: &[u8], sealed: bool) {
    let payload = bytes.get(..bytes.len().min(MAX_PAYLOAD)).unwrap_or(&[]);
    let mut encoder = Encoder::new();
    let mut decoder = Decoder::new();
    if sealed {
        encoder.set_cipher(&KEY);
        decoder.set_cipher(&KEY);
    }
    let mut out = vec![0u8; payload.len().saturating_add(512)];
    let mut rng = generator();
    let Ok(length) = encoder.encode(payload, &mut rng, &mut out) else {
        return;
    };
    let mut frame = out.get(..length).unwrap_or(&[]).to_vec();
    let Ok(Decoded::Packet {
        payload: read_back,
        length: used,
    }) = decoder.decode(&mut frame)
    else {
        panic!("what this crate framed, this crate does not read");
    };
    assert_eq!(used, length, "the packet is not as long as it was framed");
    assert_eq!(read_back, payload, "the payload did not survive");
}
