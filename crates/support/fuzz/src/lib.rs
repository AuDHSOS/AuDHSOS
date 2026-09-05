// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

pub mod corpus;
pub mod counters;
pub mod dictionary;
pub mod engine;
pub mod entry;
pub mod feature;
pub mod mutate;
pub mod options;
pub mod pool;
pub mod rng;
pub mod sancov;

pub use corpus::{CorpusError, Outcome, files_under, replay_args, replay_paths};

#[cfg(test)]
mod tests;
