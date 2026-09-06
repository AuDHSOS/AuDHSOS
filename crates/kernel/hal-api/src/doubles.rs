// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! In-memory implementations of the HAL traits that record every call.

use std::collections::HashMap;
#[cfg(feature = "port-io")]
use std::collections::VecDeque;

use kernel_types::{Page, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

use crate::console::DebugConsole;
use crate::exit::{ExitStatus, TestExit};
use crate::interrupt::{InterruptController, InterruptError, InterruptLine, Vector};
use crate::paging::{
    AddressSpaceControl, FRAME_BYTES, FrameAccess, FrameBytes, FrameSource, TlbControl,
};
use crate::platform::{MemoryRegion, MemoryRegionKind, Platform};
use crate::timer::{Timer, TimerError};

/// Page tables stored in a map from frame to table. A range of frames can
/// be declared as memory, so that a table appears there on the first
/// modifying access, the way a freshly allocated frame in the kernel is
/// already reachable through the physical window.
#[derive(Debug)]
pub struct MemoryFrameAccess<T> {
    tables: HashMap<PhysFrame, Box<T>>,
    memory: Option<PhysFrameRange>,
}

impl<T> Default for MemoryFrameAccess<T> {
    fn default() -> Self {
        MemoryFrameAccess {
            tables: HashMap::new(),
            memory: None,
        }
    }
}

impl<T> MemoryFrameAccess<T> {
    /// An access with no reachable frames.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An access over which every frame of `ram` becomes reachable with a
    /// default table as soon as it is modified.
    #[must_use]
    pub fn with_lazy_tables(ram: PhysFrameRange) -> Self {
        MemoryFrameAccess {
            tables: HashMap::new(),
            memory: Some(ram),
        }
    }

    /// The range that materializes tables on demand, if there is one.
    #[must_use]
    pub const fn lazy_range(&self) -> Option<PhysFrameRange> {
        self.memory
    }

    /// Makes `frame` reachable with `table` as its content.
    pub fn insert(&mut self, frame: PhysFrame, table: T) {
        self.tables.insert(frame, Box::new(table));
    }

    /// Makes `frame` unreachable and returns its content.
    pub fn remove(&mut self, frame: PhysFrame) -> Option<T> {
        self.tables.remove(&frame).map(|boxed| *boxed)
    }

    /// `true` if `frame` is reachable.
    #[must_use]
    pub fn contains(&self, frame: PhysFrame) -> bool {
        self.tables.contains_key(&frame)
    }

    /// The number of reachable frames.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// `true` if no frame is reachable.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

impl<T: Default> FrameAccess<T> for MemoryFrameAccess<T> {
    fn table(&self, frame: PhysFrame) -> Option<&T> {
        self.tables.get(&frame).map(AsRef::as_ref)
    }

    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut T> {
        if !self.tables.contains_key(&frame) && self.memory.is_some_and(|ram| ram.contains(frame)) {
            self.tables.insert(frame, Box::new(T::default()));
        }
        self.tables.get_mut(&frame).map(AsMut::as_mut)
    }
}

/// Frames whose bytes live in a map, so that a test can read and write what
/// the kernel wrote into one.
#[derive(Debug, Default)]
pub struct MemoryFrameBytes {
    frames: HashMap<PhysFrame, Box<[u8; FRAME_BYTES]>>,
}

impl MemoryFrameBytes {
    /// No reachable frame.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `frame` reachable, filled with zeros.
    pub fn add(&mut self, frame: PhysFrame) {
        self.frames
            .entry(frame)
            .or_insert_with(|| Box::new([0; FRAME_BYTES]));
    }

    /// `true` if `frame` is reachable.
    #[must_use]
    pub fn contains(&self, frame: PhysFrame) -> bool {
        self.frames.contains_key(&frame)
    }

    /// How many frames are reachable.
    #[must_use]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// `true` if no frame is reachable.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
}

impl FrameBytes for MemoryFrameBytes {
    fn frame_bytes(&self, frame: PhysFrame) -> Option<&[u8; FRAME_BYTES]> {
        self.frames.get(&frame).map(AsRef::as_ref)
    }

    fn frame_bytes_mut(&mut self, frame: PhysFrame) -> Option<&mut [u8; FRAME_BYTES]> {
        self.frames.get_mut(&frame).map(AsMut::as_mut)
    }
}

/// A recorded TLB operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flush {
    /// One page was flushed.
    Page(Page),
    /// Everything was flushed.
    All,
}

/// Records flush requests.
#[derive(Debug, Default)]
pub struct RecordingTlb {
    flushes: Vec<Flush>,
}

impl RecordingTlb {
    /// A recorder with no flushes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The recorded flushes in order.
    #[must_use]
    pub fn flushes(&self) -> &[Flush] {
        &self.flushes
    }

    /// Forgets the recorded flushes.
    pub fn clear(&mut self) {
        self.flushes.clear();
    }
}

impl TlbControl for RecordingTlb {
    fn flush_page(&mut self, page: Page) {
        self.flushes.push(Flush::Page(page));
    }

    fn flush_all(&mut self) {
        self.flushes.push(Flush::All);
    }
}

/// Hands out consecutive frames from a starting frame up to an optional
/// limit and records releases.
#[derive(Debug)]
pub struct CountingFrameSource {
    next: PhysFrame,
    remaining: Option<u64>,
    allocated: Vec<PhysFrame>,
    released: Vec<PhysFrame>,
}

impl CountingFrameSource {
    /// A source that starts at `first` and hands out at most `limit` frames
    /// (`None` for no limit other than the address width).
    #[must_use]
    pub const fn new(first: PhysFrame, limit: Option<u64>) -> Self {
        CountingFrameSource {
            next: first,
            remaining: limit,
            allocated: Vec::new(),
            released: Vec::new(),
        }
    }

    /// Every frame handed out, in order.
    #[must_use]
    pub fn allocated(&self) -> &[PhysFrame] {
        &self.allocated
    }

    /// Every frame released, in order.
    #[must_use]
    pub fn released(&self) -> &[PhysFrame] {
        &self.released
    }

    /// Frames handed out and not yet released.
    #[must_use]
    pub const fn outstanding(&self) -> usize {
        self.allocated.len().saturating_sub(self.released.len())
    }
}

impl FrameSource for CountingFrameSource {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if self.remaining == Some(0) {
            return None;
        }
        let frame = self.next;
        self.remaining = self.remaining.map(|r| r.saturating_sub(1));
        match self.next.checked_add(1) {
            Some(next) => self.next = next,
            None => self.remaining = Some(0),
        }
        self.allocated.push(frame);
        Some(frame)
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        self.released.push(frame);
    }
}

