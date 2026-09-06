// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The calls that need the root authority: interrupts, I/O port ranges,
//! device memory, and the information about the machine.
//!
//! Invariants: at most one interrupt object names a line, and at most one
//! port range covers a port, so a capability to either is exclusive; a
//! device memory object covers no frame the machine reported as usable,
//! because those belong to the memory server; a port access outside the
//! range of the capability it was made through never reaches the hardware.

use audhsos_abi::layout::TICKS_PER_SECOND;
use audhsos_abi::{Error, Rights};
use kernel_ipc::interrupt;
use kernel_objects::handle_table::Entry;
use kernel_objects::object::{
    AnyObjectId, Interrupt, IoPortRange, MemoryKind, MemoryObject, Notification, ProcessId,
};
use kernel_types::{CachePolicy, PhysFrame, PhysFrameRange};

use crate::dispatch::{Machine, Reply, Request};
use crate::environment::Environment;

/// The widths a port access may have, in bytes.
const WIDTHS: [u8; 3] = [1, 2, 4];

/// `interrupt_create`: an interrupt object for an ISA line.
///
/// The call takes no vector: the vector of a line is fixed by the plan of
/// the machine, so the kernel derives it and refuses a line the plan
/// reserves none for.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a line above what a byte holds and for one
/// the plan reserves no vector for; [`Error::AlreadyExists`] for a line an
/// interrupt object already names; [`Error::QuotaExceeded`],
/// [`Error::PoolExhausted`], or [`Error::OutOfHandles`] as the other
/// creating calls.
pub fn interrupt_create<
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
    let line = u8::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let vector = machine
        .environment
        .interrupt_vector(line)
        .ok_or(Error::InvalidArgument)?;
    if interrupt::interrupt_of_line(machine.objects, line).is_some() {
        return Err(Error::AlreadyExists);
    }
    super::charge_object(machine, process)?;
    if let Err(error) = machine.environment.route_interrupt(line, vector) {
        super::refund_object(machine, process);
        return Err(error);
    }
    let id = match machine
        .objects
        .interrupts
        .allocate(Interrupt::new(line, vector))
    {
        Ok(id) => id,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(
        AnyObjectId::of(id),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw())),
        Err(error) => {
            let _ = machine.objects.interrupts.release(id);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `interrupt_bind`: the interrupt signals one bit of a notification.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for either handle; [`Error::AccessDenied`] when
/// the notification handle lacks `BIND`; [`Error::InvalidArgument`] for a
/// bit index above sixty-three; [`Error::AlreadyExists`] when the
/// notification is bound to another interrupt.
pub fn interrupt_bind<
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
    let (id, _rights) = machine
        .objects
        .resolve::<Interrupt>(process, handle, Rights::MANAGE)?;
    let second = audhsos_abi::Handle::from_raw(request.argument(1)).ok_or(Error::InvalidHandle)?;
    let (notification, _rights) =
        machine
            .objects
            .resolve::<Notification>(process, second, Rights::BIND)?;
    let bit = u8::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    interrupt::bind(machine.objects, id, notification, bit)?;
    Ok(Reply::DONE)
}

/// `interrupt_ack`: the line is unmasked and the next interrupt may arrive.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for an interrupt object that is gone.
pub fn interrupt_ack<
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
    let (id, _rights) = machine
        .objects
        .resolve::<Interrupt>(process, handle, Rights::MANAGE)?;
    let line = interrupt::acknowledge(machine.objects, id)?;
    machine.environment.unmask_interrupt(line);
    Ok(Reply::DONE)
}

