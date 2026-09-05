// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The global kernel state.
//!
//! Invariant: there is one cell, initialized once during boot, and every
//! access goes through the borrow flag of [`audhsos_sync::Global`].

use audhsos_sync::{Global, UncontendedToken};

/// Everything the kernel keeps between system calls. Phase 2 kept the
/// number of traps the kernel has reported; Phase 4 adds what the interrupt
/// hardware delivered. The object pools, the address spaces, and the
/// scheduler arrive in later phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelState {
    /// How many traps the kernel has reported.
    pub traps: u64,
    /// How many timer ticks the kernel has counted.
    pub ticks: u64,
    /// How many device interrupts the kernel has taken, the timer
    /// included.
    pub interrupts: u64,
    /// How many spurious interrupts the kernel has taken.
    pub spurious: u64,
}

impl KernelState {
    /// A state that has seen nothing.
    #[must_use]
    pub const fn new() -> Self {
        KernelState {
            traps: 0,
            ticks: 0,
            interrupts: 0,
            spurious: 0,
        }
    }

    /// Counts one reported trap.
    pub const fn record_trap(&mut self) {
        self.traps = self.traps.saturating_add(1);
    }

    /// Counts one timer tick.
    pub const fn record_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Counts one device interrupt.
    pub const fn record_interrupt(&mut self) {
        self.interrupts = self.interrupts.saturating_add(1);
    }

    /// Counts one spurious interrupt.
    pub const fn record_spurious(&mut self) {
        self.spurious = self.spurious.saturating_add(1);
    }
}

/// The one cell holding the kernel state.
pub static KERNEL: Global<KernelState> = Global::new();

/// Runs `body` with the kernel state, if it is reachable. It is not before
/// the boot sequence has stored it, and not while another borrow is alive,
/// which is what an interrupt inside a system call looks like.
pub fn with_state<R>(body: impl FnOnce(&mut KernelState) -> R) -> Option<R> {
    let mut state = KERNEL.borrow(&UncontendedToken).ok()?;
    Some(body(&mut state))
}
