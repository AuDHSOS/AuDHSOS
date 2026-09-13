// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the command line and of the walk the measurements use.

use std::collections::BTreeSet;

use crate::bench::{
    bytes_moved, chase_words, cycle, gigabytes_per_second, nanoseconds_per_step, stream_words,
    working_sets,
};
use crate::config::{Config, MEBIBYTE};

fn arguments(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn the_options_set_what_they_name() {
    let parsed = Config::parse(&arguments(&[
        "--size", "8", "--passes", "2", "--steps", "1000",
    ]));
    assert_eq!(
        parsed,
        Ok(Some(Config {
            bytes: 8 * MEBIBYTE,
            chase_bytes: Config::default().chase_bytes,
            passes: 2,
            steps: 1000,
        }))
    );
}

#[test]
fn no_option_leaves_every_default() {
    assert_eq!(Config::parse(&[]), Ok(Some(Config::default())));
}

#[test]
fn the_usage_is_asked_for_by_name() {
    assert_eq!(Config::parse(&arguments(&["--help"])), Ok(None));
    assert_eq!(Config::parse(&arguments(&["-h"])), Ok(None));
}

#[test]
fn a_refused_command_line_says_what_it_refused() {
    for (given, message) in [
        (vec!["--nonsense"], "unknown option `--nonsense`"),
        (vec!["--size"], "--size wants a number"),
        (
            vec!["--steps", "0"],
            "--steps wants a number above zero, not `0`",
        ),
        (
            vec!["--passes", "many"],
            "--passes wants a number above zero, not `many`",
        ),
        (
            vec!["--size", "18446744073709551615"],
            "--size 18446744073709551615 is above the 65536 MiB this measures at most",
        ),
        (
            vec!["--chase", "65537"],
            "--chase 65537 is above the 65536 MiB this measures at most",
        ),
    ] {
        assert_eq!(
            Config::parse(&arguments(&given)),
            Err(message.to_owned()),
            "for {given:?}"
        );
    }
}

#[test]
fn the_chase_limit_drops_the_working_sets_above_it() {
    let names =
        |limit| -> Vec<&'static str> { working_sets(limit).into_iter().map(|set| set.1).collect() };
    assert_eq!(names(Config::default().chase_bytes).len(), 5);
    assert_eq!(
        names(4 * MEBIBYTE),
        vec!["16 KiB (L1)", "128 KiB (L2)", "4 MiB (L2/L3)"]
    );
    assert_eq!(names(0), vec!["16 KiB (L1)"], "a run measures something");
}

#[test]
fn a_buffer_holds_its_words_and_never_none() {
    assert_eq!(stream_words(MEBIBYTE), 131_072);
    assert_eq!(stream_words(0), 1);
    assert_eq!(stream_words(7), 1);
    assert_eq!(bytes_moved(4, 8, 3), 96);
    let words = chase_words(MEBIBYTE);
    assert_eq!(words * size_of::<usize>(), 1 << 20);
}

#[test]
fn the_walk_visits_every_word_exactly_once() {
    for words in [1usize, 2, 3, 17, 4096] {
        let buffer = cycle(words).expect("the machine holds a walk this small");
        assert_eq!(buffer.len(), words);
        let mut seen = BTreeSet::new();
        let mut position = 0usize;
        for _ in 0..words {
            assert!(seen.insert(position), "the walk repeats before it closes");
            position = buffer.get(position).copied().unwrap_or(usize::MAX);
        }
        assert_eq!(seen.len(), words, "the walk misses a word");
        assert_eq!(position, 0, "the walk does not close");
    }
}

#[test]
fn a_measurement_without_time_or_steps_reports_zero() {
    assert!((gigabytes_per_second(1_000_000, 0.0) - 0.0).abs() < f64::EPSILON);
    assert!((nanoseconds_per_step(1.0, 0) - 0.0).abs() < f64::EPSILON);
    assert!((gigabytes_per_second(2_000_000_000, 1.0) - 2.0).abs() < 1e-9);
    assert!((nanoseconds_per_step(1.0, 1_000_000_000) - 1.0).abs() < 1e-9);
}