/// `ioport_create`: the permission to touch a range of ports.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a first port or a count above what a word
/// of sixteen bits holds, for a count of zero, and for a range that runs
/// past the last port; [`Error::AlreadyExists`] for a range that overlaps
/// one that exists.
pub fn ioport_create<
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
    let first = u16::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let count = u16::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    let range = IoPortRange::new(first, count)?;
    if machine
        .objects
        .ports
        .iter()
        .any(|(_, held)| held.overlaps(&range))
    {
        return Err(Error::AlreadyExists);
    }
    super::charge_object(machine, process)?;
    let id = match machine.objects.ports.allocate(range) {
        Ok(id) => id,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(
        AnyObjectId::of(id),
        Rights::READ | Rights::WRITE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw())),
        Err(error) => {
            let _ = machine.objects.ports.release(id);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `ioport_read`.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for a range that is gone;
/// [`Error::AccessDenied`] without `READ`; [`Error::InvalidArgument`] for a
/// width that is not 1, 2, or 4, for a port outside the range, and for an
/// access that would run past its end.
pub fn ioport_read<
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
    let (port, width) = port_of(machine, process, request, Rights::READ)?;
    let value = machine.environment.read_port(port, width)?;
    Ok(Reply::value(value))
}

/// `ioport_write`.
///
/// # Errors
///
/// As [`ioport_read`], with `WRITE` instead of `READ`.
pub fn ioport_write<
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
    let (port, width) = port_of(machine, process, request, Rights::WRITE)?;
    machine
        .environment
        .write_port(port, width, request.argument(3))?;
    Ok(Reply::DONE)
}

/// The port and the width an access names, checked against the range the
/// handle carries.
fn port_of<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    required: Rights,
) -> Result<(u16, u8), Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<IoPortRange>(process, handle, required)?;
    let port = u16::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    let width = u8::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    if !WIDTHS.contains(&width) {
        return Err(Error::InvalidArgument);
    }
    let range = machine.objects.ports.get(id)?;
    if !range.holds(port, width) {
        return Err(Error::InvalidArgument);
    }
    Ok((port, width))
}

/// `memory_create_device`: a memory object over an aperture of the machine.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a count of zero, for a frame number or a
/// count the address space cannot hold, and for a range that meets memory
/// the machine reported as usable; [`Error::QuotaExceeded`],
/// [`Error::PoolExhausted`], or [`Error::OutOfHandles`] as the other
/// creating calls.
pub fn memory_create_device<
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
    let first = request.argument(1);
    let count = request.argument(2);
    if count == 0 {
        return Err(Error::InvalidArgument);
    }
    let start = PhysFrame::from_number(first).map_err(|_| Error::InvalidArgument)?;
    let frames = PhysFrameRange::new(start, count).map_err(|_| Error::InvalidArgument)?;
    if machine.environment.meets_ram(frames) {
        return Err(Error::InvalidArgument);
    }
    super::charge_object(machine, process)?;
    let object = MemoryObject::new(frames, MemoryKind::Device, CachePolicy::Uncached);
    let id = match machine.objects.memory.allocate(object) {
        Ok(id) => id,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(
        AnyObjectId::of(id),
        Rights::READ
            | Rights::WRITE
            | Rights::MAP
            | Rights::INFO
            | Rights::DUPLICATE
            | Rights::TRANSFER,
    );
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw())),
        Err(error) => {
            let _ = machine.objects.memory.release(id);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `system_info`: twenty words about the machine, in the message area of the
/// caller's own buffer.
///
/// For each of the eight pools its capacity and its live count, in the order
/// processes, threads, memory objects, endpoints, notifications, replies,
/// interrupts, port ranges, then the capacity and the live count of the
/// handle arena, then the tick frequency, then the physical address of the
/// root system description pointer, which is zero when the platform named
/// none.
///
/// # Errors
///
/// None beyond the checks of the dispatcher.
pub fn system_info<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
) -> Result<Reply, Error> {
    let capacities = machine.objects.capacities();
    let counts = machine.objects.counts();
    let mut words = [0_u64; 20];
    for (index, (capacity, live)) in capacities.iter().zip(counts.iter()).enumerate() {
        let at = index.saturating_mul(2);
        if let Some(slot) = words.get_mut(at) {
            *slot = u64::from(*capacity);
        }
        if let Some(slot) = words.get_mut(at.saturating_add(1)) {
            *slot = u64::from(*live);
        }
    }
    if let Some(slot) = words.get_mut(18) {
        *slot = u64::from(TICKS_PER_SECOND);
    }
    if let Some(slot) = words.get_mut(19) {
        *slot = machine.environment.acpi_pointer();
    }
    Ok(Reply::message(&words))
}
