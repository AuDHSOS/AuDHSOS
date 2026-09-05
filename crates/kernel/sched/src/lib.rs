// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod scheduler;
pub mod transition;

pub use scheduler::{Outcome, PRIORITIES, Scheduler};
pub use transition::{Event, TRANSITIONS, is_legal, next};

#[cfg(test)]
mod tests;
