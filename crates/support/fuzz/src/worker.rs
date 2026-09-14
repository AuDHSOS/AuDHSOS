// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A worker: mutate, run, and say what was reached.
//!
//! A worker owns no corpus and decides nothing. It holds a copy of the
//! coverage table, runs what the orchestrator hands it, and keeps only the
//! features the table lets it claim. That claim is the whole point: it is
//! the same test the pool makes, made against a table that is never
//! cheaper than the pool's, so a feature a worker drops the pool would
//! have dropped too. What goes up the pipe is therefore an input that
//! reached something and nothing else — a feature list per run would be
//! gigabytes a second.
//!
//! Invariants: the worker claims a feature in its own table as it reports
//! it, so it reports each one once however often it reaches it again; it
//! ends when the pipe ends, so an orchestrator that died leaves no worker
//! behind.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use crate::corpus::files_under;
use crate::cover::Cover;
use crate::engine::{MUTATE_DEPTH, Outcome, Runner, now, watchdog};
use crate::options::Options;
use crate::proto::{Down, Draw, NO_FILE, Round, Up};
use crate::sancov;

/// Serves the orchestrator on the standard input and output of this
/// process. The standard output carries the protocol and nothing else;
/// everything the engine says goes to the standard error output, as it
/// does in a run of one process.
pub(crate) fn serve(runner: &mut Runner<'_>, options: &Options) -> ExitCode {
    watchdog();
    runner.read_dictionary(options);
    let input = BufReader::new(std::io::stdin());
    let output = BufWriter::new(std::io::stdout());
    let mut worker = Worker::new(runner, options, Box::new(output));
    worker.run(input)
}

/// How many inputs a worker will hold by their place in the pool. A place
/// past this is a pipe that went wrong, and is dropped rather than turned
/// into an allocation.
const CACHE_LIMIT: usize = 1 << 24;

/// What one input amounted to for a worker.
enum Probe {
    /// It reached nothing this worker had not seen reached as cheaply.
    Nothing,
    /// It claimed these features.
    Reached(Vec<u32>),
    /// It made the target panic.
    Crashed,
}

/// The state a worker keeps between messages.
pub(crate) struct Worker<'a, 'b> {
    /// The target and the counters it fills.
    runner: &'a mut Runner<'b>,
    /// What this worker has seen reached, and how cheaply.
    cover: Cover,
    /// The inputs the orchestrator has sent, by their place in the pool,
    /// so that a draw costs four bytes after the first time.
    cache: Vec<Vec<u8>>,
    /// The corpus files of the run, in the order the orchestrator numbers
    /// them.
    files: Vec<PathBuf>,
    /// Where the protocol goes.
    output: Box<dyn Write>,
    /// Whether an input that reaches a feature in fewer bytes takes it
    /// over.
    shrink: bool,
    /// When the run must be over, in milliseconds since this process
    /// started, or `None` for as long as it takes.
    deadline: Option<u64>,
}

impl<'a, 'b> Worker<'a, 'b> {
    /// A worker that reports on `output`.
    pub(crate) fn new(
        runner: &'a mut Runner<'b>,
        options: &Options,
        output: Box<dyn Write>,
    ) -> Self {
        Self {
            runner,
            cover: Cover::new(),
            cache: Vec::new(),
            files: options.paths.iter().flat_map(|p| files_under(p)).collect(),
            output,
            shrink: options.reduce_inputs,
            deadline: options
                .max_total_time
                .map(|seconds| seconds.saturating_mul(1000)),
        }
    }

