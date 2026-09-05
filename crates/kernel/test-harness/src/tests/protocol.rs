// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::protocol`, against the grammar of 3.1.7.

use kernel_hal_api::doubles::RecordingConsole;

use crate::protocol::{failed, passed, start, summary};

#[test]
fn a_passing_test_writes_one_line_in_the_documented_shape() {
    let mut console = RecordingConsole::new();
    start(&mut console, "boot::banner");
    passed(&mut console);
    assert_eq!(console.text(), "[test] boot::banner ... ok\n");
}

#[test]
fn a_failing_test_carries_its_message() {
    let mut console = RecordingConsole::new();
    start(&mut console, "boot::banner");
    failed(&mut console, format_args!("{} != {}", 1, 2));
    assert_eq!(console.text(), "[test] boot::banner ... FAILED: 1 != 2\n");
}

#[test]
fn a_failing_test_may_carry_an_empty_message() {
    let mut console = RecordingConsole::new();
    start(&mut console, "t");
    failed(&mut console, format_args!(""));
    assert_eq!(console.text(), "[test] t ... FAILED: \n");
}

#[test]
fn the_summary_names_both_counts() {
    let mut console = RecordingConsole::new();
    summary(&mut console, 7, 0);
    summary(&mut console, 0, 3);
    assert_eq!(
        console.text(),
        "[summary] passed=7 failed=0\n[summary] passed=0 failed=3\n"
    );
}
