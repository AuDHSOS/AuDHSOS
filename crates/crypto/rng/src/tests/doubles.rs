// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The doubles, which the protocol tests depend on as much as on the
//! generator.

use crate::doubles::{CountingEntropy, FailingEntropy, ScriptedRng};
use crate::error::{EntropyError, RngError};
use crate::source::{Entropy, Rng};
use crate::tests::hex;

#[test]
fn the_scripted_generator_hands_out_its_script_and_then_refuses() {
    let script: Vec<u8> = (0u8..10).collect();
    let mut generator = ScriptedRng::new(&script);
    assert_eq!(generator.left(), 10);

    let mut first = [0u8; 4];
    generator.fill(&mut first).expect("four of ten are left");
    assert_eq!(hex(&first), "00010203");
    assert_eq!(generator.left(), 6);

    let mut second = [0u8; 6];
    generator.fill(&mut second).expect("six of six are left");
    assert_eq!(hex(&second), "040506070809");
    assert_eq!(generator.left(), 0);

    let mut third = [0u8; 1];
    assert_eq!(generator.fill(&mut third), Err(RngError::Exhausted));
    assert_eq!(third, [0u8; 1], "a refused request writes nothing");

    let mut nothing: [u8; 0] = [];
    assert_eq!(generator.fill(&mut nothing), Ok(()));
}

#[test]
fn a_request_longer_than_the_script_is_refused_whole() {
    let script: Vec<u8> = (0u8..4).collect();
    let mut generator = ScriptedRng::new(&script);
    let mut out = [0xFFu8; 5];
    assert_eq!(generator.fill(&mut out), Err(RngError::Exhausted));
    assert_eq!(out, [0xFFu8; 5]);
    assert_eq!(generator.left(), 4, "nothing was consumed");
}

#[test]
fn the_counting_source_counts_up_and_reports_its_calls() {
    let mut source = CountingEntropy::new(0xFE);
    assert_eq!(source.calls(), 0);

    let mut out = [0u8; 4];
    source.fill(&mut out).expect("the source delivers");
    assert_eq!(hex(&out), "feff0001", "the counter wraps");
    assert_eq!(source.calls(), 1);

    source.fill(&mut out).expect("the source delivers");
    assert_eq!(hex(&out), "02030405");
    assert_eq!(source.calls(), 2);
}

#[test]
fn the_failing_source_never_delivers() {
    let mut source = FailingEntropy;
    let mut out = [7u8; 8];
    assert_eq!(source.fill(&mut out), Err(EntropyError::Unavailable));
    assert_eq!(out, [7u8; 8]);
}
