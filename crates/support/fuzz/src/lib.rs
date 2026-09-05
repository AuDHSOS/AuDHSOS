// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

pub mod corpus;
pub mod entry;

pub use corpus::{CorpusError, Outcome, files_under, replay_args, replay_paths};
pub use entry::input;

#[cfg(test)]
mod tests;
