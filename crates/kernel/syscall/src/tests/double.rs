// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The recording environment the call tests run against, and the small
//! machine they run on.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, Status};
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle, Rights, Syscall};
use kernel_mm::page_table::Permissions;
use kernel_objects::handle_table::{Entry, HandleList};
use kernel_objects::object::{
    AnyObjectId, MemoryKind, MemoryObject, MemoryObjectId, Process, ProcessId, Thread, ThreadId,
};
use kernel_objects::quota::Quota;
use kernel_objects::store::Objects;
use kernel_sched::{Outcome, Scheduler};
use kernel_types::{CachePolicy, Page, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

use crate::dispatch::Machine;
use crate::environment::{Environment, KernelStack};

/// What the environment was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Call {
    /// A page-table root was made.
    CreateAddressSpace(PhysFrame),
    /// A page-table root was taken apart.
    DestroyAddressSpace(PhysFrame),
    /// A page was mapped.
    Map(PhysFrame, Page, PhysFrame),
    /// A page was unmapped.
    Unmap(PhysFrame, Page),
    /// A mapping changed what it allows.
    Protect(PhysFrame, Page, Permissions),
    /// A kernel stack was handed out.
    AllocateStack(u32),
    /// The frame a new thread returns through was written.
    PrepareThread(VirtAddr),
    /// A kernel stack went back.
    ReleaseStack(u32),
    /// A frame was handed out.
    AllocateFrame(PhysFrame),
    /// A frame went back.
    ReleaseFrame(PhysFrame),
    /// Bytes reached the console.
    Log(usize),
    /// The buffer in a frame was reached.
    Buffer(PhysFrame),
    /// A port was read, with the width in bytes.
    ReadPort(u16, u8),
    /// A port was written, with the width in bytes and the value.
    WritePort(u16, u8, u64),
    /// An interrupt line was routed to a vector.
    Route(u8, u8),
    /// An interrupt line was masked.
    Mask(u8),
    /// An interrupt line was unmasked.
    Unmask(u8),
}

/// An environment that records what it was asked and answers with what the
/// test told it to answer.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per request a test can make fail, which is what a recording double is for"
)]
#[derive(Debug, Default)]
pub(super) struct Recorder {
    /// Every call, in order.
    pub(super) calls: Vec<Call>,
    /// The next address space fails.
    pub(super) no_address_space: bool,
    /// The next kernel stack fails.
    pub(super) no_stack: bool,
    /// Whether the frame of the next thread is refused.
    pub(super) no_context: bool,
    /// The next frame fails.
    pub(super) no_frame: bool,
    /// Every mapping fails.
    pub(super) no_mapping: bool,
    /// Unmapping fails.
    pub(super) not_mapped: bool,
    /// The next root, stack slot, and frame to hand out.
    next: u64,
    /// The IPC buffers of the threads other than the caller.
    pub(super) buffers: std::collections::HashMap<PhysFrame, Box<[u8; SIZE]>>,
    /// What the next port read answers, per port.
    pub(super) port_reads: std::collections::HashMap<u16, u64>,
    /// The lines the plan of this machine reserves no vector for.
    pub(super) unroutable: Vec<u8>,
    /// The lines the controller has already routed.
    routed: Vec<u8>,
    /// The frames the machine reported as usable.
    pub(super) ram: Option<PhysFrameRange>,
    /// The address of the root system description pointer.
    pub(super) rsdp: u64,
}

impl Recorder {
    /// A recorder that answers every request.
    #[must_use]
    pub(super) fn new() -> Self {
        Recorder::default()
    }

    /// How often the environment was asked for `call`.
    #[must_use]
    pub(super) fn count(&self, wanted: &Call) -> usize {
        self.calls.iter().filter(|call| *call == wanted).count()
    }

    /// Every frame the environment handed out and never got back.
    #[must_use]
    pub(super) fn frames_out(&self) -> usize {
        let out = self
            .calls
            .iter()
            .filter(|call| matches!(call, Call::AllocateFrame(_)))
            .count();
        let back = self
            .calls
            .iter()
            .filter(|call| matches!(call, Call::ReleaseFrame(_)))
            .count();
        out.saturating_sub(back)
    }

    /// Every kernel stack the environment handed out and never got back.
    #[must_use]
    pub(super) fn stacks_out(&self) -> usize {
        let out = self
            .calls
            .iter()
            .filter(|call| matches!(call, Call::AllocateStack(_)))
            .count();
        let back = self
            .calls
            .iter()
            .filter(|call| matches!(call, Call::ReleaseStack(_)))
            .count();
        out.saturating_sub(back)
    }

    pub(super) fn fresh_frame(&mut self) -> PhysFrame {
        self.next = self.next.saturating_add(1);
        PhysFrame::containing(PhysAddr::new(self.next.saturating_mul(PAGE_SIZE)).unwrap())
    }
}

