// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The generator: its stream, its rekeying, and its reseeding.
//!
//! The expected streams follow from the construction, which the module
//! documents: the nonce is the sequence number, the output is the stream
//! from block one onwards, and block zero of the same stream becomes the
//! next key. They were computed from `ChaCha20` by an implementation
//! outside this repository before they were written down.

use crate::chacha::{ChaChaRng, RESEED_BYTES};
use crate::doubles::{CountingEntropy, FailingEntropy};
use crate::error::{EntropyError, RngError};
use crate::source::Rng;
use crate::tests::hex;

/// The seed of the vectors: the bytes zero to thirty-one.
fn seed() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (slot, value) in bytes.iter_mut().zip(0u8..) {
        *slot = value;
    }
    bytes
}

#[test]
fn a_seeded_generator_produces_the_documented_stream() {
    let mut generator = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let cases: [(usize, &str); 5] = [
        (
            32,
            "18b84231ade6a6d113615c61af434e27f8b1f3f5e1ad5b5cecf8fc122a35755c",
        ),
        (16, "8af5abdfbf8b035f024acc8b8dddec7d"),
        (1, "7a"),
        (
            64,
            "54f915de5eaea4748e6fc27d08df34ed6070d2b1c2725d62b829467114b71d10\
             14930f923d0822653b84120aa84924cb83a449753edfcff7a76d81f5bb1af2d3",
        ),
        (
            100,
            "04ecb9f020cfe21e5123538420db8100b6fb2eed27dcbce34a3f9b578d3c7204\
             0083af86457f771e61eaaf30a738845e144966f7281bbb39ff866170f1e6157e\
             6a95f243738b706a4a7ed3ebcea74bfd7642cccaaf10b287158fe942dc76b702\
             1137a659",
        ),
    ];
    for (length, expected) in cases {
        let mut out = vec![0u8; length];
        generator.fill(&mut out).expect("the budget is untouched");
        assert_eq!(
            hex(&out),
            expected.replace(['\n', ' '], ""),
            "length {length}"
        );
    }
}

#[test]
fn a_request_of_nothing_leaves_the_state_alone() {
    let mut generator = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let mut nothing: [u8; 0] = [];
    assert_eq!(generator.fill(&mut nothing), Ok(()));

    let mut first = [0u8; 32];
    generator.fill(&mut first).expect("the budget is untouched");
    assert_eq!(
        hex(&first),
        "18b84231ade6a6d113615c61af434e27f8b1f3f5e1ad5b5cecf8fc122a35755c"
    );
}

#[test]
fn the_generator_rekeys_after_every_request() {
    // One request of sixty-four bytes and two of thirty-two start from the
    // same key, so their first halves agree; the second halves cannot,
    // because the key changed in between.
    let mut long = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let mut whole = [0u8; 64];
    long.fill(&mut whole).expect("the budget is untouched");

    let mut short = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let mut first = [0u8; 32];
    let mut second = [0u8; 32];
    short.fill(&mut first).expect("the budget is untouched");
    short.fill(&mut second).expect("the budget is untouched");

    let (head, tail) = whole.split_at(32);
    assert_eq!(
        hex(head),
        hex(&first),
        "the first request is the same stream"
    );
    assert_ne!(hex(tail), hex(&second), "the second request is a new key");
}

#[test]
fn two_generators_on_one_seed_agree_and_on_two_seeds_do_not() {
    let mut first = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let mut same = ChaChaRng::from_seed(&seed(), CountingEntropy::new(9));
    let mut other_seed = seed();
    other_seed[31] ^= 0x01;
    let mut other = ChaChaRng::from_seed(&other_seed, CountingEntropy::new(0));

    let mut a = [0u8; 48];
    let mut b = [0u8; 48];
    let mut c = [0u8; 48];
    first.fill(&mut a).expect("the budget is untouched");
    same.fill(&mut b).expect("the budget is untouched");
    other.fill(&mut c).expect("the budget is untouched");

    assert_eq!(
        hex(&a),
        hex(&b),
        "the seed decides the stream, not the source"
    );
    assert_ne!(hex(&a), hex(&c));
}

#[test]
fn seeding_asks_the_source_once_and_a_failing_source_yields_no_generator() {
    let source = CountingEntropy::new(0x40);
    let generator = ChaChaRng::new(source).expect("the counting source delivers");
    assert_eq!(generator.source_calls(), 1);

    assert_eq!(
        ChaChaRng::new(FailingEntropy).err(),
        Some(RngError::Entropy(EntropyError::Unavailable))
    );
}

