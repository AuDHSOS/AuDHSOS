// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Finding QEMU and its firmware, running one disk image, and reading the
//! serial protocol the machine writes.
//!
//! Invariants: the command line is the one
//! [03-target-platform.md 3.1.1](../../../docs/03-target-platform.md)
//! prescribes and nobody types it by hand; a run that does not end by
//! itself is killed and reported as a crash; the protocol grammar is the
//! one `kernel-test-harness` writes, so that writer and reader cannot
//! drift apart.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use kernel_test_harness::protocol::{FAILED, OK, SEPARATOR, SUMMARY_PREFIX, TEST_PREFIX};

use crate::error::Error;

/// The QEMU binary the reference machine runs on.
const QEMU_BINARY: &str = "qemu-system-x86_64";

/// Names the QEMU binary, instead of searching the `PATH`.
const QEMU_VARIABLE: &str = "AUDHSOS_QEMU";

/// Names the firmware image, instead of looking next to QEMU.
const FIRMWARE_VARIABLE: &str = "AUDHSOS_OVMF";

/// Names the time limit of one run in seconds.
const TIMEOUT_VARIABLE: &str = "AUDHSOS_QEMU_TIMEOUT";

/// Where the firmware lives, relative to the directory holding QEMU.
const FIRMWARE_RELATIVE: &str = "../share/qemu/edk2-x86_64-code.fd";

/// Time limit of one run in seconds.
const DEFAULT_TIMEOUT: u64 = 60;

/// How long the runner waits between two checks on the machine.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The exit status of a machine that reported success.
pub(crate) const EXIT_SUCCESS: i32 = 33;

/// The exit status of a machine that reported a test failure.
pub(crate) const EXIT_TEST_FAILURE: i32 = 35;

/// The exit status of a machine whose loader reported a failure.
pub(crate) const EXIT_LOADER_FAILURE: i32 = 37;

/// What one run of the machine amounted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The machine reported success.
    Success,
    /// The machine reported a failed test.
    TestFailure,
    /// The loader reported a failure.
    LoaderFailure,
    /// The machine did not report anything: a triple fault, a hang the
    /// time limit ended, or QEMU itself failing.
    Crash,
}

impl Outcome {
    /// The name the report uses.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::TestFailure => "test failure",
            Outcome::LoaderFailure => "loader failure",
            Outcome::Crash => "crash",
        }
    }
}

/// The outcome an exit status means. A run the time limit ended is a
/// crash whatever the status says.
pub(crate) const fn outcome_of(status: Option<i32>, timed_out: bool) -> Outcome {
    if timed_out {
        return Outcome::Crash;
    }
    match status {
        Some(EXIT_SUCCESS) => Outcome::Success,
        Some(EXIT_TEST_FAILURE) => Outcome::TestFailure,
        Some(EXIT_LOADER_FAILURE) => Outcome::LoaderFailure,
        _ => Outcome::Crash,
    }
}

/// One run of the machine.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    /// The exit status, if QEMU exited by itself.
    pub(crate) status: Option<i32>,
    /// Everything the serial port carried.
    pub(crate) output: String,
    /// Whether the time limit ended the run.
    pub(crate) timed_out: bool,
}

impl Run {
    /// What the run amounted to.
    pub(crate) const fn outcome(&self) -> Outcome {
        outcome_of(self.status, self.timed_out)
    }
}

/// The machine the tests run on.
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    qemu: PathBuf,
    firmware: PathBuf,
    timeout: Duration,
}

impl Machine {
    /// The machine of this development environment.
    ///
    /// # Errors
    ///
    /// [`Error::Usage`] if QEMU or the firmware image cannot be found.
    pub(crate) fn locate() -> Result<Machine, Error> {
        let qemu = match std::env::var_os(QEMU_VARIABLE) {
            Some(value) => PathBuf::from(value),
            None => search_path(QEMU_BINARY).ok_or_else(|| {
                Error::Usage(format!(
                    "`{QEMU_BINARY}` is not on the PATH; set {QEMU_VARIABLE} to its path"
                ))
            })?,
        };
        if !qemu.is_file() {
            return Err(Error::Usage(format!(
                "{} is not a file; set {QEMU_VARIABLE} to the QEMU binary",
                qemu.display()
            )));
        }
        let firmware = match std::env::var_os(FIRMWARE_VARIABLE) {
            Some(value) => PathBuf::from(value),
            None => firmware_next_to(&qemu),
        };
        if !firmware.is_file() {
            return Err(Error::Usage(format!(
                "the firmware {} does not exist; set {FIRMWARE_VARIABLE} to it",
                firmware.display()
            )));
        }
        Ok(Machine {
            qemu,
            firmware,
            timeout: Duration::from_secs(timeout_seconds()),
        })
    }

