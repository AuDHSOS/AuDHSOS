// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two calls that ask the machine itself: what time it is, and for
//! entropy.
//!
//! Invariant: neither takes a handle and neither changes an object, so
//! neither can fail for a reason that belongs to the caller; the only
//! error is the machine's.

use audhsos_abi::Error;

use crate::dispatch::{Machine, Reply};
use crate::environment::Environment;

/// `clock_now`: the microseconds since the kernel started.
///
/// The resolution is the timer tick and not the microsecond: the count is
/// derived from the ticks the kernel has taken, and at a thousand ticks a
/// second the clock moves in steps of a millisecond.
///
/// # Errors
///
/// None. The result is a `Result` because every call of the table is.
pub fn clock_now<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
) -> Result<Reply, Error> {
    Ok(Reply::value(machine.environment.now_micros()))
}

/// `random_bytes`: four words out of the entropy source of the machine,
/// which is the thirty-two bytes a stream cipher takes as a seed.
///
/// The words travel in the message area, with their count in the first
/// return word, which is the convention of every result that does not fit
/// into two words.
///
/// # Errors
///
/// [`Error::Unavailable`] when the source would not deliver. Nothing of a
/// partial draw reaches the caller.
pub fn random_bytes<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
) -> Result<Reply, Error> {
    let seed = machine.environment.random_seed()?;
    Ok(Reply::message(&seed))
}
