// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! The loop: take an input, change it, run it, keep it if it reached
//! something new.
//!
//! This is libFuzzer's loop, ported. What it does per input is: clear the
//! counters, run the target, read the counters back as features, and offer
//! the input and its features to the pool. What it does per round is take
//! one input out of the pool and change it a few times, testing after each
//! change, so that a change which pays is built on rather than thrown away.
//!
//! Invariants: the target runs on the thread the engine was started on and
//! on no other, which is what lets the instrumentation write into a place
//! that is never locked; an input that made the target panic is written out
//! before the run ends, so that a find is never lost to a full terminal;
//! the length limit only ever grows, so an input already in the corpus is
//! never too long for the run that reads it.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::corpus::files_under;
use crate::counters;
use crate::dictionary::Word;
use crate::feature::feature_of;
use crate::mutate::Mutator;
use crate::options::{Mode, Options, parse, parse_dictionary};
use crate::pool::{Pool, Verdict};
use crate::sancov;

/// How many times one input drawn from the pool is changed before another
/// is drawn. A change that pays is built on; libFuzzer calls this the
/// mutation depth and uses the same number.
const MUTATE_DEPTH: usize = 5;

/// How often the run prints a line when nothing has happened.
const PULSE_RUNS: u64 = 1 << 20;

/// How often the watchdog looks at the clock.
const WATCHDOG_INTERVAL_MS: u64 = 250;

/// When the process started, for a clock the watchdog can read out of an
/// integer.
static START: OnceLock<Instant> = OnceLock::new();

/// When the run in progress began, in milliseconds since the start of the process, or
/// zero between runs.
static RUN_BEGAN: AtomicU64 = AtomicU64::new(0);

/// How long one input may take, in milliseconds.
static RUN_LIMIT: AtomicU64 = AtomicU64::new(0);

/// Milliseconds since the process started.
fn now() -> u64 {
    let started = START.get_or_init(Instant::now);
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Runs a fuzz target's `body` under the command line in `args`.
///
/// This is what the `main` of a fuzz target built with the instrumentation
/// calls. The same target built without it replays a corpus instead, which
/// is [`crate::corpus::replay_args`].
pub fn run(args: impl IntoIterator<Item = OsString>, body: &mut dyn FnMut(&[u8])) -> ExitCode {
    let _ = now();
    let options = match parse(args) {
        Ok(options) => options,
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::FAILURE;
        }
    };
    let mut runner = Runner::new(&options, body);
    match options.mode {
        Mode::Fuzz => runner.fuzz(&options),
        Mode::RunOnce => runner.run_once(&options),
        Mode::Merge => runner.merge(&options),
        Mode::MinimizeCrash => runner.minimize(&options),
    }
}

/// What one run of the target amounted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// The target returned.
    Returned,
    /// The target panicked.
    Panicked,
}

/// The state of a run: the target, the counters it fills, and the pool the
/// engine builds out of them.
struct Runner<'a> {
    /// The target.
    body: &'a mut dyn FnMut(&[u8]),
    /// The features of the last run, reused so that a run allocates
    /// nothing.
    features: Vec<u32>,
    /// The first feature number the value profile may use, which is past
    /// every number a counter can produce.
    value_base: u32,
    /// Whether the value profile is kept.
    value_profile: bool,
    /// The inputs the run keeps.
    pool: Pool,
    /// How the run changes an input.
    mutator: Mutator,
    /// How many times the target has been run.
    runs: u64,
    /// Where a crashing input is written.
    artifact_prefix: PathBuf,
}

impl<'a> Runner<'a> {
    /// A run of `body` set up as `options` asks.
    fn new(options: &Options, body: &'a mut dyn FnMut(&[u8])) -> Self {
        let seed = options
            .seed
            .unwrap_or_else(|| now().wrapping_add(GOLDEN_TIME));
        let value_base = u32::try_from(counters::total().saturating_mul(8)).unwrap_or(u32::MAX);
        sancov::with_trace(|trace| trace.value_profile = options.value_profile);
        RUN_LIMIT.store(options.timeout.saturating_mul(1000), Ordering::Relaxed);
        Self {
            body,
            features: Vec::new(),
            value_base,
            value_profile: options.value_profile,
            pool: Pool::new(),
            mutator: Mutator::new(seed),
            runs: 0,
            artifact_prefix: options.artifact_prefix.clone(),
        }
    }

