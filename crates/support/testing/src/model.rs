// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Model-based testing: a generated sequence of operations is applied to
//! the component under test and to a reference model; every step compares
//! the observable results.
//!
//! Invariants: a failing sequence is shrunk to a shorter sequence that still
//! fails, using the vector shrinking of the property engine.

use std::fmt::Debug;

use crate::generators::{BoxGen, vec};
use crate::property::{Config, Failure, check_with};

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

/// Runs sequences of up to `max_ops` operations and returns the shrunk
/// failure instead of panicking.
///
/// # Errors
///
/// Returns the first failing sequence after shrinking.
pub fn run_model_test_with<M: ModelTest>(
    config: &Config,
    name: &str,
    test: &M,
    max_ops: usize,
) -> Result<(), Failure> {
    let sequences = vec(test.generator(), 0..=max_ops);
    check_with(config, name, &sequences, |ops| {
        let mut sut = test.new_sut();
        let mut model = test.new_model();
        for (index, op) in ops.iter().enumerate() {
            test.step(&mut sut, &mut model, op)
                .map_err(|message| format!("step {index} ({op:?}): {message}"))?;
        }
        Ok(())
    })
}
