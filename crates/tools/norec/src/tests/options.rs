// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::options`.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::Error;
use crate::options::{Mode, Options, Request, parse};

/// The options a command line parses to, or a panic naming what happened.
fn options(args: &[&str]) -> Options {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    match parse(&args) {
        Ok(Request::Run(options)) => *options,
        Ok(Request::Help) => panic!("asked for help"),
        Err(error) => panic!("{error}"),
    }
}

/// The message of a usage error.
fn refused(args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    match parse(&args) {
        Err(Error::Usage(message)) => message,
        Err(other) => format!("not a usage error: {other}"),
        Ok(_) => "accepted".to_owned(),
    }
}

#[test]
fn an_empty_command_line_runs_a_hundred_cases_from_seed_one() {
    let parsed = options(&[]);
    assert_eq!(parsed.seed, 1);
    assert_eq!(parsed.runs, 100);
    assert_eq!(parsed.timeout, Duration::from_secs(10));
    assert_eq!(parsed.reports, PathBuf::from("target/norec"));
    assert_eq!(parsed.reduce, 400);
    assert_eq!(parsed.mode, Mode::Run);
    assert!(!parsed.stop);
    assert!(!parsed.verbose);
}

#[test]
fn every_option_reaches_the_field_it_names() {
    let parsed = options(&[
        "--seed",
        "5",
        "--runs",
        "9",
        "--sqlite",
        "/bin/sqlite3",
        "--timeout",
        "3",
        "--reports",
        "/tmp/out",
        "--reduce",
        "0",
        "--stop",
        "--verbose",
        "--script",
    ]);
    assert_eq!(parsed.seed, 5);
    assert_eq!(parsed.runs, 9);
    assert_eq!(parsed.program, PathBuf::from("/bin/sqlite3"));
    assert_eq!(parsed.timeout, Duration::from_secs(3));
    assert_eq!(parsed.reports, PathBuf::from("/tmp/out"));
    assert_eq!(parsed.reduce, 0);
    assert_eq!(parsed.mode, Mode::Script);
    assert!(parsed.stop);
    assert!(parsed.verbose);
}

#[test]
fn help_is_a_request_of_its_own() {
    for flag in ["--help", "-h"] {
        let args = vec![flag.to_owned()];
        assert!(matches!(parse(&args), Ok(Request::Help)));
    }
}

#[test]
fn an_option_that_wants_a_value_and_gets_none_is_refused() {
    assert_eq!(refused(&["--seed"]), "--seed wants a value");
    assert_eq!(refused(&["--sqlite"]), "--sqlite wants a value");
}

#[test]
fn a_value_that_is_not_a_number_is_refused_with_the_option_that_wanted_it() {
    assert_eq!(
        refused(&["--runs", "many"]),
        "--runs wants a whole number, got `many`"
    );
    assert_eq!(
        refused(&["--reduce", "99999999999"]),
        "--reduce is too large"
    );
}

#[test]
fn an_unknown_option_is_refused_before_anything_runs() {
    assert_eq!(refused(&["--nonsense"]), "unknown option `--nonsense`");
}
