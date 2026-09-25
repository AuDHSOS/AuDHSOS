// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tick`.

use audhsos_abi::layout::TICKS_PER_SECOND;

use crate::tick::{micros, milliseconds, seconds};

#[test]
fn ticks_become_whole_seconds_and_milliseconds() {
    let rate = u64::from(TICKS_PER_SECOND);
    assert_eq!(seconds(0), 0);
    assert_eq!(seconds(rate - 1), 0);
    assert_eq!(seconds(rate), 1);
    assert_eq!(seconds(3 * rate + 7), 3);
    assert_eq!(milliseconds(rate), 1000);
    assert_eq!(milliseconds(0), 0);
}

#[test]
fn ticks_become_microseconds_at_the_resolution_of_the_tick() {
    let rate = u64::from(TICKS_PER_SECOND);
    assert_eq!(micros(0), 0);
    assert_eq!(micros(1), 1_000_000 / rate, "one tick is the reciprocal");
    assert_eq!(micros(rate), 1_000_000);
    assert_eq!(micros(50), 50_000, "fifty ticks are fifty milliseconds");
}

#[test]
fn a_microsecond_count_that_would_overflow_saturates() {
    // A machine left running answers the largest microsecond it can rather
    // than travelling backwards.
    let rate = u64::from(TICKS_PER_SECOND);
    assert_eq!(micros(u64::MAX), u64::MAX / rate);
    assert!(micros(u64::MAX) >= micros(u64::MAX - 1));
}
