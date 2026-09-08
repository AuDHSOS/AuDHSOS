// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The mouse packet against arbitrary bytes: no input may panic, no input
//! may make the decoder grow, and every delta it hands out must lie inside
//! the nine bits a packet carries.

use driver_i8042::device::WHEEL_ID;
use driver_i8042::mouse::{BUTTON_MASK, DELTA_MAX, DELTA_MIN, Decoder};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    for id in [0, WHEEL_ID] {
        let mut decoder = Decoder::for_id(id);
        for byte in bytes {
            let event = decoder.feed(*byte);
            assert!(
                decoder.pending() < decoder.packet_len(),
                "{} bytes stand in a packet of {}",
                decoder.pending(),
                decoder.packet_len()
            );
            let Some(event) = event else {
                continue;
            };
            // The deltas are nine bits, sign and magnitude apart, and the
            // vertical one is turned around; both stay inside what those
            // nine bits hold.
            assert!(
                (DELTA_MIN..=DELTA_MAX).contains(&event.dx),
                "the horizontal delta is {}",
                event.dx
            );
            assert!(
                (DELTA_MAX.saturating_neg()..=DELTA_MIN.saturating_neg()).contains(&event.dy),
                "the vertical delta is {}",
                event.dy
            );
            assert!((-8..=7).contains(&event.wheel), "the wheel is {}", event.wheel);
            assert_eq!(
                event.buttons & !BUTTON_MASK,
                0,
                "a button the packet does not carry"
            );
        }
        decoder.reset();
        assert_eq!(decoder.pending(), 0);
    }
});
