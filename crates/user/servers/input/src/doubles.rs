// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The clients and the interrupt lines in memory: rings that are byte
//! vectors, wake-ups that are recorded, and clients that can be made to be
//! gone.

use user_proto::input::{Event, RING_PAGE_LEN, RingReader, RingWriter};

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

/// The interrupt objects of the two lines, as a test holds them.
#[derive(Debug, Default)]
pub struct RecordingLines {
    /// Every acknowledgement, in order.
    acknowledged: Vec<Line>,
}

impl RecordingLines {
    /// Two lines nobody has acknowledged.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every acknowledgement, in order.
    #[must_use]
    pub fn acknowledged(&self) -> &[Line] {
        &self.acknowledged
    }
}

impl Lines for RecordingLines {
    fn acknowledge(&mut self, line: Line) {
        self.acknowledged.push(line);
    }
}
