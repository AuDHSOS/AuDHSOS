// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one process that owns the corpus, and the workers that do not.
//!
//! A corpus is loaded by running every file in it, which is the expensive
//! part of starting a run: a large corpus of a slow target costs minutes.
//! Every worker doing that is that cost times the number of workers, for
//! one answer. So the corpus is read once, here, and the files are dealt
//! round robin to the workers, which run them and report what they
//! reached. From then on this holds the pool, draws the seeds, and decides
//! `New`, `Reduced`, or nothing; a worker only mutates and runs.
//!
//! The draw is why the pool stays in one place. [`crate::pool::Pool::choose`] weights
//! an input by the features it owns times its place in the pool, and no
//! process that sees a part of the corpus can compute that. A shared
//! coverage bitmap would let workers agree on what is new and leave no one
//! able to say who owns what, which is the weighting thrown away.
//!
//! Invariants: nothing is written to a worker that has not asked for work,
//! so no write blocks on a pipe a busy worker is not reading; a reader
//! thread per worker drains its output the whole time, so no worker blocks
//! writing either; a worker is sent an input's bytes once and its place
//! after that.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitCode, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use crate::corpus::files_under;
use crate::engine::{PULSE_RUNS, Runner, SMALLEST_LIMIT, log2, now};
use crate::options::Options;
use crate::pool::Verdict;
use crate::proto::{Down, Draw, NO_FILE, Round, Up};

/// How many rounds a worker is given at a time. One message per round
/// would make a round trip out of every few runs; this many amortises it
/// to well under a percent and still redraws often enough that a find is
/// in the next batch every worker gets.
const BATCH_ROUNDS: usize = 64;

/// How long the orchestrator waits for a message before it looks at the
/// clock again.
const TICK: Duration = Duration::from_millis(50);

/// One worker, from the orchestrator's side.
pub(crate) struct Hand {
    /// Where its work goes.
    out: Box<dyn Write + Send>,
    /// Which places in the pool it has been sent the bytes of.
    sent: Vec<bool>,
    /// Whether it is waiting for work.
    idle: bool,
    /// Whether it is still there.
    alive: bool,
    /// How many runs it has reported.
    runs: u64,
}

impl Hand {
    /// A worker that writes to `out` and has nothing yet.
    pub(crate) fn new(out: Box<dyn Write + Send>) -> Self {
        Self {
            out,
            sent: Vec::new(),
            idle: true,
            alive: true,
            runs: 0,
        }
    }

    /// Whether this worker holds the bytes of the input in `index`, and
    /// notes that it does from now on.
    fn holds(&mut self, index: usize) -> bool {
        if self.sent.len() <= index {
            self.sent.resize(index.saturating_add(1), false);
        }
        self.sent
            .get_mut(index)
            .is_none_or(|slot| core::mem::replace(slot, true))
    }
}

/// The workers of a run and the one channel their messages arrive on.
pub(crate) struct Fleet {
    /// One per worker.
    hands: Vec<Hand>,
    /// Every worker's messages, tagged with the worker, and `None` where
    /// one ended.
    inbox: Receiver<(usize, Option<Up>)>,
    /// The processes, for a fleet that has any.
    children: Vec<Child>,
}

impl Fleet {
    /// A fleet of the given workers, reading from `inbox`.
    pub(crate) const fn new(
        hands: Vec<Hand>,
        inbox: Receiver<(usize, Option<Up>)>,
        children: Vec<Child>,
    ) -> Self {
        Self {
            hands,
            inbox,
            children,
        }
    }

