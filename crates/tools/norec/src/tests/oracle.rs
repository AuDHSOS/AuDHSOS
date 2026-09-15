// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::oracle`.

use crate::case::CountStyle;
use crate::engine::Run;
use crate::oracle::{self, Verdict};
use crate::tests::{Fake, answer, case};

#[test]
fn the_script_builds_the_database_and_asks_both_questions_between_markers() {
    let script = oracle::script(&case(1), CountStyle::Rows);
    let lines: Vec<&str> = script.lines().collect();
    assert!(lines.contains(&".mode list"));
    assert!(lines.contains(&"CREATE TABLE t0(c0 INTEGER, c1 TEXT);"));
    assert!(lines.contains(&"SELECT '--norec:optimized--';"));
    assert!(lines.contains(&"SELECT * FROM t0 WHERE (t0.c0 = 1);"));
    assert!(lines.contains(&"SELECT '--norec:unoptimized--';"));
    assert!(lines.contains(&"SELECT '--norec:end--';"));
}

#[test]
fn equal_counts_agree_and_different_ones_are_a_finding() {
    let agree = answer(&["1|a", "2|b"], "2");
    assert_eq!(
        oracle::read(&agree, CountStyle::Rows).unwrap(),
        Verdict::Agree(2)
    );
    let differ = answer(&["1|a", "2|b"], "1");
    assert_eq!(
        oracle::read(&differ, CountStyle::Rows).unwrap(),
        Verdict::Mismatch {
            optimized: 2,
            unoptimized: 1
        }
    );
}

#[test]
fn the_count_style_reads_the_number_the_engine_printed() {
    let run = answer(&["3"], "3");
    assert_eq!(
        oracle::read(&run, CountStyle::Count).unwrap(),
        Verdict::Agree(3)
    );
    // The same output read as rows counts one row, not three.
    assert_eq!(
        oracle::read(&run, CountStyle::Rows).unwrap(),
        Verdict::Mismatch {
            optimized: 1,
            unoptimized: 3
        }
    );
}

#[test]
fn a_sum_over_no_rows_is_null_and_counts_as_zero() {
    let run = answer(&[], "");
    assert_eq!(
        oracle::read(&run, CountStyle::Rows).unwrap(),
        Verdict::Agree(0)
    );
}

#[test]
fn a_message_on_the_error_stream_refuses_the_case() {
    let run = Run {
        stdout: String::new(),
        stderr: "\nParse error near line 4: no such column: x\n".to_owned(),
        timed_out: false,
    };
    assert_eq!(
        oracle::read(&run, CountStyle::Rows).unwrap(),
        Verdict::Failed("Parse error near line 4: no such column: x".to_owned())
    );
}

#[test]
fn a_case_the_engine_did_not_finish_is_refused_and_not_read() {
    let run = Run {
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
    };
    assert_eq!(
        oracle::read(&run, CountStyle::Rows).unwrap(),
        Verdict::Failed("the engine did not finish".to_owned())
    );
}

#[test]
fn output_without_the_markers_is_an_error_and_not_a_verdict() {
    let run = Run {
        stdout: "--norec:optimized--\n1\n".to_owned(),
        stderr: String::new(),
        timed_out: false,
    };
    assert!(oracle::read(&run, CountStyle::Rows).is_err());
    let empty = Run::default();
    assert!(oracle::read(&empty, CountStyle::Rows).is_err());
}

#[test]
fn a_count_that_is_not_a_number_is_an_error() {
    let run = answer(&["1"], "many");
    assert!(oracle::read(&run, CountStyle::Rows).is_err());
}

#[test]
fn checking_runs_the_script_and_mismatches_answers_only_whether_it_differed() {
    let mut agreeing = Fake {
        answer: |_: &str| answer(&["1"], "1"),
    };
    assert_eq!(
        oracle::check(&mut agreeing, &case(1), CountStyle::Rows).unwrap(),
        Verdict::Agree(1)
    );
    assert!(!oracle::mismatches(&mut agreeing, &case(1), CountStyle::Rows).unwrap());

    let mut differing = Fake {
        answer: |_: &str| answer(&["1", "2"], "1"),
    };
    assert!(oracle::mismatches(&mut differing, &case(1), CountStyle::Rows).unwrap());
}
