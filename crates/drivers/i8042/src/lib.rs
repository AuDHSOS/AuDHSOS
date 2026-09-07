// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod controller;
pub mod device;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod keyboard;
pub mod mouse;

pub use controller::{Controller, Devices, Error, MAX_POLLS, Ports};
pub use keyboard::{KeyCode, KeyEvent};
pub use mouse::PointerEvent;

#[cfg(test)]
mod tests;
