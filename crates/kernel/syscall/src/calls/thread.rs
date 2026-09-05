// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The thread calls.
//!
//! Invariant: a thread the kernel hands out is complete before its handle
//! is; a `thread_create` that fails leaves no kernel stack, no IPC buffer,
//! and no pool slot behind.

use audhsos_abi::layout::{PRIORITY_COUNT, USER_SPACE_START, ipc_buffer_address};
use audhsos_abi::{Error, Rights, ThreadState};
use kernel_mm::page_table::Permissions;
use kernel_objects::handle_table::Entry;
use kernel_objects::object::{AnyObjectId, Process, ProcessId, Thread, ThreadId};
use kernel_types::{CachePolicy, Page, PhysFrame, VirtAddr};

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The thread a handle names, checked against the process that holds it.
fn thread_of<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<ThreadId, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<Thread>(process, handle, Rights::EMPTY)?;
    Ok(id)
}

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

/// `thread_create`: a thread of the process the handle names, with its own
/// kernel stack and IPC buffer out of the reserve. The new thread may not
/// be allowed more than the thread that creates it is (2.5.3).
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a priority above the maximum, a maximum
/// above what the calling thread may hand out, an entry or a stack outside
/// user space, or a reserved argument that is not zero;
/// [`Error::QuotaExceeded`] when the process may hold no further object;
/// [`Error::OutOfKernelMemory`] when the reserve has no kernel stack or no
/// frame left; [`Error::PoolExhausted`] when no thread slot is left;
/// [`Error::OutOfHandles`] when the caller has no slot for the handle. In
/// every case nothing of the thread is left behind.
pub fn create<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, process, request)?;
    let entry = user_address(request.argument(1))?;
    let stack = user_address(request.argument(2))?;
    let priority = u8::try_from(request.argument(3)).map_err(|_| Error::InvalidArgument)?;
    let max_priority = u8::try_from(request.argument(4)).map_err(|_| Error::InvalidArgument)?;
    if request.argument(5) != 0 {
        return Err(Error::InvalidArgument);
    }
    if max_priority >= PRIORITY_COUNT || priority > max_priority {
        return Err(Error::InvalidArgument);
    }
    let ceiling = machine.objects.threads.get(caller)?.max_priority;
    if max_priority > ceiling {
        return Err(Error::InvalidArgument);
    }

    // Everything that can fail, and every step undone in the order it was
    // taken, so that a refused call leaves the machine as it found it.
    charge_object(machine, target)?;
    let stack_area = match machine.environment.allocate_kernel_stack() {
        Ok(area) => area,
        Err(error) => {
            refund_object(machine, target);
            return Err(error);
        }
    };
    let ipc_buffer = match machine.environment.allocate_frame() {
        Ok(frame) => frame,
        Err(error) => {
            machine.environment.release_kernel_stack(stack_area.slot);
            refund_object(machine, target);
            return Err(error);
        }
    };
    let thread = Thread::new(target, priority, max_priority, stack_area.slot, ipc_buffer)?;
    let id = match machine.objects.threads.allocate(thread) {
        Ok(id) => id,
        Err(error) => {
            machine.environment.release_frame(ipc_buffer);
            machine.environment.release_kernel_stack(stack_area.slot);
            refund_object(machine, target);
            return Err(Error::from(error));
        }
    };
    let recorded = machine
        .objects
        .processes
        .get_mut(target)
        .map_err(Error::from)
        .and_then(|holder| holder.add_thread(id));
    let slot = match recorded {
        Ok(slot) => slot,
        Err(error) => {
            let _ = machine.objects.threads.release(id);
            machine.environment.release_frame(ipc_buffer);
            machine.environment.release_kernel_stack(stack_area.slot);
            refund_object(machine, target);
            return Err(error);
        }
    };

    // The buffer is the one page of the address space the thread does not
    // ask for: the kernel maps it and hands the thread its address.
    let placed = map_buffer(machine, target, slot, ipc_buffer);
    let ipc_address = match placed {
        Ok(address) => address,
        Err(error) => {
            forget_thread(machine, target, id);
            machine.environment.release_frame(ipc_buffer);
            machine.environment.release_kernel_stack(stack_area.slot);
            refund_object(machine, target);
            return Err(error);
        }
    };
    machine.objects.with_thread(id, |created| {
        *created = created.starting_at(entry, stack, ipc_address);
    });

    match install(machine, process, id) {
        Ok(handle) => Ok(Reply::value(handle)),
        Err(error) => {
            unmap_buffer(machine, target, ipc_address);
            forget_thread(machine, target, id);
            machine.environment.release_frame(ipc_buffer);
            machine.environment.release_kernel_stack(stack_area.slot);
            refund_object(machine, target);
            Err(error)
        }
    }
}

/// Maps the IPC buffer of the thread in `slot` into the address space of
/// `process` and returns where it landed.
fn map_buffer<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    slot: usize,
    frame: PhysFrame,
) -> Result<VirtAddr, Error> {
    let page = ipc_page(slot).ok_or(Error::QuotaExceeded)?;
    let address = page.start();
    let root = machine.objects.processes.get(process)?.root;
    machine.environment.map(
        root,
        page,
        frame,
        Permissions::READ_WRITE.for_user(),
        CachePolicy::WriteBack,
    )?;
    Ok(address)
}

