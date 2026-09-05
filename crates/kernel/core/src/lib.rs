// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod boot;
pub mod config;
pub mod machine;
pub mod memory;
pub mod print;
pub mod state;
pub mod tick;
pub mod trap;

pub use machine::{MACHINE, Machine, with_machine};
pub use memory::{KernelMemory, MEMORY, MemoryError, with_memory};
pub use state::{KERNEL, KernelState, with_state};
pub use tick::on_tick;
pub use trap::Exception;

#[cfg(test)]
mod tests;
