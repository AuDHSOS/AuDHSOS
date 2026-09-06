// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod handle;
pub mod heap;
pub mod message;
pub mod report;
pub mod startup;

pub use handle::{
    EndpointHandle, InterruptHandle, IoPortHandle, MemoryHandle, NotificationHandle, ProcessHandle,
    ReplyHandle, SystemControlHandle, ThreadHandle, Typed,
};
pub use heap::{AllocError, Allocator, Block, Extent};
pub use message::{CodecError, MAX_BYTES, Reader, Writer};
pub use report::Line;
pub use startup::{MAX_RAM_OBJECTS, ReadError, Startup};

#[cfg(test)]
mod tests;