    /// Starts `options.workers` copies of this program as workers.
    ///
    /// # Errors
    ///
    /// Whatever starting a process or taking its pipes reports.
    pub(crate) fn spawn(options: &Options) -> std::io::Result<Self> {
        let program = std::env::current_exe()?;
        let (sender, inbox) = channel();
        let mut hands = Vec::with_capacity(options.workers);
        let mut children = Vec::with_capacity(options.workers);
        for number in 0..options.workers {
            let mut child = worker_command(&program, options, number).spawn()?;
            let (stdin, stdout) = (child.stdin.take(), child.stdout.take());
            let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
                return Err(std::io::Error::other("a worker was started without pipes"));
            };
            relay(number, stdout, sender.clone());
            hands.push(Hand::new(Box::new(stdin)));
            children.push(child);
        }
        Ok(Self::new(hands, inbox, children))
    }

    /// How many workers the fleet has.
    const fn len(&self) -> usize {
        self.hands.len()
    }

    /// Sends one message to one worker, and writes the worker off if the
    /// pipe will not take it.
    fn send(&mut self, number: usize, message: &Down) {
        let Some(hand) = self.hands.get_mut(number) else {
            return;
        };
        if !hand.alive {
            return;
        }
        if message.write(&mut hand.out).is_err() {
            hand.alive = false;
        }
    }

    /// Sends one message to every worker.
    fn broadcast(&mut self, message: &Down) {
        for number in 0..self.len() {
            self.send(number, message);
        }
    }

    /// Waits a short while for one message.
    fn recv(&self) -> Result<(usize, Option<Up>), RecvTimeoutError> {
        self.inbox.recv_timeout(TICK)
    }

    /// Stops every worker and waits for the processes to end. Answers
    /// whether all of them ended the way they were asked to. A worker is
    /// between batches when this is sent, so the wait is one batch long.
    fn stop(&mut self) -> bool {
        self.broadcast(&Down::Stop);
        self.close();
        let mut clean = true;
        for child in &mut self.children {
            clean &= child.wait().is_ok_and(|status| status.success());
        }
        clean
    }

    /// Ends every worker now.
    ///
    /// This is for a run that is over: a worker in the middle of a corpus
    /// shard would take minutes to reach the next message, and there is
    /// nothing left to wait for it to say.
    fn halt(&mut self) {
        self.close();
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Drops the writing end of every pipe, which is what a worker that is
    /// reading sees as the end of the run.
    fn close(&mut self) {
        for hand in &mut self.hands {
            hand.out = Box::new(std::io::sink());
            hand.alive = false;
        }
    }
}

/// The command line one worker is started with. It is given everything
/// that decides what a run of the target is, and nothing that decides
/// what a run keeps: the pool is the orchestrator's.
fn worker_command(program: &Path, options: &Options, number: usize) -> Command {
    let mut command = Command::new(program);
    let seed = options
        .seed
        .unwrap_or_else(now)
        .wrapping_mul(WORKER_STRIDE)
        .wrapping_add(
            u64::try_from(number)
                .unwrap_or(0)
                .wrapping_mul(WORKER_STRIDE),
        )
        | 1;
    command
        .arg("-fuzz_worker=1")
        .arg(format!("-seed={seed}"))
        .arg(format!("-max_len={}", options.max_len))
        .arg(format!("-timeout={}", options.timeout))
        .arg(format!(
            "-reduce_inputs={}",
            u8::from(options.reduce_inputs)
        ))
        .arg(format!(
            "-use_value_profile={}",
            u8::from(options.value_profile)
        ));
    if let Some(seconds) = options.max_total_time {
        command.arg(format!("-max_total_time={seconds}"));
    }
    if let Some(dictionary) = &options.dictionary {
        let mut flag = std::ffi::OsString::from("-dict=");
        flag.push(dictionary);
        command.arg(flag);
    }
    for path in &options.paths {
        command.arg(path);
    }
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    command
}

/// What one worker's seed is moved by, so that two workers of one run and
/// two runs of one command walk different mutations.
const WORKER_STRIDE: u64 = 0x9E37_79B9_7F4A_7C15;

/// Starts the thread that turns one worker's output into messages on the
/// channel. It runs until the pipe ends, so a worker never blocks writing
/// while the orchestrator is busy with the pool.
fn relay(number: usize, stdout: ChildStdout, sender: Sender<(usize, Option<Up>)>) {
    let _ = std::thread::Builder::new()
        .name(format!("fuzz-worker-{number}"))
        .spawn(move || {
            let mut input = BufReader::new(stdout);
            while let Ok(message) = Up::read(&mut input) {
                if sender.send((number, Some(message))).is_err() {
                    return;
                }
            }
            let _ = sender.send((number, None));
        });
}

/// What a phase of the run ended with.
enum Stage {
    /// It finished.
    Done,
    /// An input made the target panic and has been written out.
    Crashed,
    /// A worker ended before it was asked to.
    Lost,
}

