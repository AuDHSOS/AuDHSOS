// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod choice;
pub mod compare;
pub mod secret;

pub use choice::Choice;
pub use compare::{
    ct_copy, ct_eq, ct_select_u8, ct_select_u32, ct_select_u64, ct_swap, ct_swap_u64,
};
pub use secret::{Secret, wipe};

#[cfg(test)]
mod tests;
