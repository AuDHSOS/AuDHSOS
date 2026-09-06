// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The process calls.
//!
//! Invariants: a process the kernel hands out has an address space that
//! carries the kernel half and maps nothing else; killing a process ends
//! its threads and gives back everything they held, so nothing of it
//! survives its last handle.

use audhsos_abi::{Error, Handle, Rights};
use kernel_objects::config::HANDLES_PER_PROCESS;
use kernel_objects::handle_table::{Entry, HandleList};
use kernel_objects::object::{AnyObjectId, Endpoint, Process, ProcessId};
use kernel_objects::quota::Quota;

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The process a handle names.
fn process_of<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<ProcessId, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<Process>(caller, handle, Rights::EMPTY)?;
    Ok(id)
}

/// `process_create`: a process with an address space of its own, a handle
/// capacity, and quotas, all of them within what the creator holds.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a handle capacity above what a process may
/// hold or a reserved argument that is not zero; [`Error::QuotaExceeded`]
/// when the creator cannot give away what the call asks for;
/// [`Error::OutOfKernelMemory`] when the reserve has no frame for the
/// address space; [`Error::PoolExhausted`] when no process slot is left;
/// [`Error::OutOfHandles`] when the creator has no slot for the handle. In
/// every case the creator keeps what it had.
pub fn create<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let parent = process_of(machine, caller, request)?;
    let capacity = u32::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let frames = u32::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    let objects = u32::try_from(request.argument(3)).map_err(|_| Error::InvalidArgument)?;
    if request.argument(4) != 0 {
        return Err(Error::InvalidArgument);
    }
    if usize::try_from(capacity).unwrap_or(usize::MAX) > HANDLES_PER_PROCESS {
        return Err(Error::InvalidArgument);
    }

    // What the creator gives away, it no longer has.
    {
        let holder = machine.objects.processes.get_mut(parent)?;
        holder.kernel_object_quota.charge(1)?;
        if let Err(error) = holder.quota.charge(frames) {
            holder.kernel_object_quota.refund(1);
            return Err(Error::from(error));
        }
        if let Err(error) = holder.kernel_object_quota.charge(objects) {
            holder.quota.refund(frames);
            holder.kernel_object_quota.refund(1);
            return Err(Error::from(error));
        }
    }

    let root = match machine.environment.create_address_space() {
        Ok(root) => root,
        Err(error) => {
            give_back(machine, parent, frames, objects);
            return Err(error);
        }
    };
    let process = Process::new(
        root,
        HandleList::with_capacity(capacity),
        Quota::new(frames),
        Quota::new(objects),
    );
    let id = match machine.objects.processes.allocate(process) {
        Ok(id) => id,
        Err(error) => {
            machine.environment.destroy_address_space(root);
            give_back(machine, parent, frames, objects);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(
        AnyObjectId::of(id),
        Rights::MANAGE | Rights::MAP | Rights::INSTALL | Rights::DUPLICATE | Rights::TRANSFER,
    );
    match machine.objects.install_handle(caller, entry) {
        Ok(handle) => {
            // The handle is a reference of its own, beside the one the
            // process holds to itself while it lives.
            machine.objects.retain(AnyObjectId::of(id))?;
            Ok(Reply::value(handle.raw()))
        }
        Err(error) => {
            machine.objects.processes.force_release(id);
            machine.environment.destroy_address_space(root);
            give_back(machine, parent, frames, objects);
            Err(error)
        }
    }
}

/// Returns what a refused `process_create` had already taken.
fn give_back<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    parent: ProcessId,
    frames: u32,
    objects: u32,
) {
    machine.objects.with_process(parent, |holder| {
        holder.quota.refund(frames);
        holder.kernel_object_quota.refund(objects);
        holder.kernel_object_quota.refund(1);
    });
}