    /// Reads messages until the orchestrator stops or the pipe ends.
    pub(crate) fn run(&mut self, input: impl Read) -> ExitCode {
        let mut input = input;
        loop {
            let Ok(message) = Down::read(&mut input) else {
                return ExitCode::SUCCESS;
            };
            match message {
                Down::Load(files) => {
                    if self.load(&files) {
                        return ExitCode::FAILURE;
                    }
                }
                Down::Cover(bytes) => self.cover.set_bytes(&bytes),
                Down::Batch { limit, rounds } => {
                    if self.batch(limit, &rounds) {
                        return ExitCode::FAILURE;
                    }
                }
                Down::Stop => return ExitCode::SUCCESS,
            }
            if self.deadline.is_some_and(|end| now() >= end) {
                return ExitCode::SUCCESS;
            }
        }
    }

    /// Runs the corpus files this worker was dealt. Answers whether the
    /// run must end, which it must when one of them panics.
    fn load(&mut self, files: &[u32]) -> bool {
        for place in files {
            let at = usize::try_from(*place).unwrap_or(usize::MAX);
            let Some(path) = self.files.get(at) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            match self.probe(&bytes) {
                Probe::Crashed => {
                    self.say(&Up::Crash(bytes));
                    return true;
                }
                Probe::Reached(features) => self.say(&Up::Found {
                    origin: *place,
                    bytes,
                    features,
                }),
                Probe::Nothing => {}
            }
        }
        let runs = self.runner.runs;
        self.say(&Up::Idle(runs));
        false
    }

    /// Works one batch of draws. Answers whether the run must end.
    fn batch(&mut self, limit: u32, rounds: &[Round]) -> bool {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        for round in rounds {
            let seed = self.store(&round.seed);
            let cross = self.store(&round.cross);
            if self.round(&seed, &cross, limit) {
                return true;
            }
        }
        let runs = self.runner.runs;
        self.say(&Up::Idle(runs));
        false
    }

    /// One round: change the input a few times, testing after each change,
    /// so that a change which pays is built on. Answers whether the run
    /// must end.
    fn round(&mut self, seed: &[u8], cross: &[u8], limit: usize) -> bool {
        let mut input = seed.to_vec();
        self.runner.mutator.begin_round();
        for _ in 0..MUTATE_DEPTH {
            let changed = sancov::with_trace(|trace| {
                self.runner
                    .mutator
                    .mutate(&mut input, limit, Some(cross), trace)
            });
            if !changed {
                break;
            }
            match self.probe(&input) {
                Probe::Crashed => {
                    self.say(&Up::Crash(input));
                    return true;
                }
                Probe::Reached(features) => {
                    self.say(&Up::Found {
                        origin: NO_FILE,
                        bytes: input.clone(),
                        features,
                    });
                    self.runner.mutator.reward();
                }
                Probe::Nothing => {}
            }
        }
        false
    }

    /// Runs `input` and keeps the features this worker can claim for it.
    fn probe(&mut self, input: &[u8]) -> Probe {
        if self.runner.execute(input) == Outcome::Panicked {
            return Probe::Crashed;
        }
        let size = u32::try_from(input.len()).unwrap_or(u32::MAX);
        let mut claimed = Vec::new();
        for feature in &self.runner.features {
            if self.cover.claim(*feature, size, self.shrink) {
                claimed.push(*feature);
            }
        }
        if claimed.is_empty() {
            Probe::Nothing
        } else {
            Probe::Reached(claimed)
        }
    }

    /// Remembers the bytes of a draw if it carried them, and answers them.
    fn store(&mut self, draw: &Draw) -> Vec<u8> {
        let at = usize::try_from(draw.index).unwrap_or(usize::MAX);
        if at >= CACHE_LIMIT {
            return Vec::new();
        }
        if let Some(bytes) = &draw.bytes {
            if self.cache.len() <= at {
                self.cache.resize(at.saturating_add(1), Vec::new());
            }
            if let Some(slot) = self.cache.get_mut(at) {
                slot.clear();
                slot.extend_from_slice(bytes);
            }
        }
        self.cache.get(at).cloned().unwrap_or_default()
    }

    /// Sends one message, and gives up on the pipe if it will not take it.
    fn say(&mut self, message: &Up) {
        let _ = message.write(&mut self.output);
    }
}
