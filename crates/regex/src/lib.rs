// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod compile;
mod error;
mod matcher;
#[path = "parse.rs"]
pub mod syntax;
use syntax as parse;

pub use compile::Regex;
pub use error::Error;
pub use matcher::{Match, Report};

/// Matching options implemented by the UTF-16 code-unit engine.
/// Unicode code-point mode and case folding are not approximated here.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// `^`/`$` also match at ECMAScript line terminators.
    pub multiline: bool,
    /// `.` also matches line terminators.
    pub dot_all: bool,
}

/// Independent compile and execution budgets. All counts are logical resources,
/// not a replacement for an embedding allocator's total-memory quota.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum pattern length in UTF-16 code units.
    pub pattern_units: usize,
    /// Maximum group nesting, additionally capped at 48.
    pub depth: usize,
    /// Maximum compiled instructions, including capture and epsilon operations.
    pub states: usize,
    /// Maximum explicit capturing groups (group zero is additional).
    pub captures: usize,
    /// Maximum character-class ranges across the pattern before normalization.
    pub ranges: usize,
    /// Maximum numeric quantifier bound before expansion.
    pub repetition: usize,
    /// Maximum number of capture-register cells reserved by the match workspace.
    pub capture_cells: usize,
    /// Maximum input length in code units.
    pub input_units: usize,
    /// Maximum work units, including state visits and capture-register copying.
    pub work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            pattern_units: 65_536,
            depth: 48,
            states: 65_536,
            captures: 64,
            ranges: 16_384,
            repetition: 10_000,
            capture_cells: 4_194_304,
            input_units: 16_777_216,
            work: 100_000_000,
        }
    }
}

#[cfg(test)]
mod tests;
