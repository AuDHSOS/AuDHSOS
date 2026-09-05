// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

use std::sync::Mutex;

/// The lock that the tests which touch the engine's global state hold, one
/// at a time: the counter registry, the trace, and the recording flag are
/// one set for the whole process, exactly as they are in a fuzz target.
pub(crate) static GLOBALS: Mutex<()> = Mutex::new(());

/// Leaks `length` bytes and registers them as a range of counters, as the
/// compiler's constructor would. Answers the address, from which a test
/// makes its own slice while nothing else holds one.
pub(crate) fn register_counters(length: usize) -> usize {
    let region: &'static mut [u8] = Vec::leak(vec![0u8; length]);
    let start = region.as_mut_ptr();
    let address = start.expose_provenance();
    // SAFETY: the bytes were leaked, so they live as long as the process,
    // and this thread holds `GLOBALS`, so nothing else touches them.
    unsafe { crate::counters::register(start, start.wrapping_add(length)) };
    address
}

/// The leaked range at `address`, `length` bytes of it.
///
/// # Safety
///
/// The caller must hold [`GLOBALS`] and must not be inside a call that
/// hands the same range to `crate::counters`.
pub(crate) unsafe fn region_at(address: usize, length: usize) -> &'static mut [u8] {
    let pointer: *mut u8 = core::ptr::with_exposed_provenance_mut(address);
    // SAFETY: the caller promises the range is the leaked one and that no
    // other reference to it is alive.
    unsafe { core::slice::from_raw_parts_mut(pointer, length) }
}

mod corpus;
mod counters;
mod dictionary;
mod engine;
mod feature;
mod mutate;
mod options;
mod pool;
mod rng;
mod sancov;
