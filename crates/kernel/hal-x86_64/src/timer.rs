// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The local APIC timer, measured once against the interval timer the
//! machine has had since the first one.
//!
//! The local APIC counts a bus clock whose rate the specification does not
//! name, so the kernel measures it: it lets the APIC timer run for a known
//! interval that the interval timer produces, and divides. The interval
//! timer is used for that one measurement and never again.
//!
//! Invariants: every poll of the interval timer is bounded, so a machine
//! whose channel two does not run makes the calibration report and not
//! hang; the tick counter only ever grows.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use kernel_hal_api::timer::TimerError;
use kernel_x86_tables::lapic;

use crate::apic::LocalApic;
use crate::instructions::{read_port_u8, write_port_u8};

/// The port that carries the gate of channel two and its output bit.
pub const PIT_GATE: u16 = 0x61;

/// The counter port of channel two.
pub const PIT_CHANNEL2: u16 = 0x42;

/// The command port of the interval timer.
pub const PIT_COMMAND: u16 = 0x43;

/// Channel two, low byte then high byte, one-shot, binary counting.
const PIT_ONE_SHOT: u8 = 0xB2;

/// The bit of [`PIT_GATE`] that lets channel two count.
const GATE_ENABLE: u8 = 0b0000_0001;

/// The bit of [`PIT_GATE`] that connects channel two to the speaker, which
/// the kernel never wants.
const SPEAKER_ENABLE: u8 = 0b0000_0010;

/// The bit of [`PIT_GATE`] that says channel two has counted down.
const GATE_OUTPUT: u8 = 0b0010_0000;

/// The number of milliseconds the calibration measures over.
pub const CALIBRATION_MS: u32 = 10;

/// The count that makes channel two run for [`CALIBRATION_MS`]
/// milliseconds at its rate of `1_193_182` hertz.
pub const CALIBRATION_COUNT: u16 = 11_932;

/// How many reads of the gate port the calibration spends before it gives
/// up. Ten milliseconds are far fewer reads than this even on a machine
/// that emulates every one of them.
const POLL_LIMIT: u32 = 10_000_000;

/// Why the local APIC timer could not be measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalibrationError {
    /// Channel two of the interval timer did not report that it had
    /// counted down.
    IntervalTimerSilent,
    /// The local APIC timer did not count at all, so its rate is unknown.
    NoBusClock,
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CalibrationError::IntervalTimerSilent => {
                f.write_str("channel two of the interval timer never finished")
            }
            CalibrationError::NoBusClock => f.write_str("the local apic timer did not count"),
        }
    }
}

/// The ticks the timer has delivered since the machine started.
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Counts one tick. The device dispatch of the kernel calls this from the
/// handler of the timer vector.
pub fn record_tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// The number of ticks the timer has delivered.
#[must_use]
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Measures how many times the local APIC timer counts down in one
/// millisecond, with the divisor the kernel uses afterwards.
///
/// The timer is left masked and stopped.
///
/// # Errors
///
/// [`CalibrationError`] for a machine whose two timers do not answer.
///
/// # Safety
///
/// The interval timer must belong to the kernel, which it does until a
/// driver in userland asks for it, and `local` must be the local APIC of
/// the processor this runs on.
pub unsafe fn calibrate(local: &mut LocalApic) -> Result<u32, CalibrationError> {
    local.set_timer(0, false, true);
    local.set_timer_divide(lapic::DIVIDE_BY_16);
    // SAFETY: the caller promises that the interval timer belongs to the
    // kernel; the sequence arms channel two without connecting it to the
    // speaker and starts it on the rising edge of its gate.
    let gate = unsafe { arm_interval_timer() };
    local.set_timer_count(u32::MAX);
    // SAFETY: the same port, read only.
    let finished = unsafe { wait_for_interval_timer() };
    let remaining = local.timer_count();
    local.set_timer_count(0);
    // SAFETY: the same port; the gate goes back to where it was.
    unsafe {
        write_port_u8(PIT_GATE, gate);
    }
    if !finished {
        return Err(CalibrationError::IntervalTimerSilent);
    }
    let elapsed = u32::MAX.wrapping_sub(remaining);
    if elapsed == 0 {
        return Err(CalibrationError::NoBusClock);
    }
    Ok(elapsed.wrapping_div(CALIBRATION_MS))
}

/// Programs channel two for one interval and starts it. Returns the gate
/// byte as it was before, so that the caller can put it back.
///
/// # Safety
///
/// The interval timer must belong to the kernel.
unsafe fn arm_interval_timer() -> u8 {
    // SAFETY: the caller promises that the ports belong to the kernel.
    let gate = unsafe { read_port_u8(PIT_GATE) };
    let quiet = (gate & !SPEAKER_ENABLE) & !GATE_ENABLE;
    // SAFETY: the gate goes low, which is the edge the one-shot needs to
    // start on when it goes high again.
    unsafe {
        write_port_u8(PIT_GATE, quiet);
    }
    // SAFETY: the command names channel two, both count bytes, one-shot.
    unsafe {
        write_port_u8(PIT_COMMAND, PIT_ONE_SHOT);
    }
    let [low, high] = CALIBRATION_COUNT.to_le_bytes();
    // SAFETY: the low byte of the count, which the channel expects first.
    unsafe {
        write_port_u8(PIT_CHANNEL2, low);
    }
    // SAFETY: the high byte of the count.
    unsafe {
        write_port_u8(PIT_CHANNEL2, high);
    }
    // SAFETY: the gate goes high, which starts the count.
    unsafe {
        write_port_u8(PIT_GATE, quiet | GATE_ENABLE);
    }
    gate
}

/// Reads the gate port until channel two reports that it has counted down,
/// at most [`POLL_LIMIT`] times. `false` if it never did.
///
/// # Safety
///
/// The interval timer must belong to the kernel.
unsafe fn wait_for_interval_timer() -> bool {
    let mut polls = 0u32;
    while polls < POLL_LIMIT {
        // SAFETY: the caller promises that the port belongs to the kernel;
        // a read of it changes nothing.
        if unsafe { read_port_u8(PIT_GATE) } & GATE_OUTPUT != 0 {
            return true;
        }
        polls = polls.saturating_add(1);
    }
    false
}

/// The count the timer is loaded with to deliver `ticks_per_second` ticks
/// per second, given that it counts `ticks_per_ms` times per millisecond.
///
/// # Errors
///
/// [`TimerError::UnsupportedFrequency`] for a rate of zero, for a rate
/// faster than the timer can count, or for one whose count does not fit
/// the register.
pub fn initial_count(ticks_per_ms: u32, ticks_per_second: u32) -> Result<u32, TimerError> {
    match lapic::count_for_rate(ticks_per_ms, ticks_per_second) {
        Some(count) => Ok(count),
        None => Err(TimerError::UnsupportedFrequency(ticks_per_second)),
    }
}