    /// The command line of the reference machine for `image`. `display`
    /// opens QEMU's own window instead of running headless.
    pub(crate) fn arguments(&self, image: &Path, display: bool) -> Vec<String> {
        arguments(&self.firmware, image, display)
    }

    /// The command line for messages.
    pub(crate) fn display(&self, image: &Path, display: bool) -> String {
        let mut text = self.qemu.display().to_string();
        for argument in self.arguments(image, display) {
            text.push(' ');
            text.push_str(&argument);
        }
        text
    }

    /// Runs `image` with the serial port captured and the time limit
    /// enforced.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if QEMU cannot be started or waited for.
    pub(crate) fn run_captured(&self, image: &Path) -> Result<Run, Error> {
        let mut command = Command::new(&self.qemu);
        command
            .args(self.arguments(image, false))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|source| Error::io(format!("starting {}", self.qemu.display()), source))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let reader = std::thread::spawn(move || read_all(stdout));
        let errors = std::thread::spawn(move || read_all(stderr));
        let (status, timed_out) = self.wait(&mut child)?;
        let mut output = reader.join().unwrap_or_default();
        output.push_str(&errors.join().unwrap_or_default());
        Ok(Run {
            status: status.code(),
            output,
            timed_out,
        })
    }

    /// Runs `image` with QEMU's streams attached to the terminal and no
    /// time limit.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if QEMU cannot be started or waited for.
    pub(crate) fn run_attached(&self, image: &Path, display: bool) -> Result<Option<i32>, Error> {
        eprintln!("$ {}", self.display(image, display));
        let status = Command::new(&self.qemu)
            .args(self.arguments(image, display))
            .status()
            .map_err(|source| Error::io(format!("running {}", self.qemu.display()), source))?;
        Ok(status.code())
    }

    /// Waits for the machine, killing it once the time limit is up.
    fn wait(&self, child: &mut std::process::Child) -> Result<(ExitStatus, bool), Error> {
        let deadline = Instant::now().checked_add(self.timeout);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok((status, false)),
                Ok(None) => {}
                Err(source) => return Err(Error::io("waiting for QEMU", source)),
            }
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                let _ = child.kill();
                let status = child
                    .wait()
                    .map_err(|source| Error::io("waiting for QEMU", source))?;
                return Ok((status, true));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// The time limit of one run.
    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// The command line of the reference machine, as
/// [03-target-platform.md 3.1.1](../../../docs/03-target-platform.md)
/// prescribes it.
pub(crate) fn arguments(firmware: &Path, image: &Path, display: bool) -> Vec<String> {
    vec![
        "-machine".to_owned(),
        "q35".to_owned(),
        "-cpu".to_owned(),
        "qemu64".to_owned(),
        "-smp".to_owned(),
        "1".to_owned(),
        "-m".to_owned(),
        "256M".to_owned(),
        "-drive".to_owned(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            firmware.display()
        ),
        "-drive".to_owned(),
        format!("format=raw,file={}", image.display()),
        "-serial".to_owned(),
        "stdio".to_owned(),
        "-display".to_owned(),
        if display {
            display_backend().to_owned()
        } else {
            "none".to_owned()
        },
        "-no-reboot".to_owned(),
        "-device".to_owned(),
        "isa-debug-exit,iobase=0xf4,iosize=0x04".to_owned(),
    ]
}

/// Reads a pipe to its end; a pipe that cannot be read yields what came
/// through before the failure.
fn read_all(stream: Option<impl Read>) -> String {
    let mut text = String::new();
    if let Some(mut stream) = stream {
        let _ = stream.read_to_string(&mut text);
    }
    text
}

/// The display backend `--display` asks for.
const fn display_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "cocoa"
    } else {
        "gtk"
    }
}

/// The time limit in seconds, from the environment or the default.
fn timeout_seconds() -> u64 {
    std::env::var(TIMEOUT_VARIABLE)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_TIMEOUT)
}

/// The firmware image next to a QEMU binary.
pub(crate) fn firmware_next_to(qemu: &Path) -> PathBuf {
    match qemu.parent() {
        Some(directory) => directory.join(FIRMWARE_RELATIVE),
        None => PathBuf::from(FIRMWARE_RELATIVE),
    }
}

