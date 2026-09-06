// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reno, as RFC 5681 states it.

use crate::congestion::{Congestion, DUPLICATE_THRESHOLD};
use crate::seq::SeqNumber;

/// The segment size these tests count in.
const SEGMENT: u32 = 1000;

/// A number that stands for the highest sent, which recovery is measured
/// against.
const HIGHEST: SeqNumber = SeqNumber::new(10_000);

#[test]
fn the_initial_window_is_the_one_equation_one_gives() {
    // Four segments for a small one, three for a middling one, two for a
    // large one.
    assert_eq!(Congestion::new(536).window(), 4 * 536);
    assert_eq!(Congestion::new(1095).window(), 4 * 1095);
    assert_eq!(Congestion::new(1460).window(), 3 * 1460);
    assert_eq!(Congestion::new(2190).window(), 3 * 2190);
    assert_eq!(Congestion::new(4000).window(), 2 * 4000);
}

#[test]
fn a_connection_that_learns_its_segment_size_resizes_its_window() {
    let mut congestion = Congestion::new(536);
    congestion.set_max_segment(1460);
    assert_eq!(congestion.window(), 3 * 1460);
}

#[test]
fn slow_start_doubles_the_window_every_round_trip() {
    let mut congestion = Congestion::new(SEGMENT);
    assert!(congestion.is_slow_start());
    let start = congestion.window();
    // One round trip acknowledges everything that was in flight.
    let segments = start / SEGMENT;
    for _ in 0..segments {
        congestion.on_ack(SEGMENT);
    }
    assert_eq!(congestion.window(), start * 2);
}

#[test]
fn an_acknowledgment_of_less_than_a_segment_grows_the_window_by_that_much() {
    let mut congestion = Congestion::new(SEGMENT);
    let start = congestion.window();
    congestion.on_ack(400);
    assert_eq!(congestion.window(), start + 400);
}

#[test]
fn congestion_avoidance_grows_by_about_one_segment_per_round_trip() {
    let mut congestion = Congestion::new(SEGMENT);
    // A timeout puts the threshold below the window and the window at one
    // segment; slow start then runs up to the threshold.
    congestion.on_timeout(20 * SEGMENT);
    assert_eq!(congestion.threshold(), 10 * SEGMENT);
    assert_eq!(congestion.window(), SEGMENT);
    while congestion.is_slow_start() {
        congestion.on_ack(SEGMENT);
    }
    let start = congestion.window();
    assert!(!congestion.is_slow_start());
    let segments = start / SEGMENT;
    for _ in 0..segments {
        congestion.on_ack(SEGMENT);
    }
    let grown = congestion.window() - start;
    assert!(
        (SEGMENT / 2..=SEGMENT + SEGMENT / 4).contains(&grown),
        "one round trip grew the window by {grown}"
    );
}

#[test]
fn two_duplicates_are_reordering_and_the_third_is_a_loss() {
    let mut congestion = Congestion::new(SEGMENT);
    let in_flight = 10 * SEGMENT;
    for _ in 1..DUPLICATE_THRESHOLD {
        assert!(!congestion.on_duplicate_ack(in_flight, HIGHEST));
        assert!(!congestion.is_recovering());
    }
    assert!(congestion.on_duplicate_ack(in_flight, HIGHEST));
    assert!(congestion.is_recovering());
    // The threshold is half of what was in flight, and the window is that
    // plus the three segments the duplicates say have left the network.
    assert_eq!(congestion.threshold(), 5 * SEGMENT);
    assert_eq!(congestion.window(), 5 * SEGMENT + 3 * SEGMENT);
}

#[test]
fn every_further_duplicate_lets_one_more_segment_go() {
    let mut congestion = Congestion::new(SEGMENT);
    let in_flight = 10 * SEGMENT;
    for _ in 0..DUPLICATE_THRESHOLD {
        congestion.on_duplicate_ack(in_flight, HIGHEST);
    }
    let inflated = congestion.window();
    assert!(!congestion.on_duplicate_ack(in_flight, HIGHEST));
    assert_eq!(congestion.window(), inflated + SEGMENT);
    assert_eq!(congestion.duplicates(), DUPLICATE_THRESHOLD + 1);
}

#[test]
fn the_first_acknowledgment_of_new_data_ends_recovery_at_the_threshold() {
    let mut congestion = Congestion::new(SEGMENT);
    for _ in 0..DUPLICATE_THRESHOLD {
        congestion.on_duplicate_ack(10 * SEGMENT, HIGHEST);
    }
    assert!(congestion.is_recovering());
    congestion.on_ack(SEGMENT);
    assert!(!congestion.is_recovering());
    assert_eq!(congestion.window(), congestion.threshold());
    assert_eq!(congestion.duplicates(), 0);
}

#[test]
fn a_timeout_puts_the_window_back_to_one_segment() {
    let mut congestion = Congestion::new(SEGMENT);
    for _ in 0..DUPLICATE_THRESHOLD {
        congestion.on_duplicate_ack(10 * SEGMENT, HIGHEST);
    }
    congestion.on_timeout(8 * SEGMENT);
    assert_eq!(congestion.window(), SEGMENT);
    assert_eq!(congestion.threshold(), 4 * SEGMENT);
    assert!(!congestion.is_recovering());
    assert!(congestion.is_slow_start());
}

#[test]
fn the_threshold_never_falls_below_two_segments() {
    let mut congestion = Congestion::new(SEGMENT);
    congestion.on_timeout(SEGMENT);
    assert_eq!(congestion.threshold(), 2 * SEGMENT);
    congestion.on_timeout(0);
    assert_eq!(congestion.threshold(), 2 * SEGMENT);
}
