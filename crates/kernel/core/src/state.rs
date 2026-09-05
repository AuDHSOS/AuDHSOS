// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The global kernel state.
//!
//! Invariant: there is one cell, initialized once during boot, and every
//! access goes through the borrow flag of [`audhsos_sync::Global`].

use audhsos_sync::Global;

/// Everything the kernel keeps between system calls. Phase 2 keeps the
/// number of traps the kernel has reported and nothing else; the object
/// pools, the address spaces, and the scheduler arrive in later phases.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KernelState {
    /// How many traps the kernel has reported.
    pub traps: u64,
}

impl KernelState {
    /// A state that has seen nothing.
    #[must_use]
    pub const fn new() -> Self {
        KernelState { traps: 0 }
    }

    /// Counts one reported trap.
    pub const fn record_trap(&mut self) {
        self.traps = self.traps.saturating_add(1);
    }
}

/// The one cell holding the kernel state.
pub static KERNEL: Global<KernelState> = Global::new();
