// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate alloc;
mod compile;
mod matcher;
pub use audhsos_regex::{Error, Match, Options};
pub use compile::Regex;
pub use matcher::Report;

/// Compile and run limits, independent of the Thompson workspace policy.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Shared parser quotas and compiled-state/input/work limits. The
    /// `capture_cells` field also bounds saved capture and progress registers.
    pub core: audhsos_regex::Limits,
    /// Maximum saved alternatives live at once, across lookarounds.
    pub backtrack_frames: usize,
    /// Maximum live nested assertion contexts (explicit heap stack).
    pub assertion_frames: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            core: audhsos_regex::Limits::default(),
            backtrack_frames: 65_536,
            assertion_frames: 48,
        }
    }
}

#[cfg(test)]
mod tests;