impl Runner<'_> {
    /// The fuzzing loop across processes: load the corpus once, hand out
    /// the coverage table, then draw and deal until the run is over.
    pub(crate) fn orchestrate(&mut self, options: &Options, mut fleet: Fleet) -> ExitCode {
        let files: Vec<PathBuf> = options.paths.iter().flat_map(|p| files_under(p)).collect();
        let outcome = self.load_fleet(options, &mut fleet, &files);
        if let Some(code) = ended(&outcome) {
            fleet.halt();
            return code;
        }
        eprintln!(
            "INFO: seed corpus: {} files, {} bytes, {} of {} blocks reached, on {} workers",
            self.pool.len(),
            self.pool.bytes(),
            self.pool.covered() / 8,
            crate::counters::blocks(),
            fleet.len()
        );
        if self.pool.is_empty() {
            self.pool.offer(&[], &[0], false);
        }
        fleet.broadcast(&Down::Cover(self.pool.cover().bytes()));
        let deadline = options
            .max_total_time
            .map(|seconds| seconds.saturating_mul(1000));
        if let Some(deadline) = deadline {
            eprintln!(
                "INFO: loading the corpus took {}s of the {}s this run has",
                now() / 1000,
                deadline / 1000
            );
        }
        let outcome = self.fuzz_fleet(options, &mut fleet, deadline);
        if let Some(code) = ended(&outcome) {
            fleet.halt();
            return code;
        }
        let clean = fleet.stop();
        if !clean {
            eprintln!("ERROR: a worker did not end cleanly");
            return ExitCode::FAILURE;
        }
        if options.print_final_stats {
            eprintln!(
                "stat::number_of_executed_units: {}\nstat::corpus_size: {}\nstat::features: {}",
                self.runs,
                self.pool.len(),
                self.pool.covered()
            );
        }
        ExitCode::SUCCESS
    }

    /// Deals the corpus round robin and takes in what the workers reached.
    fn load_fleet(&mut self, options: &Options, fleet: &mut Fleet, files: &[PathBuf]) -> Stage {
        let workers = fleet.len().max(1);
        let mut shards = vec![Vec::new(); workers];
        let mut at = 0usize;
        for (place, _) in files.iter().enumerate() {
            if let (Some(shard), Ok(number)) = (shards.get_mut(at), u32::try_from(place)) {
                shard.push(number);
            }
            at = at.saturating_add(1);
            if at >= workers {
                at = 0;
            }
        }
        for (number, shard) in shards.into_iter().enumerate() {
            fleet.send(number, &Down::Load(shard));
        }
        self.pump(options, fleet, files, SMALLEST_LIMIT, workers, None)
    }

    /// Draws and deals until the run is out of time or out of runs.
    fn fuzz_fleet(&mut self, options: &Options, fleet: &mut Fleet, deadline: Option<u64>) -> Stage {
        let mut limit = SMALLEST_LIMIT.min(options.max_len);
        let mut last_find = 0u64;
        let mut printed = 0u64;
        let workers = u64::try_from(fleet.len().max(1)).unwrap_or(1);
        loop {
            if deadline.is_some_and(|end| now() >= end) {
                break;
            }
            if options.runs.is_some_and(|runs| self.runs >= runs) {
                break;
            }
            for number in 0..fleet.len() {
                if !fleet.hands.get(number).is_some_and(|hand| hand.idle) {
                    continue;
                }
                let Some(batch) = self.deal(fleet, number, limit) else {
                    continue;
                };
                if let Some(hand) = fleet.hands.get_mut(number) {
                    hand.idle = false;
                }
                fleet.send(number, &batch);
            }
            let found = self.pump(options, fleet, &[], limit, 1, Some(&mut last_find));
            if !matches!(found, Stage::Done) {
                return found;
            }
            if options.len_control > 0 && limit < options.max_len {
                // The patience of one process, times the number of them,
                // so that the limit grows as fast in wall time as it does
                // in a run of one worker.
                let patience = options
                    .len_control
                    .saturating_mul(u64::from(log2(limit).max(1)))
                    .saturating_mul(workers);
                if self.runs.saturating_sub(last_find) > patience {
                    let step = usize::try_from(log2(limit).max(1)).unwrap_or(1);
                    limit = limit.saturating_add(step).min(options.max_len);
                    last_find = self.runs;
                }
            }
            if self.runs.saturating_sub(printed) >= PULSE_RUNS {
                printed = self.runs;
                self.report("pulse", limit, None);
            }
        }
        self.report("DONE", limit, None);
        Stage::Done
    }

    /// Takes in messages until `wanted` workers have gone idle, or until
    /// nothing has arrived for a tick.
    fn pump(
        &mut self,
        options: &Options,
        fleet: &mut Fleet,
        files: &[PathBuf],
        limit: usize,
        wanted: usize,
        last_find: Option<&mut u64>,
    ) -> Stage {
        let mut rewarded = last_find;
        let mut left = wanted;
        while left > 0 {
            let (number, message) = match fleet.recv() {
                Ok(message) => message,
                // Nothing said in a tick is the orchestrator's chance to
                // look at the clock; a channel with no sender left is
                // every worker gone.
                Err(RecvTimeoutError::Timeout) if rewarded.is_some() => return Stage::Done,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    eprintln!("ERROR: every worker ended before it was asked to");
                    return Stage::Lost;
                }
            };
            let Some(message) = message else {
                if let Some(hand) = fleet.hands.get_mut(number) {
                    hand.alive = false;
                    hand.idle = false;
                }
                eprintln!("ERROR: worker {number} ended before it was asked to");
                return Stage::Lost;
            };
            match message {
                Up::Crash(bytes) => {
                    self.save_artifact(&bytes);
                    return Stage::Crashed;
                }
                Up::Idle(runs) => {
                    if let Some(hand) = fleet.hands.get_mut(number) {
                        hand.idle = true;
                        hand.runs = runs;
                    }
                    self.runs = fleet.hands.iter().map(|hand| hand.runs).sum();
                    left = left.saturating_sub(1);
                }
                Up::Found {
                    origin,
                    bytes,
                    features,
                } => {
                    if self.absorb(options, files, origin, &bytes, &features, limit)
                        && let Some(mark) = rewarded.as_mut()
                    {
                        **mark = self.runs;
                    }
                }
            }
        }
        Stage::Done
    }

    /// Offers what a worker reached to the pool, and answers whether the
    /// pool kept it.
    ///
    /// An input the worker read out of the corpus is already on disk under
    /// the name the corpus gives it, so it is only recorded; one the worker
    /// made is written out and reported.
    fn absorb(
        &mut self,
        options: &Options,
        files: &[PathBuf],
        origin: u32,
        bytes: &[u8],
        features: &[u32],
        limit: usize,
    ) -> bool {
        let verdict = self.pool.offer(bytes, features, options.reduce_inputs);
        self.delete_retired();
        if verdict == Verdict::Nothing {
            return false;
        }
        let index = self.pool.next_index().saturating_sub(1);
        if origin != NO_FILE {
            let at = usize::try_from(origin).unwrap_or(usize::MAX);
            if let Some(file) = files.get(at) {
                self.pool.set_file(index, file.clone());
            }
            return true;
        }
        if let Some(file) = Self::write_corpus(bytes, options) {
            self.pool.set_file(index, file);
        }
        let label = if verdict == Verdict::New {
            "NEW"
        } else {
            "REDUCE"
        };
        self.report(label, limit, Some(bytes.len()));
        true
    }

    /// The batch one worker gets: draws from the pool, each with its bytes
    /// the first time that worker sees it and its place after that.
    fn deal(&mut self, fleet: &mut Fleet, number: usize, limit: usize) -> Option<Down> {
        let mut rounds = Vec::with_capacity(BATCH_ROUNDS);
        for _ in 0..BATCH_ROUNDS {
            let seed = self.draw(fleet, number)?;
            let cross = self.draw(fleet, number)?;
            rounds.push(Round { seed, cross });
        }
        Some(Down::Batch {
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
            rounds,
        })
    }

    /// One draw for one worker.
    fn draw(&mut self, fleet: &mut Fleet, number: usize) -> Option<Draw> {
        let index = self.pool.choose(&mut self.mutator.rng)?;
        let held = fleet.hands.get_mut(number)?.holds(index);
        let bytes = if held {
            None
        } else {
            Some(self.pool.get(index)?.bytes.clone())
        };
        Some(Draw {
            index: u32::try_from(index).ok()?,
            bytes,
        })
    }
}

/// The exit code a phase that did not finish ends the run with.
const fn ended(stage: &Stage) -> Option<ExitCode> {
    match stage {
        Stage::Done => None,
        Stage::Crashed | Stage::Lost => Some(ExitCode::FAILURE),
    }
}
