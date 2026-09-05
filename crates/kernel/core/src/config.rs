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
//! and Phase 6. Phase 3 uses the numbers that size the memory bring-up:
//! [`KERNEL_STACKS`], [`KERNEL_REGIONS`], and [`KERNEL_IMAGE_MAX_PAGES`].

use audhsos_abi::layout::{KERNEL_STACK_SLOTS, PAGE_SIZE};

/// Number of processes the machine holds.
pub const PROCESSES: usize = 256;

/// Number of threads the machine holds.
pub const THREADS: usize = 1024;

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

/// Number of kernel stacks the machine holds.
pub const KERNEL_STACKS: usize = 1024;

/// Number of bitmap words the kernel stack pool needs.
pub const KERNEL_STACK_WORDS: usize = KERNEL_STACKS / 64;

/// Number of regions one process may map.
pub const REGIONS_PER_PROCESS: usize = 64;

/// Number of handles one process may hold.
pub const HANDLES_PER_PROCESS: usize = 1 << 16;

/// Number of regions the kernel address space holds: the physical window,
/// the boot stack, the boot information page, and one per run of mapped
/// pages of the kernel image.
pub const KERNEL_REGIONS: usize = 64;

/// How far above the kernel base the bring-up looks for mapped pages of
/// the kernel image, in pages. The image of a debug build of a test kernel
/// stays well below this.
pub const KERNEL_IMAGE_MAX_PAGES: u64 = 8192;

const _: () = assert!(KERNEL_STACKS == KERNEL_STACK_WORDS * 64);
const _: () = assert!(KERNEL_STACKS == 1024 && KERNEL_STACK_SLOTS == 1024);
const _: () = assert!(KERNEL_IMAGE_MAX_PAGES * PAGE_SIZE == 32 * 1024 * 1024);