impl Environment for Recorder {
    fn create_address_space(&mut self) -> Result<PhysFrame, Error> {
        if self.no_address_space {
            return Err(Error::OutOfKernelMemory);
        }
        let root = self.fresh_frame();
        self.calls.push(Call::CreateAddressSpace(root));
        Ok(root)
    }

    fn destroy_address_space(&mut self, root: PhysFrame) {
        self.calls.push(Call::DestroyAddressSpace(root));
    }

    fn map(
        &mut self,
        root: PhysFrame,
        page: Page,
        frame: PhysFrame,
        _perms: Permissions,
        _cache: CachePolicy,
    ) -> Result<(), Error> {
        if self.no_mapping {
            return Err(Error::OutOfKernelMemory);
        }
        self.calls.push(Call::Map(root, page, frame));
        Ok(())
    }

    fn unmap(&mut self, root: PhysFrame, page: Page) -> Result<(), Error> {
        if self.not_mapped {
            return Err(Error::NotMapped);
        }
        self.calls.push(Call::Unmap(root, page));
        Ok(())
    }

    fn protect(&mut self, root: PhysFrame, page: Page, perms: Permissions) -> Result<(), Error> {
        if self.not_mapped {
            return Err(Error::NotMapped);
        }
        self.calls.push(Call::Protect(root, page, perms));
        Ok(())
    }

    fn allocate_kernel_stack(&mut self) -> Result<KernelStack, Error> {
        if self.no_stack {
            return Err(Error::OutOfKernelMemory);
        }
        self.next = self.next.saturating_add(1);
        let slot = u32::try_from(self.next).unwrap_or(u32::MAX);
        self.calls.push(Call::AllocateStack(slot));
        Ok(KernelStack {
            slot,
            top: VirtAddr::new(0xFFFF_FFFF_4000_0000).unwrap(),
        })
    }

    fn prepare_thread(
        &mut self,
        stack_top: VirtAddr,
        _entry: VirtAddr,
        _user_stack: VirtAddr,
        _ipc_buffer: VirtAddr,
    ) -> Result<VirtAddr, Error> {
        if self.no_context {
            return Err(Error::OutOfKernelMemory);
        }
        self.calls.push(Call::PrepareThread(stack_top));
        // A word below the top, which is where a frame written into the
        // stack leaves the pointer.
        stack_top.checked_sub(8).ok_or(Error::OutOfKernelMemory)
    }

    fn release_kernel_stack(&mut self, slot: u32) {
        self.calls.push(Call::ReleaseStack(slot));
    }

    fn allocate_frame(&mut self) -> Result<PhysFrame, Error> {
        if self.no_frame {
            return Err(Error::OutOfKernelMemory);
        }
        let frame = self.fresh_frame();
        self.calls.push(Call::AllocateFrame(frame));
        Ok(frame)
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        self.calls.push(Call::ReleaseFrame(frame));
    }

    fn log(&mut self, bytes: &[u8]) {
        self.calls.push(Call::Log(bytes.len()));
    }

    fn with_buffer<R>(
        &mut self,
        frame: PhysFrame,
        body: impl FnOnce(&mut [u8; SIZE]) -> R,
    ) -> Result<R, Error> {
        self.calls.push(Call::Buffer(frame));
        let bytes = self.buffers.get_mut(&frame).ok_or(Error::InvalidArgument)?;
        Ok(body(bytes))
    }

    fn read_port(&mut self, port: u16, width: u8) -> Result<u64, Error> {
        if !matches!(width, 1 | 2 | 4) {
            return Err(Error::InvalidArgument);
        }
        self.calls.push(Call::ReadPort(port, width));
        Ok(self.port_reads.get(&port).copied().unwrap_or(0))
    }

    fn write_port(&mut self, port: u16, width: u8, value: u64) -> Result<(), Error> {
        if !matches!(width, 1 | 2 | 4) {
            return Err(Error::InvalidArgument);
        }
        self.calls.push(Call::WritePort(port, width, value));
        Ok(())
    }

    fn interrupt_vector(&self, line: u8) -> Option<u8> {
        if self.unroutable.contains(&line) {
            return None;
        }
        line.checked_add(0x40)
    }

    fn route_interrupt(&mut self, line: u8, vector: u8) -> Result<(), Error> {
        if self.routed.contains(&line) {
            return Err(Error::AlreadyExists);
        }
        self.routed.push(line);
        self.calls.push(Call::Route(line, vector));
        Ok(())
    }

    fn mask_interrupt(&mut self, line: u8) {
        self.calls.push(Call::Mask(line));
    }

    fn unmask_interrupt(&mut self, line: u8) {
        self.calls.push(Call::Unmask(line));
    }

