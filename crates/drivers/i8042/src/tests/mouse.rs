// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mouse`.

use test_support::generators::bytes;
use test_support::property::check;

use crate::device::WHEEL_ID;
use crate::mouse::{
    BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, DELTA_MAX, DELTA_MIN, Decoder, PACKET_LEN,
    PointerEvent, SYNC, WHEEL_PACKET_LEN, X_OVERFLOW, X_SIGN, Y_OVERFLOW, Y_SIGN, packet_len,
};

/// Every event a stream of bytes produces on a decoder for `id`.
fn decode(id: u8, stream: &[u8]) -> Vec<PointerEvent> {
    let mut decoder = Decoder::for_id(id);
    stream
        .iter()
        .filter_map(|byte| decoder.feed(*byte))
        .collect()
}

#[test]
fn the_identifier_of_the_device_says_how_long_a_packet_is() {
    assert_eq!(packet_len(0), PACKET_LEN);
    assert_eq!(packet_len(WHEEL_ID), WHEEL_PACKET_LEN);
    assert_eq!(
        packet_len(4),
        PACKET_LEN,
        "an identifier this driver does not know sends three bytes"
    );
    assert_eq!(Decoder::for_id(WHEEL_ID).packet_len(), WHEEL_PACKET_LEN);
    assert_eq!(Decoder::new(9).packet_len(), PACKET_LEN);
    assert_eq!(Decoder::default().packet_len(), PACKET_LEN);
}

#[test]
fn a_three_byte_packet_carries_the_buttons_and_the_two_deltas() {
    assert_eq!(
        decode(0, &[SYNC | BUTTON_LEFT, 5, 7]),
        vec![PointerEvent {
            dx: 5,
            dy: -7,
            wheel: 0,
            buttons: BUTTON_LEFT,
        }],
        "the wire counts rows upwards and a surface counts them downwards"
    );
    assert_eq!(
        decode(0, &[SYNC | BUTTON_RIGHT | BUTTON_MIDDLE, 0, 0])
            .first()
            .map(|event| event.buttons),
        Some(BUTTON_RIGHT | BUTTON_MIDDLE)
    );
}

#[test]
fn a_four_byte_packet_carries_the_wheel_as_well() {
    assert_eq!(
        decode(WHEEL_ID, &[SYNC, 0, 0, 0x01]),
        vec![PointerEvent {
            dx: 0,
            dy: 0,
            wheel: 1,
            buttons: 0,
        }]
    );
    assert_eq!(
        decode(WHEEL_ID, &[SYNC, 0, 0, 0x0F])
            .first()
            .map(|event| event.wheel),
        Some(-1),
        "the wheel field is four bits in two's complement"
    );
    assert_eq!(
        decode(WHEEL_ID, &[SYNC, 0, 0, 0x08])
            .first()
            .map(|event| event.wheel),
        Some(-8)
    );
    assert_eq!(
        decode(WHEEL_ID, &[SYNC, 0, 0, 0xF7])
            .first()
            .map(|event| event.wheel),
        Some(7),
        "the bits above the field are the fourth and fifth buttons and are not the wheel"
    );
}

#[test]
fn a_three_byte_decoder_reports_no_wheel_however_the_fourth_byte_reads() {
    // The fourth byte of the stream is the first byte of the next packet,
    // and it has no sync bit, so it is dropped rather than read as a wheel.
    assert_eq!(
        decode(0, &[SYNC, 1, 1, 0x0F]),
        vec![PointerEvent {
            dx: 1,
            dy: -1,
            wheel: 0,
            buttons: 0,
        }]
    );
}

#[test]
fn the_sign_bits_extend_the_deltas_downwards() {
    assert_eq!(
        decode(0, &[SYNC | X_SIGN | Y_SIGN, 0xFF, 0xFF]),
        vec![PointerEvent {
            dx: -1,
            dy: 1,
            wheel: 0,
            buttons: 0,
        }]
    );
    assert_eq!(
        decode(0, &[SYNC | X_SIGN, 0x00, 0x00])
            .first()
            .map(|event| event.dx),
        Some(DELTA_MIN),
        "a sign with nothing under it is the smallest delta of the nine bits"
    );
}

#[test]
fn the_overflow_bits_clamp_the_deltas_instead_of_wrapping_them() {
    assert_eq!(
        decode(0, &[SYNC | X_OVERFLOW | Y_OVERFLOW, 0x01, 0x01]),
        vec![PointerEvent {
            dx: DELTA_MAX,
            dy: DELTA_MAX.saturating_neg(),
            wheel: 0,
            buttons: 0,
        }]
    );
    assert_eq!(
        decode(
            0,
            &[SYNC | X_OVERFLOW | X_SIGN | Y_OVERFLOW | Y_SIGN, 0x01, 0x01]
        ),
        vec![PointerEvent {
            dx: DELTA_MIN,
            dy: DELTA_MIN.saturating_neg(),
            wheel: 0,
            buttons: 0,
        }]
    );
}

#[test]
fn a_byte_without_the_sync_bit_is_dropped_until_a_first_byte_arrives() {
    let mut decoder = Decoder::new(PACKET_LEN);
    for byte in [0x00, 0x01, 0x07, 0xF0] {
        assert_eq!(decoder.feed(byte), None, "{byte:#04x} is no first byte");
        assert_eq!(decoder.pending(), 0, "and nothing of it was kept");
    }
    assert_eq!(decoder.feed(SYNC | BUTTON_LEFT), None);
    assert_eq!(decoder.pending(), 1);
    assert_eq!(decoder.feed(2), None);
    assert_eq!(
        decoder.feed(3),
        Some(PointerEvent {
            dx: 2,
            dy: -3,
            wheel: 0,
            buttons: BUTTON_LEFT,
        })
    );
    assert_eq!(decoder.pending(), 0, "the buffer is empty again");
}

#[test]
fn a_stream_that_lost_a_byte_finds_the_next_packet_that_begins() {
    // The first packet is short by one byte, so the first byte of the
    // second is read as its last. The packet after that is whole.
    let stream = [SYNC, 1, SYNC | BUTTON_RIGHT, 4, 5, SYNC, 6, 7];
    let events = decode(0, &stream);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events.last().copied(),
        Some(PointerEvent {
            dx: 6,
            dy: -7,
            wheel: 0,
            buttons: 0,
        })
    );
}

#[test]
fn a_decoder_that_has_lost_bytes_throws_the_half_packet_away() {
    let mut decoder = Decoder::new(PACKET_LEN);
    assert_eq!(decoder.feed(SYNC), None);
    assert_eq!(decoder.pending(), 1);
    decoder.reset();
    assert_eq!(decoder.pending(), 0);
    assert_eq!(
        decoder.feed(0x01),
        None,
        "and the next byte is judged as a first byte, which it is not"
    );
    assert_eq!(decoder.pending(), 0);
}

#[test]
fn any_stream_of_bytes_packetizes_without_panicking_and_with_bounded_state() {
    check("mouse packets", &bytes(0..=64), |stream| {
        for id in [0, WHEEL_ID] {
            let mut decoder = Decoder::for_id(id);
            for byte in stream {
                let _event = decoder.feed(*byte);
                if decoder.pending() >= decoder.packet_len() {
                    return Err(format!("{} bytes stand in the buffer", decoder.pending()));
                }
            }
        }
        Ok(())
    });
}
