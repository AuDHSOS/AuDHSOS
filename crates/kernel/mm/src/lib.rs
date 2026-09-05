// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod address_space;
pub mod frame_allocator;
pub mod kernel_half;
pub mod mapper;
pub mod memory_map;
pub mod page_table;
pub mod reserve;
pub mod stack;
#[cfg(any(test, feature = "test-strategies"))]
pub mod strategies;

pub use address_space::{Region, RegionError, RegionTable, Removed};
pub use frame_allocator::{BitmapFrameAllocator, FrameError, NoFrames};
pub use kernel_half::{ShareError, free_user_half, share as share_kernel_half};
pub use mapper::{MapError, Mapper, Progress};
pub use memory_map::{MapError as MemoryMapError, NormalizedMap, normalize};
pub use page_table::{CachePolicy, EntryError, EntryFormat, PageTable, Permissions, X86Entry};
pub use reserve::select_reserve;
pub use stack::{KernelStack, StackError, StackPool};

#[cfg(test)]
mod tests;
