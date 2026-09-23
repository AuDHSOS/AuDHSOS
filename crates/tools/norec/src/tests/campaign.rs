// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the run loop in `crate`.

use crate::case::CountStyle;
use crate::engine::Run;
use crate::options::{Mode, Options};
use crate::tests::{Fake, Scratch, agreeing, answer};
use crate::{campaign, generated, start};

/// The scripts a campaign of `runs` cases from `seed` hands the engine.
fn scripts(scratch: &Scratch, seed: u64, runs: u64) -> Vec<String> {
    let mut sent = Vec::new();
    let mut engine = Fake {
        answer: |script: &str| {
            sent.push(script.to_owned());
            answer(&["1"], "1")
        },
    };
    let mut settings = options(scratch, runs);
    settings.seed = seed;
    campaign(&mut engine, &settings).unwrap();
    sent
}

/// Options that run `runs` cases into `scratch` and shrink nothing.
fn options(scratch: &Scratch, runs: u64) -> Options {
    Options {
        runs,
        reports: scratch.0.clone(),
        reduce: 0,
        ..Options::default()
    }
}

#[test]
fn a_campaign_over_an_engine_that_agrees_finds_nothing() {
    let scratch = Scratch::new();
    let mut engine = agreeing();
    let summary = campaign(&mut engine, &options(&scratch, 5)).unwrap();
    assert_eq!(summary.cases, 5);
    assert_eq!(summary.agreed, 5);
    assert_eq!(summary.refusals(), 0);
    assert!(summary.findings.is_empty());
}

#[test]
fn what_the_engine_refuses_is_counted_by_the_message_it_refused_with() {
    let scratch = Scratch::new();
    let mut engine = Fake {
        answer: |_: &str| Run {
            stdout: String::new(),
            stderr: "no such column: x\n".to_owned(),
            timed_out: false,
        },
    };
    let summary = campaign(&mut engine, &options(&scratch, 3)).unwrap();
    assert_eq!(summary.refusals(), 3);
    assert_eq!(summary.refused.get("no such column: x"), Some(&3));
    assert_eq!(summary.agreed, 0);
}

#[test]
fn a_disagreement_is_written_out_and_named_after_its_seed() {
    let scratch = Scratch::new();
    let mut engine = Fake {
        answer: |_: &str| answer(&["2", "2"], "1"),
    };
    let mut settings = options(&scratch, 1);
    settings.seed = 11;
    let summary = campaign(&mut engine, &settings).unwrap();
    assert_eq!(summary.findings.len(), 1);
    let first = summary.findings.first().unwrap();
    assert_eq!(first.seed, 11);
    assert_eq!((first.optimized, first.unoptimized), (2, 1));
    let written = std::fs::read_to_string(&first.path).unwrap();
    assert!(written.contains("-- seed 11"));
    assert!(first.path.ends_with("norec-11.sql"));
}

#[test]
fn stopping_at_the_first_finding_leaves_the_rest_unrun() {
    let scratch = Scratch::new();
    let mut engine = Fake {
        answer: |_: &str| answer(&["2", "2"], "1"),
    };
    let mut settings = options(&scratch, 10);
    settings.stop = true;
    settings.verbose = true;
    let summary = campaign(&mut engine, &settings).unwrap();
    assert_eq!(summary.cases, 1);
    assert_eq!(summary.findings.len(), 1);
}

#[test]
fn a_finding_is_shrunk_before_it_is_written_when_there_is_a_budget() {
    let scratch = Scratch::new();
    let mut engine = Fake {
        answer: |_: &str| answer(&["2", "2"], "1"),
    };
    let mut settings = options(&scratch, 1);
    settings.reduce = 200;
    let summary = campaign(&mut engine, &settings).unwrap();
    let finding = summary.findings.first().unwrap();
    let written = std::fs::read_to_string(&finding.path).unwrap();
    assert!(!written.contains("INSERT"));
}

#[test]
fn an_engine_that_answers_nothing_readable_stops_the_campaign() {
    let scratch = Scratch::new();
    let mut engine = Fake {
        answer: |_: &str| Run::default(),
    };
    assert!(campaign(&mut engine, &options(&scratch, 1)).is_err());
}

#[test]
fn asking_for_help_or_for_the_scripts_needs_no_engine() {
    assert!(start(&["--help".to_owned()]).unwrap());
    let args = ["--script", "--runs", "2", "--seed", "3"].map(str::to_owned);
    assert!(start(&args).unwrap());
}

#[test]
fn a_command_line_that_is_wrong_is_an_error_and_not_a_run() {
    assert!(start(&["--nonsense".to_owned()]).is_err());
}

#[test]
fn the_script_mode_is_what_the_default_is_not() {
    assert_eq!(Options::default().mode, Mode::Run);
}

#[test]
fn the_count_style_follows_the_seed_and_not_the_position_in_the_run() {
    assert_eq!(generated(0).1, CountStyle::Rows);
    assert_eq!(generated(1).1, CountStyle::Count);
    assert_eq!(generated(124).1, CountStyle::Rows);
    assert_eq!(generated(u64::MAX).1, CountStyle::Count);
}

#[test]
fn a_seed_rerun_alone_hands_the_engine_the_script_it_had_in_the_campaign() {
    let scratch = Scratch::new();
    let campaign = scripts(&scratch, 1, 2);
    let alone = scripts(&scratch, 2, 1);
    assert_eq!(campaign.len(), 2);
    assert_eq!(campaign.last(), alone.first());
}
