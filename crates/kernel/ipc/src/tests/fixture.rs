// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The small machine the tests of this crate run on: processes, the threads
//! a test asks for, and the handles it installs by hand the way the root
//! task will.

use audhsos_abi::ipc_buffer::{BufferMut, SIZE};
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Handle, Rights, ThreadState};
use kernel_objects::handle_table::{Entry, HandleList};
use kernel_objects::object::{
    AnyObjectId, Endpoint, EndpointId, MemoryKind, MemoryObject, MemoryObjectId, Notification,
    NotificationId, Process, ProcessId, Thread, ThreadId,
};
use kernel_objects::quota::Quota;
use kernel_objects::store::Objects;
use kernel_sched::Scheduler;
use kernel_types::{CachePolicy, PhysAddr, PhysFrame, PhysFrameRange};

/// A machine of a handful of slots, which a test can hold on its stack.
pub(super) type Small = Objects<4, 16, 8, 64>;

/// The machine and the run queues over it.
pub(super) struct Fixture {
    pub(super) objects: Small,
    pub(super) scheduler: Scheduler,
    next_frame: u64,
}

impl Fixture {
    /// An empty machine.
    pub(super) fn new() -> Self {
        Fixture {
            objects: Small::new(),
            scheduler: Scheduler::new(),
            next_frame: 0x100,
        }
    }

    /// A frame nothing else uses.
    pub(super) fn frame(&mut self) -> PhysFrame {
        self.next_frame = self.next_frame.saturating_add(1);
        PhysFrame::containing(PhysAddr::new(self.next_frame.saturating_mul(PAGE_SIZE)).unwrap())
    }

    /// A process with room for `handles` handles.
    pub(super) fn process(&mut self, handles: u32) -> ProcessId {
        let root = self.frame();
        self.objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(handles),
                Quota::new(64),
                Quota::new(64),
            ))
            .unwrap()
    }

    /// An inactive thread of `process` at `priority`.
    pub(super) fn thread(&mut self, process: ProcessId, priority: u8) -> ThreadId {
        let buffer = self.frame();
        let thread = self
            .objects
            .threads
            .allocate(Thread::new(process, priority, 31, 0, buffer).unwrap())
            .unwrap();
        self.objects
            .processes
            .get_mut(process)
            .unwrap()
            .add_thread(thread)
            .unwrap();
        thread
    }

    /// Puts `thread` on the processor, which is what a thread that has just
    /// made a system call is: `Running`, in no queue, and the one the
    /// scheduler calls current.
    pub(super) fn on_the_processor(&mut self, thread: ThreadId) {
        self.objects.threads.get_mut(thread).unwrap().state = ThreadState::Running;
        self.scheduler.adopt(thread);
    }

    /// A thread of `process` at `priority`, on the processor.
    pub(super) fn running(&mut self, process: ProcessId, priority: u8) -> ThreadId {
        let thread = self.thread(process, priority);
        self.on_the_processor(thread);
        thread
    }

    /// The state of `thread`, or `None` when the pool no longer holds it.
    pub(super) fn state(&self, thread: ThreadId) -> Option<ThreadState> {
        self.objects.threads.get(thread).ok().map(|held| held.state)
    }

    /// What `thread` waits on.
    pub(super) fn wait_of(&self, thread: ThreadId) -> kernel_objects::object::Wait {
        self.objects.threads.get(thread).unwrap().wait
    }

    /// An endpoint nobody waits on.
    pub(super) fn endpoint(&mut self) -> EndpointId {
        self.objects.endpoints.allocate(Endpoint::new()).unwrap()
    }

    /// The threads waiting to send on `endpoint`, in order.
    pub(super) fn senders(&self, endpoint: EndpointId) -> Vec<ThreadId> {
        self.objects
            .endpoints
            .get(endpoint)
            .unwrap()
            .senders
            .iter(&self.objects.threads)
            .collect()
    }

    /// The threads waiting to receive on `endpoint`, in order.
    pub(super) fn receivers(&self, endpoint: EndpointId) -> Vec<ThreadId> {
        self.objects
            .endpoints
            .get(endpoint)
            .unwrap()
            .receivers
            .iter(&self.objects.threads)
            .collect()
    }

    /// A notification with nothing signalled.
    pub(super) fn notification(&mut self) -> NotificationId {
        self.objects
            .notifications
            .allocate(Notification::new())
            .unwrap()
    }

    /// A memory object of one frame.
    pub(super) fn memory(&mut self) -> MemoryObjectId {
        let start = self.frame();
        self.objects
            .memory
            .allocate(MemoryObject::new(
                PhysFrameRange::new(start, 1).unwrap(),
                MemoryKind::Ram,
                CachePolicy::WriteBack,
            ))
            .unwrap()
    }

    /// Installs a handle to `object` with `rights` in `process`.
    pub(super) fn install(
        &mut self,
        process: ProcessId,
        object: AnyObjectId,
        rights: Rights,
    ) -> Handle {
        let mut list = self.objects.processes.get(process).unwrap().handles;
        let handle = self
            .objects
            .handles
            .insert(process, &mut list, Entry::new(object, rights))
            .unwrap();
        self.objects.processes.get_mut(process).unwrap().handles = list;
        handle
    }

    /// Installs handles until the list of `process` is full.
    pub(super) fn fill_handles(&mut self, process: ProcessId, filler: AnyObjectId) {
        loop {
            let list = self.objects.processes.get(process).unwrap().handles;
            if list.free_slots() == 0 {
                break;
            }
            self.install(process, filler, Rights::EMPTY);
        }
    }

    /// How many handles `process` holds.
    pub(super) fn handle_count(&self, process: ProcessId) -> u32 {
        self.objects.processes.get(process).unwrap().handles.count()
    }
}

/// A buffer holding a message: a label, `words` payload words counting up
/// from one, and the handles.
pub(super) fn message(label: u64, words: usize, handles: &[Handle]) -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_label(label);
    writer.set_counts(words, handles.len()).unwrap();
    for index in 0..words {
        writer.set_word(index, u64::try_from(index).unwrap().saturating_add(1));
    }
    for (index, handle) in handles.iter().enumerate() {
        writer.set_handle(index, *handle);
    }
    bytes
}
