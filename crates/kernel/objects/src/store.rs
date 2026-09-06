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
use crate::object::{
    AnyObjectId, Endpoint, EndpointId, Interrupt, IoPortRange, MemoryObject, Notification,
    NotificationId, Object, Process, ProcessId, Reply, ReplyId, Thread, ThreadId,
};
use crate::pool::{ObjectId, Pool, PoolError};

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
    /// The rendezvous points. Sized from [`config`] directly and not by a
    /// parameter: the five pools of Phase 6 stay under 200 KiB together,
    /// against the 1.2 MiB the four parameterized pools reach, so a host
    /// test can hold a machine with all of them at full size, and the
    /// signature of everything that touches the machine stays four
    /// parameters wide instead of nine.
    pub endpoints: Pool<Endpoint, { config::ENDPOINTS }>,
    /// The notifications.
    pub notifications: Pool<Notification, { config::NOTIFICATIONS }>,
    /// The reply objects.
    pub replies: Pool<Reply, { config::REPLIES }>,
    /// The interrupt objects.
    pub interrupts: Pool<Interrupt, { config::INTERRUPTS }>,
    /// The I/O port ranges.
    pub ports: Pool<IoPortRange, { config::IO_PORT_RANGES }>,
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
            endpoints: Pool::new(),
            notifications: Pool::new(),
            replies: Pool::new(),
            interrupts: Pool::new(),
            ports: Pool::new(),
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

    /// Does something to the process `id` names, if the machine holds it,
    /// and says whether it did.
    ///
    /// Every caller that changes a process it has already looked up goes
    /// through this: the lookup cannot fail there, and writing that out at
    /// each of them would be a branch per caller that nothing can reach.
    pub fn with_process(&mut self, id: ProcessId, body: impl FnOnce(&mut Process)) -> bool {
        match self.processes.get_mut(id) {
            Ok(process) => {
                body(process);
                true
            }
            Err(_) => false,
        }
    }

    /// Does something to the thread `id` names, if the machine holds it,
    /// and says whether it did.
    pub fn with_thread(&mut self, id: ThreadId, body: impl FnOnce(&mut Thread)) -> bool {
        match self.threads.get_mut(id) {
            Ok(thread) => {
                body(thread);
                true
            }
            Err(_) => false,
        }
    }

    /// How many objects of every kind the machine holds, in the order the
    /// eight pools are declared in, then the handle arena. This is what
    /// `system_info` reports beside [`Objects::capacities`].
    #[must_use]
    pub const fn counts(&self) -> [u32; 9] {
        [
            self.processes.live(),
            self.threads.live(),
            self.memory.live(),
            self.endpoints.live(),
            self.notifications.live(),
            self.replies.live(),
            self.interrupts.live(),
            self.ports.live(),
            self.handles.live(),
        ]
    }

    /// How many of each kind the machine could hold, in the order of
    /// [`Objects::counts`].
    #[must_use]
    pub fn capacities(&self) -> [u32; 9] {
        [
            self.processes.capacity(),
            self.threads.capacity(),
            self.memory.capacity(),
            self.endpoints.capacity(),
            self.notifications.capacity(),
            self.replies.capacity(),
            self.interrupts.capacity(),
            self.ports.capacity(),
            self.handles.capacity(),
        ]
    }

    /// Installs `entry` as a handle of `process` and returns the handle.
    ///
    /// The list of a process lives in the process object and the slots live
    /// in the arena, so an insertion reads the list out, changes it, and
    /// writes it back. That is one place and not one per caller.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the process is gone;
    /// [`Error::QuotaExceeded`] when it holds as many handles as its creator
    /// granted it; [`Error::OutOfHandles`] when the arena has no slot left.
    pub fn install_handle(&mut self, process: ProcessId, entry: Entry) -> Result<Handle, Error> {
        let mut list = self.processes.get(process)?.handles;
        let handle = self.handles.insert(process, &mut list, entry)?;
        self.processes.with(process, |held| held.handles = list);
        Ok(handle)
    }

    /// Removes `handle` from `process` and returns what it named.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the process is gone or holds no such
    /// handle.
    pub fn close_handle(&mut self, process: ProcessId, handle: Handle) -> Result<Entry, Error> {
        let mut list = self.processes.get(process)?.handles;
        let entry = self.handles.close(process, &mut list, handle)?;
        self.processes.with(process, |held| held.handles = list);
        Ok(entry)
    }

    /// Closes the first handle of `process` and returns what it named, or
    /// `None` when it holds none.
    ///
    /// One at a time, so that every entry a dying process held passes
    /// through the caller's release path.
    pub fn close_next_handle(&mut self, process: ProcessId) -> Option<Entry> {
        let mut list = self.processes.get(process).ok()?.handles;
        let entry = self.handles.close_next(&mut list);
        self.processes.with(process, |held| held.handles = list);
        entry
    }

    /// Adds one reference to the object `id` names, whichever pool holds
    /// it.
    ///
    /// The system control capability holds nothing and lives in no pool, so
    /// a reference to it is counted nowhere and this succeeds without
    /// looking anything up.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the id names no live object;
    /// [`Error::QuotaExceeded`] when the count is at its maximum.
    pub fn retain(&mut self, id: AnyObjectId) -> Result<(), Error> {
        let result = match id.object_type() {
            ObjectType::Process => self.processes.retain(id.typed()?),
            ObjectType::Thread => self.threads.retain(id.typed()?),
            ObjectType::MemoryObject => self.memory.retain(id.typed()?),
            ObjectType::Endpoint => self.endpoints.retain(id.typed()?),
            ObjectType::Notification => self.notifications.retain(id.typed()?),
            ObjectType::Reply => self.replies.retain(id.typed()?),
            ObjectType::Interrupt => self.interrupts.retain(id.typed()?),
            ObjectType::IoPortRange => self.ports.retain(id.typed()?),
            ObjectType::SystemControl => Ok(()),
        };
        result.map_err(Error::from)
    }

    /// Drops one reference to the object `id` names and says what was
    /// destroyed when that was the last one.
    ///
    /// This is the one place a reference count of zero turns into the
    /// destruction of an object. What destruction does to the threads that
    /// waited on the object is not done here: the waiters travel back to
    /// the caller in [`Destroyed`], because waking a thread needs the
    /// scheduler and writing its status word needs its IPC buffer, and
    /// neither is reachable from this crate.
    ///
    /// `None` says that the object lives on, or that the id named nothing.
    pub fn destroy(&mut self, id: AnyObjectId) -> Option<Destroyed> {
        match id.object_type() {
            ObjectType::Endpoint => {
                let endpoint = id.typed::<Endpoint>().ok()?;
                let held = *self.endpoints.get(endpoint).ok()?;
                gone(self.endpoints.release(endpoint))
                    .then_some(Destroyed::Endpoint(endpoint, held))
            }
            ObjectType::Notification => {
                let notification = id.typed::<Notification>().ok()?;
                let held = *self.notifications.get(notification).ok()?;
                gone(self.notifications.release(notification))
                    .then_some(Destroyed::Notification(notification, held))
            }
            ObjectType::Reply => {
                let reply = id.typed::<Reply>().ok()?;
                let held = *self.replies.get(reply).ok()?;
                gone(self.replies.release(reply)).then_some(Destroyed::Reply(reply, held))
            }
            ObjectType::Process => self.quietly(id, |objects, id| objects.processes.release(id)),
            ObjectType::Thread => self.quietly(id, |objects, id| objects.threads.release(id)),
            ObjectType::MemoryObject => self.quietly(id, |objects, id| objects.memory.release(id)),
            ObjectType::Interrupt => self.quietly(id, |objects, id| objects.interrupts.release(id)),
            ObjectType::IoPortRange => self.quietly(id, |objects, id| objects.ports.release(id)),
            // The system control capability lives in no pool: there is no
            // count to drop and nothing to destroy.
            ObjectType::SystemControl => None,
        }
    }

    /// Drops one reference in a pool whose objects no thread can wait on.
    fn quietly<T: Object>(
        &mut self,
        id: AnyObjectId,
        release: impl FnOnce(&mut Self, ObjectId<T>) -> Result<bool, PoolError>,
    ) -> Option<Destroyed> {
        let typed = id.typed::<T>().ok()?;
        gone(release(self, typed)).then_some(Destroyed::Quietly(id))
    }
}

/// `true` when a release took the last reference.
const fn gone(result: Result<bool, PoolError>) -> bool {
    matches!(result, Ok(true))
}

/// What [`Objects::destroy`] destroyed, and what it left waiting.
///
/// The three object types a thread can wait on carry the object away with
/// them, because the pool slot is free by the time the caller sees this and
/// the queues are the only record of who has to be woken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destroyed {
    /// An object nobody could have been waiting on.
    Quietly(AnyObjectId),
    /// An endpoint, with the two queues it held.
    Endpoint(EndpointId, Endpoint),
    /// A notification, with the waiter it held.
    Notification(NotificationId, Notification),
    /// A reply object, with the caller that waits for its answer.
    Reply(ReplyId, Reply),
}

impl Destroyed {
    /// The object that was destroyed.
    #[must_use]
    pub const fn id(&self) -> AnyObjectId {
        match self {
            Destroyed::Quietly(id) => *id,
            Destroyed::Endpoint(id, _) => AnyObjectId::of(*id),
            Destroyed::Notification(id, _) => AnyObjectId::of(*id),
            Destroyed::Reply(id, _) => AnyObjectId::of(*id),
        }
    }
}
