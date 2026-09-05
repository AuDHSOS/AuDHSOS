// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![allow(unsafe_code)]
#![doc = include_str!("../README.md")]

//! Invariants: at most one [`GlobalRef`] to a given [`Global`] exists at any
//! time; the value is written exactly once, before any borrow succeeds. The
//! same holds for [`PresetRef`] and [`Preset`], whose value is there from
//! the start and is never written by the cell at all.

use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// Errors of [`Global`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `init` was called on a cell that already holds a value.
    AlreadyInitialized,
    /// `borrow` was called before `init`.
    Uninitialized,
    /// Another [`GlobalRef`] to the same cell is still alive.
    AlreadyBorrowed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::AlreadyInitialized => "the global cell already holds a value",
            Error::Uninitialized => "the global cell holds no value yet",
            Error::AlreadyBorrowed => "the global cell is already borrowed",
        })
    }
}

/// Proof, carried by the caller, that nothing else can run on this CPU while
/// the borrow lives. The kernel implements it for its interrupt guard; the
/// userland runtime and tests use [`UncontendedToken`].
pub trait ExclusiveToken {}

/// The token for contexts with a single thread of execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct UncontendedToken;

impl ExclusiveToken for UncontendedToken {}

/// A cell for global state.
pub struct Global<T> {
    borrowed: AtomicBool,
    value: UnsafeCell<Option<T>>,
}

// SAFETY: the value is reached only through `slot`, which is called only
// while the caller holds the `borrowed` flag. The flag is taken with an
// atomic compare-and-swap, so at most one thread of execution reaches the
// value at a time, which is the guarantee `Sync` requires. `T: Send` keeps
// values that must stay on one thread out of the cell.
unsafe impl<T: Send> Sync for Global<T> {}

impl<T> Global<T> {
    /// An empty cell. Usable in a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Global {
            borrowed: AtomicBool::new(false),
            value: UnsafeCell::new(None),
        }
    }

    /// Stores the value. Succeeds exactly once.
    ///
    /// # Errors
    ///
    /// `AlreadyInitialized` after the first success; `AlreadyBorrowed`
    /// while a borrow is alive.
    pub fn init(&self, value: T) -> Result<(), Error> {
        self.acquire()?;
        let slot = self.slot();
        let result = if slot.is_some() {
            Err(Error::AlreadyInitialized)
        } else {
            *slot = Some(value);
            Ok(())
        };
        self.release();
        result
    }

    /// Borrows the value exclusively.
    ///
    /// # Errors
    ///
    /// `AlreadyBorrowed` while another borrow is alive; `Uninitialized`
    /// before `init`.
    pub fn borrow<'a>(&'a self, _token: &impl ExclusiveToken) -> Result<GlobalRef<'a, T>, Error> {
        self.acquire()?;
        if let Some(value) = self.slot().as_mut() {
            Ok(GlobalRef { cell: self, value })
        } else {
            self.release();
            Err(Error::Uninitialized)
        }
    }

    /// `true` while a [`GlobalRef`] is alive.
    #[must_use]
    pub fn is_borrowed(&self) -> bool {
        self.borrowed.load(Ordering::Acquire)
    }

    fn acquire(&self) -> Result<(), Error> {
        self.borrowed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::AlreadyBorrowed)
    }

    fn release(&self) {
        self.borrowed.store(false, Ordering::Release);
    }

    /// The slot behind the cell. Callers hold the `borrowed` flag, which
    /// makes them the only code that can reach the slot until they release
    /// it.
    #[expect(
        clippy::mut_from_ref,
        reason = "interior mutability guarded by the borrowed flag"
    )]
    fn slot(&self) -> &mut Option<T> {
        // SAFETY: `acquire` succeeded for the caller and has not been
        // released, so no other reference to the slot exists: `borrow`
        // hands its reference to a `GlobalRef` that releases the flag only
        // when it is dropped, and `init` drops its reference before it
        // releases. The pointer comes from a live `UnsafeCell` and is
        // therefore valid and aligned.
        unsafe { &mut *self.value.get() }
    }
}

