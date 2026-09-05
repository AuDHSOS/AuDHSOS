// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two interrupt controllers of a modern machine: the local APIC of
//! the processor and the I/O APICs of the board.
//!
//! Invariants: every register access goes through one volatile read or one
//! volatile write of a `u32` at an offset the specification defines; a
//! register window is mapped uncached for the whole run before a value of
//! these types exists; a line an I/O APIC hands out is routed masked, so
//! nothing arrives before the kernel says it may.

use kernel_acpi::madt::{Madt, Polarity, Trigger};
use kernel_hal_api::interrupt::{InterruptController, InterruptError, InterruptLine, Vector};
use kernel_hal_api::timer::{Timer, TimerError};
use kernel_types::VirtAddr;
use kernel_x86_tables::ioapic;
use kernel_x86_tables::lapic;

use crate::instructions::{read_msr, write_msr};
use crate::timer::{self, CalibrationError};

/// The model-specific register that carries the base address of the local
/// APIC and the bits that turn it on.
pub const IA32_APIC_BASE: u32 = 0x1B;

/// The bit of [`IA32_APIC_BASE`] that turns the local APIC on.
pub const APIC_BASE_ENABLE: u64 = 1 << 11;

/// The local APIC of this processor, reached through its register window.
#[derive(Debug)]
pub struct LocalApic {
    base: VirtAddr,
}

impl LocalApic {
    /// The local APIC whose register window starts at `base`.
    ///
    /// # Safety
    ///
    /// `base` must be the address of the local APIC register window, one
    /// page long, mapped read and write and uncached for as long as this
    /// value exists, and this must be the only value that reaches it.
    #[must_use]
    pub const unsafe fn new(base: VirtAddr) -> Self {
        LocalApic { base }
    }

    /// The pointer to the register at `offset`.
    fn register(&self, offset: usize) -> *mut u32 {
        let address = self
            .base
            .as_u64()
            .wrapping_add(u64::try_from(offset).unwrap_or(0));
        core::ptr::without_provenance_mut::<u32>(usize::try_from(address).unwrap_or(0))
    }

    /// Reads the register at `offset`.
    fn read_register(&self, offset: usize) -> u32 {
        // SAFETY: the constructor promises that the whole window is mapped
        // read and write for the lifetime of this value, and every offset
        // this module uses is inside it; the read is volatile because the
        // value is the device's and not the compiler's.
        unsafe { self.register(offset).read_volatile() }
    }

    /// Writes the register at `offset`.
    fn write_register(&mut self, offset: usize, value: u32) {
        // SAFETY: the same as the read, and the exclusive borrow means no
        // other access to the window is in flight.
        unsafe {
            self.register(offset).write_volatile(value);
        }
    }

    /// Turns the unit on: the enable bit of the model-specific register,
    /// then the enable bit of the spurious vector register with `spurious`
    /// as the vector a spurious interrupt arrives on. The task priority
    /// goes to zero, so that no vector is held back.
    ///
    /// # Safety
    ///
    /// The interrupt descriptor table must already carry a handler for
    /// `spurious` and for every vector the kernel routes afterwards.
    pub unsafe fn enable(&mut self, spurious: u8) {
        // SAFETY: every processor that has a local APIC implements this
        // register, and the value keeps every bit but the enable bit.
        let base = unsafe { read_msr(IA32_APIC_BASE) };
        // SAFETY: the same register, with the bit that turns the unit on.
        unsafe {
            write_msr(IA32_APIC_BASE, base | APIC_BASE_ENABLE);
        }
        self.write_register(lapic::TPR, 0);
        self.write_register(lapic::SVR, lapic::spurious(spurious));
    }

    /// Reports that the handler of the interrupt that is running has
    /// finished.
    pub fn end_of_interrupt(&mut self) {
        self.write_register(lapic::EOI, 0);
    }

    /// The identifier of this local APIC, which is the destination an I/O
    /// APIC sends to.
    #[must_use]
    pub fn id(&self) -> u8 {
        u8::try_from(self.read_register(lapic::ID) >> 24).unwrap_or(0)
    }