    /// Runs the target once on `input` and leaves its features in
    /// `self.features`.
    fn execute(&mut self, input: &[u8]) -> Outcome {
        if self.value_profile {
            sancov::with_trace(|trace| trace.values.clear());
        }
        RUN_BEGAN.store(now().max(1), Ordering::Relaxed);
        let body = &mut self.body;
        let outcome = sancov::record(|| std::panic::catch_unwind(AssertUnwindSafe(|| body(input))));
        RUN_BEGAN.store(0, Ordering::Relaxed);
        self.runs = self.runs.saturating_add(1);
        self.features.clear();
        let features = &mut self.features;
        counters::for_each_nonzero(|index, count| features.push(feature_of(index, count)));
        if self.value_profile {
            let base = self.value_base;
            sancov::with_trace(|trace| {
                trace
                    .values
                    .for_each(|bit| features.push(base.saturating_add(bit)));
            });
        }
        if outcome.is_err() {
            Outcome::Panicked
        } else {
            Outcome::Returned
        }
    }

    /// Reads every file the command line named into the pool, and reports
    /// what they reached.
    fn load(&mut self, options: &Options) -> Result<(), ExitCode> {
        let mut longest = 0usize;
        for path in &options.paths {
            for file in files_under(path) {
                let Ok(bytes) = std::fs::read(&file) else {
                    continue;
                };
                longest = longest.max(bytes.len());
                if self.execute(&bytes) == Outcome::Panicked {
                    eprintln!("the corpus file {} makes the target panic", file.display());
                    self.save_artifact(&bytes);
                    return Err(ExitCode::FAILURE);
                }
                let features = core::mem::take(&mut self.features);
                let verdict = self.pool.offer(&bytes, &features, options.reduce_inputs);
                self.features = features;
                if verdict != Verdict::Nothing {
                    let index = self.pool.next_index().saturating_sub(1);
                    self.pool.set_file(index, file);
                }
            }
        }
        eprintln!(
            "INFO: seed corpus: {} files, {} bytes, {} of {} blocks reached",
            self.pool.len(),
            self.pool.bytes(),
            self.pool.covered() / 8,
            counters::blocks()
        );
        let _ = longest;
        Ok(())
    }

