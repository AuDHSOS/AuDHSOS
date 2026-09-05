// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bringing the interrupt hardware of the machine up and keeping it
//! reachable from an interrupt handler.
//!
//! Invariants: by the time [`bring_up`] reports success the two legacy
//! controllers are masked, every I/O APIC line is masked, and the local
//! APIC is on with a handler behind its spurious vector; the controller
//! lives in one cell that a handler borrows for as long as one
//! acknowledgement takes.

use core::fmt;

use audhsos_sync::{Global, UncontendedToken};
use kernel_acpi::madt::Madt;
use kernel_hal_api::timer::{Timer, TimerError};
use kernel_types::{PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};
use kernel_x86_tables::vectors;

use crate::acpi::{self, TableError};
use crate::apic::{Apics, IoApic, LocalApic};
use crate::bootinfo::X86Platform;
use crate::instructions::InterruptGuard;
use crate::pic;
use crate::timer::CalibrationError;

/// Why the interrupt hardware could not be brought up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApicError {
    /// The tables of the machine could not be read.
    Tables(TableError),
    /// A register window could not be mapped.
    Window(PhysAddr),
    /// The controller is already in place.
    AlreadyUp,
    /// The controller is not in place, or a handler is holding it.
    Unreachable,
    /// The local APIC timer could not be measured.
    Calibration(CalibrationError),
    /// The tick rate is not one the timer can produce.
    Rate(TimerError),
}

impl fmt::Display for ApicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApicError::Tables(error) => write!(f, "{error}"),
            ApicError::Window(address) => {
                write!(f, "the register window at {address} could not be mapped")
            }
            ApicError::AlreadyUp => f.write_str("the interrupt controller is already in place"),
            ApicError::Unreachable => f.write_str("the interrupt controller is not reachable"),
            ApicError::Calibration(error) => write!(f, "{error}"),
            ApicError::Rate(error) => write!(f, "{error}"),
        }
    }
}

impl From<TableError> for ApicError {
    fn from(error: TableError) -> Self {
        ApicError::Tables(error)
    }
}

impl From<CalibrationError> for ApicError {
    fn from(error: CalibrationError) -> Self {
        ApicError::Calibration(error)
    }
}

impl From<TimerError> for ApicError {
    fn from(error: TimerError) -> Self {
        ApicError::Rate(error)
    }
}

/// The interrupt hardware of the machine, reachable from a handler.
static CONTROLLER: Global<Apics> = Global::new();

/// Reads the tables of the machine, maps the register windows through
/// `map`, quiets the two legacy controllers, turns the local APIC on, and
/// masks every I/O APIC line.
///
/// The mapping is the caller's, because only the kernel knows its own
/// address space: `map` takes the frames of a register window and answers
/// with the address they are reachable at, uncached.
///
/// # Errors
///
/// [`ApicError`] for a machine whose tables the kernel cannot read or
/// whose register windows it cannot map.
///
/// # Safety
///
/// The tables the loader built must be active, the interrupt descriptor
/// table must be loaded with a handler for every vector of the plan,
/// interrupts must be off, and this must run once on the boot processor.
pub unsafe fn bring_up<M>(platform: &X86Platform, mut map: M) -> Result<(), ApicError>
where
    M: FnMut(PhysFrameRange) -> Option<VirtAddr>,
{
    // SAFETY: the caller promises that the loader's tables are active.
    let madt = unsafe { acpi::find_madt(platform) }?;
    let local_base = window_for(&mut map, madt.lapic_address)?;
    // SAFETY: the window was just mapped read and write and uncached for
    // the rest of the run, and this is the only value that reaches it.
    let local = unsafe { LocalApic::new(local_base) };
    let mut io: [Option<IoApic>; kernel_acpi::MAX_IO_APICS] =
        [const { None }; kernel_acpi::MAX_IO_APICS];
    for (slot, entry) in io.iter_mut().zip(madt.io_apics.iter().flatten()) {
        let base = window_for(&mut map, entry.address)?;
        // SAFETY: the same as the local APIC window.
        *slot = Some(unsafe { IoApic::new(base, entry.gsi_base) });
    }
    if madt.has_legacy_pic() {
        // SAFETY: the caller promises that the descriptor table carries a
        // handler for every vector of the plan, which is where the two
        // controllers are moved to before they are masked.
        unsafe {
            pic::disable();
        }
    }
    let mut apics = Apics::new(local, io, madt);
    // SAFETY: the caller promises that the descriptor table carries a
    // handler for the spurious vector.
    unsafe {
        apics.local_mut().enable(vectors::SPURIOUS);
    }
    apics.mask_all();
    CONTROLLER.init(apics).map_err(|_| ApicError::AlreadyUp)
}

