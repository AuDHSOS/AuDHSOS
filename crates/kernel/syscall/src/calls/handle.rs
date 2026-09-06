// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handle calls.
//!
//! Invariant: a handle operation touches the list of the calling process
//! and nothing else; a refused duplicate leaves the original as it was.

use audhsos_abi::{Error, Rights};
use kernel_objects::object::ProcessId;

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// `handle_duplicate`: a second handle of the caller to the same object,
/// with the rights the call asks for, which may not be more than the
/// original carries.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the caller holds no such handle;
/// [`Error::InvalidArgument`] for bits that name no right;
/// [`Error::AccessDenied`] when the original lacks `DUPLICATE` or does not
/// carry every right asked for; [`Error::QuotaExceeded`] or
/// [`Error::OutOfHandles`] when there is no slot for the copy.
pub fn duplicate<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let bits = u32::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let rights = Rights::from_bits(bits)?;
    let object = machine.objects.entry(process, handle)?.object;
    let mut list = machine.objects.processes.get(process)?.handles;
    let copy = machine
        .objects
        .handles
        .duplicate(process, &mut list, handle, rights)?;
    machine
        .objects
        .processes
        .with(process, |held| held.handles = list);
    // The second handle holds a second reference.
    machine.objects.retain(object)?;
    Ok(Reply::value(copy.raw()))
}

/// `handle_close`: the caller gives up a handle, and with it the reference
/// the handle held. What it named lives on until its last handle is gone;
/// closing that one destroys it and wakes everyone who waited on it.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the caller holds no such handle, a second
/// close of the same one included.
pub fn close<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let entry = machine.objects.close_handle(process, handle)?;
    let switch = crate::lifetime::release(machine, entry.object)?;
    let reply = Reply::DONE;
    Ok(if switch { reply.reschedule() } else { reply })
}
