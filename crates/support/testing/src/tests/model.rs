// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::model`.

#![allow(clippy::arithmetic_side_effects)]

use crate::generators::{BoxGen, Generator, one_of};
use crate::model::{ModelFailure, ModelTest, run_model_test, run_model_test_with};
use crate::property::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Increment,
    Get,
}

/// A counter that jumps to 100 on the third increment.
struct BuggyCounter {
    value: u32,
    increments: u32,
}

struct CounterTest {
    buggy: bool,
    /// What the run has to reach, if anything.
    required: &'static [&'static str],
}

impl ModelTest for CounterTest {
    type Op = Op;
    type Sut = BuggyCounter;
    type Model = u32;

    fn required(&self) -> &'static [&'static str] {
        self.required
    }

    fn reached(&self, model: &u32) -> Vec<&'static str> {
        // A counter that ever got past two is the only thing this model
        // has to say about where a run went.
        if *model > 2 {
            return vec!["past two"];
        }
        Vec::new()
    }

    fn generator(&self) -> BoxGen<Op> {
        one_of(vec![Op::Increment, Op::Get]).boxed()
    }

    fn new_sut(&self) -> BuggyCounter {
        BuggyCounter {
            value: 0,
            increments: 0,
        }
    }

    fn new_model(&self) -> u32 {
        0
    }

    fn step(&self, sut: &mut BuggyCounter, model: &mut u32, op: &Op) -> Result<(), String> {
        match op {
            Op::Increment => {
                sut.increments += 1;
                sut.value = if self.buggy && sut.increments == 3 {
                    100
                } else {
                    sut.value + 1
                };
                *model += 1;
            }
            Op::Get => {}
        }
        if sut.value == *model {
            Ok(())
        } else {
            Err(format!(
                "counter is {} but the model says {}",
                sut.value, *model
            ))
        }
    }
}

#[test]
fn correct_component_passes() {
    run_model_test(
        "correct_counter",
        &CounterTest {
            buggy: false,
            required: &[],
        },
        20,
    );
}

#[test]
fn a_run_that_reaches_what_it_requires_passes() {
    run_model_test(
        "counting_past_two",
        &CounterTest {
            buggy: false,
            required: &["past two"],
        },
        20,
    );
}

#[test]
fn a_run_that_reaches_nothing_it_requires_fails_although_nothing_disagreed() {
    // Sequences of at most two operations cannot count past two, so every
    // one of them passes and the run says nothing. That is the one way a
    // model test rots, and it is a failure here.
    let failure = run_model_test_with(
        &Config::default(),
        "counting_past_two_in_two_steps",
        &CounterTest {
            buggy: false,
            required: &["past two", "never"],
        },
        2,
    )
    .expect_err("a run that reaches nothing");
    let ModelFailure::Vacuous {
        missed, reached, ..
    } = &failure
    else {
        panic!("a run in which nothing disagreed is not a disagreement");
    };
    // In the order the test named them, which is the order it reads
    // them in; what was reached comes back sorted, being a set.
    assert_eq!(*missed, vec!["past two", "never"]);
    assert!(reached.is_empty());
    let report = failure.to_string();
    assert!(
        report.contains("never reached: past two, never"),
        "{report}"
    );
    assert!(report.contains("reached:       nothing"), "{report}");
    assert!(report.contains("AUDHSOS_PROPTEST_SEED="), "{report}");
}

#[test]
fn a_requirement_that_is_reached_is_not_reported_with_one_that_is_not() {
    let failure = run_model_test_with(
        &Config::default(),
        "counting_past_two_and_further",
        &CounterTest {
            buggy: false,
            required: &["past two", "never"],
        },
        20,
    )
    .expect_err("one requirement is unreachable");
    let ModelFailure::Vacuous {
        missed, reached, ..
    } = &failure
    else {
        panic!("nothing disagreed");
    };
    assert_eq!(*missed, vec!["never"]);
    assert_eq!(*reached, vec!["past two"]);
}

#[test]
fn injected_bug_is_found_and_shrunk_to_the_shortest_sequence() {
    let failure = run_model_test_with(
        &Config {
            cases: 300,
            seed: None,
            max_shrink_steps: 4000,
        },
        "buggy_counter",
        &CounterTest {
            buggy: true,
            required: &[],
        },
        20,
    )
    .unwrap_err();
    let ModelFailure::Disagreed(failure) = failure else {
        panic!("a disagreement is reported as one");
    };
    assert_eq!(failure.shrunk, "[Increment, Increment, Increment]");
    assert!(failure.message.starts_with("step 2 (Increment)"));
}

#[test]
#[should_panic(expected = "property `buggy_counter_panics` failed")]
fn run_model_test_panics_on_failure() {
    run_model_test(
        "buggy_counter_panics",
        &CounterTest {
            buggy: true,
            required: &[],
        },
        20,
    );
}

#[test]
#[should_panic(expected = "never reached what it requires")]
fn run_model_test_panics_on_a_run_that_reached_nothing() {
    run_model_test(
        "vacuous_counter_panics",
        &CounterTest {
            buggy: false,
            required: &["never"],
        },
        20,
    );
}

#[test]
fn a_disagreement_is_reported_before_a_missed_requirement() {
    // The component is wrong and the run also reaches nothing it
    // requires; what the reader needs first is the failing sequence.
    let failure = run_model_test_with(
        &Config::default(),
        "buggy_and_vacuous",
        &CounterTest {
            buggy: true,
            required: &["never"],
        },
        20,
    )
    .expect_err("a buggy counter disagrees");
    assert!(matches!(failure, ModelFailure::Disagreed(_)), "{failure:?}");
}
