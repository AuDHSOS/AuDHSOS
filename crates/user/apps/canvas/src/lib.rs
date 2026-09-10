// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod state;

pub use state::{BACKGROUND, Canvas, ENDS, INK, PEN, Step, TEXT_ORIGIN, span};

#[cfg(test)]
mod tests;
