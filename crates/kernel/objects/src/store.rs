// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every object the machine holds, in one structure.
//!
//! Invariants: the structure is all zeros when empty and its constructor is
//! `const`, so the kernel keeps it in one `static` cell without ever
//! building it on a stack (D-66); every handle the arena holds names an
//! object of one of these pools, and a lookup that returns an id has
//! checked the type of that object.

use audhsos_abi::{Error, Handle, ObjectType, Rights};

use crate::config;
use crate::handle_table::{Entry, HandleArena};
use crate::object::{MemoryObject, Object, Process, ProcessId, Thread, ThreadId};
use crate::pool::{ObjectId, Pool};

/// Everything the kernel keeps about objects.
///
/// The sizes are parameters rather than the constants of [`crate::config`]
/// so that a test can hold a machine of a handful of slots. The kernel uses
/// [`MachineObjects`], which fills them in from the configuration; nothing
/// else should name the parameters directly.
#[derive(Debug)]
pub struct Objects<
    const PROCESSES: usize,
    const THREADS: usize,
    const MEMORY_OBJECTS: usize,
    const HANDLE_ENTRIES: usize,
> {
    /// The processes.
    pub processes: Pool<Process, PROCESSES>,
    /// The threads.
    pub threads: Pool<Thread, THREADS>,
    /// The memory objects.
    pub memory: Pool<MemoryObject, MEMORY_OBJECTS>,
    /// The handle slots of every process.
    pub handles: HandleArena<HANDLE_ENTRIES>,
}

/// The objects of this machine, sized by [`crate::config`]. One `static`
/// of this type is what the kernel holds.
pub type MachineObjects = Objects<
    { config::PROCESSES },
    { config::THREADS },
    { config::MEMORY_OBJECTS },
    { config::HANDLE_ENTRIES },
>;

impl<const NP: usize, const NT: usize, const NM: usize, const NH: usize> Default
    for Objects<NP, NT, NM, NH>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const NP: usize, const NT: usize, const NM: usize, const NH: usize> Objects<NP, NT, NM, NH> {
    /// A machine that holds nothing. All zeros, and `const`, so the cell
    /// that holds it is `.bss`.
    #[must_use]
    pub const fn new() -> Self {
        Objects {
            processes: Pool::new(),
            threads: Pool::new(),
            memory: Pool::new(),
            handles: HandleArena::new(),
        }
    }

    /// What `handle` of `process` names, with the rights it carries.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the handle names no live entry of this
    /// process.
    pub fn entry(&self, process: ProcessId, handle: Handle) -> Result<Entry, Error> {
        self.handles.lookup(process, handle).copied()
    }

    /// The object of type `T` that `handle` of `process` names, and the
    /// rights the handle carries.
    ///
    /// The order of the checks is the order of the system call interface:
    /// the handle first, then the type, then the rights. `required` is
    /// what the operation needs; [`Rights::EMPTY`] asks for nothing.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the handle names no live entry of this
    /// process; [`Error::WrongObjectType`] when the object has another
    /// type; [`Error::AccessDenied`] when the handle lacks a right.
    pub fn resolve<T: Object>(
        &self,
        process: ProcessId,
        handle: Handle,
        required: Rights,
    ) -> Result<(ObjectId<T>, Rights), Error> {
        let entry = self.entry(process, handle)?;
        let id = entry.object.typed::<T>()?;
        if !entry.rights.contains(required) {
            return Err(Error::AccessDenied);
        }
        Ok((id, entry.rights))
    }

    /// The type of the object `handle` of `process` names, for the calls
    /// that accept a handle of any type.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] as [`Objects::entry`].
    pub fn object_type(&self, process: ProcessId, handle: Handle) -> Result<ObjectType, Error> {
        Ok(self.entry(process, handle)?.object_type())
    }

    /// `true` if `id` names a live process.
    #[must_use]
    pub fn holds_process(&self, id: ProcessId) -> bool {
        self.processes.get(id).is_ok()
    }

    /// `true` if `id` names a live thread.
    #[must_use]
    pub fn holds_thread(&self, id: ThreadId) -> bool {
        self.threads.get(id).is_ok()
    }

    /// How many objects of every kind the machine holds, in the order
    /// processes, threads, memory objects, handles.
    #[must_use]
    pub const fn counts(&self) -> [u32; 4] {
        [
            self.processes.live(),
            self.threads.live(),
            self.memory.live(),
            self.handles.live(),
        ]
    }
}
