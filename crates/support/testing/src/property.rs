// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The property runner: generates cases, finds a failure, shrinks it, and
//! reports it with the seed needed to replay it.
//!
//! Invariants: with the same seed and the same generator, the same cases are
//! produced in the same order; shrinking only ever moves to a candidate that
//! still fails.

use std::fmt;

use crate::generators::Generator;
use crate::rng::Rng;
use crate::tree::Tree;

/// Environment variable that overrides the seed of every property.
pub const SEED_VARIABLE: &str = "AUDHSOS_PROPTEST_SEED";

/// Runner settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Number of cases to run.
    pub cases: u32,
    /// Seed override; `None` derives the seed from the property name or the
    /// environment.
    pub seed: Option<u64>,
    /// Upper bound on property evaluations spent on shrinking.
    pub max_shrink_steps: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            cases: 512,
            seed: None,
            max_shrink_steps: 4096,
        }
    }
}

/// A failed property with the information needed to replay it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Name of the property.
    pub name: String,
    /// Seed that reproduces the failure.
    pub seed: u64,
    /// Index of the failing case.
    pub case: u32,
    /// Debug rendering of the first failing value.
    pub original: String,
    /// Debug rendering of the shrunk value.
    pub shrunk: String,
    /// Property evaluations spent on shrinking.
    pub shrink_steps: u32,
    /// Message of the property for the shrunk value.
    pub message: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "property `{}` failed", self.name)?;
        writeln!(f, "  message:  {}", self.message)?;
        writeln!(f, "  shrunk:   {}", self.shrunk)?;
        writeln!(f, "  original: {} (case {})", self.original, self.case)?;
        writeln!(f, "  shrink steps: {}", self.shrink_steps)?;
        write!(f, "  replay with {SEED_VARIABLE}={}", self.seed)
    }
}

/// Parses a seed given in decimal or as `0x`-prefixed hexadecimal.
#[must_use]
pub fn parse_seed(text: &str) -> Option<u64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// FNV-1a hash of the name; the default seed of a property.
#[must_use]
pub fn seed_from_name(name: &str) -> u64 {
    name.bytes().fold(0xCBF2_9CE4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3)
    })
}

/// The seed for `name`: the environment override if present and valid,
/// otherwise [`seed_from_name`].
#[must_use]
pub fn seed_for(name: &str) -> u64 {
    std::env::var(SEED_VARIABLE)
        .ok()
        .and_then(|text| parse_seed(&text))
        .unwrap_or_else(|| seed_from_name(name))
}

/// Runs `property` on cases from `generator` with the default configuration.
///
/// # Panics
///
/// Panics with the failure report if a case fails.
#[expect(
    clippy::panic,
    reason = "a failed property is reported by panicking, like assert!"
)]
pub fn check<G, P>(name: &str, generator: &G, property: P)
where
    G: Generator,
    P: Fn(&G::Value) -> Result<(), String>,
{
    if let Err(failure) = check_with(&Config::default(), name, generator, property) {
        panic!("{failure}");
    }
}

/// Runs `property` on cases from `generator` and returns the shrunk failure
/// instead of panicking.
///
/// # Errors
///
/// Returns the first failing case after shrinking.
pub fn check_with<G, P>(
    config: &Config,
    name: &str,
    generator: &G,
    property: P,
) -> Result<(), Failure>
where
    G: Generator,
    P: Fn(&G::Value) -> Result<(), String>,
{
    let seed = config.seed.unwrap_or_else(|| seed_for(name));
    let mut rng = Rng::from_seed(seed);
    for case in 0..config.cases {
        let tree = generator.generate(&mut rng);
        if let Err(message) = property(tree.value()) {
            let original = format!("{:?}", tree.value());
            let (shrunk, shrink_steps, message) =
                shrink(tree, message, config.max_shrink_steps, &property);
            return Err(Failure {
                name: name.to_owned(),
                seed,
                case,
                original,
                shrunk: format!("{:?}", shrunk.value()),
                shrink_steps,
                message,
            });
        }
    }
    Ok(())
}

fn shrink<T, P>(
    tree: Tree<T>,
    message: String,
    max_steps: u32,
    property: &P,
) -> (Tree<T>, u32, String)
where
    T: Clone + 'static,
    P: Fn(&T) -> Result<(), String>,
{
    let mut current = tree;
    let mut current_message = message;
    let mut steps = 0u32;
    'descend: while steps < max_steps {
        for child in current.shrinks() {
            steps = steps.saturating_add(1);
            if let Err(message) = property(child.value()) {
                current = child;
                current_message = message;
                continue 'descend;
            }
            if steps >= max_steps {
                break;
            }
        }
        break;
    }
    (current, steps, current_message)
}
