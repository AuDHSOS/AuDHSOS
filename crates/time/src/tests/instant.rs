// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Monotonic time: the saturating arithmetic, the checked arithmetic, and
//! the ordering.

use test_support::generators::pair;
use test_support::property::check;

use crate::instant::{Duration, Instant};
use crate::strategies::{any_duration, any_instant};

#[test]
fn a_duration_converts_between_its_units() {
    assert_eq!(Duration::from_secs(2).as_micros(), 2_000_000);
    assert_eq!(Duration::from_millis(1500).as_micros(), 1_500_000);
    assert_eq!(Duration::from_micros(1_500_000).as_millis(), 1500);
    assert_eq!(Duration::from_micros(1_999_999).as_secs(), 1);
    assert_eq!(Duration::from_micros(999).as_millis(), 0);
    assert_eq!(Duration::from_secs(90).as_secs_i64(), 90);
    assert!(Duration::ZERO.is_zero());
    assert!(!Duration::from_micros(1).is_zero());
    assert_eq!(Duration::default(), Duration::ZERO);
}

#[test]
fn a_duration_that_would_not_fit_its_unit_saturates() {
    assert_eq!(Duration::from_secs(u64::MAX), Duration::MAX);
    assert_eq!(Duration::from_millis(u64::MAX), Duration::MAX);
}

#[test]
fn the_arithmetic_of_a_duration_is_checked_or_saturating() {
    let one = Duration::from_secs(1);
    let two = Duration::from_secs(2);
    assert_eq!(one.checked_add(one), Some(two));
    assert_eq!(Duration::MAX.checked_add(one), None);
    assert_eq!(two.checked_sub(one), Some(one));
    assert_eq!(one.checked_sub(two), None);
    assert_eq!(one.checked_mul(60), Some(Duration::from_secs(60)));
    assert_eq!(Duration::MAX.checked_mul(2), None);
    assert_eq!(Duration::MAX.saturating_add(one), Duration::MAX);
    assert_eq!(one.saturating_sub(two), Duration::ZERO);
    assert_eq!(one + one, two);
    assert_eq!(one - two, Duration::ZERO);
}

#[test]
fn the_addition_of_an_instant_saturates_at_the_maximum() {
    let point = Instant::from_micros(10);
    assert_eq!(
        point.saturating_add(Duration::from_micros(5)),
        Instant::from_micros(15)
    );
    assert_eq!(
        Instant::MAX.saturating_add(Duration::from_micros(1)),
        Instant::MAX
    );
    assert_eq!(Instant::MAX + Duration::MAX, Instant::MAX);
    let mut moving = point;
    moving += Duration::from_micros(5);
    assert_eq!(moving, Instant::from_micros(15));
    moving += Duration::MAX;
    assert_eq!(moving, Instant::MAX);
}

#[test]
fn the_checked_arithmetic_of_an_instant_reports_the_ends() {
    let point = Instant::from_micros(10);
    assert_eq!(
        point.checked_add(Duration::from_micros(5)),
        Some(Instant::from_micros(15))
    );
    assert_eq!(Instant::MAX.checked_add(Duration::from_micros(1)), None);
    assert_eq!(
        point.checked_sub(Duration::from_micros(10)),
        Some(Instant::ZERO)
    );
    assert_eq!(point.checked_sub(Duration::from_micros(11)), None);
}

#[test]
fn the_span_since_an_earlier_instant_is_zero() {
    let earlier = Instant::from_micros(10);
    let later = Instant::from_micros(60);
    assert_eq!(
        later.saturating_duration_since(earlier),
        Duration::from_micros(50)
    );
    assert_eq!(earlier.saturating_duration_since(later), Duration::ZERO);
    assert_eq!(earlier.saturating_duration_since(earlier), Duration::ZERO);
    assert_eq!(later - earlier, Duration::from_micros(50));
    assert_eq!(earlier - later, Duration::ZERO);
    assert_eq!(
        later.checked_duration_since(earlier),
        Some(Duration::from_micros(50))
    );
    assert_eq!(earlier.checked_duration_since(later), None);
}

#[test]
fn the_order_of_two_instants_is_total() {
    let points = [
        Instant::ZERO,
        Instant::from_micros(1),
        Instant::from_micros(1_000_000),
        Instant::MAX,
    ];
    for (index, earlier) in points.iter().enumerate() {
        for later in points.iter().skip(index.saturating_add(1)) {
            assert!(earlier < later);
            assert!(later > earlier);
            assert_ne!(earlier, later);
        }
        assert_eq!(earlier.cmp(earlier), core::cmp::Ordering::Equal);
    }
    assert_eq!(Instant::default(), Instant::ZERO);
    assert_eq!(Instant::ZERO.as_micros(), 0);
}

#[test]
fn a_span_added_to_an_instant_is_the_span_back_again() {
    check(
        "an instant plus a duration is that duration later",
        &pair(any_instant(), any_duration()),
        |&(point, span)| {
            let later = point.saturating_add(span);
            if later.saturating_duration_since(point) == span {
                Ok(())
            } else {
                Err(format!("{point:?} plus {span:?} is {later:?}"))
            }
        },
    );
}