/// Collects console output.
#[derive(Debug, Default)]
pub struct RecordingConsole {
    output: Vec<u8>,
}

impl RecordingConsole {
    /// An empty console.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything written so far.
    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Everything written so far, as text with invalid sequences replaced.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output).into_owned()
    }
}

impl DebugConsole for RecordingConsole {
    fn write_bytes(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }
}

/// Records the requested exit status.
#[derive(Debug, Default)]
pub struct RecordingExit {
    status: Option<ExitStatus>,
}

impl RecordingExit {
    /// A recorder without a status.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The first requested status, if any.
    #[must_use]
    pub const fn status(&self) -> Option<ExitStatus> {
        self.status
    }
}

impl TestExit for RecordingExit {
    fn exit(&mut self, status: ExitStatus) {
        self.status.get_or_insert(status);
    }
}

/// A timer whose ticks advance only when the test says so.
#[derive(Debug, Default)]
pub struct FakeTimer {
    frequency: Option<u32>,
    ticks: u64,
    rejected: Vec<u32>,
}

impl FakeTimer {
    /// A stopped timer that accepts every frequency not in `rejected`.
    #[must_use]
    pub const fn new(rejected: Vec<u32>) -> Self {
        FakeTimer {
            frequency: None,
            ticks: 0,
            rejected,
        }
    }

    /// The frequency the timer was started with.
    #[must_use]
    pub const fn frequency(&self) -> Option<u32> {
        self.frequency
    }

    /// Advances the tick counter.
    pub const fn advance(&mut self, ticks: u64) {
        self.ticks = self.ticks.wrapping_add(ticks);
    }
}

impl Timer for FakeTimer {
    fn start_periodic(&mut self, ticks_per_second: u32) -> Result<(), TimerError> {
        if self.rejected.contains(&ticks_per_second) {
            return Err(TimerError::UnsupportedFrequency(ticks_per_second));
        }
        self.frequency = Some(ticks_per_second);
        Ok(())
    }

    fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// A recorded interrupt controller operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrqEvent {
    /// A line was routed to a vector.
    Route(InterruptLine, Vector),
    /// A line was masked.
    Mask(InterruptLine),
    /// A line was unmasked.
    Unmask(InterruptLine),
    /// An end of interrupt was sent for a vector.
    EndOfInterrupt(Vector),
}

