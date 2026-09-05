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
//! Phase 3 uses the numbers that size the memory bring-up:
//! [`KERNEL_STACKS`], [`KERNEL_REGIONS`], and [`KERNEL_IMAGE_MAX_PAGES`].

use audhsos_abi::layout::{KERNEL_STACK_SLOTS, PAGE_SIZE};

/// Number of processes the machine holds.
pub const PROCESSES: usize = 64;

/// Number of threads the machine holds. Every thread costs five frames of
/// the reserve: four for its kernel stack and one for its IPC buffer
/// (D-57).
pub const THREADS: usize = 256;

/// Number of memory objects the machine holds.
pub const MEMORY_OBJECTS: usize = 4096;

/// Number of endpoints the machine holds.
pub const ENDPOINTS: usize = 1024;

/// Number of notifications the machine holds.
pub const NOTIFICATIONS: usize = 1024;

/// Number of reply objects the machine holds.
pub const REPLIES: usize = 1024;

/// Number of interrupt objects the machine holds.
pub const INTERRUPTS: usize = 64;

/// Number of port ranges the machine hands out.
pub const IO_PORT_RANGES: usize = 64;

/// Number of kernel stacks the machine holds, one per thread.
pub const KERNEL_STACKS: usize = THREADS;

/// Number of bitmap words the kernel stack pool needs.
pub const KERNEL_STACK_WORDS: usize = KERNEL_STACKS / 64;

/// Number of regions one process may map.
pub const REGIONS_PER_PROCESS: usize = 64;

/// Number of handles one process may hold. The handle table lives in the
/// `Process` object and therefore in the pool, so this number multiplied by
/// [`PROCESSES`] and by the size of an entry is memory the kernel image
/// carries whether it is used or not. The largest known demand is the root
/// task with one memory object per boot region, its servers, and their
/// endpoints (D-57).
pub const HANDLES_PER_PROCESS: usize = 1024;

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
