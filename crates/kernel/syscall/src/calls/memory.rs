// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory calls.
//!
//! Invariants: an operation over a range does at most
//! [`MAX_PAGES_PER_CALL`] pages and reports how far it came, so no call
//! runs for an unbounded time; a mapping is recorded in the region table of
//! the process exactly when it exists in its page tables.

use audhsos_abi::layout::{MAX_PAGES_PER_CALL, PAGE_SIZE};
use audhsos_abi::{Error, Handle, Rights};
use kernel_mm::address_space::{Inserted, Region, user_range};
use kernel_mm::page_table::Permissions;
use kernel_objects::handle_table::Entry;
use kernel_objects::object::{AnyObjectId, MemoryObject, MemoryObjectId, Process, ProcessId};
use kernel_types::{PageRange, PhysFrame, PhysFrameRange};

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The permissions a `memory_map` or `memory_protect` argument names: bit
/// zero writable, bit one executable. A mapping always allows reading, and
/// one a user thread may reach is always a user mapping.
const fn permissions_of(bits: u64) -> Result<Permissions, Error> {
    if bits & !0b11 != 0 {
        return Err(Error::InvalidArgument);
    }
    Ok(Permissions {
        write: bits & 0b1 != 0,
        execute: bits & 0b10 != 0,
        user: true,
    })
}

/// The process a handle names, which is the address space the call works
/// on.
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

/// `memory_map`: maps part of a memory object into the address space of a
/// process. Bounded: at most [`MAX_PAGES_PER_CALL`] pages per call, and a
/// call that stopped early says how many pages it mapped.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a handle the caller does not hold;
/// [`Error::WrongObjectType`] when the second handle names no memory
/// object; [`Error::AccessDenied`] when a permission asked for is not in
/// the rights of that handle; [`Error::Unaligned`] for an offset that is
/// no page; [`Error::InvalidArgument`] for a range outside user space or
/// beyond what the object holds; [`Error::AddressInUse`] when something is
/// mapped there; [`Error::OutOfKernelMemory`] when a page table is missing
/// and the reserve is empty, in which case the pages already mapped are
/// taken back; [`Error::QuotaExceeded`] when the region table is full.
pub fn map<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, caller, request)?;
    let object_handle =
        audhsos_abi::Handle::from_raw(request.argument(1)).ok_or(Error::InvalidHandle)?;
    let (object_id, object_rights) =
        machine
            .objects
            .resolve::<MemoryObject>(caller, object_handle, Rights::MAP)?;
    let address = request.argument(2);
    let offset = request.argument(3);
    let length = request.argument(4);
    let perms = permissions_of(request.argument(5))?;
    if perms.write && !object_rights.contains(Rights::WRITE) {
        return Err(Error::AccessDenied);
    }
    if perms.execute && !object_rights.contains(Rights::EXECUTE) {
        return Err(Error::AccessDenied);
    }
    if !offset.is_multiple_of(PAGE_SIZE) {
        return Err(Error::Unaligned);
    }
    let pages = user_range(address, length)?;
    let object = *machine.objects.memory.get(object_id)?;
    let first_frame = frame_at(object.frames, offset)?;
    let available = object
        .frames
        .count()
        .saturating_sub(offset.wrapping_div(PAGE_SIZE));
    if pages.count() > available {
        return Err(Error::InvalidArgument);
    }

    let holder = machine.objects.processes.get(target)?;
    if holder.regions.find(pages.start()).is_some() {
        return Err(Error::AddressInUse);
    }
    let root = holder.root;
    let budget = pages.count().min(MAX_PAGES_PER_CALL);
    let mut mapped = 0_u64;
    while mapped < budget {
        let Some(page) = pages.start().checked_add(mapped) else {
            break;
        };
        let Some(frame) = first_frame.checked_add(mapped) else {
            break;
        };
        if let Err(error) = machine
            .environment
            .map(root, page, frame, perms, object.cache)
        {
            unwind(machine, root, pages, mapped);
            return Err(error);
        }
        mapped = mapped.saturating_add(1);
    }

    let region = Region {
        pages: PageRange::new(pages.start(), mapped).map_err(|_| Error::InvalidArgument)?,
        backing: object_id,
        offset,
        perms,
    };
    let inserted = machine
        .objects
        .processes
        .get_mut(target)
        .map_err(Error::from)
        .and_then(|holder| holder.regions.insert(region).map_err(Error::from));
    match inserted {
        Err(error) => {
            unwind(machine, root, pages, mapped);
            return Err(error);
        }
        // The object is held by the regions that name it, one reference
        // each, and a call that continued a region made no new one (D-104).
        Ok(Inserted::Merged) => {}
        Ok(Inserted::Added) => {
            machine.objects.memory.retain(object_id)?;
        }
    }
    if mapped < pages.count() {
        return Ok(Reply::partial(mapped));
    }
    Ok(Reply::value(mapped))
}

