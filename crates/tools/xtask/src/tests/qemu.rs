// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::qemu`: the serial protocol parser, the exit status
//! mapping, and the command line of the reference machine.

use std::path::Path;

use crate::qemu::{
    EXIT_LOADER_FAILURE, EXIT_SUCCESS, EXIT_TEST_FAILURE, Measurement, Outcome, Report, Run,
    TestOutcome, arguments, check, firmware_next_to, outcome_of, parse, strip_escapes,
};

/// A run whose lines are all well formed.
const PASSING: &str = "\
[test] boot::the_kernel_reaches_the_test_harness ... ok
[test] boot::the_loader_reports_usable_memory ... ok
[summary] passed=2 failed=0
";

/// A run that ended the way `status`, `signal` and `timed_out` say.
fn run(status: Option<i32>, signal: Option<i32>, timed_out: bool) -> Run {
    Run {
        status,
        signal,
        output: String::new(),
        timed_out,
    }
}

/// A run the machine ended by itself with `code`.
fn ended_with(code: i32) -> Run {
    run(Some(code), None, false)
}

#[test]
fn well_formed_lines_are_read_with_their_names_and_outcomes() {
    let report = parse(PASSING);
    assert_eq!(report.tests.len(), 2);
    assert_eq!(report.passed(), 2);
    assert_eq!(report.failed(), 0);
    assert_eq!(report.summary, Some((2, 0)));
    assert!(check(&report, &ended_with(EXIT_SUCCESS)).is_ok());
}

#[test]
fn a_failed_line_carries_its_message_and_an_empty_message_is_allowed() {
    let output =
        "[test] a ... FAILED: it broke\n[test] b ... FAILED: \n[summary] passed=0 failed=2\n";
    let report = parse(output);
    assert_eq!(
        report.tests,
        vec![
            ("a".to_owned(), TestOutcome::Failed("it broke".to_owned())),
            ("b".to_owned(), TestOutcome::Failed(String::new())),
        ]
    );
    assert_eq!(
        report.failures(),
        vec!["a: it broke".to_owned(), "b".to_owned()]
    );
    assert!(check(&report, &ended_with(EXIT_TEST_FAILURE)).is_err());
}

#[test]
fn a_summary_that_disagrees_with_the_lines_is_a_runner_error() {
    let report = parse("[test] a ... ok\n[summary] passed=7 failed=0\n");
    let error = check(&report, &ended_with(EXIT_SUCCESS)).unwrap_err();
    assert!(format!("{error}").contains("the summary says passed=7"));
}

#[test]
fn output_without_a_summary_is_a_runner_error() {
    let report = parse("[test] a ... ok\n");
    assert_eq!(report.summary, None);
    let error = check(&report, &ended_with(EXIT_SUCCESS)).unwrap_err();
    assert!(format!("{error}").contains("no summary line"));
}

#[test]
fn output_that_is_not_the_protocol_is_ignored() {
    let output = "BdsDxe: starting Boot0001\n[loader] 262144 KiB of memory\n[test] a ... maybe\n[summary] passed=x failed=0\nplain text\n";
    let report = parse(output);
    assert_eq!(report, Report::default());
}

#[test]
fn a_protocol_line_behind_a_terminal_escape_is_still_read() {
    let output = "\u{1b}[2J\u{1b}[01;01H[test] a ... ok\n[summary] passed=1 failed=0\n";
    let report = parse(output);
    assert_eq!(report.passed(), 1);
    assert_eq!(strip_escapes("plain"), "plain");
    assert_eq!(strip_escapes("\u{1b}[2J"), "");
}

#[test]
fn every_exit_status_maps_to_one_outcome() {
    assert_eq!(outcome_of(Some(EXIT_SUCCESS), false), Outcome::Success);
    assert_eq!(
        outcome_of(Some(EXIT_TEST_FAILURE), false),
        Outcome::TestFailure
    );
    assert_eq!(
        outcome_of(Some(EXIT_LOADER_FAILURE), false),
        Outcome::LoaderFailure
    );
    assert_eq!(outcome_of(Some(0), false), Outcome::Crash);
    assert_eq!(outcome_of(Some(1), false), Outcome::Crash);
    assert_eq!(outcome_of(None, false), Outcome::Crash);
    assert_eq!(outcome_of(Some(EXIT_SUCCESS), true), Outcome::Crash);
    assert_eq!(Outcome::Success.name(), "success");
    assert_eq!(Outcome::TestFailure.name(), "test failure");
    assert_eq!(Outcome::LoaderFailure.name(), "loader failure");
    assert_eq!(Outcome::Crash.name(), "crash");
}

