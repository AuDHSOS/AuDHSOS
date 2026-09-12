// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A controller in memory: it replays the bytes a test wrote for it,
//! records every write, and can be told to stand still so that a wait runs
//! into its bound.

use std::collections::VecDeque;

use crate::controller::{AUX, OUTPUT_FULL, Ports, READ_CONFIG, SELF_TEST, TEST_AUX, TEST_KBD};

/// One recorded access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The status register was read and answered this.
    Status(u8),
    /// The data register was read and answered this.
    Data(u8),
    /// The data register was written.
    WriteData(u8),
    /// The command register was written.
    WriteCommand(u8),
}

/// The commands the controller answers with a byte. A double that handed
/// its script out before one of these was sent would be a controller that
/// speaks unasked, and the flush at the start of the initialization would
/// swallow the whole of it.
const ANSWERING: [u8; 4] = [READ_CONFIG, SELF_TEST, TEST_KBD, TEST_AUX];

/// A controller that answers out of a script.
///
/// It holds two things a test fills: the output buffer, which stands full
/// of whatever the firmware left in it, and the answers, which the
/// controller hands out one at a time once it has been asked something. A
/// flush therefore finds the first and not the second, which is what makes
/// the initialization testable at all.
///
/// A test that wants a controller which stands still says so with
/// [`fixed_status`](ScriptedPorts::fixed_status), and then neither is ever
/// reached.
#[derive(Debug, Default)]
pub struct ScriptedPorts {
    /// What stands in the output buffer, in order.
    buffer: VecDeque<(u8, bool)>,
    /// What the controller answers once it has been asked, in order.
    script: VecDeque<(u8, bool)>,
    /// Whether it has been asked.
    flowing: bool,
    /// A status register that answers this and nothing else.
    fixed: Option<u8>,
    /// Every access, in order.
    log: Vec<Access>,
}

impl ScriptedPorts {
    /// A controller whose output buffer is empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A controller with `bytes` standing in its output buffer.
    #[must_use]
    pub fn with(bytes: &[u8]) -> Self {
        let mut ports = Self::new();
        ports.push_all(bytes);
        ports
    }

    /// Puts one byte from the keyboard into the output buffer.
    pub fn push(&mut self, byte: u8) {
        self.buffer.push_back((byte, false));
    }

    /// Puts every byte from the keyboard into the output buffer.
    pub fn push_all(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.push(*byte);
        }
    }

    /// Puts one byte from the mouse into the output buffer.
    pub fn push_aux(&mut self, byte: u8) {
        self.buffer.push_back((byte, true));
    }

    /// Queues one byte the controller answers with once it has been asked.
    pub fn answer(&mut self, byte: u8) {
        self.script.push_back((byte, false));
    }

    /// Queues every byte the controller answers with, in order.
    pub fn answer_all(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.answer(*byte);
        }
    }

    /// Makes the status register answer `value` and nothing else, so that
    /// a wait for a byte or for the controller runs into its bound.
    pub const fn fixed_status(&mut self, value: u8) {
        self.fixed = Some(value);
    }

    /// How many answers are left unread.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.buffer.len().saturating_add(self.script.len())
    }

    /// Every access in order.
    #[must_use]
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    /// The command bytes in order.
    #[must_use]
    pub fn commands(&self) -> Vec<u8> {
        self.log
            .iter()
            .filter_map(|access| match access {
                Access::WriteCommand(value) => Some(*value),
                _other => None,
            })
            .collect()
    }

    /// The bytes written to the data register, in order.
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        self.log
            .iter()
            .filter_map(|access| match access {
                Access::WriteData(value) => Some(*value),
                _other => None,
            })
            .collect()
    }

    /// How many bytes were read out of the data register.
    #[must_use]
    pub fn reads(&self) -> usize {
        self.log
            .iter()
            .filter(|access| matches!(access, Access::Data(_)))
            .count()
    }

    /// Forgets the recorded accesses.
    pub fn clear(&mut self) {
        self.log.clear();
    }

    /// Moves the next answer into the output buffer, once the controller
    /// has been asked something and the buffer is empty.
    fn refill(&mut self) {
        if self.flowing
            && self.buffer.is_empty()
            && let Some(answer) = self.script.pop_front()
        {
            self.buffer.push_back(answer);
        }
    }
}

impl Ports for ScriptedPorts {
    fn read_data(&mut self) -> u8 {
        self.refill();
        let value = self.buffer.pop_front().map_or(0, |(byte, _aux)| byte);
        self.log.push(Access::Data(value));
        value
    }

    fn read_status(&mut self) -> u8 {
        if self.fixed.is_none() {
            self.refill();
        }
        let value = match self.fixed {
            Some(fixed) => fixed,
            None => match self.buffer.front() {
                Some(&(_byte, true)) => OUTPUT_FULL | AUX,
                Some(&(_byte, false)) => OUTPUT_FULL,
                None => 0,
            },
        };
        self.log.push(Access::Status(value));
        value
    }

    fn write_data(&mut self, value: u8) {
        self.flowing = true;
        self.log.push(Access::WriteData(value));
    }

    fn write_command(&mut self, value: u8) {
        if ANSWERING.contains(&value) {
            self.flowing = true;
        }
        self.log.push(Access::WriteCommand(value));
    }
}
