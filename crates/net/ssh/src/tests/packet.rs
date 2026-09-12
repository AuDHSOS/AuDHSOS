// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The binary packet of RFC 4253, section 6: what is framed, what is
//! refused, and the sequence numbers of section 6.4.

use crypto_rng::doubles::ScriptedRng;
use test_support::generators::bytes;
use test_support::property::check;

use crate::error::SshError;
use crate::packet::{
    Decoded, Decoder, Encoder, MAX_BLOCK, MAX_FRAME, MAX_PACKET, MAX_PAYLOAD, MIN_BLOCK,
    MIN_PACKET, SequenceNumber,
};

/// Padding bytes for a test that does not care what they are.
const PADDING: [u8; 512] = [0x5a; 512];

#[test]
fn an_empty_payload_makes_the_smallest_packet_the_document_allows() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    assert_eq!(encoder.encode(&[], &mut rng, &mut out), Ok(MIN_PACKET));
    // Section 6: the length field counts everything after it, and at
    // least four bytes of that are padding.
    assert_eq!(
        out.get(..5),
        Some([0x00, 0x00, 0x00, 0x0c, 0x0b].as_slice())
    );
    assert_eq!(out.get(5..16), Some([0x5a; 11].as_slice()));
}

#[test]
fn a_packet_is_a_whole_number_of_blocks_and_never_less_than_four_bytes_of_padding() {
    for block in [MIN_BLOCK, 16, 32, MAX_BLOCK] {
        let mut encoder = Encoder::new();
        assert_eq!(encoder.set_block(block), Ok(()));
        for payload_len in 0..64usize {
            let payload = PADDING.get(..payload_len).unwrap_or(&[]);
            let mut rng = ScriptedRng::new(&PADDING);
            let mut out = [0u8; MAX_BLOCK * 2 + 64];
            let framed = encoder.encode(payload, &mut rng, &mut out);
            let total = framed.unwrap_or_default();
            assert_eq!(framed, Ok(total), "block {block}, payload {payload_len}");
            let padding = usize::from(out.get(4).copied().unwrap_or_default());
            assert_eq!(total % block, 0, "block {block}, payload {payload_len}");
            assert!(padding >= 4, "block {block}, payload {payload_len}");
            assert!(padding <= 255, "block {block}, payload {payload_len}");
            assert!(total >= MIN_PACKET);
        }
    }
}

#[test]
fn what_was_framed_comes_back_out() {
    check("a packet round-trips", &bytes(0..=600), |payload| {
        let mut encoder = Encoder::new();
        let mut decoder = Decoder::new();
        let mut rng = ScriptedRng::new(&PADDING);
        let mut out = [0u8; 1024];
        let total = encoder
            .encode(payload, &mut rng, &mut out)
            .map_err(|error| format!("{error}"))?;
        match decoder.decode(out.get(..total).unwrap_or(&[])) {
            Ok(Decoded::Packet {
                payload: read,
                length,
            }) if read == payload && length == total => Ok(()),
            other => Err(format!("{other:?} is not the packet that was framed")),
        }
    });
}

#[test]
fn a_decoder_waits_for_the_length_and_then_for_the_packet() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    let total = encoder.encode(b"hello", &mut rng, &mut out).unwrap_or(0);

    let mut decoder = Decoder::new();
    for len in 0..4usize {
        assert_eq!(
            decoder.decode(out.get(..len).unwrap_or(&[])),
            Ok(Decoded::Incomplete { needed: 4 })
        );
    }
    for len in 4..total {
        assert_eq!(
            decoder.decode(out.get(..len).unwrap_or(&[])),
            Ok(Decoded::Incomplete { needed: total })
        );
    }
    // Nothing was counted while the packet was incomplete.
    assert_eq!(decoder.sequence(), 0);
    assert!(matches!(
        decoder.decode(out.get(..total).unwrap_or(&[])),
        Ok(Decoded::Packet { .. })
    ));
    assert_eq!(decoder.sequence(), 1);
}

#[test]
fn a_length_no_packet_has_is_refused_before_the_packet_is_waited_for() {
    let mut decoder = Decoder::new();
    // Under the sixteen bytes of section 6.
    assert_eq!(
        decoder.decode(&[0x00, 0x00, 0x00, 0x04]),
        Err(SshError::PacketLength(4))
    );
    // Not a whole number of blocks.
    assert_eq!(
        decoder.decode(&[0x00, 0x00, 0x00, 0x0d]),
        Err(SshError::PacketLength(13))
    );
    // Above what section 6.1 makes mandatory, and answered from four
    // bytes rather than from a buffer of that size.
    assert_eq!(
        decoder.decode(&[0x00, 0x00, 0xff, 0xfc]),
        Err(SshError::PacketLength(65532))
    );
    assert_eq!(
        decoder.decode(&[0xff, 0xff, 0xff, 0xfc]),
        Err(SshError::PacketLength(4_294_967_292))
    );
    assert_eq!(decoder.sequence(), 0);
}