#[test]
fn a_run_the_time_limit_ended_is_a_crash_whatever_the_status_says() {
    let report = parse("");
    assert!(check(&report, &run(Some(EXIT_SUCCESS), None, true)).is_err());
}

#[test]
fn the_command_line_is_the_one_the_target_platform_document_prescribes() {
    let line = arguments(
        Path::new("/fw/edk2-x86_64-code.fd"),
        Path::new("/img/audhsos.img"),
        false,
    )
    .join(" ");
    assert_eq!(
        line,
        "-machine q35 -cpu qemu64 -smp 1 -m 256M \
         -drive if=pflash,format=raw,readonly=on,file=/fw/edk2-x86_64-code.fd \
         -drive format=raw,file=/img/audhsos.img \
         -serial stdio -display none -no-reboot \
         -device isa-debug-exit,iobase=0xf4,iosize=0x04"
    );
    let windowed = arguments(Path::new("/fw"), Path::new("/img"), true).join(" ");
    assert!(!windowed.contains("-display none"));
}

#[test]
fn the_firmware_lies_next_to_the_qemu_binary() {
    assert_eq!(
        firmware_next_to(Path::new("/opt/local/bin/qemu-system-x86_64")),
        Path::new("/opt/local/bin/../share/qemu/edk2-x86_64-code.fd")
    );
    assert_eq!(
        firmware_next_to(Path::new("qemu-system-x86_64")),
        Path::new("../share/qemu/edk2-x86_64-code.fd")
    );
}

#[test]
fn a_crash_says_which_of_the_three_it_was() {
    assert_eq!(
        run(Some(EXIT_SUCCESS), None, true).how(),
        "the time limit ended it while it was still running"
    );
    assert_eq!(run(Some(7), None, false).how(), "it exited with code 7");
    assert_eq!(
        run(None, Some(9), false).how(),
        "something killed QEMU with signal 9"
    );
    assert_eq!(
        run(None, None, false).how(),
        "QEMU ended without a code and without a signal"
    );
}

#[test]
fn the_violation_of_a_crash_carries_the_reason() {
    let report = parse("");
    let error = check(&report, &run(None, Some(9), false)).unwrap_err();
    let text = format!("{error}");
    assert!(
        text.contains("signal 9"),
        "the reason is not in the report: {text}"
    );
}

#[test]
fn a_bench_line_is_read_with_its_name_its_figure_and_its_count() {
    let report = parse(
        "[bench] bench::syscall_round_trip ... 4231 ticks (n=10000)\n\
         [bench] bench::ipc_round_trip ... 19004 ticks (n=10000)\n\
         [summary] passed=0 failed=0\n",
    );
    assert_eq!(
        report.measurements,
        vec![
            Measurement {
                name: "bench::syscall_round_trip".to_owned(),
                ticks: 4231,
                samples: 10_000,
            },
            Measurement {
                name: "bench::ipc_round_trip".to_owned(),
                ticks: 19_004,
                samples: 10_000,
            },
        ]
    );
}

#[test]
fn a_bench_line_that_is_not_the_grammar_is_ignored() {
    for line in [
        "[bench] no separator 12 ticks (n=1)",
        "[bench] name ... ticks (n=1)",
        "[bench] name ... 12 (n=1)",
        "[bench] name ... 12 ticks",
        "[bench] name ... 12 ticks (n=)",
        "[bench] name ... 12 ticks (n=1",
    ] {
        let report = parse(&format!("{line}\n[summary] passed=0 failed=0\n"));
        assert!(
            report.measurements.is_empty(),
            "`{line}` was read as a measurement"
        );
    }
}

#[test]
fn a_bench_line_does_not_count_as_a_test() {
    let report = parse("[bench] b ... 1 ticks (n=1)\n[summary] passed=0 failed=0\n");
    assert_eq!(report.passed(), 0);
    assert_eq!(report.failed(), 0);
    assert!(check(&report, &ended_with(EXIT_SUCCESS)).is_ok());
}
