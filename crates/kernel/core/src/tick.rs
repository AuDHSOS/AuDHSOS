// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Conversions of a tick count to seconds, milliseconds and microseconds.
//! The kernel image counts ticks through `interrupts::ticks` of
//! `kernel-hal-x86_64` and schedules in `on_timer_tick` of
//! `crates/kernel/bin/src/main.rs`.

#![expect(
    clippy::as_conversions,
    reason = "the one conversion here widens the tick rate of the layout, and it happens in a constant"
)]

use audhsos_abi::layout::TICKS_PER_SECOND;

/// The rate the kernel runs its timer at, as the divisor of a count.
const RATE: u64 = TICKS_PER_SECOND as u64;

const _: () = assert!(RATE > 0);

/// The number of whole seconds `ticks` ticks are.
#[must_use]
pub const fn seconds(ticks: u64) -> u64 {
    ticks.wrapping_div(RATE)
}

/// The number of milliseconds `ticks` ticks are, for a rate that divides a
/// second evenly.
#[must_use]
pub const fn milliseconds(ticks: u64) -> u64 {
    ticks.saturating_mul(1000).wrapping_div(RATE)
}

/// The number of microseconds `ticks` ticks are, which is the scale
/// `clock_now` answers in.
///
/// The unit is the microsecond and the resolution is the tick: at
/// [`TICKS_PER_SECOND`] the clock moves in steps of a millisecond, and a
/// count that would overflow the microsecond saturates rather than
/// wrapping, so a machine left running does not travel backwards.
#[must_use]
pub const fn micros(ticks: u64) -> u64 {
    ticks.saturating_mul(1_000_000).wrapping_div(RATE)
}
