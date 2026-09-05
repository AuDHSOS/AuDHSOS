// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::harness`.

use core::cell::Cell;
use core::panic::Location;

use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::doubles::{RecordingConsole, RecordingExit};
use kernel_hal_api::exit::ExitStatus;

use crate::harness::{Harness, Testable};

/// A test with a name the runner reports and a body the test controls.
struct Named {
    name: &'static str,
    ran: &'static Cell<u32>,
}

impl Testable for Named {
    fn run(&self) {
        self.ran.set(self.ran.get().saturating_add(1));
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

thread_local! {
    static RUNS: &'static Cell<u32> = Box::leak(Box::new(Cell::new(0)));
}

fn counter() -> &'static Cell<u32> {
    RUNS.with(|cell| *cell)
}

#[test]
fn a_run_without_tests_passes_and_exits_with_a_success() {
    let mut harness = Harness::new(RecordingConsole::new(), RecordingExit::new());
    harness.run(&[]);
    assert_eq!(harness.passed(), 0);
}

#[test]
fn every_test_gets_a_line_and_the_summary_counts_them() {
    let counter = counter();
    counter.set(0);
    let first = Named {
        name: "boot::banner",
        ran: counter,
    };
    let second = Named {
        name: "boot::regions",
        ran: counter,
    };
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.run(&[&first, &second]);
        assert_eq!(harness.passed(), 2);
    }
    assert_eq!(counter.get(), 2, "every test ran once");
    assert_eq!(
        console.text(),
        "[test] boot::banner ... ok\n\
         [test] boot::regions ... ok\n\
         [summary] passed=2 failed=0\n"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Success));
}

#[test]
fn a_closure_is_a_test_and_carries_its_own_name() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    let body = || {};
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.run(&[&body]);
    }
    let text = console.text();
    assert!(text.starts_with("[test] "), "{text}");
    assert!(text.contains(" ... ok\n"), "{text}");
    assert!(
        text.contains("harness"),
        "the name names the closure: {text}"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Success));
}

#[test]
fn a_failure_inside_a_test_closes_its_line_and_ends_the_run() {
    let counter = counter();
    counter.set(0);
    let first = Named {
        name: "one",
        ran: counter,
    };
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.run(&[&first]);
        harness.fail(format_args!("the machine is wrong"));
    }
    assert!(
        console.text().contains("FAILED: the machine is wrong"),
        "{}",
        console.text()
    );
    assert_eq!(
        exit.status(),
        Some(ExitStatus::Success),
        "the first exit request wins, as on the machine"
    );
}

#[test]
fn a_failure_outside_a_test_opens_its_own_line() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.fail(format_args!("boom"));
    }
    assert_eq!(
        console.text(),
        "[test] outside a test ... FAILED: boom\n[summary] passed=0 failed=1\n"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
}

#[test]
fn an_image_that_expects_a_panic_passes_when_one_arrives() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::expecting_panic(&mut console, &mut exit);
        harness.fail(format_args!("stack overflow"));
    }
    assert_eq!(
        console.text(),
        "[test] outside a test ... ok\n[summary] passed=1 failed=0\n"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Success));
}

#[test]
fn an_image_that_expects_a_panic_fails_when_none_arrives() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::expecting_panic(&mut console, &mut exit);
        harness.run(&[]);
    }
    assert_eq!(
        console.text(),
        "[test] expected panic ... FAILED: no panic\n[summary] passed=0 failed=1\n"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
}

#[test]
fn the_console_is_reachable_for_an_image_that_writes_its_own_lines() {
    let mut harness = Harness::new(RecordingConsole::new(), RecordingExit::new());
    harness.console().write_bytes(b"hello");
    harness.run(&[]);
    assert!(format!("{harness:?}").contains("Harness"));
}

#[test]
fn a_failure_may_name_where_it_happened() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.fail_at(format_args!("assertion failed"), Some(Location::caller()));
    }
    let text = console.text();
    assert!(text.contains("assertion failed at "), "{text}");
    assert!(text.contains("harness.rs"), "{text}");

    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.fail_at(format_args!("no location"), None);
    }
    assert_eq!(
        console.text(),
        "[test] outside a test ... FAILED: no location\n[summary] passed=0 failed=1\n"
    );
}

#[test]
fn an_image_may_open_and_close_its_own_test_lines() {
    let mut console = RecordingConsole::new();
    let mut exit = RecordingExit::new();
    {
        let mut harness = Harness::new(&mut console, &mut exit);
        harness.begin("manual::first");
        harness.end();
        harness.end();
        harness.begin("manual::second");
        harness.fail(format_args!("the machine said no"));
    }
    assert_eq!(
        console.text(),
        "[test] manual::first ... ok\n\
         [test] manual::second ... FAILED: the machine said no\n\
         [summary] passed=1 failed=1\n"
    );
    assert_eq!(exit.status(), Some(ExitStatus::Failure));
}