#[test]
fn padding_that_is_not_inside_the_packet_is_refused() {
    let mut decoder = Decoder::new();
    let mut packet = [0u8; MIN_PACKET];
    packet
        .get_mut(..5)
        .unwrap_or_default()
        .copy_from_slice(&[0x00, 0x00, 0x00, 0x0c, 0x0c]);
    assert_eq!(decoder.decode(&packet), Err(SshError::PaddingLength(12)));

    // Section 6: there MUST be at least four bytes of padding.
    packet
        .get_mut(4..5)
        .unwrap_or_default()
        .copy_from_slice(&[0x03]);
    assert_eq!(decoder.decode(&packet), Err(SshError::PaddingLength(3)));
    assert_eq!(decoder.sequence(), 0);
}

#[test]
fn a_payload_above_what_the_document_requires_is_refused_at_both_ends() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    let payload = vec![0u8; MAX_PAYLOAD.saturating_add(1)];
    assert_eq!(
        encoder.encode(&payload, &mut rng, &mut out),
        Err(SshError::PayloadLength(MAX_PAYLOAD + 1))
    );

    // A packet within 35000 bytes can still carry a payload above 32768,
    // and this client does not have to take it. The largest packet that
    // can be one at all is MAX_FRAME, so a length above that is refused
    // from the four bytes that carry it, while one under it is refused
    // when the padding says what the payload is.
    let mut decoder = Decoder::new();
    let mut packet = vec![0u8; 33_024];
    let length = u32::try_from(packet.len().saturating_sub(4)).unwrap_or_default();
    packet
        .get_mut(..4)
        .unwrap_or_default()
        .copy_from_slice(&length.to_be_bytes());
    packet
        .get_mut(4..5)
        .unwrap_or_default()
        .copy_from_slice(&[0x04]);
    assert!(packet.len() <= MAX_FRAME && packet.len() <= MAX_PACKET);
    assert_eq!(
        decoder.decode(&packet),
        Err(SshError::PayloadLength(33_015))
    );

    let beyond = u32::try_from(MAX_FRAME.saturating_add(4)).unwrap_or_default();
    assert_eq!(
        decoder.decode(&beyond.to_be_bytes()),
        Err(SshError::PacketLength(beyond))
    );
}

#[test]
fn a_buffer_that_is_short_by_any_number_of_bytes_frames_no_packet() {
    let mut out = [0u8; 32];
    let total = Encoder::new()
        .encode(b"hello", &mut ScriptedRng::new(&PADDING), &mut out)
        .unwrap_or(0);
    assert_eq!(total, 16);
    for len in 0..total {
        let mut encoder = Encoder::new();
        let mut rng = ScriptedRng::new(&PADDING);
        let mut small = [0u8; 32];
        let answer = encoder.encode(b"hello", &mut rng, small.get_mut(..len).unwrap_or(&mut []));
        assert!(matches!(answer, Err(SshError::OutOfBounds { .. })), "{len}");
        assert_eq!(encoder.sequence(), 0);
    }
}

#[test]
fn a_generator_with_nothing_left_frames_no_packet() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&[]);
    let mut out = [0u8; 64];
    assert_eq!(
        encoder.encode(b"hello", &mut rng, &mut out),
        Err(SshError::Rng(crypto_rng::RngError::Exhausted))
    );
    assert_eq!(encoder.sequence(), 0);
}

#[test]
fn the_padding_is_one_call_on_the_generator() {
    // D-121: nothing draws randomness per byte. The scripted generator
    // hands out exactly what is asked of it, so what it has left says how
    // much one packet took.
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    let total = encoder.encode(b"hello", &mut rng, &mut out).unwrap_or(0);
    let padding = usize::from(out.get(4).copied().unwrap_or_default());
    assert_eq!(total, 16);
    assert_eq!(PADDING.len().saturating_sub(rng.left()), padding);
}

#[test]
fn a_block_no_packet_can_be_padded_to_is_refused_and_the_one_in_use_stands() {
    let mut encoder = Encoder::new();
    let mut decoder = Decoder::new();
    assert_eq!(encoder.set_block(4), Err(SshError::BlockSize(4)));
    assert_eq!(
        decoder.set_block(MAX_BLOCK + 1),
        Err(SshError::BlockSize(MAX_BLOCK + 1))
    );
    // The refused block did not take: packets are still framed and read
    // to the block that was in use.
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    assert_eq!(encoder.encode(b"x", &mut rng, &mut out), Ok(16));
    assert!(matches!(
        decoder.decode(&out),
        Ok(Decoded::Packet { length: 16, .. })
    ));
    assert_eq!(encoder.set_block(MIN_BLOCK), Ok(()));
    assert_eq!(decoder.set_block(MAX_BLOCK), Ok(()));
}

