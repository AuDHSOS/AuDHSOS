// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod bar;
pub mod state;
pub mod window;

pub use bar::Clock;
pub use state::{DESKTOP, Desk, ENDS, NOBODY, Outcome, SHOWS, WINDOWS};
pub use window::Window;

#[cfg(test)]
mod tests;