/// Takes back the pages a failed `memory_map` had already made.
fn unwind<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    root: PhysFrame,
    pages: PageRange,
    count: u64,
) {
    for page in pages.into_iter().take(count_of(count)) {
        let _ = machine.environment.unmap(root, page);
    }
}

/// A count of pages as a number of steps of an iterator. A count that does
/// not fit an index is every page there is, which no range of this system
/// reaches.
fn count_of(count: u64) -> usize {
    usize::try_from(count).unwrap_or(usize::MAX)
}

/// The frame `offset` bytes into `frames`. The offset is a whole number of
/// pages, which the caller has checked.
fn frame_at(frames: PhysFrameRange, offset: u64) -> Result<PhysFrame, Error> {
    let index = offset.wrapping_div(PAGE_SIZE);
    if index >= frames.count() {
        return Err(Error::InvalidArgument);
    }
    frames
        .start()
        .checked_add(index)
        .ok_or(Error::InvalidArgument)
}

/// `memory_unmap`: takes a range out of an address space. Bounded like
/// [`map`].
///
/// # Errors
///
/// [`Error::InvalidHandle`] or [`Error::WrongObjectType`] for the process
/// handle; [`Error::InvalidArgument`] for a range outside user space;
/// [`Error::NotMapped`] when nothing is mapped there.
pub fn unmap<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let target = process_of(machine, caller, request)?;
    let pages = user_range(request.argument(1), request.argument(2))?;
    let holder = machine.objects.processes.get(target)?;
    let root = holder.root;
    if holder.regions.find(pages.start()).is_none() {
        return Err(Error::NotMapped);
    }
    let budget = pages.count().min(MAX_PAGES_PER_CALL);
    let mut done = 0_u64;
    for page in pages.into_iter().take(count_of(budget)) {
        machine.environment.unmap(root, page)?;
        done = done.saturating_add(1);
    }
    let removed = machine
        .objects
        .processes
        .get_mut(target)?
        .regions
        .remove::<2>(PageRange::new(pages.start(), done).map_err(|_| Error::InvalidArgument)?)?;
    // One reference per region, so a removal that only shortened a region
    // gives none back: the region, and what it holds, is still there.
    for piece in removed.taken() {
        if piece.vanished {
            machine.objects.memory.release(piece.region.backing)?;
        }
    }
    if done < pages.count() {
        return Ok(Reply::partial(done));
    }
    Ok(Reply::value(done))
}

/// `memory_protect`: changes what a mapped range allows. Bounded like
/// [`map`].
///
/// # Errors
///
/// [`Error::InvalidHandle`] or [`Error::WrongObjectType`] for the process
/// handle; [`Error::InvalidArgument`] for a range outside user space or
/// permission bits that name nothing; [`Error::NotMapped`] when nothing is
/// mapped there.
pub fn protect<
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
    let pages = user_range(request.argument(1), request.argument(2))?;
    let perms = permissions_of(request.argument(3))?;
    let holder = machine.objects.processes.get(target)?;
    let root = holder.root;
    if holder.regions.find(pages.start()).is_none() {
        return Err(Error::NotMapped);
    }
    let budget = pages.count().min(MAX_PAGES_PER_CALL);
    let mut done = 0_u64;
    for page in pages.into_iter().take(count_of(budget)) {
        machine.environment.protect(root, page, perms)?;
        done = done.saturating_add(1);
    }
    machine.objects.processes.get_mut(target)?.regions.protect(
        PageRange::new(pages.start(), done).map_err(|_| Error::InvalidArgument)?,
        perms,
    )?;
    if done < pages.count() {
        return Ok(Reply::partial(done));
    }
    Ok(Reply::value(done))
}

/// `memory_split`: two objects out of one, at a page-aligned offset. The
/// caller keeps its handle to the first part and receives a handle to the
/// second.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a handle the caller does not hold;
/// [`Error::AccessDenied`] without `MAP`; [`Error::Unaligned`] for an
/// offset that is no page; [`Error::InvalidArgument`] for an offset at
/// either end, which would leave one part empty;
/// [`Error::QuotaExceeded`] or [`Error::PoolExhausted`] when there is no
/// room for the second object, in which case the first is unchanged.
pub fn split<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, rights) = machine
        .objects
        .resolve::<MemoryObject>(process, handle, Rights::MAP)?;
    let offset = request.argument(1);
    if !offset.is_multiple_of(PAGE_SIZE) {
        return Err(Error::Unaligned);
    }
    let object = *machine.objects.memory.get(id)?;
    let index = offset.wrapping_div(PAGE_SIZE);
    if index == 0 || index >= object.frames.count() {
        return Err(Error::InvalidArgument);
    }
    let start = object.frames.start();
    let tail_start = start.checked_add(index).ok_or(Error::InvalidArgument)?;
    let tail = PhysFrameRange::new(tail_start, object.frames.count().saturating_sub(index))
        .map_err(|_| Error::InvalidArgument)?;
    let head = PhysFrameRange::new(start, index).map_err(|_| Error::InvalidArgument)?;

    charge_object(machine, process)?;
    let second =
        match machine
            .objects
            .memory
            .allocate(MemoryObject::new(tail, object.kind, object.cache))
        {
            Ok(id) => id,
            Err(error) => {
                refund_object(machine, process);
                return Err(Error::from(error));
            }
        };
    match install(machine, process, second, rights) {
        Ok(raw) => {
            machine.objects.memory.get_mut(id)?.frames = head;
            Ok(Reply::value(raw))
        }
        Err(error) => {
            let _ = machine.objects.memory.release(second);
            refund_object(machine, process);
            Err(error)
        }
    }
}

