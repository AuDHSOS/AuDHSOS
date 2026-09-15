// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

use std::path::PathBuf;
use std::sync::Mutex;

/// The lock that the tests which touch the engine's global state hold, one
/// at a time: the counter registry, the trace, and the recording flag are
/// one set for the whole process, exactly as they are in a fuzz target.
pub(crate) static GLOBALS: Mutex<()> = Mutex::new(());

/// The scratch directory of one test, under the system temporary
/// directory and named after the process as well as the test.
///
/// The process id belongs in the name: a test deletes its directory
/// before and after it runs, so two test binaries of this crate started
/// at the same time — a second session, a second coverage pass — would
/// delete each other's files under a name fixed at compile time. Most of
/// those collisions fail a test, but not all of them: a run where the
/// dictionary file disappeared still ends in `SUCCESS`, and then the
/// coverage report is a branch or two short of what the sources say.
pub(crate) fn scratch_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "audhsos-fuzz-support-{name}-{}",
        std::process::id()
    ))
}

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

/// Adds one to counter `slot` of the leaked range of `length` bytes at
/// `address`, which is what a body under test does to be seen.
///
/// This is the one place a test body reaches the range, so the argument
/// for it is made once: the test holds [`GLOBALS`], so the range is the
/// one it registered and no other reference to it is alive, and a body
/// runs between the calls the engine makes into `crate::counters` and not
/// inside one.
pub(crate) fn bump(address: usize, length: usize, slot: usize) {
    // SAFETY: as the paragraph above says.
    let region = unsafe { region_at(address, length) };
    if let Some(counter) = region.get_mut(slot.checked_rem(length).unwrap_or(0)) {
        *counter = counter.saturating_add(1);
    }
}

mod corpus;
mod counters;
mod cover;
mod dictionary;
mod engine;
mod feature;
mod mutate;
mod options;
mod orchestrator;
mod pool;
mod proto;
mod rng;
mod sancov;
mod worker;
