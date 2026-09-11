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

use audhsos_abi::ipc_buffer::{SIZE, WORDS};
use audhsos_abi::layout::{MAX_MESSAGE_BYTES, MAX_RESULT_WORDS, TICKS_PER_SECOND};
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

/// The width of a string port access, which moves one byte at a time.
const BYTE: u8 = 1;

/// Where the six words of the framebuffer begin in the result of
/// [`system_info`].
const FRAMEBUFFER_WORD: usize = 20;

/// Where the four words about the configuration window of the bus begin.
const ECAM_WORD: usize = 26;

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

/// `interrupt_create_msi`: an interrupt object for a message interrupt.
///
/// The call allocates one vector out of the space the lines are allocated
/// from and answers with the handle, the address the device writes to, and
/// the value it writes. Nothing is routed: the driver puts those two into
/// its device's MSI-X table, which is in the device's own window and
/// therefore in the driver's address space and not the kernel's (D-111).
///
/// # Errors
///
/// [`Error::NoVector`] when the vector space has nothing left;
/// [`Error::Unsupported`] on a machine whose controller is not up;
/// [`Error::QuotaExceeded`], [`Error::PoolExhausted`], or
/// [`Error::OutOfHandles`] as [`interrupt_create`] answers them. A call
/// that fails after the vector was taken gives it back.
pub fn interrupt_create_msi<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
) -> Result<Reply, Error> {
    super::charge_object(machine, process)?;
    let (vector, address, data) = match machine.environment.allocate_message_vector() {
        Ok(message) => message,
        Err(error) => {
            super::refund_object(machine, process);
            return Err(error);
        }
    };
    let id = match machine
        .objects
        .interrupts
        .allocate(Interrupt::message(vector))
    {
        Ok(id) => id,
        Err(error) => {
            machine.environment.release_message_vector(vector);
            super::refund_object(machine, process);
            return Err(Error::from(error));
        }
    };
    let entry = Entry::new(
        AnyObjectId::of(id),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    match machine.objects.install_handle(process, entry) {
        Ok(handle) => Ok(Reply::value(handle.raw()).with_words(&[address, u64::from(data)])),
        Err(error) => {
            let _ = machine.objects.interrupts.release(id);
            machine.environment.release_message_vector(vector);
            super::refund_object(machine, process);
            Err(error)
        }
    }
}

/// `interrupt_bind`: the interrupt signals one bit of a notification.
///
/// The line is unmasked here, because this is the moment it has somewhere
/// to go: a line the plan routes but no notification names would assert
/// into nothing, so [`interrupt_create`] leaves it masked and the binding
/// arms it. After that the cycle of
/// [2.7](../../../../docs/02-architecture.md#27-interrupts-and-devices)
/// runs: the line asserts, the kernel masks it and signals, the driver
/// services the device and acknowledges, which unmasks it again.
///
/// # Errors
///
/// [`Error::InvalidHandle`] for either handle; [`Error::AccessDenied`] when
/// the notification handle lacks `BIND`; [`Error::InvalidArgument`] for a
/// bit index above sixty-three. A notification another interrupt already
/// signals into is bound all the same, on a bit of its own (D-108).
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
    let line = machine.objects.interrupts.get(id).map(|held| held.line);
    if let Ok(Some(line)) = line {
        machine.environment.unmask_interrupt(line);
    }
    Ok(Reply::DONE)
}

/// `interrupt_ack`: the interrupt is no longer held off and the next one
/// may arrive.
///
/// For a line that is an unmask at the interrupt controller. A message
/// interrupt has no line: its mask bit lies in the device's own table,
/// which is mapped in the driver, so the call clears the outstanding flag
/// and touches no hardware (D-111). A device that raises interrupts faster
/// than its driver services them is quieted by its driver.
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
    if let Some(line) = interrupt::acknowledge(machine.objects, id)? {
        machine.environment.unmask_interrupt(line);
    }
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