/// `memory_merge`: one object out of two that lie side by side in physical
/// memory. The caller keeps its handle to the lower part, which grows to
/// cover both, and gives up its handle to the upper part, which ceases to
/// exist.
///
/// This is the operation that lets memory recover. Without it an object can
/// only ever become smaller, so a server that hands memory out and takes it
/// back grinds its objects down to single pages and can never serve a large
/// request again — which is fragmentation with no floor under it (D-90).
///
/// Both objects must be held by the caller and by nothing else: a mapping
/// is a reference, and an object that something maps may not be dissolved
/// under it.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a handle the caller does not hold, and for
/// a second argument that is no handle; [`Error::AccessDenied`] without
/// `MAP` on either, or when the two carry different rights;
/// [`Error::InvalidArgument`] when the two are the same object, when they
/// do not lie side by side in that order, or when their kind or cache
/// policy differs; [`Error::Busy`] when either is mapped or held under a
/// second handle. Nothing is changed in any of those cases.
pub fn merge<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let lower_handle = request.handle.ok_or(Error::InvalidHandle)?;
    let upper_handle = Handle::from_raw(request.argument(1)).ok_or(Error::InvalidHandle)?;
    let (lower, lower_rights) =
        machine
            .objects
            .resolve::<MemoryObject>(process, lower_handle, Rights::MAP)?;
    let (upper, upper_rights) =
        machine
            .objects
            .resolve::<MemoryObject>(process, upper_handle, Rights::MAP)?;
    if lower == upper {
        return Err(Error::InvalidArgument);
    }
    if lower_rights != upper_rights {
        return Err(Error::AccessDenied);
    }

    let low = *machine.objects.memory.get(lower)?;
    let high = *machine.objects.memory.get(upper)?;
    if low.kind != high.kind || low.cache != high.cache {
        return Err(Error::InvalidArgument);
    }
    let end = low
        .frames
        .start()
        .checked_add(low.frames.count())
        .ok_or(Error::InvalidArgument)?;
    if end != high.frames.start() {
        return Err(Error::InvalidArgument);
    }
    let count = low
        .frames
        .count()
        .checked_add(high.frames.count())
        .ok_or(Error::InvalidArgument)?;
    let joined =
        PhysFrameRange::new(low.frames.start(), count).map_err(|_| Error::InvalidArgument)?;

    // One reference each: the handle in this call and nothing else. A
    // mapping is a reference too, so this is what says neither is mapped.
    if machine.objects.memory.references(lower)? != 1
        || machine.objects.memory.references(upper)? != 1
    {
        return Err(Error::Busy);
    }

    // From here nothing can fail. The upper object leaves through the same
    // path a closed handle takes, so the pool slot and the quota go back
    // exactly once.
    let entry = machine.objects.close_handle(process, upper_handle)?;
    let switch = crate::lifetime::release(machine, entry.object)?;
    machine.objects.memory.get_mut(lower)?.frames = joined;
    refund_object(machine, process);
    let reply = Reply::DONE;
    Ok(if switch { reply.reschedule() } else { reply })
}

/// `memory_info`: where the object lies in physical memory and how large
/// it is.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a handle the caller does not hold;
/// [`Error::AccessDenied`] without `INFO`.
pub fn info<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<Reply, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<MemoryObject>(process, handle, Rights::INFO)?;
    let object = machine.objects.memory.get(id)?;
    let start = object.frames.start().start().as_u64();
    let length = object.frames.count().saturating_mul(PAGE_SIZE);
    Ok(Reply::values(start, length))
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
    machine
        .objects
        .processes
        .get_mut(process)?
        .kernel_object_quota
        .charge(1)?;
    Ok(())
}

/// Gives one kernel object back.
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

/// Installs a handle to `object` in `process`.
fn install<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    object: MemoryObjectId,
    rights: Rights,
) -> Result<u64, Error> {
    let entry = Entry::new(AnyObjectId::of(object), rights);
    let mut list = machine.objects.processes.get(process)?.handles;
    let handle = machine.objects.handles.insert(process, &mut list, entry)?;
    machine.objects.processes.get_mut(process)?.handles = list;
    Ok(handle.raw())
}
