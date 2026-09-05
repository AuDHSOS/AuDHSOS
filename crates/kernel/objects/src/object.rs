// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The objects the kernel holds and the ids that name them.
//!
//! Invariants: every object type of the ABI that this kernel holds has
//! exactly one structure here and one [`Object`] implementation, so an id
//! and a type always agree; an [`AnyObjectId`] carries the type of the
//! object it names, and turning one back into a typed id checks that type.

use audhsos_abi::layout::{PRIORITY_COUNT, REGIONS_PER_PROCESS, THREADS_PER_PROCESS};
use audhsos_abi::{Error, ObjectType, ThreadState};
use kernel_mm::address_space::RegionTable;
use kernel_types::{CachePolicy, PhysFrame, PhysFrameRange, VirtAddr};

use crate::handle_table::HandleList;
use crate::pool::ObjectId;
use crate::quota::Quota;

/// An object the kernel holds in a pool. The associated type code is what
/// lets an [`AnyObjectId`] be checked against the type an operation needs.
pub trait Object {
    /// The type of the object in the ABI.
    const TYPE: ObjectType;
}

/// The name of a process.
pub type ProcessId = ObjectId<Process>;

/// The name of a thread.
pub type ThreadId = ObjectId<Thread>;

/// The name of a memory object.
pub type MemoryObjectId = ObjectId<MemoryObject>;

/// An object id with the type of its object, as a handle table entry holds
/// it. The typed id is recovered with [`AnyObjectId::typed`], which is
/// where the type of an operation is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnyObjectId {
    object_type: ObjectType,
    index: u32,
    generation: u32,
}

impl AnyObjectId {
    /// The untyped form of `id`.
    #[must_use]
    pub const fn of<T: Object>(id: ObjectId<T>) -> Self {
        AnyObjectId {
            object_type: T::TYPE,
            index: id.index(),
            generation: id.generation(),
        }
    }

    /// The type of the object this id names.
    #[must_use]
    pub const fn object_type(self) -> ObjectType {
        self.object_type
    }

    /// The typed id, if the object has the type `T`.
    ///
    /// # Errors
    ///
    /// [`Error::WrongObjectType`] when the object has another type.
    pub const fn typed<T: Object>(self) -> Result<ObjectId<T>, Error> {
        if self.object_type.code() == T::TYPE.code() {
            Ok(ObjectId::new(self.index, self.generation))
        } else {
            Err(Error::WrongObjectType)
        }
    }

    /// The slot index of the object, which says nothing about what the
    /// object is.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation of the object's pool slot.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// What a memory object is backed by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MemoryKind {
    /// Ordinary memory the root task handed out.
    #[default]
    Ram,
    /// A device aperture the root task created from `SystemControl`.
    Device,
}

impl MemoryKind {
    /// The name of the kind.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            MemoryKind::Ram => "Ram",
            MemoryKind::Device => "Device",
        }
    }
}

/// A range of physical memory a process may map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemoryObject {
    /// The frames the object covers.
    pub frames: PhysFrameRange,
    /// What the frames are.
    pub kind: MemoryKind,
    /// How the processor may cache the frames.
    pub cache: CachePolicy,
}

impl Object for MemoryObject {
    const TYPE: ObjectType = ObjectType::MemoryObject;
}

impl MemoryObject {
    /// A memory object over `frames`.
    #[must_use]
    pub const fn new(frames: PhysFrameRange, kind: MemoryKind, cache: CachePolicy) -> Self {
        MemoryObject {
            frames,
            kind,
            cache,
        }
    }

    /// How many frames the object covers.
    #[must_use]
    pub const fn frame_count(&self) -> u64 {
        self.frames.count()
    }
}

/// An address space, a handle list, threads, and quotas.
///
/// The address space is the page-table root and the region table, both
/// fields of this structure: a process has exactly one address space,
/// nothing shares one, and there is therefore no address-space object and
/// no id for one (D-65).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    /// The page-table root of the address space.
    pub root: PhysFrame,
    /// What the address space maps, and what backs each range.
    pub regions: RegionTable<MemoryObjectId, REGIONS_PER_PROCESS>,
    /// The handles the process holds and the ceiling its creator granted.
    pub handles: HandleList,
    /// The threads of the process.
    pub threads: [Option<ThreadId>; THREADS_PER_PROCESS],
    /// The frames of the kernel reserve the process may still take.
    pub quota: Quota,
    /// The kernel objects the process may still create.
    pub kernel_object_quota: Quota,
    /// Where faults of this process are reported, once endpoints exist.
    /// Phase 5 leaves it empty and every fault stops its thread.
    pub fault_handler: Option<AnyObjectId>,
}

