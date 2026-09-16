// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod command;
pub mod screen;
pub mod state;

pub use command::{Command, GET_USAGE, HELP, Locator, SSH_USAGE, Target, parse};
pub use screen::{COLUMNS, ROWS, Screen};
pub use state::{HEIGHT, Line, MAX_INPUT, PROMPT, Paint, Shell, Step, WIDTH};

#[cfg(test)]
mod tests;
