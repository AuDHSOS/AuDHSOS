// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::keyboard`.

use test_support::generators::bytes;
use test_support::property::check;

use crate::device::{ACK, RESEND};
use crate::keyboard::{
    Decoder, EXTENDED_PREFIX, Encoding, KeyCode, KeyEvent, PAUSE_PREFIX, RELEASE_PREFIX, State,
};

/// The eight bytes the pause key sends, which is the whole of it: the key
/// has no break code.
const PAUSE: [u8; 8] = [0xE1, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];

/// Every event a stream produces.
fn decode(bytes: &[u8]) -> Vec<KeyEvent> {
    let mut decoder = Decoder::new();
    bytes
        .iter()
        .filter_map(|byte| decoder.feed(*byte))
        .collect()
}

#[test]
fn every_key_has_its_own_code_and_its_own_encoding() {
    let mut codes: Vec<u16> = KeyCode::ALL.iter().map(|key| key.code()).collect();
    let count = codes.len();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), count, "two keys share a code");
    assert_eq!(count, 105, "the layout is a 105-key one");

    let mut encodings: Vec<Encoding> = KeyCode::ALL.iter().map(|key| key.encoding()).collect();
    encodings.sort_unstable_by_key(|encoding| match encoding {
        Encoding::Plain(scan) => (0u8, *scan),
        Encoding::Extended(scan) => (1, *scan),
        Encoding::Sequence(scan) => (2, *scan),
    });
    let all = encodings.len();
    encodings.dedup();
    assert_eq!(encodings.len(), all, "two keys share an encoding");
}

#[test]
fn a_code_decodes_back_to_the_key_it_names_and_nothing_else_does() {
    for key in KeyCode::ALL {
        assert_eq!(KeyCode::from_code(key.code()), Some(*key));
        assert!(!key.name().is_empty());
    }
    assert_eq!(KeyCode::from_code(0), None);
    assert_eq!(KeyCode::from_code(u16::MAX), None);
}

#[test]
fn every_key_of_the_table_is_found_by_the_bytes_that_spell_it() {
    for key in KeyCode::ALL {
        match key.encoding() {
            Encoding::Plain(scan) => {
                assert_eq!(KeyCode::from_set2(scan), Some(*key), "{}", key.name());
                assert_eq!(decode(&[scan]), vec![KeyEvent::down(*key)]);
                assert_eq!(
                    decode(&[RELEASE_PREFIX, scan]),
                    vec![KeyEvent::up(*key)],
                    "the release marker of {}",
                    key.name()
                );
            }
            Encoding::Extended(scan) => {
                assert_eq!(
                    KeyCode::from_set2_extended(scan),
                    Some(*key),
                    "{}",
                    key.name()
                );
                assert_eq!(
                    decode(&[EXTENDED_PREFIX, scan]),
                    vec![KeyEvent::down(*key)],
                    "the extended prefix of {}",
                    key.name()
                );
                assert_eq!(
                    decode(&[EXTENDED_PREFIX, RELEASE_PREFIX, scan]),
                    vec![KeyEvent::up(*key)]
                );
            }
            Encoding::Sequence(prefix) => {
                assert_eq!(prefix, PAUSE_PREFIX);
                assert_eq!(decode(&PAUSE), vec![KeyEvent::down(*key)]);
            }
        }
    }
}

#[test]
fn a_word_typed_and_let_go_of_comes_out_in_the_order_it_was_typed() {
    let typed = [0x1C, 0xF0, 0x1C, 0x1B, 0xF0, 0x1B, 0x23, 0xF0, 0x23];
    assert_eq!(
        decode(&typed),
        vec![
            KeyEvent::down(KeyCode::A),
            KeyEvent::up(KeyCode::A),
            KeyEvent::down(KeyCode::S),
            KeyEvent::up(KeyCode::S),
            KeyEvent::down(KeyCode::D),
            KeyEvent::up(KeyCode::D),
        ]
    );
}