/// The address the page holding `address` is reachable at, plus the offset
/// of `address` inside it.
fn window_for<M>(map: &mut M, address: PhysAddr) -> Result<VirtAddr, ApicError>
where
    M: FnMut(PhysFrameRange) -> Option<VirtAddr>,
{
    let frame = PhysFrame::containing(address);
    let range = PhysFrameRange::new(frame, 1).map_err(|_| ApicError::Window(address))?;
    let base = map(range).ok_or(ApicError::Window(address))?;
    let offset = address.as_u64().wrapping_sub(frame.start().as_u64());
    VirtAddr::new(base.as_u64().wrapping_add(offset)).map_err(|_| ApicError::Window(address))
}

/// Runs `body` with the interrupt hardware, if it is in place.
///
/// Interrupts are off for as long as the borrow lasts, because the handler
/// of every device vector needs the same cell to acknowledge: a handler
/// that found it busy would return without acknowledging, and the local
/// APIC would deliver nothing after that. Inside a handler the guard is a
/// no-op, because an interrupt gate has already turned interrupts off.
pub fn with_controller<R>(body: impl FnOnce(&mut Apics) -> R) -> Option<R> {
    let _guard = InterruptGuard::new();
    let mut controller = CONTROLLER.borrow(&UncontendedToken).ok()?;
    Some(body(&mut controller))
}

/// The table the routing follows, if the controller is in place.
pub fn with_madt<R>(body: impl FnOnce(&Madt) -> R) -> Option<R> {
    let _guard = InterruptGuard::new();
    let controller = CONTROLLER.borrow(&UncontendedToken).ok()?;
    Some(body(controller.madt()))
}

/// Measures the local APIC timer and starts it at `ticks_per_second`.
///
/// # Errors
///
/// [`ApicError`] if the controller is not reachable, if the timer cannot
/// be measured, or if the rate is not one it can produce.
///
/// # Safety
///
/// The interval timer must belong to the kernel, interrupts must be off,
/// and this must run on the processor the controller belongs to.
pub unsafe fn start_timer(ticks_per_second: u32) -> Result<(), ApicError> {
    with_controller(|apics| {
        // SAFETY: the caller promises that the interval timer belongs to
        // the kernel and that this is the right processor.
        unsafe { apics.calibrate() }?;
        apics.start_periodic(ticks_per_second)?;
        Ok(())
    })
    .unwrap_or(Err(ApicError::Unreachable))
}

/// Acknowledges the interrupt of `vector` at the hardware and counts a
/// timer tick if that is what it was.
///
/// A spurious interrupt is the one that is never acknowledged: the local
/// APIC raises it when a vector it had already accepted turns out to have
/// no source, and it expects no end-of-interrupt for it.
pub fn acknowledge(vector: u8) {
    if vector == vectors::TIMER {
        crate::timer::record_tick();
    }
    if vector == vectors::SPURIOUS {
        return;
    }
    with_controller(|apics| apics.local_mut().end_of_interrupt());
}

/// The number of ticks the timer has delivered.
#[must_use]
pub fn ticks() -> u64 {
    crate::timer::ticks()
}
