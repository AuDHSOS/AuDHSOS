// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A register block in memory that records every access and can script
//! what a read returns.

use std::collections::{HashMap, VecDeque};

use crate::uart::{Register, Registers};

/// One recorded access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A register was read and returned the value.
    Read(Register, u8),
    /// A register was written.
    Write(Register, u8),
}

/// A register file that answers reads from a script, then from the last
/// written value, and records every access.
#[derive(Debug, Default)]
pub struct RecordingRegisters {
    values: HashMap<Register, u8>,
    scripted: HashMap<Register, VecDeque<u8>>,
    log: Vec<Access>,
}

impl RecordingRegisters {
    /// A block whose registers all read zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets what `register` reads until it is written or scripted.
    pub fn set(&mut self, register: Register, value: u8) {
        self.values.insert(register, value);
    }

    /// Queues `values` as the next reads of `register`.
    pub fn script(&mut self, register: Register, values: &[u8]) {
        self.scripted
            .entry(register)
            .or_default()
            .extend(values.iter().copied());
    }

    /// Every access in order.
    #[must_use]
    pub fn log(&self) -> &[Access] {
        &self.log
    }

    /// The writes in order, without the reads.
    #[must_use]
    pub fn writes(&self) -> Vec<(Register, u8)> {
        self.log
            .iter()
            .filter_map(|access| match access {
                Access::Write(register, value) => Some((*register, *value)),
                Access::Read(..) => None,
            })
            .collect()
    }

    /// The number of times `register` was read.
    #[must_use]
    pub fn reads_of(&self, register: Register) -> usize {
        self.log
            .iter()
            .filter(|access| matches!(access, Access::Read(read, _) if *read == register))
            .count()
    }

    /// Forgets the recorded accesses.
    pub fn clear(&mut self) {
        self.log.clear();
    }
}

impl Registers for RecordingRegisters {
    fn read(&mut self, register: Register) -> u8 {
        let value = self
            .scripted
            .get_mut(&register)
            .and_then(VecDeque::pop_front)
            .or_else(|| self.values.get(&register).copied())
            .unwrap_or(0);
        self.log.push(Access::Read(register, value));
        value
    }

    fn write(&mut self, register: Register, value: u8) {
        self.values.insert(register, value);
        self.log.push(Access::Write(register, value));
    }
}
