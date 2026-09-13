// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the run loop in `crate`.

use crate::engine::Run;
use crate::options::{Mode, Options};
use crate::tests::{Fake, Scratch, agreeing, answer};
use crate::{campaign, start};

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
        answer: |_: &str| answer(&["1", "2"], "1"),
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
        answer: |_: &str| answer(&["1", "2"], "1"),
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
        answer: |_: &str| answer(&["1", "2"], "1"),
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
