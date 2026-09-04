// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::model`.

#![allow(clippy::arithmetic_side_effects)]

use crate::generators::{BoxGen, Generator, one_of};
use crate::model::{ModelTest, run_model_test, run_model_test_with};
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
}

impl ModelTest for CounterTest {
    type Op = Op;
    type Sut = BuggyCounter;
    type Model = u32;

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
    run_model_test("correct_counter", &CounterTest { buggy: false }, 20);
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
        &CounterTest { buggy: true },
        20,
    )
    .unwrap_err();
    assert_eq!(failure.shrunk, "[Increment, Increment, Increment]");
    assert!(failure.message.starts_with("step 2 (Increment)"));
}

#[test]
#[should_panic(expected = "property `buggy_counter_panics` failed")]
fn run_model_test_panics_on_failure() {
    run_model_test("buggy_counter_panics", &CounterTest { buggy: true }, 20);
}
