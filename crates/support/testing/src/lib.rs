// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod generators;
pub mod model;
pub mod property;
pub mod rng;
pub mod tree;

#[cfg(test)]
mod tests;
