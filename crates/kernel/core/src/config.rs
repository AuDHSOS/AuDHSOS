// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! How many of each kind of kernel object the machine holds, and how large
//! the fixed tables of the kernel address space are.
//!
//! Invariant: every number here sizes a `static` array, so a change to one
//! of them changes how much of the kernel reserve the kernel occupies
//! before it hands anything out.
//!
//! The pools of [`PROCESSES`], [`THREADS`], [`MEMORY_OBJECTS`],
//! [`ENDPOINTS`], [`NOTIFICATIONS`], [`REPLIES`], [`INTERRUPTS`], and
//! [`IO_PORT_RANGES`] arrive with the object types they hold, in Phase 5
//! and Phase 6. They are `static` cells and therefore live in the `.bss` of
//! the kernel image, not in the kernel reserve, because a `Pool<T, N>` is a
//! typed array and putting one into raw frames would need `unsafe` in a
//! logic crate (D-57). The reserve holds what the count of is not known
//! before boot: page tables, kernel stacks, and IPC buffers.
//!
//! The handle slots are one shared arena of [`HANDLE_ENTRIES`] rather than
//! a table inside every `Process`, so that a process that needs many
//! handles does not cost every other process the same memory (D-58).
//!
//! Phase 3 uses the numbers that size the memory bring-up:
//! [`KERNEL_STACKS`], [`KERNEL_REGIONS`], and [`KERNEL_IMAGE_MAX_PAGES`].

use audhsos_abi::layout::{KERNEL_STACK_SLOTS, PAGE_SIZE, REGIONS_PER_PROCESS as ABI_REGIONS};

// The counts of the objects live beside the objects, in
// `kernel_objects::config`, so that the structure holding the pools can
// name its own sizes. They are re-exported here, where the rest of the
// kernel has always looked for them.
pub use kernel_objects::config::{
    ENDPOINTS, HANDLE_ENTRIES, HANDLES_PER_PROCESS, INTERRUPTS, IO_PORT_RANGES, MEMORY_OBJECTS,
    NOTIFICATIONS, OBJECTS, PROCESSES, REPLIES, THREADS,
};

/// Number of kernel stacks the machine holds, one per thread.
pub const KERNEL_STACKS: usize = THREADS;

/// Number of bitmap words the kernel stack pool needs.
pub const KERNEL_STACK_WORDS: usize = KERNEL_STACKS / 64;

/// Number of regions one process may map. The number is part of the ABI,
/// because a process sees it when a mapping is refused.
pub const REGIONS_PER_PROCESS: usize = ABI_REGIONS;

/// Number of regions the kernel address space holds: the physical window,
/// the boot stack, the boot information page, and one per run of mapped
/// pages of the kernel image.
pub const KERNEL_REGIONS: usize = 64;

/// How far above the kernel base the bring-up looks for mapped pages of
/// the kernel image, in pages. The image of a debug build of a test kernel
/// stays well below this.
pub const KERNEL_IMAGE_MAX_PAGES: u64 = 8192;

const _: () = assert!(KERNEL_STACKS == KERNEL_STACK_WORDS * 64);
const _: () = assert!(KERNEL_STACKS == 256 && KERNEL_STACK_SLOTS == 256);
const _: () = assert!(KERNEL_IMAGE_MAX_PAGES * PAGE_SIZE == 32 * 1024 * 1024);
