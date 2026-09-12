// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the option checks on the command line.

use std::path::Path;

use crate::commands;
use crate::error::Error;

/// The message of a `Usage` error, or the error's kind for anything else.
fn usage_message(result: Result<(), Error>) -> String {
    match result {
        Err(Error::Usage(message)) => message,
        Err(other) => format!("not a usage error: {other}"),
        Ok(()) => "no error".to_owned(),
    }
}

#[test]
fn a_subcommand_without_options_refuses_one() {
    let message = usage_message(crate::none("lint", &["--quiet".to_owned()]));
    assert_eq!(message, "unknown option `--quiet` for lint");
    assert!(crate::none("lint", &[]).is_ok());
}

#[test]
fn check_refuses_an_unknown_option_before_it_runs_anything() {
    let message = usage_message(commands::check(
        Path::new("/definitely/missing"),
        "nightly",
        &["--nonsense".to_owned()],
    ));
    assert_eq!(message, "unknown option `--nonsense` for check");
}

#[test]
fn jrs_check_refuses_unknown_or_extra_options_before_running_tools() {
    for options in [
        vec!["--typo".to_owned()],
        vec!["--fix-format".to_owned(), "extra".to_owned()],
    ] {
        let message = usage_message(commands::jrs_check(
            Path::new("/definitely/missing"),
            &options,
        ));
        assert_eq!(message, "jrs-check accepts only --fix-format");
    }
}

#[test]
fn the_scratch_disk_of_a_run_is_named_after_it_and_lies_under_target() {
    let path = commands::scratch_path(Path::new("/work"), "audhsos");
    assert_eq!(path, Path::new("/work/target/qemu/audhsos.scratch.img"));
    assert_ne!(path, commands::scratch_path(Path::new("/work"), "console"));
}

#[test]
fn run_refuses_an_unknown_option_before_it_builds_anything() {
    let message = usage_message(commands::run(
        Path::new("/definitely/missing"),
        &["--scratchh".to_owned()],
    ));
    assert_eq!(message, "unknown option `--scratchh` for run");
}
