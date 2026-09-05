// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::out`.
//!
//! The flag is process-wide, so one test holds all of it and gives it
//! back; a test running beside this one has seen at most a line of
//! progress more or less.

use crate::out;
use crate::process::Cmd;

#[test]
fn a_quiet_run_keeps_the_status_of_what_it_ran() {
    assert!(!out::quiet());
    out::set_quiet(true);
    assert!(out::quiet());
    let ok = Cmd::new("sh").args(["-c", "echo out; echo err 1>&2"]).run();
    let failed = Cmd::new("sh").args(["-c", "echo out; exit 3"]).run();
    out::set_quiet(false);
    assert!(!out::quiet());
    assert!(ok.is_ok());
    assert!(failed.is_err());
}
