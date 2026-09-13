// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The measurements: a sequential stream and a random walk.

use std::hint::black_box;
use std::time::Instant;

use crate::config::{Config, MEBIBYTE_SHIFT};

/// Bytes in one word of the streamed buffers, which hold `u64`.
const WORD_BYTES: u64 = 8;

/// The working sets the pointer chase is measured on, in bytes, with what
/// each is meant to reach on a common machine. Everything above the size a
/// run asks for is left out.
const WORKING_SETS: [(u64, &str); 5] = [
    (16 << 10, "16 KiB (L1)"),
    (128 << 10, "128 KiB (L2)"),
    (4 << 20, "4 MiB (L2/L3)"),
    (64 << 20, "64 MiB (L3)"),
    (512 << 20, "512 MiB (RAM)"),
];

/// Measures what `config` asks for and writes the report.
///
/// # Errors
///
/// The message naming the buffer the machine has no room for.
pub(crate) fn run(config: Config) -> Result<(), String> {
    let words = stream_words(config.bytes);
    println!(
        "sequential, two buffers of {} MiB, {} passes",
        config.bytes >> MEBIBYTE_SHIFT,
        config.passes
    );
    let source = filled(words, 1u64)?;
    let mut destination = filled(words, 2u64)?;
    let moved = bytes_moved(words, WORD_BYTES, config.passes);
    report("read", moved, read(&source, config.passes));
    report("write", moved, write(&mut destination, config.passes));
    // A copy reads as much as it writes, and both cross the bus.
    report(
        "copy",
        moved.saturating_mul(2),
        copy(&mut destination, &source, config.passes),
    );
    drop(source);
    drop(destination);
    println!("random pointer chase, {} steps", config.steps);
    for (bytes, name) in working_sets(config.chase_bytes) {
        let nanoseconds = chase(chase_words(bytes), config.steps)?;
        println!("  {name:<14} {nanoseconds:8.2} ns");
    }
    Ok(())
}

/// The working sets up to `limit`, and the smallest one whatever the
/// limit: a run always measures something.
pub(crate) fn working_sets(limit: u64) -> Vec<(u64, &'static str)> {
    let chosen: Vec<(u64, &'static str)> = WORKING_SETS
        .into_iter()
        .filter(|(bytes, _)| *bytes <= limit)
        .collect();
    if chosen.is_empty() {
        return WORKING_SETS.into_iter().take(1).collect();
    }
    chosen
}

/// The `u64` words a streamed buffer of `bytes` holds, at least one.
pub(crate) fn stream_words(bytes: u64) -> usize {
    words_of(bytes, WORD_BYTES)
}

/// The `usize` words the chase buffer of a working set of `bytes` holds, at
/// least one. A word of the chase is a `usize`, which is not eight bytes
/// everywhere, and a working set is what it says only if that is counted.
pub(crate) fn chase_words(bytes: u64) -> usize {
    words_of(
        bytes,
        u64::try_from(size_of::<usize>()).unwrap_or(WORD_BYTES),
    )
}

/// The words of `word_bytes` that `bytes` holds, at least one.
fn words_of(bytes: u64, word_bytes: u64) -> usize {
    let words = bytes.checked_div(word_bytes).unwrap_or(0).max(1);
    usize::try_from(words).unwrap_or(usize::MAX)
}

/// The bytes one bandwidth measurement touches.
pub(crate) fn bytes_moved(words: usize, word_bytes: u64, passes: u64) -> u64 {
    u64::try_from(words)
        .unwrap_or(u64::MAX)
        .saturating_mul(word_bytes)
        .saturating_mul(passes)
}

/// A buffer of `words` words, every one of them `fill`.
///
/// The room is asked for before it is taken, so that a size the machine
/// cannot hold is a message and not an abort.
fn filled<T: Clone>(words: usize, fill: T) -> Result<Vec<T>, String> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(words)
        .map_err(|_| format!("no room for a buffer of {words} words"))?;
    buffer.resize(words, fill);
    Ok(buffer)
}

