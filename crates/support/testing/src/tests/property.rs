// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::property`.

use crate::generators::{range, vec};
use crate::property::{Config, check, check_with, parse_seed, seed_for, seed_from_name};

fn config(seed: Option<u64>) -> Config {
    Config {
        cases: 200,
        seed,
        max_shrink_steps: 1000,
    }
}

#[test]
fn passing_property_returns_ok() {
    let result = check_with(&config(None), "always", &range(0u8..=9), |_| Ok(()));
    assert_eq!(result, Ok(()));
    check("always_panic_free", &range(0u8..=9), |_| Ok(()));
}

#[test]
fn monotone_property_shrinks_to_the_smallest_failing_value() {
    let failure = check_with(&config(None), "below_fifty", &range(0u32..=100), |&v| {
        if v < 50 {
            Ok(())
        } else {
            Err(format!("{v} is not below 50"))
        }
    })
    .unwrap_err();
    assert_eq!(failure.shrunk, "50");
    assert_eq!(failure.message, "50 is not below 50");
    assert!(failure.shrink_steps <= 1000);
}

#[test]
fn shrunk_value_still_fails_and_the_seed_replays_the_same_original() {
    let property = |v: &Vec<u8>| {
        if v.iter().map(|&b| u32::from(b)).sum::<u32>() < 40 {
            Ok(())
        } else {
            Err("sum too large".to_owned())
        }
    };
    let first =
        check_with(&config(None), "sum", &vec(range(0u8..=20), 0..=8), property).unwrap_err();
    let replay = check_with(
        &config(Some(first.seed)),
        "sum",
        &vec(range(0u8..=20), 0..=8),
        property,
    )
    .unwrap_err();
    assert_eq!(first.original, replay.original);
    assert_eq!(first.shrunk, replay.shrunk);
    assert_eq!(first.case, replay.case);
    assert!(first.shrunk.len() <= first.original.len());
}

#[test]
fn shrinking_stops_at_the_step_limit() {
    let tight = Config {
        cases: 10,
        seed: Some(1),
        max_shrink_steps: 3,
    };
    let failure = check_with(&tight, "limit", &range(0u64..=u64::MAX), |_| {
        Err("always".to_owned())
    })
    .unwrap_err();
    assert!(failure.shrink_steps <= 3);
}

#[test]
fn zero_shrink_budget_keeps_the_original_value() {
    let none = Config {
        cases: 10,
        seed: Some(3),
        max_shrink_steps: 0,
    };
    let failure = check_with(&none, "no_budget", &range(50u32..=100), |_| {
        Err("always".to_owned())
    })
    .unwrap_err();
    assert_eq!(failure.shrink_steps, 0);
    assert_eq!(failure.shrunk, failure.original);
}

#[test]
fn budget_exhausted_while_candidates_pass_stops_inside_the_candidate_loop() {
    let one = Config {
        cases: 50,
        seed: Some(4),
        max_shrink_steps: 1,
    };
    let failure = check_with(&one, "one_step", &range(0u32..=100), |&v| {
        if v < 50 {
            Ok(())
        } else {
            Err("too large".to_owned())
        }
    })
    .unwrap_err();
    assert_eq!(failure.shrink_steps, 1);
    assert_eq!(failure.shrunk, failure.original);
}

#[test]
fn report_names_the_seed_and_the_values() {
    let failure = check_with(&config(Some(42)), "report", &range(1u8..=1), |_| {
        Err("no".to_owned())
    })
    .unwrap_err();
    let text = failure.to_string();
    assert!(text.contains("property `report` failed"));
    assert!(text.contains("AUDHSOS_PROPTEST_SEED=42"));
    assert!(text.contains("shrunk:   1"));
    assert_eq!(failure.case, 0);
}

#[test]
#[should_panic(expected = "property `panics` failed")]
fn check_panics_with_the_report() {
    check("panics", &range(0u8..=0), |_| Err("no".to_owned()));
}

#[test]
fn seed_parsing_accepts_decimal_and_hex_and_rejects_garbage() {
    assert_eq!(parse_seed("42"), Some(42));
    assert_eq!(parse_seed(" 0x2A "), Some(42));
    assert_eq!(parse_seed("0X2a"), Some(42));
    assert_eq!(parse_seed("forty-two"), None);
    assert_eq!(parse_seed(""), None);
    assert_eq!(parse_seed("0x"), None);
}

#[test]
fn default_seed_depends_only_on_the_name() {
    assert_eq!(seed_from_name("a"), seed_from_name("a"));
    assert_ne!(seed_from_name("a"), seed_from_name("b"));
    assert_eq!(
        seed_for("name_without_override"),
        seed_from_name("name_without_override")
    );
}
