// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tick`.

use audhsos_abi::layout::TICKS_PER_SECOND;

use crate::state::KernelState;
use crate::tick::{milliseconds, on_tick, seconds};

#[test]
fn every_tick_raises_the_count_by_one() {
    let mut state = KernelState::new();
    assert_eq!(state.ticks, 0);
    assert_eq!(on_tick(&mut state), 1);
    assert_eq!(on_tick(&mut state), 2);
    assert_eq!(state.ticks, 2);
}

#[test]
fn a_count_that_would_wrap_stays_where_it_is() {
    let mut state = KernelState {
        ticks: u64::MAX,
        ..KernelState::new()
    };
    assert_eq!(on_tick(&mut state), u64::MAX);
}

#[test]
fn ticks_become_whole_seconds_and_milliseconds() {
    let rate = u64::from(TICKS_PER_SECOND);
    assert_eq!(seconds(0), 0);
    assert_eq!(seconds(rate - 1), 0);
    assert_eq!(seconds(rate), 1);
    assert_eq!(seconds(3 * rate + 7), 3);
    assert_eq!(milliseconds(rate), 1000);
    assert_eq!(milliseconds(0), 0);
}

#[test]
fn the_other_counters_of_the_state_are_kept_apart() {
    let mut state = KernelState::new();
    state.record_trap();
    state.record_interrupt();
    state.record_interrupt();
    state.record_spurious();
    on_tick(&mut state);
    assert_eq!(state.traps, 1);
    assert_eq!(state.interrupts, 2);
    assert_eq!(state.spurious, 1);
    assert_eq!(state.ticks, 1);
}
