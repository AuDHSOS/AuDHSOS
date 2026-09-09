// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::test_ext`. Nothing here reaches the network: what is
//! tested is the reading of the command line, the reading of a directory,
//! and the step each state asks for.

use crate::error::Error;
use crate::policy::{EXTERNAL_SUITES, ExternalSuite};
use crate::test_ext::{
    Action, History, Mode, State, action, fetch_arguments, parse, report, state, violation,
};

const SUITE: ExternalSuite = ExternalSuite {
    name: "example",
    path: "docs/test-ext/example",
    url: "https://example.invalid/example",
    revision: "1111111111111111111111111111111111111111",
};

fn options(list: &[&str]) -> Vec<String> {
    list.iter().map(|option| (*option).to_owned()).collect()
}

#[test]
fn no_argument_selects_every_suite_for_preparation() {
    let (mode, suites) = parse(&[]).expect("the empty command line is valid");
    assert_eq!(mode, Mode::Prepare);
    assert_eq!(suites.len(), EXTERNAL_SUITES.len());
}

#[test]
fn a_name_selects_one_suite_and_status_asks_only_what_is_there() {
    let (mode, suites) = parse(&options(&["--status", "test262"])).expect("a valid command line");
    assert_eq!(mode, Mode::Status);
    assert_eq!(suites.len(), 1);
    assert_eq!(suites[0].name, "test262");
    assert_eq!(suites[0].path, "docs/test-ext/test262");
    assert_eq!(suites[0].revision.len(), 40);
}

#[test]
fn an_unknown_option_or_suite_is_a_usage_error() {
    let Err(Error::Usage(option)) = parse(&options(&["--all"])) else {
        panic!("an unknown option is a usage error");
    };
    assert_eq!(option, "unknown option `--all` for test-ext");
    let Err(Error::Usage(name)) = parse(&options(&["wpt"])) else {
        panic!("an unknown suite is a usage error");
    };
    assert_eq!(name, "unknown suite `wpt`; known: test262");
}

#[test]
fn every_state_asks_for_the_step_that_fits_it() {
    let cases = [
        (State::Missing, Action::Create),
        (State::Foreign, Action::Refuse),
        (State::Unborn, Action::Move),
        (State::At(SUITE.revision.to_owned()), Action::Ready),
        (State::At("2".repeat(40)), Action::Move),
    ];
    for (state, expected) in cases {
        assert_eq!(action(&state, SUITE.revision), expected, "{state:?}");
    }
}

#[test]
fn the_report_names_what_is_there_and_what_is_pinned() {
    assert_eq!(
        report(&SUITE, &State::At(SUITE.revision.to_owned())),
        "example: at 1111111111111111111111111111111111111111"
    );
    assert_eq!(
        report(&SUITE, &State::At("2".repeat(40))),
        "example: at 2222222222222222222222222222222222222222, \
         pinned at 1111111111111111111111111111111111111111"
    );
    assert_eq!(
        report(&SUITE, &State::Missing),
        "example: missing at docs/test-ext/example"
    );
    assert_eq!(
        report(&SUITE, &State::Foreign),
        "example: docs/test-ext/example is not a git checkout"
    );
    assert_eq!(
        report(&SUITE, &State::Unborn),
        "example: docs/test-ext/example holds no commit"
    );
}

#[test]
fn only_the_pinned_revision_passes_a_status_run() {
    assert_eq!(
        violation(&SUITE, &State::At(SUITE.revision.to_owned())),
        None
    );
    let missing = violation(&SUITE, &State::Missing).expect("a missing suite is a violation");
    assert_eq!(
        missing,
        "example: missing at docs/test-ext/example; run `cargo xtask test-ext example`"
    );
    let moved = violation(&SUITE, &State::At("2".repeat(40))).expect("another revision is one too");
    assert!(
        moved.ends_with("run `cargo xtask test-ext example`"),
        "{moved}"
    );
    let foreign = violation(&SUITE, &State::Foreign).expect("a foreign directory is one too");
    assert_eq!(
        foreign,
        "example: docs/test-ext/example is not a git checkout; move that directory aside"
    );
}

#[test]
fn a_directory_is_read_without_the_network() {
    // The scratch path carries the process id: the tests of this workspace
    // share `TMPDIR` and run at the same time.
    let directory = std::env::temp_dir().join(format!("audhsos-test-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    assert_eq!(
        state(&directory).expect("a missing directory has a state"),
        State::Missing
    );
    std::fs::create_dir_all(&directory).expect("creating the scratch directory");
    assert_eq!(
        state(&directory).expect("a plain directory has a state"),
        State::Foreign
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn the_pinned_suite_is_the_one_the_documents_name() {
    let test262 = EXTERNAL_SUITES
        .iter()
        .find(|suite| suite.name == "test262")
        .expect("the table holds test262");
    assert_eq!(test262.url, "https://github.com/tc39/test262");
    assert!(
        test262
            .revision
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
}

#[test]
fn a_new_checkout_takes_one_revision_and_an_old_one_keeps_its_history() {
    assert_eq!(
        fetch_arguments(History::Skip, SUITE.revision),
        ["fetch", "--depth", "1", "origin", SUITE.revision]
    );
    assert_eq!(
        fetch_arguments(History::Keep, SUITE.revision),
        ["fetch", "origin", SUITE.revision]
    );
}
