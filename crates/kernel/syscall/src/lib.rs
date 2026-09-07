// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod calls;
pub mod dispatch;
pub mod environment;
pub mod fault;
pub mod lifetime;
pub mod reaper;
pub mod watch;

pub use dispatch::{Machine, Reply, Request, decode, dispatch, required_rights};
pub use environment::{Environment, KernelStack};
pub use reaper::{has_work, reap};

#[cfg(test)]
mod tests;