    fn meets_ram(&self, frames: PhysFrameRange) -> bool {
        self.ram.is_some_and(|ram| {
            let ends = frames.start().number().saturating_add(frames.count());
            let ram_ends = ram.start().number().saturating_add(ram.count());
            frames.start().number() < ram_ends && ram.start().number() < ends
        })
    }

    fn acpi_pointer(&self) -> u64 {
        self.rsdp
    }
}

/// The machine the tests run on: small enough to keep on a stack.
pub(super) type Small = Objects<4, 8, 8, 32>;

/// A machine with one process, one thread of it, and the handles the
/// process holds to both.
#[derive(Debug)]
pub(super) struct Fixture {
    /// The objects.
    pub(super) objects: Small,
    /// The run queues.
    pub(super) scheduler: Scheduler,
    /// The environment.
    pub(super) environment: Recorder,
    /// The process every call is made by.
    pub(super) process: ProcessId,
    /// The thread every call is made by.
    pub(super) thread: ThreadId,
    /// The handle the process holds to itself.
    pub(super) own_process: Handle,
    /// The handle the process holds to its thread.
    pub(super) own_thread: Handle,
}

impl Fixture {
    /// A machine with one process and one running thread.
    #[must_use]
    pub(super) fn new() -> Self {
        let mut objects = Small::new();
        let mut scheduler = Scheduler::new();
        let root = PhysFrame::containing(PhysAddr::new(0x10_0000).unwrap());
        let process = objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(16),
                Quota::new(64),
                Quota::new(16),
            ))
            .unwrap();
        let buffer = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
        let mut environment = Recorder::new();
        // The buffer of the calling thread is reachable through the
        // environment as well as through the argument the dispatcher takes:
        // a thread that is answered by another one is answered there.
        environment.buffers.insert(buffer, Box::new([0; SIZE]));
        let thread = objects
            .threads
            .allocate(Thread::new(process, 4, 8, 0, buffer).unwrap())
            .unwrap();
        objects
            .processes
            .get_mut(process)
            .unwrap()
            .add_thread(thread)
            .unwrap();
        scheduler.start(&mut objects.threads, thread).unwrap();
        scheduler.pick_next(&mut objects.threads).unwrap();

        let mut list = objects.processes.get(process).unwrap().handles;
        let own_process = objects
            .handles
            .insert(
                process,
                &mut list,
                Entry::new(
                    AnyObjectId::of(process),
                    Rights::MANAGE
                        | Rights::MAP
                        | Rights::INSTALL
                        | Rights::DUPLICATE
                        | Rights::TRANSFER,
                ),
            )
            .unwrap();
        let own_thread = objects
            .handles
            .insert(
                process,
                &mut list,
                Entry::new(
                    AnyObjectId::of(thread),
                    Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
                ),
            )
            .unwrap();
        objects.processes.get_mut(process).unwrap().handles = list;
        // Every handle is a reference, the way the calls that install one
        // make it: the process and the thread each hold one to themselves
        // besides.
        objects.retain(AnyObjectId::of(process)).unwrap();
        objects.retain(AnyObjectId::of(thread)).unwrap();

        Fixture {
            objects,
            scheduler,
            environment,
            process,
            thread,
            own_process,
            own_thread,
        }
    }

    /// The machine, as the dispatcher takes it.
    pub(super) fn machine(&mut self) -> Machine<'_, Recorder, 4, 8, 8, 32> {
        Machine {
            objects: &mut self.objects,
            scheduler: &mut self.scheduler,
            environment: &mut self.environment,
        }
    }

    /// Installs a handle to `object` with `rights` in the process, with the
    /// reference the handle holds.
    pub(super) fn install(&mut self, object: AnyObjectId, rights: Rights) -> Handle {
        let handle = self.install_first(object, rights);
        self.objects.retain(object).unwrap();
        handle
    }

    /// Installs the first handle to an object that was just allocated, the
    /// way `memory_split`, `endpoint_create`, and `notification_create` do:
    /// the reference the allocation gave is the one that handle holds.
    pub(super) fn install_first(&mut self, object: AnyObjectId, rights: Rights) -> Handle {
        self.objects
            .install_handle(self.process, Entry::new(object, rights))
            .unwrap()
    }

    /// A second process with `capacity` handle slots and no thread.
    pub(super) fn process(&mut self, capacity: u32) -> ProcessId {
        let root = self.environment.fresh_frame();
        self.objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(capacity),
                Quota::new(64),
                Quota::new(16),
            ))
            .unwrap()
    }

    /// A thread of `process`, with its IPC buffer reachable through the
    /// environment the way the buffer of a live thread is.
    pub(super) fn add_thread(&mut self, process: ProcessId, priority: u8) -> ThreadId {
        let buffer = self.environment.fresh_frame();
        self.environment.buffers.insert(buffer, Box::new([0; SIZE]));
        let thread = self
            .objects
            .threads
            .allocate(Thread::new(process, priority, 8, 0, buffer).unwrap())
            .unwrap();
        self.objects
            .processes
            .get_mut(process)
            .unwrap()
            .add_thread(thread)
            .unwrap();
        thread
    }

    /// A thread of `process` that is running, which is what a thread that
    /// made a system call is.
    pub(super) fn running(&mut self, process: ProcessId, priority: u8) -> ThreadId {
        let thread = self.add_thread(process, priority);
        self.scheduler
            .start(&mut self.objects.threads, thread)
            .unwrap();
        self.objects.threads.get_mut(thread).unwrap().state = audhsos_abi::ThreadState::Running;
        let _ = self.scheduler.dequeue(&mut self.objects.threads, thread);
        thread
    }

    /// The buffer of `thread`, as the environment holds it.
    pub(super) fn buffer_of(&self, thread: kernel_objects::object::ThreadId) -> &[u8; SIZE] {
        let frame = self.objects.threads.get(thread).unwrap().ipc_buffer;
        self.environment.buffers.get(&frame).unwrap()
    }

    /// The status word `thread` found in its buffer.
    pub(super) fn status_of(&self, thread: kernel_objects::object::ThreadId) -> Status {
        Buffer::new(self.buffer_of(thread)).status().unwrap()
    }

    /// The return words `thread` found in its buffer.
    pub(super) fn returns_of(&self, thread: kernel_objects::object::ThreadId) -> [u64; 2] {
        let view = Buffer::new(self.buffer_of(thread));
        [view.return_word(0).unwrap(), view.return_word(1).unwrap()]
    }

    /// Writes `message` into the buffer of `thread`, which is what a thread
    /// that is about to send does.
    pub(super) fn write_buffer(
        &mut self,
        thread: kernel_objects::object::ThreadId,
        message: &[u8; SIZE],
    ) {
        let frame = self.objects.threads.get(thread).unwrap().ipc_buffer;
        self.environment.buffers.insert(frame, Box::new(*message));
    }

    /// Installs handles until the list of the process is full, so that the
    /// next one a call wants has nowhere to go.
    pub(super) fn fill_handles(&mut self) {
        loop {
            let list = self.objects.processes.get(self.process).unwrap().handles;
            if list.free_slots() == 0 {
                break;
            }
            self.install(AnyObjectId::of(self.thread), Rights::EMPTY);
        }
    }

    /// Adds a memory object of `frames` frames and a handle to it.
    pub(super) fn memory(
        &mut self,
        first: u64,
        frames: u64,
        rights: Rights,
    ) -> (MemoryObjectId, Handle) {
        let start = PhysFrame::containing(PhysAddr::new(first.saturating_mul(PAGE_SIZE)).unwrap());
        let range = PhysFrameRange::new(start, frames).unwrap();
        let id = self
            .objects
            .memory
            .allocate(MemoryObject::new(
                range,
                MemoryKind::Ram,
                CachePolicy::WriteBack,
            ))
            .unwrap();
        let handle = self.install_first(AnyObjectId::of(id), rights);
        (id, handle)
    }
}

