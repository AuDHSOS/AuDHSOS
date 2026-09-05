// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! The counters the instrumentation writes and the engine reads.
//!
//! The compiler gives every basic block of the target a byte, and gives
//! each object file a constructor that hands the engine the range those
//! bytes live in. This module is the only place that touches those ranges,
//! and the only place in the project where a fuzzing run is unsafe code.
//!
//! Invariants: a range is registered once, before `main`, and never moves
//! or is freed, because the linker placed it; the engine is single
//! threaded, so a range is read or written by one thread at a time; the
//! ranges are numbered in registration order, and a counter's number in
//! that order is its identity for the whole run.

use core::sync::atomic::{AtomicUsize, Ordering};

/// The most ranges the engine keeps. One object file with instrumented
/// code registers one range, and a release build of a fuzz target of this
/// project has well under a hundred of them; a range beyond this many is
/// dropped, which loses coverage but never correctness.
const MAX_REGIONS: usize = 1024;

/// How many ranges have been registered.
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// The first byte of each range, as an address with its provenance
/// exposed.
static STARTS: [AtomicUsize; MAX_REGIONS] = [const { AtomicUsize::new(0) }; MAX_REGIONS];

/// The length of each range, in bytes.
static LENGTHS: [AtomicUsize; MAX_REGIONS] = [const { AtomicUsize::new(0) }; MAX_REGIONS];

/// How many blocks the program table describes, which is how many counters
/// the compiler emitted whether or not their range was registered.
static BLOCKS: AtomicUsize = AtomicUsize::new(0);

/// Registers the range `start .. stop` of counters.
///
/// A range that is empty, malformed, or beyond the most this engine keeps is ignored.
///
/// # Safety
///
/// `start` and `stop` must bound one array of bytes that stays alive and
/// at rest for the whole run, and no other thread may write it.
pub unsafe fn register(start: *mut u8, stop: *mut u8) {
    let (start_address, stop_address) = (start.expose_provenance(), stop.expose_provenance());
    let Some(length) = stop_address.checked_sub(start_address) else {
        return;
    };
    if length == 0 {
        return;
    }
    let slot = COUNT.fetch_add(1, Ordering::Relaxed);
    let (Some(into_start), Some(into_length)) = (STARTS.get(slot), LENGTHS.get(slot)) else {
        return;
    };
    into_length.store(length, Ordering::Relaxed);
    into_start.store(start_address, Ordering::Relaxed);
}

/// Records that the program table describes `blocks` more blocks.
pub fn add_blocks(blocks: usize) {
    BLOCKS.fetch_add(blocks, Ordering::Relaxed);
}

/// How many blocks the compiler instrumented.
#[must_use]
pub fn blocks() -> usize {
    BLOCKS.load(Ordering::Relaxed)
}

/// How many counters the registered ranges hold together.
#[must_use]
pub fn total() -> usize {
    let mut total = 0usize;
    for slot in 0..registered() {
        total = total.saturating_add(length_of(slot));
    }
    total
}

/// How many ranges were registered, never more than the most this engine keeps.
fn registered() -> usize {
    COUNT.load(Ordering::Relaxed).min(MAX_REGIONS)
}

/// The length of the range in `slot`.
fn length_of(slot: usize) -> usize {
    LENGTHS
        .get(slot)
        .map_or(0, |length| length.load(Ordering::Relaxed))
}

/// Calls `body` with each registered range in registration order.
///
/// The caller must not let the target run while it holds a range: the
/// target is what writes the counters.
fn with_regions(mut body: impl FnMut(&mut [u8])) {
    for slot in 0..registered() {
        let length = length_of(slot);
        let Some(start) = STARTS.get(slot).map(|start| start.load(Ordering::Relaxed)) else {
            continue;
        };
        if start == 0 || length == 0 {
            continue;
        }
        let pointer: *mut u8 = core::ptr::with_exposed_provenance_mut(start);
        // SAFETY: `register` was promised one live array of `length` bytes
        // at this address, the linker keeps it for the whole run, and this
        // engine is single threaded, so no other reference to it is alive.
        let region = unsafe { core::slice::from_raw_parts_mut(pointer, length) };
        body(region);
    }
}

/// Sets every counter to zero. This is what separates one run of the
/// target from the next.
pub fn clear() {
    with_regions(|region| region.fill(0));
}

/// Calls `visit` with the number and the count of every counter that is
/// not zero.
///
/// The number is the counter's position across all ranges, so that a
/// counter keeps one number for the whole run. Whole words of zeroes are
/// skipped without looking at their bytes, which is what makes reading the
/// counters cost about as little as clearing them.
pub fn for_each_nonzero(mut visit: impl FnMut(usize, u8)) {
    let mut base = 0usize;
    with_regions(|region| {
        let (blocks, tail) = region.as_chunks::<STRIDE>();
        for (index, block) in blocks.iter().enumerate() {
            let (words, _) = block.as_chunks::<8>();
            // The reduction is what makes this loop worth writing: it is a
            // handful of vector loads and ors per block, and the work
            // below it is done only for the blocks a run actually reached.
            let any = words
                .iter()
                .fold(0u64, |seen, word| seen | u64::from_le_bytes(*word));
            if any == 0 {
                continue;
            }
            let at = base.saturating_add(index.saturating_mul(STRIDE));
            for (which, word) in words.iter().enumerate() {
                visit_word(
                    at.saturating_add(which.saturating_mul(8)),
                    u64::from_le_bytes(*word),
                    &mut visit,
                );
            }
        }
        let at = base.saturating_add(blocks.len().saturating_mul(STRIDE));
        for (step, byte) in tail.iter().enumerate() {
            if *byte != 0 {
                visit(at.saturating_add(step), *byte);
            }
        }
        base = base.saturating_add(region.len());
    });
}

/// Calls `visit` for each non-zero byte of `word`, whose first byte is
/// counter `at`.
///
/// The bytes that are not zero are found by their bits rather than by
/// looking at all eight, because a word that is reached at all usually has
/// one counter in it and not eight.
fn visit_word(at: usize, word: u64, visit: &mut impl FnMut(usize, u8)) {
    let mut rest = word;
    while rest != 0 {
        let step = rest.trailing_zeros() / 8;
        let byte = u8::try_from(rest.wrapping_shr(step.wrapping_mul(8)) & 0xff).unwrap_or(0);
        visit(at.saturating_add(usize::try_from(step).unwrap_or(0)), byte);
        rest &= !0xffu64.wrapping_shl(step.wrapping_mul(8));
    }
}

/// How many counters are looked at in one go before any of them is looked
/// at on its own.
const STRIDE: usize = 32;
