// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod program;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;
pub mod tar;

pub use program::{Plan, ProgramError, Region, STACK_PAGES, USER_CONSTRAINTS, plan};
pub use tar::{
    Archive, BLOCK, Builder, Entries, Entry, Kind, MAX_PATH, Path, TarError, WriteError,
    archive_len,
};

#[cfg(test)]
mod tests;
