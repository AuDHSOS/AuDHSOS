// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The periodic timer.

use core::fmt;

/// Errors of the timer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerError {
    /// The hardware cannot produce the requested tick rate.
    UnsupportedFrequency(u32),
}

impl fmt::Display for TimerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimerError::UnsupportedFrequency(hz) => {
                write!(f, "{hz} ticks per second are not supported")
            }
        }
    }
}

/// A periodic tick source.
pub trait Timer {
    /// Starts ticking `ticks_per_second` times per second.
    ///
    /// # Errors
    ///
    /// Fails if the hardware cannot produce the rate.
    fn start_periodic(&mut self, ticks_per_second: u32) -> Result<(), TimerError>;

    /// The number of ticks since the timer started.
    fn ticks(&self) -> u64;
}
