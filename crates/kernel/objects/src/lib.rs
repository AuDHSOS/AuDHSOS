// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod config;
pub mod handle_table;
pub mod object;
pub mod pool;
pub mod quota;
pub mod store;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use handle_table::{Entry, HandleArena, HandleList};
pub use object::{
    AnyObjectId, MemoryKind, MemoryObject, MemoryObjectId, Object, Process, ProcessId, Thread,
    ThreadId,
};
pub use pool::{ObjectId, Pool, PoolError};
pub use quota::{Quota, QuotaExceeded};
pub use store::{MachineObjects, Objects};

#[cfg(test)]
mod tests;
