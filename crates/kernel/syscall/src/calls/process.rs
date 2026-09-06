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
use kernel_objects::object::{AnyObjectId, Process, ProcessId};
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
    let mut list = machine.objects.processes.get(caller)?.handles;
    match machine.objects.handles.insert(caller, &mut list, entry) {
        Ok(handle) => {
            machine.objects.processes.get_mut(caller)?.handles = list;
            Ok(Reply::value(handle.raw()))
        }
        Err(error) => {
            let _ = machine.objects.processes.release(id);
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
    let mut list = machine.objects.processes.get(target)?.handles;
    let handle = machine
        .objects
        .handles
        .insert(target, &mut list, installed)?;
    machine.objects.processes.get_mut(target)?.handles = list;
    Ok(Reply::value(handle.raw()))
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
    // caller is standing on, which may be one of these.
    for id in holder.threads() {
        if let Ok(outcome) = machine.scheduler.exit(&mut machine.objects.threads, id) {
            reschedule |= outcome.reschedule;
        }
    }
    let mut list = holder.handles;
    while machine.objects.handles.close_next(&mut list).is_some() {}
    machine.environment.destroy_address_space(holder.root);
    machine.objects.with_process(target, |entry| {
        entry.handles = list;
        for id in holder.threads() {
            entry.remove_thread(id);
        }
    });
    let _ = machine.objects.processes.release(target);
    let reply = Reply::DONE;
    Ok(if reschedule {
        reply.reschedule()
    } else {
        reply
    })
}
