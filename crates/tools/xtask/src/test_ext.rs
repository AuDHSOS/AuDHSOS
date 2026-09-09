// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The external conformance suites under `docs/test-ext/`: bringing a
//! checkout to the revision this repository pins, and saying where one
//! stands.
//!
//! The suites are large, they are not part of this repository, and their
//! upstream branches move several times a day. What pins one is a commit
//! hash in [`crate::policy::EXTERNAL_SUITES`]; a commit hash is the
//! checksum of a tree and git verifies it on checkout, which is why no
//! checksum is recorded per file the way the reference documents record
//! one.
//!
//! This is the only subcommand that reaches the network, so it is the only
//! one that is never a step of `check`: a check runs offline, and a suite
//! that is missing is a run that could not be measured, not a run that
//! failed.

use std::path::Path;

use crate::error::Error;
use crate::out::note;
use crate::policy::{EXTERNAL_SUITES, ExternalSuite, find_suite};
use crate::process::Cmd;

/// What a run of the subcommand does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Bring every selected suite to its pinned revision.
    Prepare,
    /// Report where every selected suite stands and touch no network.
    Status,
}

/// Where a checkout stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// The directory does not exist.
    Missing,
    /// The directory exists and is not a git checkout.
    Foreign,
    /// A git checkout that has no commit yet, which is what an interrupted
    /// preparation leaves behind.
    Unborn,
    /// A checkout whose `HEAD` is this revision.
    At(String),
}

/// How much history a fetch asks for. A checkout this subcommand has just
/// created needs the pinned revision and nothing else, which is the smaller
/// half of the download. A checkout that is already there keeps whatever
/// history it has: `--depth` against a full clone truncates it, and
/// throwing away somebody's history to save a download nobody asked to
/// repeat is not a trade this makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum History {
    /// Only the pinned revision.
    Skip,
    /// Whatever is there already.
    Keep,
}

/// What bringing a checkout to the pinned revision takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Nothing: it is already there.
    Ready,
    /// Create the checkout, then fetch the revision into it.
    Create,
    /// Fetch the revision into the checkout that is there and check it out.
    Move,
    /// Refuse: the directory is something other than a checkout, and this
    /// deletes nobody's files.
    Refuse,
}

/// Prepares or reports the external suites.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option or an unknown suite;
/// [`Error::Violations`] when `--status` finds a suite that is missing or
/// at another revision, or when a directory is in the way; the errors of
/// git.
pub(crate) fn command(root: &Path, options: &[String]) -> Result<(), Error> {
    let (mode, suites) = parse(options)?;
    let mut violations = Vec::new();
    for suite in suites {
        let directory = root.join(suite.path);
        let state = state(&directory)?;
        match mode {
            Mode::Status => match violation(suite, &state) {
                None => eprintln!("{}", report(suite, &state)),
                Some(problem) => violations.push(problem),
            },
            Mode::Prepare => prepare(suite, &directory, &state)?,
        }
    }
    Error::from_violations(violations)
}

/// The mode and the selected suites of a command line, which is
/// `[--status] [<suite>...]`. Without a name, every suite is selected.
pub(crate) fn parse(options: &[String]) -> Result<(Mode, Vec<&'static ExternalSuite>), Error> {
    let mut mode = Mode::Prepare;
    let mut selected: Vec<&'static ExternalSuite> = Vec::new();
    for option in options {
        match option.as_str() {
            "--status" => mode = Mode::Status,
            name if name.starts_with('-') => {
                return Err(Error::Usage(format!(
                    "unknown option `{name}` for test-ext"
                )));
            }
            name => selected.push(find_suite(name).ok_or_else(|| {
                Error::Usage(format!("unknown suite `{name}`; {}", known_suites()))
            })?),
        }
    }
    if selected.is_empty() {
        selected.extend(EXTERNAL_SUITES);
    }
    Ok((mode, selected))
}

/// The names of the suites, for the message of an unknown one.
fn known_suites() -> String {
    let names: Vec<&str> = EXTERNAL_SUITES.iter().map(|suite| suite.name).collect();
    format!("known: {}", names.join(", "))
}

/// What `--status` prints for one suite.
pub(crate) fn report(suite: &ExternalSuite, state: &State) -> String {
    match state {
        State::Missing => format!("{}: missing at {}", suite.name, suite.path),
        State::Foreign => format!("{}: {} is not a git checkout", suite.name, suite.path),
        State::Unborn => format!("{}: {} holds no commit", suite.name, suite.path),
        State::At(revision) if revision == suite.revision => {
            format!("{}: at {}", suite.name, suite.revision)
        }
        State::At(revision) => format!(
            "{}: at {revision}, pinned at {}",
            suite.name, suite.revision
        ),
    }
}

