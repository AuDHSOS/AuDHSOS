// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The objects of the machine and the scheduler over them, in the one cell
//! that holds them.
//!
//! Invariant: there is one cell, it holds its value from the start, and
//! every access goes through the borrow flag of
//! [`audhsos_sync::Preset`]. The cell is `const`-initialized with a value
//! that is all zeros, so the pools reach the `.bss` of the kernel image
//! without ever being built on the boot stack (D-66).

use audhsos_sync::{Preset, UncontendedToken};
use kernel_objects::MachineObjects;
use kernel_sched::Scheduler;

/// Everything the kernel keeps about objects and about who runs.
#[derive(Debug)]
pub struct Machine {
    /// The pools and the handle arena.
    pub objects: MachineObjects,
    /// The run queues.
    pub scheduler: Scheduler,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    /// A machine that holds nothing.
    #[must_use]
    pub const fn new() -> Self {
        Machine {
            objects: MachineObjects::new(),
            scheduler: Scheduler::new(),
        }
    }
}

/// The one cell holding the objects of the machine.
pub static MACHINE: Preset<Machine> = Preset::new(Machine::new());

/// Runs `body` with the objects of the machine, if they are reachable.
/// They are not while another borrow is alive, which is what an interrupt
/// inside a system call looks like.
pub fn with_machine<R>(body: impl FnOnce(&mut Machine) -> R) -> Option<R> {
    let mut machine = MACHINE.borrow(&UncontendedToken).ok()?;
    Some(body(&mut machine))
}
