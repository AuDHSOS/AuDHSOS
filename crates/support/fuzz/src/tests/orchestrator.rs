// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::orchestrator`.
//!
//! The orchestrator never runs the target: it deals work and decides what
//! the pool keeps. So these give it a fleet whose workers are a channel of
//! messages written ahead of time and a buffer per worker to write into,
//! which is the whole of what it sees of a process.

use std::io::Write;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use crate::engine::Runner;
use crate::options::Options;
use crate::orchestrator::{Fleet, Hand, fleet_size};
use crate::proto::{Down, NO_FILE, Up};

use super::{GLOBALS, register_counters, scratch_path};

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
    fn messages(&self) -> Vec<Down> {
        let held = self
            .bytes
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default();
        let mut input = &*held;
        let mut messages = Vec::new();
        while let Ok(message) = Down::read(&mut input) {
            messages.push(message);
        }
        messages
    }
}

/// A corpus directory of its own, removed when it is dropped.
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

    /// What is in it now.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// What a run of the orchestrator over `said` amounted to: the exit code,
/// and what each worker was told.
fn orchestrate(options: &Options, said: Vec<(usize, Option<Up>)>, workers: usize) -> Run {
    let (sender, inbox) = channel();
    for message in said {
        let _ = sender.send(message);
    }
    drop(sender);
    let recorders: Vec<Recorder> = (0..workers).map(|_| Recorder::default()).collect();
    let hands = recorders
        .iter()
        .map(|recorder| Hand::new(Box::new(recorder.clone())))
        .collect();
    let fleet = Fleet::new(hands, inbox, Vec::new());
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(options, &mut target);
    let code = runner.orchestrate(options, fleet);
    Run {
        code: format!("{code:?}"),
        told: recorders.iter().map(Recorder::messages).collect(),
        corpus: runner.pool.len(),
    }
}

/// What a run of the orchestrator amounted to.
struct Run {
    /// How it ended.
    code: String,
    /// What each worker was told.
    told: Vec<Vec<Down>>,
    /// How many inputs the pool holds.
    corpus: usize,
}

/// How an exit code prints, for a test that compares one.
fn code(code: ExitCode) -> String {
    format!("{code:?}")
}