    /// The fuzzing loop.
    fn fuzz(&mut self, options: &Options) -> ExitCode {
        watchdog();
        self.read_dictionary(options);
        if let Err(code) = self.load(options) {
            return code;
        }
        if self.pool.is_empty() {
            let empty: [u8; 0] = [];
            if self.offer_input(&empty, options).is_none() {
                self.pool.offer(&empty, &[0], false);
            }
        }
        let deadline = options
            .max_total_time
            .map(|seconds| now().saturating_add(seconds.saturating_mul(1000)));
        let mut limit = SMALLEST_LIMIT.min(options.max_len);
        let mut last_find = 0u64;
        let mut printed = 0u64;
        loop {
            if let Some(deadline) = deadline
                && now() >= deadline
            {
                break;
            }
            if let Some(runs) = options.runs
                && self.runs >= runs
            {
                break;
            }
            if self.round(options, limit, &mut last_find) == Some(Outcome::Panicked) {
                return ExitCode::FAILURE;
            }
            if options.len_control > 0 && limit < options.max_len {
                let patience = options
                    .len_control
                    .saturating_mul(u64::from(log2(limit).max(1)));
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

    /// One round: take an input, change it up to a few times,
    /// and test after each change.
    fn round(&mut self, options: &Options, limit: usize, last_find: &mut u64) -> Option<Outcome> {
        let index = self.pool.choose(&mut self.mutator.rng)?;
        let mut input = self.pool.get(index)?.bytes.clone();
        let cross = self
            .pool
            .choose(&mut self.mutator.rng)
            .and_then(|other| self.pool.get(other))
            .map(|other| other.bytes.clone());
        self.mutator.begin_round();
        for _ in 0..MUTATE_DEPTH {
            let changed = sancov::with_trace(|trace| {
                self.mutator
                    .mutate(&mut input, limit, cross.as_deref(), trace)
            });
            if !changed {
                break;
            }
            match self.offer_input(&input, options) {
                Some(Outcome::Panicked) => return Some(Outcome::Panicked),
                Some(Outcome::Returned) => {
                    *last_find = self.runs;
                    self.mutator.reward();
                }
                None => {}
            }
        }
        Some(Outcome::Returned)
    }

    /// Runs `input` and keeps it if it reached something. Answers `None`
    /// when it reached nothing, and the outcome when it did or panicked.
    fn offer_input(&mut self, input: &[u8], options: &Options) -> Option<Outcome> {
        if self.execute(input) == Outcome::Panicked {
            self.save_artifact(input);
            return Some(Outcome::Panicked);
        }
        let features = core::mem::take(&mut self.features);
        let verdict = self.pool.offer(input, &features, options.reduce_inputs);
        self.features = features;
        if verdict == Verdict::Nothing {
            return None;
        }
        let index = self.pool.next_index().saturating_sub(1);
        if let Some(file) = Self::write_corpus(input, options) {
            self.pool.set_file(index, file);
        }
        let label = if verdict == Verdict::New {
            "NEW"
        } else {
            "REDUCE"
        };
        self.report(label, options.max_len, Some(input.len()));
        Some(Outcome::Returned)
    }

    /// Runs every file once and reports, without changing anything.
    fn run_once(&mut self, options: &Options) -> ExitCode {
        watchdog();
        match self.load(options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        }
    }

    /// Folds the corpora after the first into the first, keeping the files
    /// that add coverage and leaving out the ones that add none.
    fn merge(&mut self, options: &Options) -> ExitCode {
        watchdog();
        let Some((destination, sources)) = options.paths.split_first() else {
            eprintln!("`-merge=1` needs a destination and at least one source");
            return ExitCode::FAILURE;
        };
        let mut kept = Options::clone(options);
        kept.paths = vec![destination.clone()];
        if let Err(code) = self.load(&kept) {
            return code;
        }
        let before = self.pool.covered();
        let mut candidates: Vec<PathBuf> = sources.iter().flat_map(|p| files_under(p)).collect();
        candidates.sort_by_key(|path| {
            (
                std::fs::metadata(path).map_or(u64::MAX, |data| data.len()),
                path.clone(),
            )
        });
        let mut added = 0usize;
        for file in candidates {
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            if self.execute(&bytes) == Outcome::Panicked {
                eprintln!("{} makes the target panic", file.display());
                self.save_artifact(&bytes);
                return ExitCode::FAILURE;
            }
            let features = core::mem::take(&mut self.features);
            let verdict = self.pool.offer(&bytes, &features, false);
            self.features = features;
            if verdict == Verdict::Nothing {
                continue;
            }
            if let Some(written) = write_into(destination, &bytes) {
                added = added.saturating_add(1);
                let index = self.pool.next_index().saturating_sub(1);
                self.pool.set_file(index, written);
            }
        }
        eprintln!(
            "MERGE: {added} files added, features {before} -> {}",
            self.pool.covered()
        );
        ExitCode::SUCCESS
    }

    /// Shrinks a crashing input while it still crashes.
    fn minimize(&mut self, options: &Options) -> ExitCode {
        watchdog();
        let Some(path) = options.paths.first() else {
            eprintln!("`-minimize_crash=1` needs a file");
            return ExitCode::FAILURE;
        };
        let Ok(mut best) = std::fs::read(path) else {
            eprintln!("cannot read {}", path.display());
            return ExitCode::FAILURE;
        };
        let quiet = quiet_panics();
        if self.execute(&best) != Outcome::Panicked {
            std::panic::set_hook(quiet);
            eprintln!("{} does not make the target panic", path.display());
            return ExitCode::FAILURE;
        }
        let deadline = options
            .max_total_time
            .map_or(u64::MAX, |s| now().saturating_add(s.saturating_mul(1000)));
        let attempts = options.runs.unwrap_or(DEFAULT_MINIMIZE_RUNS);
        let mut candidate = Vec::new();
        for _ in 0..attempts {
            if now() >= deadline {
                break;
            }
            candidate.clear();
            candidate.extend_from_slice(&best);
            self.mutator.begin_round();
            let limit = best.len().saturating_sub(1).max(1);
            let changed =
                sancov::with_trace(|trace| self.mutator.mutate(&mut candidate, limit, None, trace));
            if !changed || candidate.len() >= best.len() {
                continue;
            }
            if self.execute(&candidate) == Outcome::Panicked {
                best.clear();
                best.extend_from_slice(&candidate);
                eprintln!("CRASH_MIN: {} bytes", best.len());
            }
        }
        std::panic::set_hook(quiet);
        let written = self.save_artifact_named("minimized-from-", &best);
        eprintln!("CRASH_MIN: done, {} bytes", best.len());
        if written.is_none() {
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    }

    /// Reads the words the run was given, if it was given any.
    fn read_dictionary(&mut self, options: &Options) {
        let Some(path) = options.dictionary.as_ref() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("cannot read the dictionary {}", path.display());
            return;
        };
        for word in parse_dictionary(&text) {
            self.mutator.add_word(Word::new(&word));
        }
        eprintln!("INFO: {} dictionary words", self.mutator.words());
    }

    /// Writes an input into the first corpus directory of the run.
    fn write_corpus(input: &[u8], options: &Options) -> Option<PathBuf> {
        let directory = options.paths.first()?;
        if !directory.is_dir() {
            return None;
        }
        write_into(directory, input)
    }

    /// Writes a crashing input where the run was told to put it.
    fn save_artifact(&self, input: &[u8]) -> Option<PathBuf> {
        self.save_artifact_named("crash-", input)
    }

    /// Writes an input under `prefix` where the run was told to put it.
    fn save_artifact_named(&self, prefix: &str, input: &[u8]) -> Option<PathBuf> {
        let name = format!("{prefix}{}", name_of(input));
        let path = self.artifact_prefix.join(name);
        match std::fs::write(&path, input) {
            Ok(()) => {
                eprintln!("Test unit written to {}", path.display());
                Some(path)
            }
            Err(problem) => {
                eprintln!("cannot write {}: {problem}", path.display());
                let _ = std::io::stderr().flush();
                None
            }
        }
    }

    /// Prints one line about where the run stands.
    fn report(&self, label: &str, limit: usize, size: Option<usize>) {
        let seconds = now().max(1) / 1000;
        let rate = self.runs.checked_div(seconds.max(1)).unwrap_or(0);
        let mut line = format!(
            "#{}\t{label:<6} cov: {} ft: {} corp: {}/{}b lim: {limit} exec/s: {rate}",
            self.runs,
            self.pool.covered() / 8,
            self.pool.covered(),
            self.pool.len(),
            self.pool.bytes()
        );
        if let Some(size) = size {
            let _ = write!(line, " L: {size}");
            let sequence = self.mutator.sequence();
            if !sequence.is_empty() {
                let _ = write!(line, " MS: {} ", sequence.len());
                for step in sequence {
                    let _ = write!(line, "{}-", step.name());
                }
            }
        }
        eprintln!("{line}");
    }
}

/// How many inputs a shrink tries before it gives up, unless it was told
/// otherwise.
const DEFAULT_MINIMIZE_RUNS: u64 = 100_000;

/// The length limit a run starts at, which grows as the run stops finding
/// things.
const SMALLEST_LIMIT: usize = 4;

/// What a seed taken from the clock is mixed with, so that two runs
/// started in the same millisecond still differ in their low bits.
const GOLDEN_TIME: u64 = 0x9E37_79B9_7F4A_7C15;

/// The base-two logarithm of `value`, at least zero.
const fn log2(value: usize) -> u32 {
    usize::BITS
        .saturating_sub(value.leading_zeros())
        .saturating_sub(1)
}

/// Silences the panic message of a caught panic and answers the hook that
/// was in place, so that a caller can put it back.
fn quiet_panics() -> Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync + 'static> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    previous
}

/// Writes `bytes` into `directory` under a name made of their hash, and
/// answers where they went.
fn write_into(directory: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let path = directory.join(name_of(bytes));
    std::fs::write(&path, bytes).ok().map(|()| path)
}

/// The name a corpus file or an artifact of `bytes` gets: a hash of them,
/// in hexadecimal.
///
/// libFuzzer uses a SHA-1 digest here. This is not one, and is not meant
/// to stand up to anyone choosing the bytes: it is only there to give one
/// input one name, so that writing the same input twice writes one file.
#[must_use]
pub fn name_of(bytes: &[u8]) -> String {
    let (mut low, mut high) = (0xcbf2_9ce4_8422_2325u64, 0x9E37_79B9_7F4A_7C15u64);
    for byte in bytes {
        low = (low ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        high = (high ^ u64::from(*byte)).wrapping_mul(0x0000_0000_01b3_0100);
        high = high.rotate_left(7) ^ low;
    }
    format!("{low:016x}{high:016x}")
}

/// Starts the thread that ends a run whose input will not finish.
fn watchdog() {
    static STARTED: AtomicU64 = AtomicU64::new(0);
    if STARTED.swap(1, Ordering::Relaxed) != 0 {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("fuzz-watchdog".to_owned())
        .spawn(|| {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(WATCHDOG_INTERVAL_MS));
                let began = RUN_BEGAN.load(Ordering::Relaxed);
                let limit = RUN_LIMIT.load(Ordering::Relaxed);
                if began == 0 || limit == 0 {
                    continue;
                }
                if now().saturating_sub(began) > limit {
                    eprintln!("ERROR: an input took longer than {limit} ms");
                    let _ = std::io::stderr().flush();
                    std::process::abort();
                }
            }
        });
}
