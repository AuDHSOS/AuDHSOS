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

/// Number of kernel objects the machine can hold at once, which is what a
/// handle can point at.
pub const OBJECTS: usize = PROCESSES
    + THREADS
    + MEMORY_OBJECTS
    + ENDPOINTS
    + NOTIFICATIONS
    + REPLIES
    + INTERRUPTS
    + IO_PORT_RANGES;

/// Number of handle slots the machine holds, shared by every process. Two
/// per object the machine can hold, because a handle may be duplicated and
/// transferred (D-58).
pub const HANDLE_ENTRIES: usize = 16384;

/// Number of handles one process may hold: a ceiling the quota enforces
/// against [`HANDLE_ENTRIES`], not memory that is set aside per process.
/// The largest demand of this system is the memory server, which holds one
/// handle per memory object it hands out and can therefore reach
/// [`MEMORY_OBJECTS`] (D-58).
pub const HANDLES_PER_PROCESS: usize = 4096;

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
const _: () = assert!(OBJECTS == 7616);
// Room for a second handle to every object the machine can hold.
const _: () = assert!(HANDLE_ENTRIES >= 2 * OBJECTS);
// A single process may not promise more than the machine has.
const _: () = assert!(HANDLES_PER_PROCESS <= HANDLE_ENTRIES);
// The memory server holds one handle per object it hands out.
const _: () = assert!(HANDLES_PER_PROCESS >= MEMORY_OBJECTS);