/// The options of a run over `scratch` that stops after `runs` runs.
fn options(scratch: &Scratch, runs: u64) -> Options {
    Options {
        paths: vec![scratch.path.clone()],
        seed: Some(5),
        runs: Some(runs),
        workers: 2,
        artifact_prefix: scratch.path.join(""),
        ..Options::default()
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn the_corpus_is_dealt_round_robin_and_read_once() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-load", &[b"a", b"b", b"c", b"d", b"e"]);
    let run = orchestrate(
        &options(&scratch, 0),
        vec![
            (
                0,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"a".to_vec(),
                    features: vec![1, 2],
                }),
            ),
            (
                1,
                Some(Up::Found {
                    origin: 1,
                    bytes: b"b".to_vec(),
                    features: vec![3],
                }),
            ),
            (0, Some(Up::Idle(3))),
            (1, Some(Up::Idle(2))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    assert_eq!(run.corpus, 2);
    assert_eq!(
        run.told.first().and_then(|told| told.first()),
        Some(&Down::Load(vec![0, 2, 4]))
    );
    assert_eq!(
        run.told.get(1).and_then(|told| told.first()),
        Some(&Down::Load(vec![1, 3]))
    );
    // The corpus was read once and nothing was written back into it.
    assert_eq!(run.told.len(), 2);
    assert_eq!(scratch.files().len(), 5);
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn every_worker_is_handed_the_coverage_table_before_it_fuzzes() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-cover", &[b"a"]);
    let run = orchestrate(
        &options(&scratch, 0),
        vec![
            (
                0,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"a".to_vec(),
                    features: vec![9],
                }),
            ),
            (0, Some(Up::Idle(1))),
            (1, Some(Up::Idle(0))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    for told in &run.told {
        let cover = told.get(1).and_then(|message| match message {
            Down::Cover(bytes) => Some(bytes.clone()),
            _ => None,
        });
        assert_eq!(
            cover.map(|bytes| bytes.len()),
            Some(crate::cover::COVER_BYTES)
        );
    }
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_find_of_a_worker_is_written_into_the_corpus_and_drawn_from_again() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-find", &[b"seed"]);
    let run = orchestrate(
        &options(&scratch, 5),
        vec![
            (
                0,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"seed".to_vec(),
                    features: vec![1],
                }),
            ),
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            (
                0,
                Some(Up::Found {
                    origin: NO_FILE,
                    bytes: b"found".to_vec(),
                    features: vec![2, 3],
                }),
            ),
            (0, Some(Up::Idle(9))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    assert_eq!(run.corpus, 2);
    let names = scratch.files();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(
        names.iter().any(|name| crate::engine::is_found_name(name)),
        "{names:?}"
    );
    // Both workers were given work, and the draws carry the bytes of an
    // input the first time and its place after that.
    for told in &run.told {
        assert!(
            told.iter()
                .any(|message| matches!(message, Down::Batch { .. })),
            "{told:?}"
        );
    }
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_crash_a_worker_reports_ends_the_run_and_is_written_out() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-crash", &[b"a"]);
    let run = orchestrate(
        &options(&scratch, 0),
        vec![(0, Some(Up::Crash(b"BOOM".to_vec())))],
        2,
    );
    assert_eq!(run.code, code(ExitCode::FAILURE));
    assert!(
        scratch
            .files()
            .iter()
            .any(|name| name.starts_with("crash-")),
        "{:?}",
        scratch.files()
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_that_ends_before_it_was_asked_to_ends_the_run() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-lost", &[b"a"]);
    let run = orchestrate(&options(&scratch, 0), vec![(1, None)], 2);
    assert_eq!(run.code, code(ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_with_an_empty_corpus_still_has_something_to_draw() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-empty", &[]);
    let mut options = options(&scratch, 3);
    options.print_final_stats = true;
    options.len_control = 1;
    let run = orchestrate(
        &options,
        vec![
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            (0, Some(Up::Idle(4))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    assert_eq!(run.corpus, 1);
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn an_input_the_pool_refuses_is_not_written_and_a_smaller_one_takes_over() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-reduce", &[b"seed"]);
    let run = orchestrate(
        &options(&scratch, 7),
        vec![
            (
                0,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"seed".to_vec(),
                    features: vec![1],
                }),
            ),
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            // The same feature in more bytes: the pool keeps nothing.
            (
                0,
                Some(Up::Found {
                    origin: NO_FILE,
                    bytes: b"longer input".to_vec(),
                    features: vec![1],
                }),
            ),
            // The same feature in fewer: the pool takes it over, and the
            // file of the input that owned it is deleted.
            (
                0,
                Some(Up::Found {
                    origin: NO_FILE,
                    bytes: b"ab".to_vec(),
                    features: vec![1],
                }),
            ),
            (0, Some(Up::Idle(8))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    assert_eq!(run.corpus, 1);
    // `seed` is a name no run gave, so it stays even though the pool
    // stopped keeping it.
    let names = scratch.files();
    assert!(names.iter().any(|name| name == "0000"), "{names:?}");
    assert_eq!(names.len(), 2, "{names:?}");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_fleet_that_says_nothing_at_all_ends_the_run() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-silent", &[b"a"]);
    let run = orchestrate(&options(&scratch, 0), Vec::new(), 2);
    assert_eq!(run.code, code(ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn the_length_limit_grows_and_the_run_pulses_as_the_runs_add_up() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-pulse", &[b"seed"]);
    let mut options = options(&scratch, 1 << 22);
    options.len_control = 1;
    options.max_len = 4096;
    let run = orchestrate(
        &options,
        vec![
            (
                0,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"seed".to_vec(),
                    features: vec![1],
                }),
            ),
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            (0, Some(Up::Idle(1 << 21))),
            (1, Some(Up::Idle(1 << 21))),
            (0, Some(Up::Idle(1 << 22))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    let grew = run.told.first().is_some_and(|told| {
        let mut limits = told.iter().filter_map(|message| match message {
            Down::Batch { limit, .. } => Some(*limit),
            _ => None,
        });
        let first = limits.next().unwrap_or(0);
        limits.any(|limit| limit > first)
    });
    assert!(grew, "the length limit never rose");
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_worker_whose_pipe_will_not_take_anything_is_written_off() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-broken", &[b"a"]);
    let (sender, inbox) = channel();
    for message in [(0, Some(Up::Idle(0))), (1, Some(Up::Idle(9)))] {
        let _ = sender.send(message);
    }
    drop(sender);
    let hands = vec![
        Hand::new(Box::new(Broken)),
        Hand::new(Box::new(Recorder::default())),
    ];
    let options = options(&scratch, 5);
    let fleet = Fleet::new(hands, inbox, Vec::new());
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(&options, &mut target);
    assert_eq!(
        format!("{:?}", runner.orchestrate(&options, fleet)),
        code(ExitCode::SUCCESS)
    );
    drop(guard);
}

/// A pipe that takes nothing, as a worker that has gone leaves behind.
struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    }
}

#[test]
#[cfg_attr(miri, ignore)]
fn the_orchestrator_looks_at_the_clock_while_the_workers_are_busy() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-tick", &[b"a"]);
    let (sender, inbox) = channel();
    for message in [(0, Some(Up::Idle(0))), (1, Some(Up::Idle(0)))] {
        let _ = sender.send(message);
    }
    // Nothing arrives for several ticks, which is what a run looks like
    // between two batches of a slow target.
    let late = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = sender.send((0, Some(Up::Idle(9))));
    });
    let recorder = Recorder::default();
    let hands = vec![
        Hand::new(Box::new(recorder.clone())),
        Hand::new(Box::new(Recorder::default())),
    ];
    let options = options(&scratch, 5);
    let fleet = Fleet::new(hands, inbox, Vec::new());
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(&options, &mut target);
    assert_eq!(
        format!("{:?}", runner.orchestrate(&options, fleet)),
        code(ExitCode::SUCCESS)
    );
    let _ = late.join();
    assert!(
        recorder
            .messages()
            .iter()
            .any(|message| matches!(message, Down::Stop)),
        "the worker was never stopped"
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_run_ends_when_the_time_it_was_given_is_up() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-deadline", &[b"a"]);
    let (sender, inbox) = channel();
    for message in [(0, Some(Up::Idle(0))), (1, Some(Up::Idle(0)))] {
        let _ = sender.send(message);
    }
    // Held open past the deadline, so that the run ends on the clock and
    // not on a channel with nothing left to say.
    let holding = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1500));
        drop(sender);
    });
    let options = Options {
        max_total_time: Some(1),
        runs: None,
        ..options(&scratch, 0)
    };
    let hands = (0..2)
        .map(|_| Hand::new(Box::new(Recorder::default())))
        .collect();
    let fleet = Fleet::new(hands, inbox, Vec::new());
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(&options, &mut target);
    assert_eq!(
        format!("{:?}", runner.orchestrate(&options, fleet)),
        code(ExitCode::SUCCESS)
    );
    let _ = holding.join();
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_crash_a_worker_makes_while_it_fuzzes_ends_the_run() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-fuzz-crash", &[b"a"]);
    let options = Options {
        len_control: 0,
        ..options(&scratch, 1 << 20)
    };
    let run = orchestrate(
        &options,
        vec![
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            (1, Some(Up::Crash(b"BOOM".to_vec()))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::FAILURE));
    assert!(
        scratch
            .files()
            .iter()
            .any(|name| name.starts_with("crash-")),
        "{:?}",
        scratch.files()
    );
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_corpus_that_is_one_file_takes_no_find_and_no_length_control() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-one-file", &[b"only"]);
    let options = Options {
        paths: vec![scratch.path.join("0000")],
        max_len: 4,
        ..options(&scratch, 9)
    };
    let run = orchestrate(
        &options,
        vec![
            // A place no corpus file has, which is recorded and no more.
            (
                0,
                Some(Up::Found {
                    origin: 99,
                    bytes: b"only".to_vec(),
                    features: vec![1],
                }),
            ),
            (0, Some(Up::Idle(0))),
            (1, Some(Up::Idle(0))),
            // A find of a run whose corpus is a file has nowhere to go.
            (
                0,
                Some(Up::Found {
                    origin: NO_FILE,
                    bytes: b"made".to_vec(),
                    features: vec![2],
                }),
            ),
            (0, Some(Up::Idle(10))),
        ],
        2,
    );
    assert_eq!(run.code, code(ExitCode::SUCCESS));
    assert_eq!(run.corpus, 2);
    assert_eq!(scratch.files(), vec!["0000".to_owned()]);
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_corpus_load_waits_for_a_worker_that_has_said_nothing_yet() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-slow-load", &[b"a", b"b"]);
    let (sender, inbox) = channel();
    let _ = sender.send((0, Some(Up::Idle(1))));
    let late = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = sender.send((1, Some(Up::Idle(1))));
    });
    let options = options(&scratch, 0);
    let hands = (0..2)
        .map(|_| Hand::new(Box::new(Recorder::default())))
        .collect();
    let fleet = Fleet::new(hands, inbox, Vec::new());
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(&options, &mut target);
    assert_eq!(
        format!("{:?}", runner.orchestrate(&options, fleet)),
        code(ExitCode::SUCCESS)
    );
    let _ = late.join();
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_fleet_of_no_workers_at_all_has_nothing_to_deal_to() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-none", &[b"a"]);
    let run = orchestrate(
        &options(&scratch, 1 << 20),
        vec![
            (0, Some(Up::Idle(0))),
            (
                7,
                Some(Up::Found {
                    origin: 0,
                    bytes: b"a".to_vec(),
                    features: vec![1],
                }),
            ),
            (7, Some(Up::Idle(3))),
        ],
        0,
    );
    assert_eq!(run.code, code(ExitCode::FAILURE));
    drop(guard);
}

#[test]
#[cfg_attr(miri, ignore)]
fn a_fleet_of_processes_is_started_from_this_program_and_ended_with_it() {
    let guard = GLOBALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let _ = register_counters(COUNTERS);
    let scratch = Scratch::new("fleet-spawn", &[b"a"]);
    let dictionary = scratch.path.join("words");
    let _ = std::fs::write(&dictionary, "\"paste\"\n");
    let options = Options {
        max_total_time: Some(1),
        dictionary: Some(dictionary),
        value_profile: true,
        ..options(&scratch, 0)
    };
    // The program a worker is started from is this one, which is the test
    // binary here and refuses the flags a worker takes. So every worker
    // ends at once, and that is what the run has to survive: it says so
    // and stops, rather than waiting for a process that is gone.
    let Ok(fleet) = Fleet::spawn(&options) else {
        panic!("no worker could be started");
    };
    let mut target = |_: &[u8]| {};
    let mut runner = Runner::new(&options, &mut target);
    assert_eq!(
        format!("{:?}", runner.orchestrate(&options, fleet)),
        code(ExitCode::FAILURE)
    );
    drop(guard);
}

#[test]
fn a_fleet_is_never_larger_than_the_machine_it_runs_on() {
    let cores = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    assert_eq!(fleet_size(1), 1);
    assert_eq!(fleet_size(usize::MAX), cores);
    assert_eq!(fleet_size(cores.saturating_add(1)), cores);
}