#[test]
fn a_new_block_size_does_not_reset_the_sequence_number() {
    // Section 6.4: the number is never reset, even if keys and algorithms
    // are renegotiated later, so a re-exchange changes the block and
    // nothing else.
    let mut encoder = Encoder::new();
    let mut decoder = Decoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 64];
    assert_eq!(encoder.encode(b"kexinit", &mut rng, &mut out), Ok(16));
    assert!(matches!(decoder.decode(&out), Ok(Decoded::Packet { .. })));
    assert_eq!(encoder.sequence(), 1);
    assert_eq!(decoder.sequence(), 1);

    assert_eq!(encoder.set_block(16), Ok(()));
    assert_eq!(decoder.set_block(16), Ok(()));
    assert_eq!(encoder.sequence(), 1);
    assert_eq!(decoder.sequence(), 1);

    let total = encoder
        .encode(b"the first packet after the new keys", &mut rng, &mut out)
        .unwrap_or_default();
    assert_eq!(total % 16, 0);
    assert!(matches!(
        decoder.decode(out.get(..total).unwrap_or(&[])),
        Ok(Decoded::Packet { .. })
    ));
    assert_eq!(encoder.sequence(), 2);
    assert_eq!(decoder.sequence(), 2);
}

#[test]
fn the_sequence_number_counts_every_packet_and_wraps_where_the_document_says() {
    let mut sequence = SequenceNumber::new();
    assert_eq!(sequence.get(), 0);
    assert_eq!(sequence.advance(), 0);
    assert_eq!(sequence.advance(), 1);
    assert_eq!(sequence.get(), 2);

    let mut wrapping = SequenceNumber::default();
    for _ in 0..3 {
        wrapping.advance();
    }
    assert_eq!(wrapping.get(), 3);

    // Section 6.4: it wraps around to zero after 2^32 packets.
    let mut last = SequenceNumber::at(u32::MAX);
    assert_eq!(last.advance(), u32::MAX);
    assert_eq!(last.get(), 0);
    assert_eq!(last.advance(), 0);
}

#[test]
fn both_directions_count_on_their_own() {
    let mut encoder = Encoder::default();
    let mut decoder = Decoder::default();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = [0u8; 128];
    for expected in 0..3u32 {
        assert_eq!(encoder.sequence(), expected);
        assert_eq!(decoder.sequence(), 0);
        let total = encoder.encode(b"x", &mut rng, &mut out).unwrap_or(0);
        assert_eq!(total, 16);
    }
    assert_eq!(encoder.sequence(), 3);
    assert_eq!(decoder.sequence(), 0);
    assert!(matches!(
        decoder.decode(&out),
        Ok(Decoded::Packet { length: 16, .. })
    ));
    assert_eq!(decoder.sequence(), 1);
}

#[test]
fn a_packet_of_the_largest_payload_fits_in_a_buffer_of_the_mandatory_size() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut out = vec![0u8; MAX_PACKET];
    let payload = vec![0x41u8; MAX_PAYLOAD];
    let total = encoder.encode(&payload, &mut rng, &mut out).unwrap_or(0);
    assert!(total <= MAX_PACKET);

    let mut decoder = Decoder::new();
    assert_eq!(
        decoder.decode(out.get(..total).unwrap_or(&[])),
        Ok(Decoded::Packet {
            payload: payload.as_slice(),
            length: total
        })
    );
}

#[test]
fn a_packet_is_read_out_of_a_stream_that_holds_more_than_one() {
    let mut encoder = Encoder::new();
    let mut rng = ScriptedRng::new(&PADDING);
    let mut stream = [0u8; 128];
    let mut written = 0usize;
    for payload in [b"one".as_slice(), b"two".as_slice()] {
        let slot = stream.get_mut(written..).unwrap_or(&mut []);
        written = written.saturating_add(encoder.encode(payload, &mut rng, slot).unwrap_or(0));
    }

    let mut decoder = Decoder::new();
    let mut read = 0usize;
    for expected in [b"one".as_slice(), b"two".as_slice()] {
        match decoder.decode(stream.get(read..written).unwrap_or(&[])) {
            Ok(Decoded::Packet { payload, length }) => {
                assert_eq!(payload, expected);
                read = read.saturating_add(length);
            }
            other => panic!("{other:?} is not a packet"),
        }
    }
    assert_eq!(read, written);
    assert_eq!(decoder.sequence(), 2);
}