    /// The version register.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.read_register(lapic::VERSION)
    }

    /// `true` if the unit is on.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.read_register(lapic::SVR) & lapic::SVR_ENABLE != 0
    }

    /// Sets the divisor the timer counts the bus clock down by.
    pub fn set_timer_divide(&mut self, divide: u32) {
        self.write_register(lapic::TIMER_DIVIDE, divide);
    }

    /// Loads the timer with `count` and starts it counting down.
    pub fn set_timer_count(&mut self, count: u32) {
        self.write_register(lapic::TIMER_INITIAL, count);
    }

    /// The count the timer has left.
    #[must_use]
    pub fn timer_count(&self) -> u32 {
        self.read_register(lapic::TIMER_CURRENT)
    }

    /// Points the timer at `vector`, periodic or one-shot, masked or not.
    pub fn set_timer(&mut self, vector: u8, periodic: bool, masked: bool) {
        self.write_register(lapic::LVT_TIMER, lapic::lvt(vector, masked, periodic));
    }

    /// The local vector table entry of the timer.
    #[must_use]
    pub fn timer_entry(&self) -> u32 {
        self.read_register(lapic::LVT_TIMER)
    }

    /// Stops or resumes the delivery of the timer.
    pub fn mask_timer(&mut self, masked: bool) {
        let entry = self.timer_entry();
        let next = if masked {
            entry | lapic::LVT_MASKED
        } else {
            entry & !lapic::LVT_MASKED
        };
        self.write_register(lapic::LVT_TIMER, next);
    }
}

/// One I/O APIC of the board, reached through its two-register window.
#[derive(Debug)]
pub struct IoApic {
    base: VirtAddr,
    gsi_base: u32,
}

impl IoApic {
    /// The I/O APIC whose register window starts at `base` and whose first
    /// global system interrupt is `gsi_base`.
    ///
    /// # Safety
    ///
    /// `base` must be the address of the register window, one page long,
    /// mapped read and write and uncached for as long as this value
    /// exists, and this must be the only value that reaches it.
    #[must_use]
    pub const unsafe fn new(base: VirtAddr, gsi_base: u32) -> Self {
        IoApic { base, gsi_base }
    }

    /// The first global system interrupt this one serves.
    #[must_use]
    pub const fn gsi_base(&self) -> u32 {
        self.gsi_base
    }

    /// The pointer to the register at `offset` of the window.
    fn register(&self, offset: usize) -> *mut u32 {
        let address = self
            .base
            .as_u64()
            .wrapping_add(u64::try_from(offset).unwrap_or(0));
        core::ptr::without_provenance_mut::<u32>(usize::try_from(address).unwrap_or(0))
    }

    /// Reads the indexed register `index`. Selecting is a write, which is
    /// why reading needs the exclusive borrow.
    fn read_index(&mut self, index: u32) -> u32 {
        // SAFETY: the constructor promises that the window is mapped read
        // and write for the lifetime of this value, and the exclusive
        // borrow means no other access is in flight.
        unsafe {
            self.register(ioapic::IOREGSEL).write_volatile(index);
        }
        // SAFETY: the same window, and the selection above decided which
        // register the data port reaches.
        unsafe { self.register(ioapic::IOWIN).read_volatile() }
    }

    /// Writes the indexed register `index`.
    fn write_index(&mut self, index: u32, value: u32) {
        // SAFETY: the same as the read.
        unsafe {
            self.register(ioapic::IOREGSEL).write_volatile(index);
        }
        // SAFETY: the same window, and the selection above decided which
        // register the data port reaches.
        unsafe {
            self.register(ioapic::IOWIN).write_volatile(value);
        }
    }

    /// The identifier the firmware gave this one.
    pub fn id(&mut self) -> u8 {
        ioapic::id_of(self.read_index(ioapic::ID))
    }

    /// Number of lines this one has.
    pub fn line_count(&mut self) -> u32 {
        ioapic::version_line_count(self.read_index(ioapic::VERSION))
    }

    /// The redirection entry of `line`.
    pub fn entry(&mut self, line: u8) -> u64 {
        let (low, high) = ioapic::redirection_index(line);
        let low_word = self.read_index(low);
        let high_word = self.read_index(high);
        ioapic::entry_from_words(low_word, high_word)
    }

    /// Writes the redirection entry of `line`. The high word goes first,
    /// so that the destination is in place before the entry can deliver.
    pub fn set_entry(&mut self, line: u8, entry: u64) {
        let (low, high) = ioapic::redirection_index(line);
        let (low_word, high_word) = ioapic::entry_words(entry);
        self.write_index(high, high_word);
        self.write_index(low, low_word);
    }

    /// Stops or resumes the delivery of `line`.
    pub fn mask(&mut self, line: u8, masked: bool) {
        let entry = ioapic::with_mask(self.entry(line), masked);
        self.set_entry(line, entry);
    }

    /// Masks every line this one has, which is the state the kernel wants
    /// before it routes anything.
    pub fn mask_all(&mut self) {
        let count = self.line_count();
        let mut line = 0u32;
        while line < count {
            if let Ok(index) = u8::try_from(line) {
                self.mask(index, true);
            }
            line = line.saturating_add(1);
        }
    }
}

/// The interrupt hardware of the machine: one local APIC and the I/O APICs
/// the firmware named, with the table that says how the ISA lines reach
/// them.
#[derive(Debug)]
pub struct Apics {
    local: LocalApic,
    io: [Option<IoApic>; kernel_acpi::MAX_IO_APICS],
    madt: Madt,
    destination: u8,
    ticks_per_ms: u32,
}

