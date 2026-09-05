// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the system call layer reaches the machine through, and the two
//! entry points the architecture layer calls: one for a system call, one
//! for a tick.
//!
//! Invariants: an address space this module hands out carries the kernel
//! half and maps nothing of the user half; a frame it hands out for an IPC
//! buffer is zeroed before the thread that gets it can read it; the
//! page-table root is loaded only when the thread that takes the processor
//! belongs to another process (D-65).

use core::marker::PhantomData;

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::SIZE;
use kernel_hal_api::console::DebugConsole;
use kernel_hal_api::paging::{AddressSpaceControl, FrameAccess, TlbControl};
use kernel_mm::BitmapFrameAllocator;
use kernel_mm::kernel_half;
use kernel_mm::mapper::Mapper;
use kernel_mm::page_table::{EntryFormat, PageTable, Permissions};
use kernel_objects::object::{ProcessId, ThreadId};
use kernel_objects::store::Objects;
use kernel_sched::Scheduler;
use kernel_syscall::dispatch::Machine as SyscallMachine;
use kernel_syscall::environment::{Environment, KernelStack};
use kernel_syscall::{dispatch, reaper};
use kernel_types::{CachePolicy, Page, PhysFrame, VirtAddr};

use crate::memory::KernelMemory;

/// The machine as the system calls reach it: the memory of the kernel, the
/// page tables, the lookaside buffer, and the console.
///
/// The type parameters are what the architecture layer supplies; nothing
/// here knows what a page-table entry looks like.
#[derive(Debug)]
pub struct KernelEnvironment<'a, F, A, T, C>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    C: DebugConsole,
{
    /// What the kernel owns after the bring-up.
    pub memory: &'a mut KernelMemory,
    /// The page tables, reachable through the physical window.
    pub access: &'a mut A,
    /// The translation lookaside buffer.
    pub tlb: &'a mut T,
    /// The debug console, in a build that has one.
    pub console: Option<&'a mut C>,
    format: PhantomData<fn() -> F>,
}

impl<'a, F, A, T, C> KernelEnvironment<'a, F, A, T, C>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    C: DebugConsole,
{
    /// The environment over these four.
    pub fn new(
        memory: &'a mut KernelMemory,
        access: &'a mut A,
        tlb: &'a mut T,
        console: Option<&'a mut C>,
    ) -> Self {
        KernelEnvironment {
            memory,
            access,
            tlb,
            console,
            format: PhantomData,
        }
    }

    /// A mapper over the address space rooted at `root`.
    fn mapper(&mut self, root: PhysFrame) -> Mapper<'_, F, A, T, BitmapFrameAllocator> {
        Mapper::new(root, self.access, self.tlb, self.memory.frames_mut())
    }

    /// Fills the frame with zeros, so that nothing of what stood there
    /// before reaches the thread that gets it.
    fn zero(&mut self, frame: PhysFrame) {
        if let Some(table) = self.access.table_mut(frame) {
            *table = PageTable::new();
        }
    }
}

impl<F, A, T, C> Environment for KernelEnvironment<'_, F, A, T, C>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    C: DebugConsole,
{
    fn create_address_space(&mut self) -> Result<PhysFrame, Error> {
        let root = self
            .memory
            .frames_mut()
            .allocate()
            .map_err(|_| Error::OutOfKernelMemory)?;
        self.zero(root);
        let kernel = self.memory.root();
        if let Err(error) = kernel_half::share::<F, A>(self.access, kernel, root) {
            self.memory.frames_mut().free(root).ok();
            return Err(Error::from(error));
        }
        Ok(root)
    }

    fn destroy_address_space(&mut self, root: PhysFrame) {
        // The tables of the user half go back to the reserve; the frames
        // the mappings pointed at belong to memory objects and stay.
        let kernel = self.memory.root();
        if root == kernel {
            return;
        }
        kernel_half::free_user_half::<F, A, BitmapFrameAllocator>(
            self.access,
            self.memory.frames_mut(),
            root,
        );
        self.tlb.flush_all();
    }

    fn map(
        &mut self,
        root: PhysFrame,
        page: Page,
        frame: PhysFrame,
        perms: Permissions,
        cache: CachePolicy,
    ) -> Result<(), Error> {
        self.mapper(root)
            .map(page, frame, perms, cache)
            .map_err(map_error)
    }

    fn unmap(&mut self, root: PhysFrame, page: Page) -> Result<(), Error> {
        self.mapper(root).unmap(page).map(|_| ()).map_err(map_error)
    }

    fn protect(&mut self, root: PhysFrame, page: Page, perms: Permissions) -> Result<(), Error> {
        self.mapper(root).protect(page, perms).map_err(map_error)
    }

    fn allocate_kernel_stack(&mut self) -> Result<KernelStack, Error> {
        let stack = self
            .memory
            .allocate_stack::<F, A, T>(self.access, self.tlb)
            .map_err(|_| Error::OutOfKernelMemory)?;
        let top = stack.top().ok_or(Error::OutOfKernelMemory)?;
        Ok(KernelStack {
            slot: stack.index(),
            top,
        })
    }