/// `process_install_handle`: copies a handle of the caller into another
/// process, with the rights the call names, and returns the number the
/// target sees.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for either handle; [`Error::InvalidArgument`]
/// for bits that name no right; [`Error::AccessDenied`] when the handle
/// may not be transferred or does not carry every right asked for;
/// [`Error::QuotaExceeded`] or [`Error::OutOfHandles`] when the target has
/// no slot left.
pub fn install_handle<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, caller, request)?;
    let source = Handle::from_raw(request.argument(1)).ok_or(Error::InvalidHandle)?;
    let bits = u32::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    let rights = Rights::from_bits(bits)?;
    let entry = machine.objects.entry(caller, source)?;
    if !entry.rights.contains(Rights::TRANSFER) {
        return Err(Error::AccessDenied);
    }
    if !rights.is_subset_of(entry.rights) {
        return Err(Error::AccessDenied);
    }
    let installed = Entry {
        object: entry.object,
        rights,
        badge: entry.badge,
    };
    let handle = machine.objects.install_handle(target, installed)?;
    // The handle the target now holds is a second reference.
    machine.objects.retain(entry.object)?;
    Ok(Reply::value(handle.raw()))
}

/// `process_set_fault_handler`: the endpoint the faults of a process are
/// reported on. A second argument of zero clears it.
///
/// The endpoint it retains, and the one it replaces it releases, so a
/// handler that is set and then replaced does not keep the old endpoint
/// alive.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for either handle; [`Error::WrongObjectType`]
/// when the second names no endpoint; [`Error::AccessDenied`] when it does
/// not carry `SEND`, which is what the kernel needs to send a fault message
/// through it.
pub fn set_fault_handler<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, caller, request)?;
    let wanted = request.argument(1);
    let handler = if wanted == 0 {
        None
    } else {
        let handle = Handle::from_raw(wanted).ok_or(Error::InvalidHandle)?;
        let (id, _rights) = machine
            .objects
            .resolve::<Endpoint>(caller, handle, Rights::SEND)?;
        machine.objects.retain(AnyObjectId::of(id))?;
        Some(id)
    };
    let previous = machine.objects.processes.get(target)?.fault_handler;
    machine
        .objects
        .processes
        .with(target, |holder| holder.fault_handler = handler);
    let switch = match previous {
        Some(old) => crate::lifetime::release(machine, AnyObjectId::of(old))?,
        None => false,
    };
    let reply = Reply::DONE;
    Ok(if switch { reply.reschedule() } else { reply })
}

/// `process_kill`: ends every thread of the process, closes every handle it
/// holds, and takes its address space apart.
///
/// # Errors
///
/// [`Error::InvalidHandle`] or [`Error::WrongObjectType`] for the handle.
pub fn kill<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, caller, request)?;
    let holder = *machine.objects.processes.get(target)?;
    let mut reschedule = false;
    // The threads end here; what they held is cleared away by
    // `crate::reaper::reap` once the kernel has switched off the stack the
    // caller is standing on, which may be one of these. A thread that waited
    // on something leaves its queue first, or the endpoint would go on
    // naming a thread that is no longer in it.
    for id in holder.threads() {
        kernel_ipc::cancel(machine.objects, id);
        if let Ok(outcome) = machine.scheduler.exit(&mut machine.objects.threads, id) {
            reschedule |= outcome.reschedule;
        }
    }
    // Every handle it held was a reference; each of them goes through the
    // release path, so an endpoint whose last handle this was is destroyed
    // and its waiters are woken.
    reschedule |= crate::lifetime::close_every_handle(machine, target);
    machine.environment.destroy_address_space(holder.root);
    machine.objects.with_process(target, |entry| {
        for id in holder.threads() {
            entry.remove_thread(id);
        }
    });
    if let Some(handler) = holder.fault_handler {
        reschedule |= crate::lifetime::release(machine, AnyObjectId::of(handler))?;
    }
    machine.objects.processes.force_release(target);
    let reply = Reply::DONE;
    Ok(if reschedule {
        reply.reschedule()
    } else {
        reply
    })
}
