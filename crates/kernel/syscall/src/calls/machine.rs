// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The three calls that ask the machine itself: how long it has been
//! running, what the date is, and for entropy.
//!
//! Invariant: none of the three takes a handle and none changes an object,
//! so none can fail for a reason that belongs to the caller; the only error
//! is the machine's.

use audhsos_abi::Error;

use crate::dispatch::{Machine, Reply};
use crate::environment::Environment;

/// Microseconds in one second, for the wall clock arithmetic.
const MICROS_PER_SECOND: u64 = 1_000_000;

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

/// `clock_wall`: the microseconds since 1970-01-01T00:00:00Z, and the code
/// of the [`WallClockSource`](audhsos_abi::WallClockSource) the machine
/// reported.
///
/// The moment is the boot moment the loader read from the firmware plus
/// what [`clock_now`] answers, so the resolution is the timer tick and the
/// drift is that of the same count: the timer is calibrated once at boot
/// and nothing corrects it afterwards. For a certificate window, which is
/// days wide, that is enough; for anything that needs the second to be
/// right after a long uptime, it is not, and this documents it rather than
/// implying a precision the machine does not have.
///
/// The source travels beside the value because it says how far it can be
/// trusted: a firmware that named no offset from universal time was read
/// as though it had named zero.
///
/// # Errors
///
/// [`Error::Unavailable`] on a machine whose firmware reported no clock,
/// or whose clock read a moment this system refused. Nothing is guessed:
/// a caller that cannot be told the date is told that, so that a
/// certificate is never judged against a made-up moment.
pub fn clock_wall<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
) -> Result<Reply, Error> {
    let (seconds, source) = machine.environment.boot_wall().ok_or(Error::Unavailable)?;
    let micros = u64::try_from(seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(MICROS_PER_SECOND))
        .and_then(|boot| boot.checked_add(machine.environment.now_micros()))
        .ok_or(Error::Unavailable)?;
    Ok(Reply::values(micros, u64::from(source.code())))
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