#[test]
fn the_same_byte_names_two_keys_and_the_prefix_says_which() {
    assert_eq!(decode(&[0x71]), vec![KeyEvent::down(KeyCode::NumpadPeriod)]);
    assert_eq!(
        decode(&[EXTENDED_PREFIX, 0x71]),
        vec![KeyEvent::down(KeyCode::Delete)]
    );
}

#[test]
fn a_prefix_at_the_end_of_the_stream_waits_for_the_next_byte() {
    let mut decoder = Decoder::new();
    assert_eq!(decoder.feed(EXTENDED_PREFIX), None);
    assert_eq!(decoder.state(), State::Extended);
    assert_eq!(decoder.feed(RELEASE_PREFIX), None);
    assert_eq!(decoder.state(), State::ExtendedRelease);
    assert_eq!(decoder.feed(0x75), Some(KeyEvent::up(KeyCode::ArrowUp)));
    assert_eq!(decoder.state(), State::Idle);

    let mut decoder = Decoder::new();
    assert_eq!(decoder.feed(RELEASE_PREFIX), None);
    assert_eq!(decoder.state(), State::Release);
}

#[test]
fn the_pause_sequence_is_counted_out_and_ends_in_one_event() {
    let mut decoder = Decoder::new();
    for (index, byte) in PAUSE.iter().enumerate() {
        let event = decoder.feed(*byte);
        if index < PAUSE.len().saturating_sub(1) {
            assert_eq!(event, None, "byte {index} of the sequence is no event");
        } else {
            assert_eq!(event, Some(KeyEvent::down(KeyCode::Pause)));
        }
    }
    assert_eq!(decoder.state(), State::Idle);
}

#[test]
fn an_unknown_code_is_dropped_without_losing_the_decoder_state() {
    let mut decoder = Decoder::new();
    assert_eq!(decoder.feed(0x00), None, "no key spells zero");
    assert_eq!(decoder.state(), State::Idle);
    assert_eq!(decoder.feed(0x1C), Some(KeyEvent::down(KeyCode::A)));

    // An unknown byte behind a prefix costs that one event and no more.
    assert_eq!(decoder.feed(EXTENDED_PREFIX), None);
    assert_eq!(decoder.feed(0x00), None);
    assert_eq!(decoder.state(), State::Idle);
    assert_eq!(decoder.feed(0x1B), Some(KeyEvent::down(KeyCode::S)));

    assert_eq!(decoder.feed(RELEASE_PREFIX), None);
    assert_eq!(decoder.feed(0x00), None);
    assert_eq!(decoder.feed(0x23), Some(KeyEvent::down(KeyCode::D)));
}

#[test]
fn the_answers_of_a_command_are_no_keys_and_leave_the_decoder_where_it_was() {
    let mut decoder = Decoder::new();
    assert_eq!(decoder.feed(ACK), None);
    assert_eq!(decoder.feed(RESEND), None);
    assert_eq!(decoder.state(), State::Idle);
    assert_eq!(decoder.feed(0x76), Some(KeyEvent::down(KeyCode::Escape)));
}

#[test]
fn a_decoder_that_has_lost_bytes_is_put_back_to_the_beginning() {
    let mut decoder = Decoder::new();
    assert_eq!(decoder.feed(EXTENDED_PREFIX), None);
    decoder.reset();
    assert_eq!(decoder.state(), State::Idle);
    assert_eq!(decoder.feed(0x1C), Some(KeyEvent::down(KeyCode::A)));
    assert_eq!(Decoder::default().state(), State::default());
}

#[test]
fn any_stream_of_bytes_decodes_without_panicking_and_with_bounded_state() {
    check("scancode stream", &bytes(0..=64), |stream| {
        let mut decoder = Decoder::new();
        for byte in stream {
            let _event = decoder.feed(*byte);
            if let State::Pause(seen) = decoder.state()
                && seen >= crate::keyboard::PAUSE_TAIL
            {
                return Err(format!("the pause counter reached {seen}"));
            }
        }
        Ok(())
    });
}
