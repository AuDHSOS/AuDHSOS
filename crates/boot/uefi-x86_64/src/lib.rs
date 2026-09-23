// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Loader calculations and exit retry policy shared with host tests.

#![cfg_attr(not(test), no_std)]
#![allow(unsafe_code)]

pub mod exit_retry;
pub mod loader_math;

use kernel_hal_api as _;
use kernel_mm as _;

#[cfg(test)]
mod tests;
