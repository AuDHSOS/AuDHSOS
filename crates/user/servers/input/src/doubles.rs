// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The clients and the interrupt lines in memory: rings that are byte
//! vectors, wake-ups that are recorded, and clients that can be made to be
//! gone.

use std::collections::VecDeque;

use user_proto::input::{Event, RING_PAGE_LEN, RingReader, RingWriter};

use driver_i8042::controller::Ports;
use driver_i8042::doubles::ScriptedPorts;

use crate::service::{Line, Lines};
use crate::state::Clients;

/// The rings of the clients and what was done to them.
#[derive(Debug, Default)]
pub struct RecordingClients {
    /// One page per slot.
    rings: Vec<Vec<u8>>,
    /// The slots whose client has ended and can no longer be woken.
    gone: Vec<usize>,
    /// Every slot that was woken, in order.
    woken: Vec<usize>,
}

impl RecordingClients {
    /// `slots` rings of one page each, every one of them made.
    #[must_use]
    pub fn new(slots: usize) -> Self {
        let mut clients = RecordingClients {
            rings: Vec::new(),
            gone: Vec::new(),
            woken: Vec::new(),
        };
        for _ in 0..slots {
            let mut page = vec![0u8; RING_PAGE_LEN];
            let _made = RingWriter::create(&mut page);
            clients.rings.push(page);
        }
        clients
    }

    /// Says that the client in `slot` has ended, so that the next wake-up
    /// of it fails the way one of a process that is gone does.
    pub fn end(&mut self, slot: usize) {
        self.gone.push(slot);
    }

    /// Every slot that was woken, in order.
    #[must_use]
    pub fn woken(&self) -> &[usize] {
        &self.woken
    }

    /// Forgets which slots were woken.
    pub fn clear(&mut self) {
        self.woken.clear();
    }

    /// Every event standing in the ring of `slot`, which also takes them
    /// out of it.
    #[must_use]
    pub fn drain(&mut self, slot: usize) -> Vec<Event> {
        let Some(bytes) = self.rings.get_mut(slot) else {
            return Vec::new();
        };
        let Some(mut reader) = RingReader::new(bytes) else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Some(event) = reader.pop() {
            events.push(event);
        }
        events
    }

    /// How many events the ring of `slot` dropped, which also forgets them.
    #[must_use]
    pub fn overflow(&mut self, slot: usize) -> u32 {
        self.rings
            .get_mut(slot)
            .and_then(|bytes| RingReader::new(bytes))
            .map_or(0, |mut reader| reader.take_overflow())
    }
}

impl Clients for RecordingClients {
    fn ring(&mut self, slot: usize) -> Option<&mut [u8]> {
        self.rings.get_mut(slot).map(Vec::as_mut_slice)
    }

    fn wake(&mut self, slot: usize) -> bool {
        if self.gone.contains(&slot) {
            return false;
        }
        self.woken.push(slot);
        true
    }
}

/// A controller a test scripts, together with the two interrupt objects the
/// thread that drains it lets go.
///
/// The process holds both in one place — the ports and the interrupts are
/// reached through the same gate — and so does this.
#[derive(Debug, Default)]
pub struct ScriptedDevice {
    /// What the controller answers.
    ports: ScriptedPorts,
    /// Every acknowledgement, in order.
    acknowledged: Vec<Line>,
    /// Bytes that stand in the buffer only once the lines have been let
    /// go, one per round. That is the race a driver of an edge-triggered
    /// line has to survive, and this is how a test makes it happen.
    late: VecDeque<u8>,
}

impl ScriptedDevice {
    /// A controller whose output buffer is empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The controller, for a test that writes its script.
    pub const fn ports(&mut self) -> &mut ScriptedPorts {
        &mut self.ports
    }

    /// Puts one byte into the buffer at the moment the lines of a round are
    /// let go, which is when a device that raised its line into the mask
    /// would have put it there.
    pub fn push_late(&mut self, byte: u8) {
        self.late.push_back(byte);
    }

    /// Every acknowledgement, in order.
    #[must_use]
    pub fn acknowledged(&self) -> &[Line] {
        &self.acknowledged
    }
}

impl Ports for ScriptedDevice {
    fn read_data(&mut self) -> u8 {
        self.ports.read_data()
    }

    fn read_status(&mut self) -> u8 {
        self.ports.read_status()
    }

    fn write_data(&mut self, value: u8) {
        self.ports.write_data(value);
    }

    fn write_command(&mut self, value: u8) {
        self.ports.write_command(value);
    }
}

impl Lines for ScriptedDevice {
    fn acknowledge(&mut self, line: Line) {
        self.acknowledged.push(line);
        // One late byte per round, and a round ends with the last line.
        if Line::ALL.last() == Some(&line)
            && let Some(byte) = self.late.pop_front()
        {
            self.ports.push(byte);
        }
    }
}
