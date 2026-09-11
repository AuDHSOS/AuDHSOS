// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the kernel does with a timer tick.
//!
//! Invariant: a tick is counted before anything else happens, so that the
//! count the kernel reports is the number of ticks the hardware delivered
//! and not the number of ticks the scheduler agreed to.

#![expect(
    clippy::as_conversions,
    reason = "the one conversion here widens the tick rate of the layout, and it happens in a constant"
)]

use audhsos_abi::layout::TICKS_PER_SECOND;

use crate::state::KernelState;

/// Counts one tick and gives the scheduler its turn. Until there are
/// threads to switch between, that turn is only the count the kernel now
/// stands at.
pub const fn on_tick(state: &mut KernelState) -> u64 {
    state.record_tick();
    schedule(state)
}

/// What the kernel does once the tick is counted. Phase 5 puts the
/// scheduler here; until then a tick changes nothing but the count.
const fn schedule(state: &KernelState) -> u64 {
    state.ticks
}

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