/// The first entry of the `PATH` that holds an executable file `name`.
fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// The outcome of one test the machine reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TestOutcome {
    /// The test passed.
    Passed,
    /// The test failed, with the message the machine wrote.
    Failed(String),
}

/// Everything the serial protocol carried.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    /// One entry per `[test]` line, in the order they arrived.
    pub(crate) tests: Vec<(String, TestOutcome)>,
    /// The counts of the `[summary]` line, if one arrived.
    pub(crate) summary: Option<(u32, u32)>,
}

impl Report {
    /// The number of tests that passed.
    pub(crate) fn passed(&self) -> u32 {
        self.count(&TestOutcome::Passed)
    }

    /// The number of tests that failed.
    pub(crate) fn failed(&self) -> u32 {
        u32::try_from(
            self.tests
                .iter()
                .filter(|(_, outcome)| *outcome != TestOutcome::Passed)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    fn count(&self, wanted: &TestOutcome) -> u32 {
        u32::try_from(
            self.tests
                .iter()
                .filter(|(_, outcome)| outcome == wanted)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// The names of the tests that failed, with their messages.
    pub(crate) fn failures(&self) -> Vec<String> {
        self.tests
            .iter()
            .filter_map(|(name, outcome)| match outcome {
                TestOutcome::Passed => None,
                TestOutcome::Failed(message) if message.is_empty() => Some(name.clone()),
                TestOutcome::Failed(message) => Some(format!("{name}: {message}")),
            })
            .collect()
    }
}

/// Reads every protocol line out of `output`. Lines that are not protocol
/// lines are ignored: the firmware writes its own.
pub(crate) fn parse(output: &str) -> Report {
    let mut report = Report::default();
    for line in output.lines() {
        let line = strip_escapes(line);
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(TEST_PREFIX) {
            if let Some(test) = parse_test(rest) {
                report.tests.push(test);
            }
        } else if let Some(rest) = line.strip_prefix(SUMMARY_PREFIX) {
            report.summary = parse_summary(rest).or(report.summary);
        }
    }
    report
}

/// One `[test]` line without its prefix.
fn parse_test(rest: &str) -> Option<(String, TestOutcome)> {
    let position = rest.find(SEPARATOR)?;
    let name = rest.get(..position)?.to_owned();
    let outcome = rest.get(position.checked_add(SEPARATOR.len())?..)?;
    if outcome == OK {
        return Some((name, TestOutcome::Passed));
    }
    let message = outcome.strip_prefix(FAILED)?;
    Some((name, TestOutcome::Failed(message.to_owned())))
}

/// One `[summary]` line without its prefix.
fn parse_summary(rest: &str) -> Option<(u32, u32)> {
    let mut passed = None;
    let mut failed = None;
    for field in rest.split_whitespace() {
        if let Some(value) = field.strip_prefix("passed=") {
            passed = value.parse().ok();
        } else if let Some(value) = field.strip_prefix("failed=") {
            failed = value.parse().ok();
        }
    }
    Some((passed?, failed?))
}

/// Removes the terminal escape sequences the firmware writes, so that a
/// protocol line that follows one on the same line is still recognized.
pub(crate) fn strip_escapes(line: &str) -> &str {
    match line.rfind('\u{1b}') {
        None => line,
        Some(start) => {
            let rest = line.get(start..).unwrap_or("");
            let end = rest
                .find(|c: char| c.is_ascii_alphabetic())
                .map_or(line.len(), |offset| {
                    start.saturating_add(offset).saturating_add(1)
                });
            line.get(end..).unwrap_or("")
        }
    }
}

/// What the report says about a run that was supposed to pass.
///
/// # Errors
///
/// [`Error::Violations`] naming every failed test, a summary that
/// disagrees with the lines, or a missing summary.
pub(crate) fn check(report: &Report, outcome: Outcome) -> Result<(), Error> {
    let mut violations = report.failures();
    match report.summary {
        None => violations.push("the machine wrote no summary line".to_owned()),
        Some((passed, failed)) if passed != report.passed() || failed != report.failed() => {
            violations.push(format!(
                "the summary says passed={passed} failed={failed}, \
                 the lines say passed={} failed={}",
                report.passed(),
                report.failed()
            ));
        }
        Some(_) => {}
    }
    if outcome != Outcome::Success {
        violations.push(format!("the machine reported a {}", outcome.name()));
    }
    Error::from_violations(violations)
}