/// What `--status` counts as a violation — anything but the pinned
/// revision — together with what would put it right. A directory that is
/// not a checkout is the one state the subcommand cannot mend itself, so
/// it is the one that asks the reader to act.
pub(crate) fn violation(suite: &ExternalSuite, state: &State) -> Option<String> {
    let remedy = match action(state, suite.revision) {
        Action::Ready => return None,
        Action::Refuse => "move that directory aside".to_owned(),
        Action::Create | Action::Move => format!("run `cargo xtask test-ext {}`", suite.name),
    };
    Some(format!("{}; {remedy}", report(suite, state)))
}

/// What a state takes to become the pinned revision.
pub(crate) fn action(state: &State, revision: &str) -> Action {
    match state {
        State::Missing => Action::Create,
        State::Foreign => Action::Refuse,
        State::At(head) if head == revision => Action::Ready,
        State::Unborn | State::At(_) => Action::Move,
    }
}

/// Where a checkout stands, without reaching the network.
pub(crate) fn state(directory: &Path) -> Result<State, Error> {
    if !directory.exists() {
        return Ok(State::Missing);
    }
    if !directory.join(".git").exists() {
        return Ok(State::Foreign);
    }
    let head = git(directory)
        .args(["rev-parse", "HEAD"])
        .capture_optional()?;
    Ok(match head {
        None => State::Unborn,
        Some(revision) => State::At(revision.trim().to_owned()),
    })
}

/// Brings one suite to its pinned revision.
fn prepare(suite: &ExternalSuite, directory: &Path, state: &State) -> Result<(), Error> {
    match action(state, suite.revision) {
        Action::Ready => {
            note!("{}: already at {}", suite.name, suite.revision);
            Ok(())
        }
        Action::Refuse => Err(Error::Violations(vec![format!(
            "{} exists and is not a git checkout; move it aside",
            directory.display()
        )])),
        Action::Create => {
            note!("{}: creating {}", suite.name, directory.display());
            std::fs::create_dir_all(directory)
                .map_err(|source| Error::io(format!("creating {}", directory.display()), source))?;
            git(directory).args(["init", "--quiet"]).run()?;
            fetch(suite, directory, History::Skip)
        }
        Action::Move => {
            refuse_dirty(suite, directory)?;
            fetch(suite, directory, History::Keep)
        }
    }
}

/// Refuses to move a checkout that has been edited. A test is not to be
/// rewritten to fit this project, and a run that discarded such an edit
/// silently would be the same mistake with the evidence gone. Untracked
/// files do not count: a checkout carries them across unchanged, so
/// nothing of them is at stake here.
fn refuse_dirty(suite: &ExternalSuite, directory: &Path) -> Result<(), Error> {
    let changed = git(directory)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .capture()?;
    if changed.trim().is_empty() {
        return Ok(());
    }
    Err(Error::Violations(vec![format!(
        "{}: {} has uncommitted changes; commit or discard them first",
        suite.name,
        directory.display()
    )]))
}

/// Fetches the pinned revision into a checkout and checks it out.
fn fetch(suite: &ExternalSuite, directory: &Path, history: History) -> Result<(), Error> {
    set_origin(suite, directory)?;
    note!("{}: fetching {}", suite.name, suite.revision);
    // A server may refuse a bare commit hash in a want line, and then every
    // branch is the only way to reach it. That refusal is a note and not a
    // failure, because the run can still finish.
    let asked = git(directory)
        .args(fetch_arguments(history, suite.revision))
        .run();
    if asked.is_err() {
        note!(
            "{}: the server would not send that revision by name; fetching the branches",
            suite.name
        );
        git(directory).args(["fetch", "origin"]).run()?;
    }
    git(directory)
        .args(["checkout", "--detach", suite.revision])
        .run()?;
    confirm(suite, directory)
}

/// The fetch that asks for one revision, with or without its history.
pub(crate) fn fetch_arguments(history: History, revision: &str) -> Vec<String> {
    let mut arguments = vec!["fetch".to_owned()];
    if history == History::Skip {
        arguments.extend(["--depth".to_owned(), "1".to_owned()]);
    }
    arguments.extend(["origin".to_owned(), revision.to_owned()]);
    arguments
}

/// Points `origin` at the suite's repository, whether or not the checkout
/// already has one.
fn set_origin(suite: &ExternalSuite, directory: &Path) -> Result<(), Error> {
    let remotes = git(directory).arg("remote").capture()?;
    let verb = if remotes.lines().any(|line| line.trim() == "origin") {
        "set-url"
    } else {
        "add"
    };
    git(directory)
        .args(["remote", verb, "origin", suite.url])
        .run()
}

/// Reads back what was checked out, so that the report says what is on the
/// disk and not what was asked for.
fn confirm(suite: &ExternalSuite, directory: &Path) -> Result<(), Error> {
    let state = state(directory)?;
    match action(&state, suite.revision) {
        Action::Ready => {
            eprintln!("{}", report(suite, &state));
            Ok(())
        }
        _ => Err(Error::Violations(vec![format!(
            "{}: after the checkout it is {}",
            suite.name,
            report(suite, &state)
        )])),
    }
}

/// Git in a directory.
fn git(directory: &Path) -> Cmd {
    Cmd::new("git").cwd(directory)
}
