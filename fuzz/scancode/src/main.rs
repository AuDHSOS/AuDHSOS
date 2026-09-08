// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The decoder of scancode set 2 against arbitrary bytes: no input may
//! panic, no input may make the decoder grow, and every event it hands out
//! must name a key of the table.

use driver_i8042::keyboard::{Decoder, KeyCode, PAUSE_BYTES, PAUSE_PREFIX, PAUSE_TAIL, State};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let mut decoder = Decoder::new();
    let mut tail = [0u8; 8];
    for byte in bytes {
        tail.rotate_left(1);
        tail[7] = *byte;
        let event = decoder.feed(*byte);
        // The state machine holds one byte of history and a counter that
        // never leaves its range; a stream that made it grow would be a
        // driver that a device could fill memory through.
        if let State::Pause(seen) = decoder.state() {
            assert!(seen < PAUSE_TAIL, "the pause counter reached {seen}");
        }
        let Some(event) = event else {
            continue;
        };
        if event.code == KeyCode::Pause {
            assert_eq!(tail[0], PAUSE_PREFIX);
            assert_eq!(
                &tail[1..],
                PAUSE_BYTES.as_slice(),
                "only the actual Pause sequence may produce Pause"
            );
        }
        assert_eq!(
            KeyCode::from_code(event.code.code()),
            Some(event.code),
            "an event names a key the table does not have"
        );
        assert_eq!(
            decoder.state(),
            State::Idle,
            "an event leaves nothing pending"
        );
    }
    // What is pending is bounded whatever came before, so a decoder that
    // was reset is a decoder at the beginning.
    decoder.reset();
    assert_eq!(decoder.state(), State::Idle);
});
