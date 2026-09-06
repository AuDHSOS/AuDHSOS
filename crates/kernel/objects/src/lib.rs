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
pub mod wait_queue;

pub use handle_table::{Entry, HandleArena, HandleList};
pub use object::{
    AnyObjectId, Endpoint, EndpointId, Interrupt, InterruptId, IoPortRange, IoPortRangeId,
    MemoryKind, MemoryObject, MemoryObjectId, Notification, NotificationId, Object, Process,
    ProcessId, Queue, Reply, ReplyId, SystemControl, SystemControlId, Thread, ThreadId, Wait,
};
pub use pool::{ObjectId, Pool, PoolError};
pub use quota::{Quota, QuotaExceeded};
pub use store::{Destroyed, MachineObjects, Objects};
pub use wait_queue::WaitQueue;

#[cfg(test)]
mod tests;
