// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Handles: an index and the generation of the slot it names.
//!
//! A bare index is a handle that comes back to life. A socket is closed,
//! the slot is used again by something else, and a caller that kept the
//! old number is now reading and writing the new socket without an error
//! anywhere. That is the same bug as a dangling pointer and it is caught
//! the same way the kernel's handles catch it: the slot counts how often
//! it has been reused, the handle carries the count it was made under,
//! and the two have to agree.
//!
//! A generation that wraps is a handle that could come back to life after
//! sixty-five thousand reuses of one slot. The counter saturates instead:
//! a slot that has been through that many is a slot that hands out no
//! more handles, which costs one slot of the table and is the only answer
//! that keeps the guarantee.

use crate::error::StackError;

/// Which slot, and which use of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Handle {
    /// Which slot of the table.
    index: u16,
    /// Which use of that slot.
    generation: u16,
}

impl Handle {
    /// Which slot it names.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.index)
    }

    /// Which use of that slot it was made under.
    #[must_use]
    pub const fn generation(self) -> u16 {
        self.generation
    }
}

/// The generations of one table's slots.
#[derive(Debug)]
pub struct Slots<const N: usize> {
    /// How often each slot has been used.
    generations: [u16; N],
}

impl<const N: usize> Default for Slots<N> {
    fn default() -> Slots<N> {
        Slots::new()
    }
}

impl<const N: usize> Slots<N> {
    /// A table whose slots have all been used once.
    #[must_use]
    pub const fn new() -> Slots<N> {
        Slots {
            generations: [1u16; N],
        }
    }

    /// A handle for the slot at `index`, as it stands now.
    ///
    /// # Errors
    ///
    /// [`StackError::Unknown`] when there is no such slot, and
    /// [`StackError::Full`] when the slot has been reused as often as a
    /// generation can count.
    pub fn handle(&self, index: usize) -> Result<Handle, StackError> {
        handle_of(&self.generations, index)
    }

    /// Which slot `handle` names, when it still names one.
    ///
    /// # Errors
    ///
    /// [`StackError::Unknown`] when there is no such slot, and
    /// [`StackError::Stale`] when the slot has been used again since.
    pub fn resolve(&self, handle: Handle) -> Result<usize, StackError> {
        resolve_in(&self.generations, handle)
    }

    /// Notes that the slot at `index` has been given up, so that every
    /// handle to it is now stale.
    pub fn retire(&mut self, index: usize) {
        retire_in(&mut self.generations, index);
    }
}

/// The three operations are written over a slice and not over the array,
/// so that a table of four slots and a table of two are one piece of
/// code. What a generic would buy here is nothing, and what it costs is
/// the same branches compiled and measured once per size.
fn handle_of(generations: &[u16], index: usize) -> Result<Handle, StackError> {
    let Ok(index) = u16::try_from(index) else {
        return Err(StackError::Unknown);
    };
    let Some(generation) = generations.get(usize::from(index)).copied() else {
        return Err(StackError::Unknown);
    };
    if generation == u16::MAX {
        return Err(StackError::Full);
    }
    Ok(Handle { index, generation })
}

/// Which slot `handle` names, when it still names one.
fn resolve_in(generations: &[u16], handle: Handle) -> Result<usize, StackError> {
    let index = handle.index();
    let Some(generation) = generations.get(index).copied() else {
        return Err(StackError::Unknown);
    };
    if generation == handle.generation {
        Ok(index)
    } else {
        Err(StackError::Stale)
    }
}

/// Notes that the slot at `index` has been given up.
fn retire_in(generations: &mut [u16], index: usize) {
    if let Some(generation) = generations.get_mut(index) {
        *generation = generation.saturating_add(1);
    }
}
