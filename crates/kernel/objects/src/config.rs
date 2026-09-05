// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! How many of each kind of object the machine holds.
//!
//! Invariant: every number here sizes a field of [`crate::store::Objects`],
//! which is one `static` cell in the `.bss` of the kernel image (D-57), so
//! a change to one of them changes how large that image is before it maps
//! anything.
//!
//! The numbers live beside the objects they count rather than in
//! `kernel-core`, which re-exports them, so that the structure holding the
//! pools can name its own sizes.

/// Number of processes the machine holds.
pub const PROCESSES: usize = 64;

/// Number of threads the machine holds. Every thread costs five frames of
/// the kernel reserve: four for its kernel stack and one for its IPC
/// buffer (D-57).
pub const THREADS: usize = 256;

/// Number of memory objects the machine holds.
pub const MEMORY_OBJECTS: usize = 4096;

/// Number of endpoints the machine holds. Phase 6 fills them.
pub const ENDPOINTS: usize = 1024;

/// Number of notifications the machine holds. Phase 6 fills them.
pub const NOTIFICATIONS: usize = 1024;

/// Number of reply objects the machine holds. Phase 6 fills them.
pub const REPLIES: usize = 1024;

/// Number of interrupt objects the machine holds. Phase 6 fills them.
pub const INTERRUPTS: usize = 64;

/// Number of port ranges the machine hands out. Phase 6 fills them.
pub const IO_PORT_RANGES: usize = 64;

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

const _: () = assert!(OBJECTS == 7616);
// Room for a second handle to every object the machine can hold.
const _: () = assert!(HANDLE_ENTRIES >= 2 * OBJECTS);
// A single process may not promise more than the machine has.
const _: () = assert!(HANDLES_PER_PROCESS <= HANDLE_ENTRIES);
// The memory server holds one handle per object it hands out.
const _: () = assert!(HANDLES_PER_PROCESS >= MEMORY_OBJECTS);