/// Writes one bandwidth line.
fn report(what: &str, bytes: u64, seconds: f64) {
    println!(
        "  {what:<6} {:8.2} GB/s",
        gigabytes_per_second(bytes, seconds)
    );
}

/// The gigabytes per second `bytes` in `seconds` amount to; zero for a
/// measurement whose clock stood still.
pub(crate) fn gigabytes_per_second(bytes: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    as_f64(bytes) / 1e9 / seconds
}

/// The seconds a sum over the whole buffer takes, once per pass.
fn read(buffer: &[u64], passes: u64) -> f64 {
    let start = Instant::now();
    for _ in 0..passes {
        let sum = black_box(buffer)
            .iter()
            .fold(0u64, |sum, word| sum.wrapping_add(*word));
        black_box(sum);
    }
    start.elapsed().as_secs_f64()
}

/// The seconds filling the whole buffer takes, once per pass.
fn write(buffer: &mut [u64], passes: u64) -> f64 {
    let start = Instant::now();
    for pass in 0..passes {
        for word in black_box(&mut *buffer) {
            *word = pass;
        }
        black_box(&buffer);
    }
    start.elapsed().as_secs_f64()
}

/// The seconds copying the whole buffer takes, once per pass.
fn copy(destination: &mut [u64], source: &[u64], passes: u64) -> f64 {
    let start = Instant::now();
    for _ in 0..passes {
        destination.copy_from_slice(black_box(source));
        black_box(&destination);
    }
    start.elapsed().as_secs_f64()
}

/// The nanoseconds one step of a random walk over `words` words costs.
///
/// The walk is one cycle through the whole buffer, so that no step can be
/// predicted and every step waits for the one before it: what is measured
/// is the latency of a load, not the bandwidth of many.
///
/// # Errors
///
/// The message naming the buffer the machine has no room for.
fn chase(words: usize, steps: u64) -> Result<f64, String> {
    let buffer = cycle(words)?;
    let mut position = 0usize;
    let start = Instant::now();
    for _ in 0..steps {
        position = buffer.get(position).copied().unwrap_or(0);
    }
    let seconds = start.elapsed().as_secs_f64();
    black_box(position);
    Ok(nanoseconds_per_step(seconds, steps))
}

/// A buffer of `words` words whose entries form one cycle over all of them.
///
/// Sattolo's algorithm, which is Fisher-Yates drawing from below the
/// current place instead of up to it: what it leaves is one cycle rather
/// than any permutation, and it needs no second array to build it.
///
/// # Errors
///
/// The message naming the buffer the machine has no room for.
pub(crate) fn cycle(words: usize) -> Result<Vec<usize>, String> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(words)
        .map_err(|_| format!("no room for a chase over {words} words"))?;
    buffer.extend(0..words);
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut index = buffer.len();
    while index > 1 {
        index = index.saturating_sub(1);
        state = next(state);
        let span = u64::try_from(index).unwrap_or(u64::MAX).max(1);
        let other = usize::try_from(state.checked_rem(span).unwrap_or(0)).unwrap_or(0);
        buffer.swap(index, other);
    }
    Ok(buffer)
}

/// The nanoseconds one of `steps` steps took; zero for no steps.
pub(crate) fn nanoseconds_per_step(seconds: f64, steps: u64) -> f64 {
    if steps == 0 {
        return 0.0;
    }
    seconds / as_f64(steps) * 1e9
}

/// One step of a xorshift generator; a shuffle needs no more than that.
const fn next(state: u64) -> u64 {
    let state = state ^ (state << 13);
    let state = state ^ (state >> 7);
    state ^ (state << 17)
}

/// A count as a float. `as` conversions are denied and `f64::from` takes no
/// `u64`, so the value is split into two halves that do convert.
fn as_f64(value: u64) -> f64 {
    let high = f64::from(u32::try_from(value >> 32).unwrap_or(u32::MAX));
    let low = f64::from(u32::try_from(value & 0xFFFF_FFFF).unwrap_or(u32::MAX));
    high * 4_294_967_296.0 + low
}
