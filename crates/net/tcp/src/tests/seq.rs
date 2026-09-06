// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The sequence arithmetic, at the wrap and away from it.

use test_support::generators::{pair, range};
use test_support::property::check;

use crate::seq::{SeqNumber, is_acceptable};

/// The last number before the circle closes.
const LAST: u32 = u32::MAX;

/// Half the circle, which is the one distance the relation cannot decide.
const HALF: u32 = 0x8000_0000;

#[test]
fn the_comparison_is_of_distances_and_not_of_magnitudes() {
    // Each row: the two numbers, and whether the first comes before the
    // second.
    let table = [
        (0u32, 1u32, true),
        (1, 0, false),
        (0, 0, false),
        // Across the wrap, where magnitudes say the opposite of distance.
        (LAST, 0, true),
        (0, LAST, false),
        (LAST - 5, 5, true),
        (5, LAST - 5, false),
        // Just inside half a circle, in both directions.
        (0, HALF - 1, true),
        (HALF - 1, 0, false),
        (LAST, HALF - 2, true),
    ];
    for (left, right, expected) in table {
        let first = SeqNumber::new(left);
        let second = SeqNumber::new(right);
        assert_eq!(first.before(second), expected, "{first} before {second}");
        assert_eq!(second.after(first), expected, "{second} after {first}");
        assert_eq!(first.before_or_equal(second), expected || left == right);
        assert_eq!(first.after_or_equal(second), !expected);
    }
}

#[test]
fn a_distance_of_exactly_half_the_circle_is_the_one_case_that_has_no_answer() {
    let origin = SeqNumber::new(0);
    let opposite = SeqNumber::new(HALF);
    // Going either way takes the same number of steps, so neither is
    // before the other. A window of at most 65535 bytes never produces
    // the case.
    assert!(!origin.before(opposite));
    assert!(!opposite.before(origin));
    assert_eq!(opposite.distance_from(origin), HALF);
}

#[test]
fn stepping_forward_and_back_crosses_the_wrap_without_a_seam() {
    let last = SeqNumber::new(LAST);
    assert_eq!(last.add(1), SeqNumber::new(0));
    assert_eq!(last.add(2), SeqNumber::new(1));
    assert_eq!(SeqNumber::new(0).sub(1), last);
    assert_eq!(SeqNumber::new(1).distance_from(last), 2);
}

#[test]
fn a_window_holds_what_lies_in_it_and_an_empty_one_holds_nothing() {
    let start = SeqNumber::new(LAST - 2);
    assert!(start.is_in_window(start, 8));
    assert!(start.add(7).is_in_window(start, 8));
    assert!(!start.add(8).is_in_window(start, 8));
    assert!(!start.sub(1).is_in_window(start, 8));
    assert!(!start.is_in_window(start, 0));
}

#[test]
fn the_four_cases_of_the_acceptance_test_are_the_ones_rfc_9293_names() {
    let next = SeqNumber::new(1000);
    // Each row: the segment's first number, how many numbers it takes, the
    // window, and whether it is acceptable.
    let table = [
        // No data, no window: only the exact number expected.
        (1000u32, 0u32, 0u32, true),
        (1001, 0, 0, false),
        (999, 0, 0, false),
        // No data, a window: anywhere in it.
        (1000, 0, 4, true),
        (1003, 0, 4, true),
        (1004, 0, 4, false),
        (999, 0, 4, false),
        // Data, no window: nothing at all.
        (1000, 1, 0, false),
        // Data and a window: either end of the segment inside it.
        (1000, 4, 4, true),
        (1003, 4, 4, true),
        (1004, 4, 4, false),
        // Begins before the window and reaches into it: taken and
        // trimmed, which is what makes a partly acknowledged
        // retransmission useful.
        (996, 8, 4, true),
        (996, 4, 4, false),
    ];
    for (seq, len, window, expected) in table {
        assert_eq!(
            is_acceptable(SeqNumber::new(seq), len, next, window),
            expected,
            "seq {seq}, len {len}, window {window}"
        );
    }
}

#[test]
fn the_acceptance_test_works_the_same_across_the_wrap() {
    let next = SeqNumber::new(LAST - 1);
    assert!(is_acceptable(next, 4, next, 4));
    assert!(is_acceptable(SeqNumber::new(1), 1, next, 4));
    assert!(!is_acceptable(SeqNumber::new(2), 1, next, 4));
    assert!(!is_acceptable(next.sub(1), 1, next, 4));
}

#[test]
fn exactly_one_of_before_after_and_equal_holds_for_any_pair_a_window_can_reach() {
    let generator = pair(range(0u32..=LAST), range(0u32..=60_000u32));
    check("sequence trichotomy", &generator, |(origin, offset)| {
        let first = SeqNumber::new(*origin);
        let second = first.add(*offset);
        let equal = first == second;
        let before = first.before(second);
        let after = first.after(second);
        if usize::from(equal) + usize::from(before) + usize::from(after) != 1 {
            return Err(format!(
                "{first} against {second}: {equal} {before} {after}"
            ));
        }
        if second.distance_from(first) != *offset {
            return Err(format!("{first} to {second} is not {offset}"));
        }
        Ok(())
    });
}
