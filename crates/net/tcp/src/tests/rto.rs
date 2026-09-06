// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The retransmission timeout of RFC 6298, sample by sample.

use audhsos_time::Duration;

use crate::rto::{INITIAL, MAXIMUM, MINIMUM, Rto};

#[test]
fn a_timer_that_has_measured_nothing_waits_one_second() {
    let rto = Rto::new();
    assert_eq!(rto.get(), INITIAL);
    assert_eq!(rto.smoothed(), None);
    assert_eq!(rto.attempts(), 0);
    assert_eq!(Rto::default().get(), INITIAL);
}

#[test]
fn the_first_measurement_is_taken_as_the_truth_with_half_of_it_as_the_spread() {
    let mut rto = Rto::new();
    // Section 2.2: SRTT <- R, RTTVAR <- R/2, RTO <- SRTT + 4 RTTVAR.
    rto.sample(Duration::from_millis(400));
    assert_eq!(rto.smoothed(), Some(Duration::from_millis(400)));
    assert_eq!(rto.variation(), Duration::from_millis(200));
    // 400 + 4 * 200 = 1200 milliseconds, which is above the floor.
    assert_eq!(rto.get(), Duration::from_millis(1200));
}

#[test]
fn a_steady_path_narrows_the_timeout_towards_the_round_trip() {
    let mut rto = Rto::new();
    rto.sample(Duration::from_millis(400));
    let first = rto.get();
    for _ in 0..20 {
        rto.sample(Duration::from_millis(400));
    }
    // The variation falls towards zero, so the timeout falls towards the
    // round trip itself, and never below the floor.
    assert!(rto.get() < first);
    assert_eq!(rto.get(), MINIMUM);
    assert_eq!(rto.smoothed(), Some(Duration::from_millis(400)));
    assert!(rto.variation() < Duration::from_millis(20));
}

#[test]
fn a_jittery_path_widens_the_timeout() {
    let mut steady = Rto::new();
    let mut jittery = Rto::new();
    for _ in 0..8 {
        steady.sample(Duration::from_millis(400));
        jittery.sample(Duration::from_millis(100));
        jittery.sample(Duration::from_millis(700));
    }
    assert!(
        jittery.variation() > steady.variation(),
        "{:?} is not wider than {:?}",
        jittery.variation(),
        steady.variation()
    );
}

#[test]
fn the_second_measurement_is_the_weighted_average_the_memo_writes() {
    let mut rto = Rto::new();
    rto.sample(Duration::from_millis(400));
    rto.sample(Duration::from_millis(800));
    // RTTVAR <- 3/4 * 200 + 1/4 * |400 - 800| = 150 + 100 = 250.
    assert_eq!(rto.variation(), Duration::from_millis(250));
    // SRTT <- 7/8 * 400 + 1/8 * 800 = 350 + 100 = 450.
    assert_eq!(rto.smoothed(), Some(Duration::from_millis(450)));
    // 450 + 4 * 250 = 1450.
    assert_eq!(rto.get(), Duration::from_millis(1450));
}

#[test]
fn a_fast_path_still_waits_the_floor_and_a_slow_one_never_the_whole_ceiling() {
    let mut fast = Rto::new();
    fast.sample(Duration::from_micros(200));
    assert_eq!(fast.get(), MINIMUM);

    let mut slow = Rto::new();
    slow.sample(Duration::from_secs(100));
    assert_eq!(slow.get(), MAXIMUM);
}

#[test]
fn the_timeout_doubles_per_attempt_and_stops_at_the_ceiling() {
    let mut rto = Rto::new();
    assert_eq!(rto.get(), Duration::from_secs(1));
    let expected = [2u64, 4, 8, 16, 32];
    for (attempt, seconds) in expected.into_iter().enumerate() {
        rto.back_off();
        assert_eq!(rto.get(), Duration::from_secs(seconds));
        assert_eq!(rto.attempts(), u32::try_from(attempt).unwrap_or(0) + 1);
    }
    for _ in 0..10 {
        rto.back_off();
    }
    assert_eq!(rto.get(), MAXIMUM);
}

#[test]
fn a_measurement_throws_the_backoff_away() {
    let mut rto = Rto::new();
    rto.back_off();
    rto.back_off();
    assert_eq!(rto.attempts(), 2);
    rto.sample(Duration::from_millis(400));
    assert_eq!(rto.attempts(), 0);
    assert_eq!(rto.get(), Duration::from_millis(1200));
}

#[test]
fn a_reset_timer_has_measured_nothing_again() {
    let mut rto = Rto::new();
    rto.sample(Duration::from_millis(400));
    rto.back_off();
    rto.reset();
    assert_eq!(rto, Rto::new());
}