impl Object for Process {
    const TYPE: ObjectType = ObjectType::Process;
}

impl Process {
    /// A process with the given address space root and quotas, and no
    /// thread yet.
    #[must_use]
    pub const fn new(
        root: PhysFrame,
        handles: HandleList,
        quota: Quota,
        kernel_object_quota: Quota,
    ) -> Self {
        Process {
            root,
            regions: RegionTable::new(),
            handles,
            threads: [None; THREADS_PER_PROCESS],
            quota,
            kernel_object_quota,
            fault_handler: None,
        }
    }

    /// Records `thread` as a thread of this process.
    ///
    /// # Errors
    ///
    /// [`Error::QuotaExceeded`] when the process already holds
    /// [`THREADS_PER_PROCESS`] threads.
    pub fn add_thread(&mut self, thread: ThreadId) -> Result<(), Error> {
        let slot = self
            .threads
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(Error::QuotaExceeded)?;
        *slot = Some(thread);
        Ok(())
    }

    /// Forgets `thread`. Returns `false` when the process does not hold it.
    pub fn remove_thread(&mut self, thread: ThreadId) -> bool {
        let Some(slot) = self.threads.iter_mut().find(|slot| **slot == Some(thread)) else {
            return false;
        };
        *slot = None;
        true
    }

    /// The threads of the process, in slot order.
    pub fn threads(&self) -> impl Iterator<Item = ThreadId> + '_ {
        self.threads.iter().flatten().copied()
    }

    /// How many threads the process holds.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.threads().count()
    }

    /// Whether the region table of the process has room for `count` more
    /// regions.
    #[must_use]
    pub const fn has_region_room(&self, count: usize) -> bool {
        self.regions.free_slots() >= count
    }
}

/// The links that chain a thread into one scheduler queue.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Links {
    /// The next thread of the queue, if any.
    pub next: Option<ThreadId>,
    /// The previous thread of the queue, if any.
    pub previous: Option<ThreadId>,
}

impl Links {
    /// A thread that is in no queue.
    pub const UNLINKED: Links = Links {
        next: None,
        previous: None,
    };

    /// `true` if the thread is in no queue.
    #[must_use]
    pub const fn is_unlinked(self) -> bool {
        self.next.is_none() && self.previous.is_none()
    }
}

/// A thread of a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thread {
    /// The process the thread belongs to.
    pub process: ProcessId,
    /// What the thread is doing.
    pub state: ThreadState,
    /// The priority the scheduler runs it at.
    pub priority: u8,
    /// The highest priority the thread may ever be given.
    pub max_priority: u8,
    /// Ticks left of its time slice.
    pub time_slice: u32,
    /// The slot of the kernel stack area the thread's stack occupies.
    pub kernel_stack: u32,
    /// The frame holding the thread's IPC buffer.
    pub ipc_buffer: PhysFrame,
    /// Where the thread starts, in its own address space.
    pub entry: VirtAddr,
    /// The stack pointer the thread starts with, in its own address space.
    pub user_stack: VirtAddr,
    /// The kernel stack pointer of the thread while it is not running.
    /// One word is the whole saved context (D-67); everything else the
    /// switch has to keep lies on the stack this points at.
    pub context: VirtAddr,
    /// The queue the scheduler has it in.
    pub links: Links,
}

impl Object for Thread {
    const TYPE: ObjectType = ObjectType::Thread;
}

impl Thread {
    /// A thread that has not started yet.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] when `priority` is above `max_priority`
    /// or `max_priority` is not a priority of this system.
    pub const fn new(
        process: ProcessId,
        priority: u8,
        max_priority: u8,
        kernel_stack: u32,
        ipc_buffer: PhysFrame,
    ) -> Result<Self, Error> {
        if max_priority >= PRIORITY_COUNT || priority > max_priority {
            return Err(Error::InvalidArgument);
        }
        Ok(Thread {
            process,
            state: ThreadState::Inactive,
            priority,
            max_priority,
            time_slice: 0,
            kernel_stack,
            ipc_buffer,
            entry: VirtAddr::ZERO,
            user_stack: VirtAddr::ZERO,
            context: VirtAddr::ZERO,
            links: Links::UNLINKED,
        })
    }

    /// The same thread, starting at `entry` on `user_stack`.
    #[must_use]
    pub const fn starting_at(self, entry: VirtAddr, user_stack: VirtAddr) -> Self {
        Thread {
            entry,
            user_stack,
            ..self
        }
    }

    /// `true` if the scheduler may pick the thread.
    #[must_use]
    pub const fn is_runnable(&self) -> bool {
        self.state.is_runnable()
    }
}