    fn release_kernel_stack(&mut self, slot: u32) {
        let Ok(stack) = self.memory.stacks_mut().stack_of(slot) else {
            return;
        };
        let _ = self
            .memory
            .release_stack::<F, A, T>(self.access, self.tlb, stack);
    }

    fn allocate_frame(&mut self) -> Result<PhysFrame, Error> {
        let frame = self
            .memory
            .frames_mut()
            .allocate()
            .map_err(|_| Error::OutOfKernelMemory)?;
        self.zero(frame);
        Ok(frame)
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        let _ = self.memory.frames_mut().free(frame);
    }

    fn log(&mut self, bytes: &[u8]) {
        if let Some(console) = self.console.as_mut() {
            console.write_bytes(bytes);
        }
    }
}

/// What a paging error says to a caller of a system call.
const fn map_error(error: kernel_mm::mapper::MapError) -> Error {
    use kernel_mm::mapper::MapError as Paging;
    match error {
        Paging::AlreadyMapped => Error::AlreadyMapped,
        Paging::NotMapped => Error::NotMapped,
        Paging::OutOfKernelMemory => Error::OutOfKernelMemory,
        Paging::UnreachableFrame | Paging::Entry(_) => Error::InvalidArgument,
    }
}

/// What the architecture layer has to do after the kernel has answered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Next {
    /// The thread that should run, when it is not the one that ran.
    pub switch: Option<Switch>,
}

/// The two threads of a switch and everything the architecture layer needs
/// to carry it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    /// The thread that leaves the processor, if one did.
    pub from: Option<ThreadId>,
    /// The thread that takes it.
    pub to: ThreadId,
    /// The saved context of the thread that takes it: the kernel stack
    /// pointer it left off at, or the frame a new thread starts through.
    pub context: VirtAddr,
    /// The top of its kernel stack, which the task state segment names so
    /// that a trap from user mode lands on it.
    pub kernel_stack_top: VirtAddr,
    /// The address space it runs in, when that is another one.
    pub address_space: Option<PhysFrame>,
}

/// Picks the thread that should run now and tells the caller what to do.
///
/// The page-table root is loaded here, through `spaces`, and only when the
/// thread that takes the processor belongs to another process; the switch
/// of the stacks is the caller's, because only the architecture layer can
/// do it.
pub fn schedule<
    S: AddressSpaceControl,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    spaces: &mut S,
    stack_top: impl Fn(u32) -> VirtAddr,
) -> Next {
    let leaving = scheduler.current();
    let Ok(next) = scheduler.pick_next(&mut objects.threads) else {
        return Next::default();
    };
    if Some(next) == leaving {
        return Next::default();
    }
    let Ok(thread) = objects.threads.get(next) else {
        return Next::default();
    };
    let context = thread.context;
    let slot = thread.kernel_stack;
    let process = thread.process;
    let root = objects
        .processes
        .get(process)
        .map(|holder| holder.root)
        .ok();
    let address_space = match root {
        Some(root) if root != spaces.active() => {
            spaces.activate(root);
            Some(root)
        }
        _ => None,
    };
    Next {
        switch: Some(Switch {
            from: leaving,
            to: next,
            context,
            kernel_stack_top: stack_top(slot),
            address_space,
        }),
    }
}

/// Writes back where a thread left off, which the architecture layer knows
/// only after it has switched away from it.
pub fn store_context<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    thread: ThreadId,
    context: VirtAddr,
) {
    if let Ok(entry) = objects.threads.get_mut(thread) {
        entry.context = context;
    }
}

/// Handles one system call of `caller` and clears away what a thread that
/// ended left behind.
///
/// The return value says whether the caller should switch threads before
/// it returns to user mode.
pub fn handle_syscall<
    F,
    A,
    T,
    C,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    environment: &mut KernelEnvironment<'_, F, A, T, C>,
    caller: ThreadId,
    buffer: &mut [u8; SIZE],
) -> bool
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    C: DebugConsole,
{
    let mut syscall = SyscallMachine {
        objects,
        scheduler,
        environment,
    };
    let outcome = dispatch(&mut syscall, caller, buffer);
    outcome.reschedule
}

/// Gives back what every thread that has ended held, except the one the
/// processor is still on. `running` is that thread: the caller of a system
/// call, or the thread the kernel has just switched to. The scheduler has
/// already forgotten a thread that ended, so it cannot be asked.
pub fn reap<F, A, T, C, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    scheduler: &mut Scheduler,
    environment: &mut KernelEnvironment<'_, F, A, T, C>,
    running: Option<ThreadId>,
) -> u32
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    C: DebugConsole,
{
    let mut syscall = SyscallMachine {
        objects,
        scheduler,
        environment,
    };
    reaper::reap(&mut syscall, running)
}

/// The process a thread belongs to, for the architecture layer.
#[must_use]
pub fn process_of<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &Objects<NP, NT, NM, NH>,
    thread: ThreadId,
) -> Option<ProcessId> {
    objects.threads.get(thread).ok().map(|t| t.process)
}