/// The page the IPC buffer of the thread in `slot` is mapped at, or `None`
/// for a slot no process has. The address is a page of the user half by
/// construction, so the three steps have one answer between them.
pub(crate) fn ipc_page(slot: usize) -> Option<Page> {
    let raw = ipc_buffer_address(slot)?;
    let address = VirtAddr::new(raw).ok()?;
    Page::from_start(address).ok()
}

/// Takes the IPC buffer of a thread out of the address space again.
fn unmap_buffer<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    address: VirtAddr,
) {
    let mapped = machine
        .objects
        .processes
        .get(process)
        .ok()
        .map(|holder| holder.root)
        .zip(Page::from_start(address).ok());
    if let Some((root, page)) = mapped {
        let _ = machine.environment.unmap(root, page);
    }
}

/// Takes a thread out of its process and out of the pool.
fn forget_thread<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    id: ThreadId,
) {
    machine.objects.with_process(process, |holder| {
        holder.remove_thread(id);
    });
    let _ = machine.objects.threads.release(id);
}

/// An address a user thread may run at or stand on.
fn user_address(value: u64) -> Result<VirtAddr, Error> {
    let address = VirtAddr::new(value).map_err(|_| Error::InvalidArgument)?;
    if !address.is_user() || value < USER_SPACE_START {
        return Err(Error::InvalidArgument);
    }
    Ok(address)
}

/// Charges one kernel object against the quota of `process`.
fn charge_object<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<(), Error> {
    let holder = machine.objects.processes.get_mut(process)?;
    holder.kernel_object_quota.charge(1)?;
    Ok(())
}

/// Gives one kernel object back to the quota of `process`.
fn refund_object<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) {
    machine.objects.with_process(process, |holder| {
        holder.kernel_object_quota.refund(1);
    });
}

/// Installs a handle to `thread` in `process` and returns its raw value.
fn install<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    thread: ThreadId,
) -> Result<u64, Error> {
    let entry = Entry::new(
        AnyObjectId::of(thread),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let mut list = machine.objects.processes.get(process)?.handles;
    let handle = machine.objects.handles.insert(process, &mut list, entry)?;
    machine.objects.processes.get_mut(process)?.handles = list;
    Ok(handle.raw())
}

/// `thread_start`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] or [`Error::WrongObjectType`] for the handle;
/// [`Error::InvalidState`] for a thread that was started already.
pub fn start<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = thread_of(machine, process, request)?;
    let outcome = machine.scheduler.start(&mut machine.objects.threads, id)?;
    Ok(Reply::DONE.after(outcome))
}

/// `thread_suspend`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for the handle; [`Error::InvalidState`] for a
/// thread that is suspended, faulted, or gone.
pub fn suspend<
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
    let id = thread_of(machine, process, request)?;
    let outcome = machine
        .scheduler
        .suspend(&mut machine.objects.threads, id)?;
    Ok(Reply::DONE.after(outcome))
}

/// `thread_resume`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for the handle; [`Error::InvalidState`] for a
/// thread that is neither suspended nor faulted.
pub fn resume<
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
    let id = thread_of(machine, process, request)?;
    let outcome = machine.scheduler.resume(&mut machine.objects.threads, id)?;
    Ok(Reply::DONE.after(outcome))
}

/// `thread_kill`: the thread ends wherever it was, and what it holds goes
/// back to the reserve.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for the handle.
pub fn kill<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = thread_of(machine, process, request)?;
    let outcome = end(machine, id)?;
    Ok(Reply::DONE.after(outcome))
}

/// `thread_exit`: the caller ends itself.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the machine does not hold the caller.
pub fn exit<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
) -> Result<Reply, Error> {
    let outcome = end(machine, caller)?;
    Ok(Reply::DONE.after(outcome))
}

/// Ends `id`. What the thread held — its kernel stack, its IPC buffer, and
/// its slot — stays until [`crate::reaper::reap`] clears it away, because a
/// thread that ends itself is still standing on that kernel stack while the
/// kernel writes its answer.
fn end<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    id: ThreadId,
) -> Result<kernel_sched::Outcome, Error> {
    let outcome = machine.scheduler.exit(&mut machine.objects.threads, id)?;
    let process = machine.objects.threads.get(id)?.process;
    machine.objects.with_process(process, |holder| {
        holder.remove_thread(id);
        holder.kernel_object_quota.refund(1);
    });
    Ok(outcome)
}

/// `thread_set_priority`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for the handle; [`Error::InvalidArgument`] for a
/// priority above the maximum of the thread or outside the priorities of
/// this system.
pub fn set_priority<
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
    let id = thread_of(machine, process, request)?;
    let priority = u8::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let outcome = machine
        .scheduler
        .set_priority(&mut machine.objects.threads, id, priority)?;
    Ok(Reply::DONE.after(outcome))
}

/// `thread_info`: the state, and the fault it stopped on if it did.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for the handle.
pub fn info<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let id = thread_of(machine, process, request)?;
    let thread = machine.objects.threads.get(id)?;
    let state = u64::from(thread.state.code());
    let faulted = u64::from(thread.state == ThreadState::Faulted);
    Ok(Reply::values(state, faulted))
}

/// `thread_yield`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] when the machine does not hold the caller;
/// [`Error::InvalidState`] when the caller is not running.
pub fn yield_now<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
) -> Result<Reply, Error> {
    let outcome = machine
        .scheduler
        .yield_now(&mut machine.objects.threads, caller)?;
    Ok(Reply::DONE.after(outcome))
}