/// Records controller operations and models routing state.
#[derive(Debug)]
pub struct FakeInterruptController {
    lines: u8,
    routes: HashMap<InterruptLine, Vector>,
    masked: HashMap<InterruptLine, bool>,
    events: Vec<IrqEvent>,
}

impl FakeInterruptController {
    /// A controller with `lines` lines numbered from zero.
    #[must_use]
    pub fn new(lines: u8) -> Self {
        FakeInterruptController {
            lines,
            routes: HashMap::new(),
            masked: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Every operation in order.
    #[must_use]
    pub fn events(&self) -> &[IrqEvent] {
        &self.events
    }

    /// The vector `line` is routed to.
    #[must_use]
    pub fn route_of(&self, line: InterruptLine) -> Option<Vector> {
        self.routes.get(&line).copied()
    }

    /// `true` if `line` is routed and masked.
    #[must_use]
    pub fn is_masked(&self, line: InterruptLine) -> bool {
        self.masked.get(&line).copied().unwrap_or(false)
    }
}

impl InterruptController for FakeInterruptController {
    fn route(&mut self, line: InterruptLine, vector: Vector) -> Result<(), InterruptError> {
        if line.number() >= self.lines {
            return Err(InterruptError::UnknownLine(line.number()));
        }
        if self.routes.contains_key(&line) {
            return Err(InterruptError::AlreadyRouted(line.number()));
        }
        self.routes.insert(line, vector);
        self.masked.insert(line, true);
        self.events.push(IrqEvent::Route(line, vector));
        Ok(())
    }

    fn mask(&mut self, line: InterruptLine) {
        if self.routes.contains_key(&line) {
            self.masked.insert(line, true);
        }
        self.events.push(IrqEvent::Mask(line));
    }

    fn unmask(&mut self, line: InterruptLine) {
        if self.routes.contains_key(&line) {
            self.masked.insert(line, false);
        }
        self.events.push(IrqEvent::Unmask(line));
    }

    fn end_of_interrupt(&mut self, vector: Vector) {
        self.events.push(IrqEvent::EndOfInterrupt(vector));
    }
}

/// A recorded port operation.
#[cfg(feature = "port-io")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortEvent {
    /// A read of the given width in bytes returned the value.
    Read {
        /// Port number.
        port: u16,
        /// Width in bytes.
        width: u8,
        /// Value returned.
        value: u32,
    },
    /// A write of the given width in bytes.
    Write {
        /// Port number.
        port: u16,
        /// Width in bytes.
        width: u8,
        /// Value written.
        value: u32,
    },
}

/// Ports whose reads are scripted per port and whose operations are
/// recorded. A read without a scripted value returns `0`.
#[cfg(feature = "port-io")]
#[derive(Debug, Default)]
pub struct RecordingPorts {
    scripted: HashMap<u16, VecDeque<u32>>,
    events: Vec<PortEvent>,
}

#[cfg(feature = "port-io")]
impl RecordingPorts {
    /// Ports without scripted reads.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `value` as the next read result of `port`.
    pub fn script_read(&mut self, port: u16, value: u32) {
        self.scripted.entry(port).or_default().push_back(value);
    }

    /// Every operation in order.
    #[must_use]
    pub fn events(&self) -> &[PortEvent] {
        &self.events
    }

    /// The values written to `port`, in order.
    #[must_use]
    pub fn writes_to(&self, port: u16) -> Vec<u32> {
        self.events
            .iter()
            .filter_map(|event| match event {
                PortEvent::Write { port: p, value, .. } if *p == port => Some(*value),
                _ => None,
            })
            .collect()
    }

    fn read(&mut self, port: u16, width: u8) -> u32 {
        let value = self
            .scripted
            .get_mut(&port)
            .and_then(VecDeque::pop_front)
            .unwrap_or(0);
        self.events.push(PortEvent::Read { port, width, value });
        value
    }

    fn write(&mut self, port: u16, width: u8, value: u32) {
        self.events.push(PortEvent::Write { port, width, value });
    }
}

#[cfg(feature = "port-io")]
impl crate::port::PortAccess for RecordingPorts {
    fn read_u8(&mut self, port: u16) -> u8 {
        u8::try_from(self.read(port, 1) & 0xFF).unwrap_or(0)
    }

    fn write_u8(&mut self, port: u16, value: u8) {
        self.write(port, 1, u32::from(value));
    }

    fn read_u16(&mut self, port: u16) -> u16 {
        u16::try_from(self.read(port, 2) & 0xFFFF).unwrap_or(0)
    }

    fn write_u16(&mut self, port: u16, value: u16) {
        self.write(port, 2, u32::from(value));
    }