impl<T> Default for Global<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> fmt::Debug for Global<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Global")
            .field("borrowed", &self.is_borrowed())
            .finish_non_exhaustive()
    }
}

/// Exclusive access to the value of a [`Global`]. Releases the cell on drop.
pub struct GlobalRef<'a, T> {
    cell: &'a Global<T>,
    value: &'a mut T,
}

impl<T> Deref for GlobalRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for GlobalRef<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T> Drop for GlobalRef<'_, T> {
    fn drop(&mut self) {
        self.cell.release();
    }
}

impl<T: fmt::Debug> fmt::Debug for GlobalRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A cell whose value is there from the start: no initialization step, no
/// `Option`, and a `const` constructor.
///
/// [`Global`] holds an `Option<T>` and takes its value by move, which is
/// what the kernel's object pools cannot afford: a pool is far larger than
/// the boot stack it would travel over, and `Some(value)` in a `static`
/// carries a non-zero discriminant, which would move the whole structure
/// out of the `.bss` and into the image file. A `Preset` holds the value
/// itself, so a `static Preset<T>` whose `T` is all zeros is `.bss` and
/// costs the image nothing (D-66).
///
/// Borrowing follows the same rule as [`Global`]: one borrow at a time,
/// against a token that proves nothing else runs on this CPU.
pub struct Preset<T> {
    borrowed: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the value is reached only through `slot`, which is called only
// while the caller holds the `borrowed` flag. The flag is taken with an
// atomic compare-and-swap, so at most one thread of execution reaches the
// value at a time, which is the guarantee `Sync` requires. `T: Send` keeps
// values that must stay on one thread out of the cell.
unsafe impl<T: Send> Sync for Preset<T> {}

impl<T> Preset<T> {
    /// A cell holding `value`. Usable in a `static`.
    #[must_use]
    pub const fn new(value: T) -> Self {
        Preset {
            borrowed: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Borrows the value exclusively.
    ///
    /// # Errors
    ///
    /// [`Error::AlreadyBorrowed`] while another borrow is alive.
    pub fn borrow<'a>(&'a self, _token: &impl ExclusiveToken) -> Result<PresetRef<'a, T>, Error> {
        self.acquire()?;
        Ok(PresetRef {
            cell: self,
            value: self.slot(),
        })
    }

    /// `true` while a [`PresetRef`] is alive.
    #[must_use]
    pub fn is_borrowed(&self) -> bool {
        self.borrowed.load(Ordering::Acquire)
    }

    /// The value, when nothing can be borrowing it because the caller owns
    /// the cell.
    pub const fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    fn acquire(&self) -> Result<(), Error> {
        self.borrowed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| Error::AlreadyBorrowed)
    }

    fn release(&self) {
        self.borrowed.store(false, Ordering::Release);
    }

    /// The value behind the cell. Callers hold the `borrowed` flag, which
    /// makes them the only code that can reach it until they release it.
    #[expect(
        clippy::mut_from_ref,
        reason = "interior mutability guarded by the borrowed flag"
    )]
    fn slot(&self) -> &mut T {
        // SAFETY: `acquire` succeeded for the caller and has not been
        // released, so no other reference to the value exists: `borrow`
        // hands its reference to a `PresetRef` that releases the flag only
        // when it is dropped. The pointer comes from a live `UnsafeCell`
        // and is therefore valid and aligned.
        unsafe { &mut *self.value.get() }
    }
}

impl<T: Default> Default for Preset<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> fmt::Debug for Preset<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Preset")
            .field("borrowed", &self.is_borrowed())
            .finish_non_exhaustive()
    }
}

/// Exclusive access to the value of a [`Preset`]. Releases the cell on
/// drop.
pub struct PresetRef<'a, T> {
    cell: &'a Preset<T>,
    value: &'a mut T,
}

impl<T> Deref for PresetRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T> DerefMut for PresetRef<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T> Drop for PresetRef<'_, T> {
    fn drop(&mut self) {
        self.cell.release();
    }
}

impl<T: fmt::Debug> fmt::Debug for PresetRef<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[cfg(test)]
mod tests;