/// An IPC buffer holding a call and its arguments.
#[must_use]
pub(super) fn request(call: Syscall, arguments: &[u64]) -> [u8; SIZE] {
    let mut bytes = [0; SIZE];
    let mut writer = BufferMut::new(&mut bytes);
    writer.set_syscall_number(u64::from(call.number()));
    for (index, value) in arguments.iter().enumerate() {
        assert!(writer.set_argument(index, *value), "argument {index}");
    }
    bytes
}

/// Runs one call of the fixture's thread and returns the status word, the
/// return values, and what the dispatcher asked the caller to do.
pub(super) fn call(fixture: &mut Fixture, buffer: &mut [u8; SIZE]) -> (Status, [u64; 2], Outcome) {
    let thread = fixture.thread;
    let outcome = crate::dispatch::dispatch(&mut fixture.machine(), thread, buffer);
    let view = Buffer::new(buffer);
    let status = view.status().unwrap();
    let values = [view.return_word(0).unwrap(), view.return_word(1).unwrap()];
    (status, values, outcome)
}

/// Runs one call and returns the error it failed with, or `None`.
pub(super) fn error_of(fixture: &mut Fixture, mut buffer: [u8; SIZE]) -> Option<Error> {
    let (status, _, _) = call(fixture, &mut buffer);
    status.error()
}

/// Runs one call that must succeed and returns its first return value.
pub(super) fn value_of(fixture: &mut Fixture, mut buffer: [u8; SIZE]) -> u64 {
    let (status, values, _) = call(fixture, &mut buffer);
    assert_eq!(status.error(), None, "the call was refused");
    values[0]
}