    fn read_u32(&mut self, port: u16) -> u32 {
        self.read(port, 4)
    }

    fn write_u32(&mut self, port: u16, value: u32) {
        self.write(port, 4, value);
    }
}

/// A platform assembled by the test.
#[derive(Clone, Debug)]
pub struct ScriptedPlatform {
    regions: Vec<MemoryRegion>,
    window_base: VirtAddr,
    rsdp: Option<PhysAddr>,
}

impl ScriptedPlatform {
    /// A platform without regions, with the window at `window_base`.
    #[must_use]
    pub const fn new(window_base: VirtAddr) -> Self {
        ScriptedPlatform {
            regions: Vec::new(),
            window_base,
            rsdp: None,
        }
    }

    /// Adds a region.
    #[must_use]
    pub fn region(mut self, start: PhysAddr, len: u64, kind: MemoryRegionKind) -> Self {
        self.regions.push(MemoryRegion { start, len, kind });
        self
    }

    /// Sets the ACPI root pointer.
    #[must_use]
    pub const fn rsdp(mut self, rsdp: PhysAddr) -> Self {
        self.rsdp = Some(rsdp);
        self
    }
}

impl Platform for ScriptedPlatform {
    fn memory_regions(&self) -> &[MemoryRegion] {
        &self.regions
    }

    fn physical_window_base(&self) -> VirtAddr {
        self.window_base
    }

    fn acpi_rsdp(&self) -> Option<PhysAddr> {
        self.rsdp
    }
}

/// An address space control that records every root it was given.
#[derive(Clone, Debug)]
pub struct RecordingAddressSpaces {
    active: PhysFrame,
    loaded: Vec<PhysFrame>,
}

impl RecordingAddressSpaces {
    /// A control that starts on `root`.
    #[must_use]
    pub const fn new(root: PhysFrame) -> Self {
        RecordingAddressSpaces {
            active: root,
            loaded: Vec::new(),
        }
    }

    /// Every root it was given, in order. A switch between threads of one
    /// process adds nothing here.
    #[must_use]
    pub fn loaded(&self) -> &[PhysFrame] {
        &self.loaded
    }

    /// How often a root was loaded.
    #[must_use]
    pub const fn switches(&self) -> usize {
        self.loaded.len()
    }
}

impl AddressSpaceControl for RecordingAddressSpaces {
    fn activate(&mut self, root: PhysFrame) {
        self.active = root;
        self.loaded.push(root);
    }

    fn active(&self) -> PhysFrame {
        self.active
    }
}

/// The interrupt controller and the ports of one machine, for the kernel
/// environment that needs both at once.
#[cfg(feature = "port-io")]
#[derive(Debug)]
pub struct RecordingDevices {
    /// The controller.
    pub interrupts: FakeInterruptController,
    /// The ports.
    pub ports: RecordingPorts,
}

#[cfg(feature = "port-io")]
impl RecordingDevices {
    /// Devices with `lines` interrupt lines and no scripted port reads.
    #[must_use]
    pub fn new(lines: u8) -> Self {
        RecordingDevices {
            interrupts: FakeInterruptController::new(lines),
            ports: RecordingPorts::new(),
        }
    }
}

#[cfg(feature = "port-io")]
impl Default for RecordingDevices {
    fn default() -> Self {
        Self::new(24)
    }
}

#[cfg(feature = "port-io")]
impl InterruptController for RecordingDevices {
    fn route(&mut self, line: InterruptLine, vector: Vector) -> Result<(), InterruptError> {
        self.interrupts.route(line, vector)
    }

    fn mask(&mut self, line: InterruptLine) {
        self.interrupts.mask(line);
    }

    fn unmask(&mut self, line: InterruptLine) {
        self.interrupts.unmask(line);
    }

    fn end_of_interrupt(&mut self, vector: Vector) {
        self.interrupts.end_of_interrupt(vector);
    }
}

#[cfg(feature = "port-io")]
impl crate::port::PortAccess for RecordingDevices {
    fn read_u8(&mut self, port: u16) -> u8 {
        self.ports.read_u8(port)
    }

    fn write_u8(&mut self, port: u16, value: u8) {
        self.ports.write_u8(port, value);
    }

    fn read_u16(&mut self, port: u16) -> u16 {
        self.ports.read_u16(port)
    }

    fn write_u16(&mut self, port: u16, value: u16) {
        self.ports.write_u16(port, value);
    }

    fn read_u32(&mut self, port: u16) -> u32 {
        self.ports.read_u32(port)
    }

    fn write_u32(&mut self, port: u16, value: u32) {
        self.ports.write_u32(port, value);
    }
}