#[test]
fn a_seeded_generator_differs_from_one_that_took_the_same_bytes_as_a_seed() {
    // Seeding mixes the fresh material into the key rather than replacing
    // it, and then advances, so a source that a reader can predict does
    // not hand the reader the state.
    let mut seeded = ChaChaRng::new(CountingEntropy::new(0)).expect("the source delivers");
    let mut direct = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));

    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    seeded.fill(&mut a).expect("the budget is fresh");
    direct.fill(&mut b).expect("the budget is untouched");
    assert_ne!(hex(&a), hex(&b));
}

#[test]
fn the_budget_triggers_exactly_one_reseed_at_its_boundary() {
    let mut generator = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    assert_eq!(
        generator.source_calls(),
        0,
        "a seeded generator asks nobody"
    );

    let full = usize::try_from(RESEED_BYTES).expect("the budget fits an address");
    let mut everything = vec![0u8; full];
    generator
        .fill(&mut everything)
        .expect("a request of exactly the budget is inside it");
    assert_eq!(
        generator.source_calls(),
        0,
        "the boundary is not yet crossed"
    );

    let mut one_more = [0u8; 1];
    generator.fill(&mut one_more).expect("the source delivers");
    assert_eq!(generator.source_calls(), 1, "one reseed, not two");

    let mut again = [0u8; 1];
    generator
        .fill(&mut again)
        .expect("the budget is fresh again");
    assert_eq!(generator.source_calls(), 1);
}

#[test]
fn a_reseed_that_fails_yields_no_bytes() {
    let mut generator = ChaChaRng::from_seed(&seed(), FailingEntropy);
    let full = usize::try_from(RESEED_BYTES).expect("the budget fits an address");
    let mut everything = vec![0u8; full];
    generator
        .fill(&mut everything)
        .expect("a request of exactly the budget is inside it");

    let mut out = [0xAAu8; 16];
    assert_eq!(
        generator.fill(&mut out),
        Err(RngError::Entropy(EntropyError::Unavailable))
    );
    assert_eq!(out, [0xAAu8; 16], "a refused request writes nothing");
}

#[test]
fn a_reseed_on_demand_changes_the_stream() {
    let mut generator = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    let mut before = [0u8; 32];
    generator
        .fill(&mut before)
        .expect("the budget is untouched");

    let mut same = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
    same.reseed().expect("the source delivers");
    let mut after = [0u8; 32];
    same.fill(&mut after).expect("the budget is fresh");

    assert_ne!(hex(&before), hex(&after));
    assert_eq!(same.source_calls(), 1);
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        RngError::Entropy(EntropyError::Unavailable),
        RngError::Exhausted,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
    assert!(!format!("{}", EntropyError::Unavailable).is_empty());
    assert_eq!(
        RngError::from(EntropyError::Unavailable),
        RngError::Entropy(EntropyError::Unavailable)
    );
}

#[test]
fn property_the_seed_decides_the_stream_whatever_the_request_pattern() {
    use test_support::generators::{range, vec};
    use test_support::property::check;

    check(
        "chacharng_pattern",
        &vec(range(0..=40usize), 1..=6),
        |lengths| {
            let mut first = ChaChaRng::from_seed(&seed(), CountingEntropy::new(0));
            let mut second = ChaChaRng::from_seed(&seed(), CountingEntropy::new(200));
            let mut other_seed = seed();
            other_seed[0] ^= 0x80;
            let mut other = ChaChaRng::from_seed(&other_seed, CountingEntropy::new(0));

            let mut from_first = Vec::new();
            let mut from_second = Vec::new();
            let mut from_other = Vec::new();
            for length in lengths {
                let mut buffer = vec![0u8; *length];
                first
                    .fill(&mut buffer)
                    .map_err(|error| format!("first: {error}"))?;
                from_first.extend_from_slice(&buffer);

                let mut buffer = vec![0u8; *length];
                second
                    .fill(&mut buffer)
                    .map_err(|error| format!("second: {error}"))?;
                from_second.extend_from_slice(&buffer);

                let mut buffer = vec![0u8; *length];
                other
                    .fill(&mut buffer)
                    .map_err(|error| format!("other: {error}"))?;
                from_other.extend_from_slice(&buffer);
            }

            if from_first != from_second {
                return Err("the same seed produced two streams".to_owned());
            }
            if !from_first.is_empty() && from_first == from_other {
                return Err("two seeds produced one stream".to_owned());
            }
            Ok(())
        },
    );
}
