// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Mapping a memory object into the program's own address space, and the
//! bytes behind it.
//!
//! This is the one thing a program cannot do without a pointer: it asks the
//! kernel to put an object at an address, and then it has to read and write
//! the bytes that are now there. Everywhere else in the userland an address
//! is a number nobody dereferences.
//!
//! A mapping takes the whole object, in as many calls as the kernel needs:
//! `memory_map` does at most [`MAX_PAGES_PER_CALL`] pages at a time and
//! says how far it came, so the loop asks again from there.
//!
//! Invariant: the address space region a [`Mapping`] holds is taken back by
//! [`Mapping::unmap`] and by nothing else; a program that drops one without
//! unmapping it keeps the region, which is a leak and not a fault.

use audhsos_abi::Error;
use audhsos_abi::layout::{MAX_PAGES_PER_CALL, PAGE_SIZE};
use user_rt::{MemoryHandle, ProcessHandle};
use user_sys_x86_64::Gate;

use crate::permissions;

/// Where a program maps what it has to reach the bytes of.
///
/// It lies far above where any program of this system is linked and far
/// below the IPC buffers at the top of the address space, so a window here
/// meets nothing.
pub const SCRATCH: u64 = 0x0000_4000_0000_0000;

/// A memory object in the program's own address space.
#[derive(Debug)]
pub struct Mapping {
    address: u64,
    len: u64,
}

impl Mapping {
    /// Maps `object`, whose length is `len`, at `address`, readable and
    /// writable.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered. A call that fails halfway leaves what
    /// it mapped behind; the caller unmaps the window it asked for.
    pub fn new(
        gate: &mut Gate,
        process: ProcessHandle,
        object: MemoryHandle,
        address: u64,
        len: u64,
    ) -> Result<Self, Error> {
        Self::window(gate, process, object, address, 0, len)
    }

    /// Maps `len` bytes of `object`, from `offset`, at `address`.
    ///
    /// This is what a program uses when the object is larger than the
    /// window it wants to look at it through.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn window(
        gate: &mut Gate,
        process: ProcessHandle,
        object: MemoryHandle,
        address: u64,
        offset: u64,
        len: u64,
    ) -> Result<Self, Error> {
        map_all(gate, process, object, address, offset, len)?;
        Ok(Mapping { address, len })
    }

    /// Where the object lies.
    #[must_use]
    pub const fn address(&self) -> u64 {
        self.address
    }

    /// How many bytes it covers.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.len
    }

    /// `true` for a mapping of no bytes, which nothing here makes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes of the mapping.
    ///
    /// # Safety
    ///
    /// The mapping must still be there: the caller may not have unmapped it
    /// and nothing else may hold a reference to the same bytes.
    #[must_use]
    pub unsafe fn bytes(&mut self) -> &mut [u8] {
        let start = usize::try_from(self.address).unwrap_or(0);
        let len = usize::try_from(self.len).unwrap_or(0);
        let pointer = core::ptr::without_provenance_mut::<u8>(start);
        // SAFETY: the kernel mapped `len` bytes there, readable and
        // writable, for this process alone, and the caller promises the
        // mapping is still standing and unshared.
        unsafe { core::slice::from_raw_parts_mut(pointer, len) }
    }

    /// Fills the mapping with zeros.
    ///
    /// # Safety
    ///
    /// As [`bytes`](Self::bytes).
    pub unsafe fn zero(&mut self) {
        // SAFETY: the caller promises what `bytes` requires.
        let bytes = unsafe { self.bytes() };
        bytes.fill(0);
    }

    /// Copies `source` into the mapping at `offset`, as far as it fits.
    ///
    /// # Safety
    ///
    /// As [`bytes`](Self::bytes).
    pub unsafe fn copy_in(&mut self, offset: u64, source: &[u8]) {
        let at = usize::try_from(offset).unwrap_or(usize::MAX);
        // SAFETY: the caller promises what `bytes` requires.
        let bytes = unsafe { self.bytes() };
        let end = at.saturating_add(source.len()).min(bytes.len());
        if let (Some(slot), Some(from)) =
            (bytes.get_mut(at..end), source.get(..end.saturating_sub(at)))
        {
            slot.copy_from_slice(from);
        }
    }

    /// Takes the mapping out of the address space again.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered.
    pub fn unmap(self, gate: &mut Gate, process: ProcessHandle) -> Result<(), Error> {
        unmap_all(gate, process, self.address, self.len)
    }
}

/// Maps `len` bytes of `object` at `address`, in as many calls as the
/// kernel needs.
fn map_all(
    gate: &mut Gate,
    process: ProcessHandle,
    object: MemoryHandle,
    address: u64,
    offset: u64,
    len: u64,
) -> Result<(), Error> {
    let mut done = 0u64;
    while done < len {
        let rest = len.wrapping_sub(done);
        let chunk = rest.min(MAX_PAGES_PER_CALL.wrapping_mul(PAGE_SIZE));
        let pages = gate.memory_map(
            process,
            object,
            address.wrapping_add(done),
            offset.wrapping_add(done),
            chunk,
            permissions::WRITE,
        )?;
        let moved = pages.wrapping_mul(PAGE_SIZE);
        if moved == 0 {
            return Err(Error::NotMapped);
        }
        done = done.wrapping_add(moved);
    }
    Ok(())
}

/// Takes `len` bytes at `address` out, in as many calls as the kernel
/// needs.
fn unmap_all(gate: &mut Gate, process: ProcessHandle, address: u64, len: u64) -> Result<(), Error> {
    let mut done = 0u64;
    while done < len {
        let rest = len.wrapping_sub(done);
        let chunk = rest.min(MAX_PAGES_PER_CALL.wrapping_mul(PAGE_SIZE));
        let pages = gate.memory_unmap(process, address.wrapping_add(done), chunk)?;
        let moved = pages.wrapping_mul(PAGE_SIZE);
        if moved == 0 {
            return Err(Error::NotMapped);
        }
        done = done.wrapping_add(moved);
    }
    Ok(())
}