/// `ioport_write_string`: the bytes of the message area to one port.
///
/// The third argument says how many bytes there are; they are the payload
/// words of the message, which the buffer holds little-endian, so the area
/// read as bytes is the run in the order it was written. One call carries
/// a whole burst, which is what makes a console line cost two calls
/// instead of two a byte.
///
/// # Errors
///
/// As [`ioport_write`], with the width fixed at one byte, and
/// [`Error::InvalidArgument`] for a count above [`MAX_MESSAGE_BYTES`].
pub fn ioport_write_string<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    buffer: &[u8; SIZE],
) -> Result<Reply, Error> {
    let port = port_at(machine, process, request, Rights::WRITE, BYTE)?;
    let count = usize::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    if count > MAX_MESSAGE_BYTES {
        return Err(Error::InvalidArgument);
    }
    let end = WORDS.checked_add(count).ok_or(Error::InvalidArgument)?;
    let bytes = buffer.get(WORDS..end).ok_or(Error::InvalidArgument)?;
    machine.environment.write_port_string(port, bytes)?;
    Ok(Reply::value(u64::try_from(count).unwrap_or(0)))
}

/// The port and the width an access names, checked against the range the
/// handle carries.
fn port_of<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    required: Rights,
) -> Result<(u16, u8), Error> {
    let width = u8::try_from(request.argument(2)).map_err(|_| Error::InvalidArgument)?;
    let port = port_at(machine, process, request, required, width)?;
    Ok((port, width))
}

/// The port an access names, checked against the range the handle carries
/// for a width the caller fixes.
fn port_at<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
    required: Rights,
    width: u8,
) -> Result<u16, Error> {
    let handle = request.handle.ok_or(Error::InvalidHandle)?;
    let (id, _rights) = machine
        .objects
        .resolve::<IoPortRange>(process, handle, required)?;
    let port = u16::try_from(request.argument(1)).map_err(|_| Error::InvalidArgument)?;
    if !WIDTHS.contains(&width) {
        return Err(Error::InvalidArgument);
    }
    let range = machine.objects.ports.get(id)?;
    if !range.holds(port, width) {
        return Err(Error::InvalidArgument);
    }
    Ok(port)
}

/// `memory_create_device`: a memory object over an aperture of the machine.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a count of zero, for a frame number or a
/// count the address space cannot hold, for a range that meets memory the
/// machine reported as usable, and for one that lies in no aperture it
/// reported as device memory; [`Error::QuotaExceeded`],
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
    if !machine.environment.is_device_memory(frames) {
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

/// `system_info`: thirty words about the machine, in the message area of
/// the caller's own buffer.
///
/// For each of the eight pools its capacity and its live count, in the order
/// processes, threads, memory objects, endpoints, notifications, replies,
/// interrupts, port ranges, then the capacity and the live count of the
/// handle arena, then the tick frequency, then the physical address of the
/// root system description pointer, which is zero when the platform named
/// none, and last the framebuffer: its physical start, its length, its
/// width, its height, its stride, and the code of its pixel format, all six
/// zero when the machine has none, and last the configuration window of the
/// bus: its base address, its segment group, its first and its last bus,
/// all four zero when the firmware published no `MCFG` table. The root task
/// makes the device memory objects of the display server and of the program
/// that enumerates the bus out of those two groups.
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
    let mut words = [0_u64; MAX_RESULT_WORDS];
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
    if let Some(window) = machine.environment.ecam() {
        let described = [
            window.base,
            u64::from(window.segment),
            u64::from(window.first_bus),
            u64::from(window.last_bus),
        ];
        for (index, value) in described.iter().enumerate() {
            if let Some(slot) = words.get_mut(index.saturating_add(ECAM_WORD)) {
                *slot = *value;
            }
        }
    }
    if let Some(framebuffer) = machine.environment.framebuffer() {
        let described = [
            framebuffer.phys_start,
            framebuffer.len,
            u64::from(framebuffer.width),
            u64::from(framebuffer.height),
            u64::from(framebuffer.stride),
            u64::from(framebuffer.format.code()),
        ];
        for (index, value) in described.iter().enumerate() {
            if let Some(slot) = words.get_mut(index.saturating_add(FRAMEBUFFER_WORD)) {
                *slot = *value;
            }
        }
    }
    Ok(Reply::message(&words))
}
