// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Model-based testing: a generated sequence of operations is applied to
//! the component under test and to a reference model; every step compares
//! the observable results.
//!
//! Invariants: a failing sequence is shrunk to a shorter sequence that still
//! fails, using the vector shrinking of the property engine.
//!
//! A model test has one way to rot that a property does not: the
//! generator drifts, or the component grows a state the operations no
//! longer reach, and the test goes on passing while testing less and
//! less. Nothing in the run says so, because reaching nothing interesting
//! looks exactly like reaching everything and finding no fault. That is
//! what [`ModelTest::required`] is for: a test names the states its
//! sequences have to arrive at, the model records what it arrived at, and
//! a run that misses one fails. It is off by default, so a model that
//! names nothing behaves as before.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Debug;

use crate::generators::{BoxGen, vec};
use crate::property::{Config, Failure, SEED_VARIABLE, check_with, seed_for};

/// A component under test together with its reference model.
pub trait ModelTest {
    /// One operation of the sequence.
    type Op: Clone + Debug + 'static;
    /// The component under test.
    type Sut;
    /// The reference model.
    type Model;

    /// The generator of single operations.
    fn generator(&self) -> BoxGen<Self::Op>;

    /// A fresh component.
    fn new_sut(&self) -> Self::Sut;

    /// A fresh model.
    fn new_model(&self) -> Self::Model;

    /// Applies `op` to both and compares the observable result.
    ///
    /// # Errors
    ///
    /// Returns a description of the first observed difference.
    fn step(
        &self,
        sut: &mut Self::Sut,
        model: &mut Self::Model,
        op: &Self::Op,
    ) -> Result<(), String>;

    /// What every run has to reach for the test to mean anything.
    ///
    /// A run in which no sequence reached one of these names fails, even
    /// though nothing disagreed. The default is nothing, and a model that
    /// keeps the default is checked for disagreement only.
    fn required(&self) -> &'static [&'static str] {
        &[]
    }

    /// What this model reached, which is compared against
    /// [`required`](ModelTest::required) once every sequence has run.
    ///
    /// A model with something to report keeps a field for it and writes
    /// that field in [`step`](ModelTest::step). It is state of the model
    /// and not of the component, and it starts empty with every sequence,
    /// which is why the answer is read at the end of one and gathered
    /// across all of them.
    fn reached(&self, model: &Self::Model) -> Vec<&'static str> {
        let _ = model;
        Vec::new()
    }
}

/// Why a model test did not pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelFailure {
    /// A sequence made the component and the model disagree. The failure
    /// carries the shrunk sequence and the seed that replays it.
    Disagreed(Failure),
    /// Every sequence passed, and the run never reached something the
    /// test requires. Nothing is wrong with the component; what is wrong
    /// is that the run says nothing about it.
    Vacuous {
        /// The name of the test.
        name: String,
        /// The seed the run used.
        seed: u64,
        /// What was required and never reached.
        missed: Vec<&'static str>,
        /// What was reached.
        reached: Vec<&'static str>,
    },
}

impl fmt::Display for ModelFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelFailure::Disagreed(failure) => write!(f, "{failure}"),
            ModelFailure::Vacuous {
                name,
                seed,
                missed,
                reached,
            } => {
                writeln!(f, "model test `{name}` never reached what it requires")?;
                writeln!(f, "  never reached: {}", missed.join(", "))?;
                writeln!(f, "  reached:       {}", list(reached))?;
                write!(f, "  replay with {SEED_VARIABLE}={seed}")
            }
        }
    }
}

/// The names, or a word for none of them.
fn list(names: &[&'static str]) -> String {
    if names.is_empty() {
        return "nothing".to_owned();
    }
    names.join(", ")
}

/// Runs sequences of up to `max_ops` operations with the default
/// configuration.
///
/// # Panics
///
/// Panics with the failure report if a sequence fails.
#[expect(
    clippy::panic,
    reason = "a failed model test is reported by panicking, like assert!"
)]
pub fn run_model_test<M: ModelTest>(name: &str, test: &M, max_ops: usize) {
    if let Err(failure) = run_model_test_with(&Config::default(), name, test, max_ops) {
        panic!("{failure}");
    }
}

/// Runs sequences of up to `max_ops` operations and returns the failure
/// instead of panicking.
///
/// # Errors
///
/// [`ModelFailure::Disagreed`] with the first failing sequence after
/// shrinking, or [`ModelFailure::Vacuous`] when every sequence passed and
/// the run never reached something [`ModelTest::required`] names.
pub fn run_model_test_with<M: ModelTest>(
    config: &Config,
    name: &str,
    test: &M,
    max_ops: usize,
) -> Result<(), ModelFailure> {
    let sequences = vec(test.generator(), 0..=max_ops);
    // Gathered across sequences, because a model starts empty with each
    // one and no single sequence has to reach everything.
    let reached = RefCell::new(BTreeSet::new());
    check_with(config, name, &sequences, |ops| {
        let mut sut = test.new_sut();
        let mut model = test.new_model();
        for (index, op) in ops.iter().enumerate() {
            test.step(&mut sut, &mut model, op)
                .map_err(|message| format!("step {index} ({op:?}): {message}"))?;
        }
        reached.borrow_mut().extend(test.reached(&model));
        Ok(())
    })
    .map_err(ModelFailure::Disagreed)?;

    let reached = reached.into_inner();
    let missed: Vec<&'static str> = test
        .required()
        .iter()
        .copied()
        .filter(|required| !reached.contains(required))
        .collect();
    if missed.is_empty() {
        return Ok(());
    }
    Err(ModelFailure::Vacuous {
        name: name.to_owned(),
        seed: config.seed.unwrap_or_else(|| seed_for(name)),
        missed,
        reached: reached.into_iter().collect(),
    })
}
