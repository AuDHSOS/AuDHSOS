// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::worker`.
//!
//! A worker is a process of its own in a run, but nothing in it needs to
//! be: it reads messages and writes messages. These hand it the bytes an
//! orchestrator would have written and read back what it said, over a
//! range of counters this module registers, as the tests of the engine do.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use crate::cover::Cover;
use crate::engine::Runner;
use crate::options::Options;
use crate::proto::{Down, Draw, NO_FILE, Round, Up};
use crate::worker::Worker;

use super::{GLOBALS, bump, register_counters, scratch_path};

/// How many counters the body below writes into.
const COUNTERS: usize = 32;

/// A pipe a test reads back.
#[derive(Clone, Default)]
struct Recorder {
    /// What was written to it.
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl Write for Recorder {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Ok(mut held) = self.bytes.lock() {
            held.extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Recorder {
    /// Every message it holds.
    fn messages(&self) -> Vec<Up> {
        let held = self
            .bytes
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default();
        let mut input = &*held;
        let mut messages = Vec::new();
        while let Ok(message) = Up::read(&mut input) {
            messages.push(message);
        }
        messages
    }
}

/// A body that reaches one counter per distinct leading byte and panics
/// on one input, as the tests of the engine have it.
fn body(address: usize) -> impl FnMut(&[u8]) {
    move |input: &[u8]| {
        bump(
            address,
            COUNTERS,
            usize::from(input.first().copied().unwrap_or(0)),
        );
        assert!(input != b"BOOM", "the input the test is looking for");
    }
}

/// A directory of its own, with the files `names` gives, removed when it
/// is dropped.
struct Scratch {
    /// Where it is.
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str, files: &[&[u8]]) -> Self {
        let path = scratch_path(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap_or(());
        for (number, bytes) in files.iter().enumerate() {
            let _ = std::fs::write(path.join(format!("{number:04}")), bytes);
        }
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The options of a run over `scratch`.
fn options(scratch: &Scratch) -> Options {
    Options {
        paths: vec![scratch.path.clone()],
        seed: Some(11),
        ..Options::default()
    }
}

/// The bytes of the messages an orchestrator would have sent.
fn line(messages: &[Down]) -> Vec<u8> {
    let mut out = Vec::new();
    for message in messages {
        message.write(&mut out).unwrap_or(());
    }
    out
}

/// Runs a worker over `sent` and answers what it said and how it ended.
fn serve(scratch: &Scratch, sent: &[Down]) -> (Vec<Up>, String) {
    let options = options(scratch);
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let mut runner = Runner::new(&options, &mut target);
    let recorder = Recorder::default();
    let mut worker = Worker::new(&mut runner, &options, Box::new(recorder.clone()));
    let code = worker.run(&*line(sent));
    (recorder.messages(), format!("{code:?}"))
}

/// How an exit code prints, for a test that compares one.
fn code(code: ExitCode) -> String {
    format!("{code:?}")
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_runs_the_corpus_files_it_was_dealt_and_says_what_they_reached() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-load", &[b"\x01one", b"\x02two", b"\x03three"]);
    let (said, ended) = serve(&scratch, &[Down::Load(vec![0, 2])]);
    assert_eq!(ended, code(ExitCode::SUCCESS));
    let found: Vec<u32> = said
        .iter()
        .filter_map(|message| match message {
            Up::Found { origin, .. } => Some(*origin),
            _ => None,
        })
        .collect();
    assert_eq!(found, vec![0, 2], "{said:?}");
    assert_eq!(said.last(), Some(&Up::Idle(2)), "{said:?}");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_reports_a_corpus_file_that_panics_and_stops() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-crash", &[b"BOOM"]);
    let (said, ended) = serve(&scratch, &[Down::Load(vec![0])]);
    assert_eq!(ended, code(ExitCode::FAILURE));
    assert_eq!(said.last(), Some(&Up::Crash(b"BOOM".to_vec())), "{said:?}");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_keeps_nothing_a_coverage_table_already_holds() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-cover", &[b"\x01one"]);
    let mut full = Cover::new();
    for slot in 0..u32::try_from(crate::cover::FEATURE_SLOTS).unwrap_or(0) {
        full.claim(slot, 1, true);
    }
    let (said, ended) = serve(&scratch, &[Down::Cover(full.bytes()), Down::Load(vec![0])]);
    assert_eq!(ended, code(ExitCode::SUCCESS));
    assert_eq!(said, vec![Up::Idle(1)], "{said:?}");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_mutates_the_draws_it_is_given_and_reports_what_they_reached() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-batch", &[]);
    let rounds: Vec<Round> = (0..8)
        .map(|round| Round {
            seed: Draw {
                index: 0,
                bytes: (round == 0).then(|| b"\x01seed".to_vec()),
            },
            cross: Draw {
                index: 1,
                bytes: (round == 0).then(|| b"\x02other".to_vec()),
            },
        })
        .collect();
    let (said, ended) = serve(&scratch, &[Down::Batch { limit: 32, rounds }, Down::Stop]);
    assert_eq!(ended, code(ExitCode::SUCCESS));
    let runs = said.iter().rev().find_map(|message| match message {
        Up::Idle(runs) => Some(*runs),
        _ => None,
    });
    assert!(runs.is_some_and(|runs| runs >= 8), "{said:?}");
    assert!(
        said.iter().any(|message| matches!(
            message,
            Up::Found {
                origin: NO_FILE,
                ..
            }
        )),
        "a worker that reached nothing at all: {said:?}"
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_ends_when_the_pipe_ends_and_when_it_carries_nonsense() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-end", &[b"\x01one"]);
    let (said, ended) = serve(&scratch, &[]);
    assert_eq!(ended, code(ExitCode::SUCCESS));
    assert!(said.is_empty(), "{said:?}");

    let options = options(&scratch);
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let mut runner = Runner::new(&options, &mut target);
    let mut worker = Worker::new(&mut runner, &options, Box::new(Recorder::default()));
    assert_eq!(
        format!("{:?}", worker.run(&[9u8, 9, 9][..])),
        code(ExitCode::SUCCESS)
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_stops_at_the_end_of_the_time_the_run_was_given() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-deadline", &[b"\x01one"]);
    let options = Options {
        max_total_time: Some(0),
        ..options(&scratch)
    };
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let mut runner = Runner::new(&options, &mut target);
    let recorder = Recorder::default();
    let mut worker = Worker::new(&mut runner, &options, Box::new(recorder.clone()));
    // A deadline of no milliseconds at all has passed by the first look.
    let sent = line(&[Down::Load(vec![0]), Down::Load(vec![0])]);
    assert_eq!(format!("{:?}", worker.run(&*sent)), code(ExitCode::SUCCESS));
    assert_eq!(
        recorder
            .messages()
            .iter()
            .filter(|message| matches!(message, Up::Idle(_)))
            .count(),
        1
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_passes_over_a_file_it_was_dealt_that_is_not_there() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-gone", &[b"\x01one"]);
    let options = options(&scratch);
    let address = register_counters(COUNTERS);
    let mut target = body(address);
    let mut runner = Runner::new(&options, &mut target);
    let recorder = Recorder::default();
    let mut worker = Worker::new(&mut runner, &options, Box::new(recorder.clone()));
    // The worker numbered the files when it started; this takes one away
    // and deals it, along with a number no file has.
    let _ = std::fs::remove_file(scratch.path.join("0000"));
    let sent = line(&[Down::Load(vec![0, 99])]);
    assert_eq!(format!("{:?}", worker.run(&*sent)), code(ExitCode::SUCCESS));
    assert_eq!(recorder.messages(), vec![Up::Idle(0)]);
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_reports_an_input_it_made_that_panics_and_stops() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("worker-boom", &[]);
    let options = options(&scratch);
    let address = register_counters(COUNTERS);
    let mut target = move |input: &[u8]| {
        bump(address, COUNTERS, 0);
        assert!(input.is_empty(), "every input the mutator makes");
    };
    let mut runner = Runner::new(&options, &mut target);
    let recorder = Recorder::default();
    let mut worker = Worker::new(&mut runner, &options, Box::new(recorder.clone()));
    let rounds = vec![Round {
        seed: Draw {
            index: 0,
            bytes: Some(b"seed".to_vec()),
        },
        cross: Draw {
            // A place no pool has: the worker holds nothing for it.
            index: u32::MAX.wrapping_sub(1),
            bytes: Some(b"cross".to_vec()),
        },
    }];
    let sent = line(&[Down::Batch { limit: 16, rounds }]);
    assert_eq!(format!("{:?}", worker.run(&*sent)), code(ExitCode::FAILURE));
    assert!(
        matches!(recorder.messages().last(), Some(Up::Crash(_))),
        "{:?}",
        recorder.messages()
    );
    drop(guard);
}