impl Apics {
    /// The controller over `local` and `io`, routing through `madt`.
    #[must_use]
    pub fn new(
        local: LocalApic,
        io: [Option<IoApic>; kernel_acpi::MAX_IO_APICS],
        madt: Madt,
    ) -> Self {
        let destination = local.id();
        Apics {
            local,
            io,
            madt,
            destination,
            ticks_per_ms: 0,
        }
    }

    /// How often the local APIC timer counts down in one millisecond, or
    /// zero before it has been measured.
    #[must_use]
    pub const fn ticks_per_ms(&self) -> u32 {
        self.ticks_per_ms
    }

    /// Measures the local APIC timer against the interval timer and keeps
    /// the result, which [`Timer::start_periodic`] needs.
    ///
    /// # Errors
    ///
    /// The errors of [`crate::timer::calibrate`].
    ///
    /// # Safety
    ///
    /// The interval timer must belong to the kernel, and this must run on
    /// the processor whose local APIC this is.
    pub unsafe fn calibrate(&mut self) -> Result<u32, CalibrationError> {
        // SAFETY: the caller promises both of the conditions the
        // calibration needs.
        let ticks_per_ms = unsafe { timer::calibrate(&mut self.local) }?;
        self.ticks_per_ms = ticks_per_ms;
        Ok(ticks_per_ms)
    }

    /// The local APIC, for the timer and the end-of-interrupt.
    pub const fn local_mut(&mut self) -> &mut LocalApic {
        &mut self.local
    }

    /// The local APIC.
    #[must_use]
    pub const fn local(&self) -> &LocalApic {
        &self.local
    }

    /// The table the routing follows.
    #[must_use]
    pub const fn madt(&self) -> &Madt {
        &self.madt
    }

    /// Masks every line of every I/O APIC.
    pub fn mask_all(&mut self) {
        for apic in self.io.iter_mut().flatten() {
            apic.mask_all();
        }
    }

    /// The I/O APIC that serves `gsi` and the line number it knows it by.
    fn locate(&mut self, gsi: u32) -> Option<(&mut IoApic, u8)> {
        let base = self
            .io
            .iter()
            .flatten()
            .map(IoApic::gsi_base)
            .filter(|start| *start <= gsi)
            .max()?;
        let apic = self
            .io
            .iter_mut()
            .flatten()
            .find(|apic| apic.gsi_base() == base)?;
        let line = u8::try_from(gsi.checked_sub(base)?).ok()?;
        if u32::from(line) >= apic.line_count() {
            return None;
        }
        Some((apic, line))
    }

    /// The redirection entry of ISA line `line` for `vector`, masked.
    fn entry_for(&self, line: InterruptLine, vector: Vector) -> u64 {
        let routing = self.madt.route_isa(line.number());
        ioapic::redirection_entry(
            vector.number(),
            routing.polarity == Polarity::ActiveLow,
            routing.trigger == Trigger::Level,
            true,
            self.destination,
        )
    }

    /// The global system interrupt an ISA line reaches.
    #[must_use]
    pub fn gsi_of(&self, line: InterruptLine) -> u32 {
        self.madt.route_isa(line.number()).gsi
    }
}

impl InterruptController for Apics {
    fn route(&mut self, line: InterruptLine, vector: Vector) -> Result<(), InterruptError> {
        let gsi = self.gsi_of(line);
        let entry = self.entry_for(line, vector);
        let (apic, index) = self
            .locate(gsi)
            .ok_or(InterruptError::UnknownLine(line.number()))?;
        if ioapic::entry_vector(apic.entry(index)) != 0 {
            return Err(InterruptError::AlreadyRouted(line.number()));
        }
        apic.set_entry(index, entry);
        Ok(())
    }

    fn mask(&mut self, line: InterruptLine) {
        let gsi = self.gsi_of(line);
        if let Some((apic, index)) = self.locate(gsi) {
            apic.mask(index, true);
        }
    }

    fn unmask(&mut self, line: InterruptLine) {
        let gsi = self.gsi_of(line);
        if let Some((apic, index)) = self.locate(gsi) {
            apic.mask(index, false);
        }
    }

    fn end_of_interrupt(&mut self, _vector: Vector) {
        self.local.end_of_interrupt();
    }
}

impl Timer for Apics {
    fn start_periodic(&mut self, ticks_per_second: u32) -> Result<(), TimerError> {
        let count = timer::initial_count(self.ticks_per_ms, ticks_per_second)?;
        self.local.set_timer_divide(lapic::DIVIDE_BY_16);
        self.local
            .set_timer(kernel_x86_tables::vectors::TIMER, true, false);
        self.local.set_timer_count(count);
        Ok(())
    }

    fn ticks(&self) -> u64 {
        timer::ticks()
    }
}
