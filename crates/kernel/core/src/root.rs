// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The root task: the one process the kernel builds itself.
//!
//! Everything else of this system is started by somebody who was started
//! before it. The root task is where that chain begins, so the kernel does
//! for it what a loader would: an address space, the program mapped, a
//! stack, a thread, an IPC buffer, the capabilities it starts with, and the
//! message that says which handle is which.
//!
//! What it is given is everything: the capability to system control, a
//! handle to itself, the boot image it was read out of, and one memory
//! object per free region of memory. From then on the kernel allocates user
//! memory never again — every later process gets its memory from the root
//! task or from a server the root task started (D-12).
//!
//! This module is architecture-neutral. What the architecture layer still
//! has to do afterwards is write the frame the first switch returns
//! through, which is the one part of starting a thread that knows what a
//! processor register is.
//!
//! Invariants: a root task that could not be built completely is an error
//! and never a half-built one that runs; the handles are installed before
//! the message that names them is written, so a program that reads the
//! message finds every handle it names.

use audhsos_abi::layout::{
    PAGE_SIZE, PRIORITY_COUNT, ROOT_TASK_BASE, THREADS_PER_PROCESS, USER_SPACE_START,
    ipc_buffer_address,
};
use audhsos_abi::startup::{Role, Writer};
use audhsos_abi::{Error, ObjectType};
use kernel_mm::page_table::Permissions;
use kernel_objects::config::HANDLES_PER_PROCESS;
use kernel_objects::handle_table::{Entry, HandleList};
use kernel_objects::object::{
    AnyObjectId, MemoryKind, MemoryObject, Process, ProcessId, SystemControl, Thread, ThreadId,
};
use kernel_objects::quota::Quota;
use kernel_objects::store::Objects;
use kernel_syscall::environment::Environment;
use kernel_types::{CachePolicy, Page, PhysFrame, PhysFrameRange, VirtAddr};

/// How many pages the stack of the root task gets.
pub const STACK_PAGES: u64 = 16;

/// The priority the root task runs at: the highest there is. It is the
/// fault handler of every process it starts, and a fault handler that waits
/// behind the process that faulted would be no help.
pub const PRIORITY: u8 = PRIORITY_COUNT.saturating_sub(1);

/// The address one past the top of the root task's stack. One unmapped page
/// below the program, so that a stack that runs over the end faults instead
/// of writing into the program's own bytes.
pub const STACK_TOP: u64 = ROOT_TASK_BASE.saturating_sub(PAGE_SIZE);

const _: () = assert!(STACK_TOP > USER_SPACE_START + STACK_PAGES * PAGE_SIZE);

/// What the kernel hands the root task besides itself.
#[derive(Clone, Copy, Debug)]
pub struct Grants<'a> {
    /// The frames the boot image occupies, which carry the archive.
    pub boot_image: PhysFrameRange,
    /// The free regions of memory, one memory object each.
    pub ram: &'a [PhysFrameRange],
}

/// The root task, once it stands.
#[derive(Clone, Copy, Debug)]
pub struct RootTask {
    /// The process.
    pub process: ProcessId,
    /// Its first thread, which has not started yet.
    pub thread: ThreadId,
    /// Where the thread begins.
    pub entry: VirtAddr,
    /// The stack pointer it begins with.
    pub stack: VirtAddr,
    /// Where its IPC buffer is mapped in its own address space.
    pub buffer: VirtAddr,
    /// The frame that buffer lies in.
    pub buffer_frame: PhysFrame,
    /// The slot of its kernel stack.
    pub kernel_stack: u32,
    /// The top of that kernel stack.
    pub kernel_stack_top: VirtAddr,
    /// The frame its page tables are rooted in.
    pub address_space: PhysFrame,
}

/// Builds the root task out of `program` and what `grants` names.
///
/// The thread is left inactive: the architecture layer writes the frame it
/// starts through and the caller puts it in a run queue.
///
/// # Errors
///
/// [`Error::InvalidArgument`] for a program that is empty or does not fit
/// below the stack of the buffers; the errors of the address space, of the
/// frames, and of the handle table. A failure leaves nothing running: the
/// process is never started, so what it holds goes back when the caller
/// gives up.
pub fn build<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    environment: &mut E,
    objects: &mut Objects<NP, NT, NM, NH>,
    program: &[u8],
    grants: &Grants<'_>,
) -> Result<RootTask, Error> {
    if program.is_empty() {
        return Err(Error::InvalidArgument);
    }
    let root = environment.create_address_space()?;
    let process = objects.processes.allocate(Process::new(
        root,
        HandleList::with_capacity(u32::try_from(HANDLES_PER_PROCESS).unwrap_or(u32::MAX)),
        // Everything there is: the root task is what hands quotas out, and
        // it can give away no more than it holds.
        Quota::new(u32::MAX),
        Quota::new(u32::MAX),
    ))?;
    // The process holds one reference to itself, as every process does
    // (D-86).
    objects.processes.retain(process)?;

    map_program(environment, root, program)?;
    map_stack(environment, root)?;

    let stack = environment.allocate_kernel_stack()?;
    let buffer_frame = environment.allocate_frame()?;
    let thread = objects.threads.allocate(
        Thread::new(process, PRIORITY, PRIORITY, stack.slot, buffer_frame)
            .map_err(|_| Error::InvalidArgument)?,
    )?;
    objects.threads.retain(thread)?;
    let slot = objects
        .processes
        .get_mut(process)?
        .add_thread(thread)
        .map_err(|_| Error::PoolExhausted)?;
    let buffer = VirtAddr::new(ipc_buffer_address(slot).ok_or(Error::InvalidArgument)?)
        .map_err(|_| Error::InvalidArgument)?;
    let page = Page::from_start(buffer).map_err(|_| Error::Unaligned)?;
    environment.map(
        root,
        page,
        buffer_frame,
        Permissions::READ_WRITE.for_user(),
        CachePolicy::WriteBack,
    )?;

    let entry = VirtAddr::new(ROOT_TASK_BASE).map_err(|_| Error::InvalidArgument)?;
    let top = VirtAddr::new(STACK_TOP).map_err(|_| Error::InvalidArgument)?;
    if let Ok(held) = objects.threads.get_mut(thread) {
        *held = held.starting_at(entry, top, buffer);
    }

    give(objects, process, grants, environment, buffer_frame)?;

    Ok(RootTask {
        process,
        thread,
        entry,
        stack: top,
        buffer,
        buffer_frame,
        kernel_stack: stack.slot,
        kernel_stack_top: stack.top,
        address_space: root,
    })
}

/// Reads the root task as an executable and maps every segment of it with
/// the permissions it asks for.
///
/// An executable and not a flat binary. A blob has one set of permissions
/// for all of it, and the root task, like every program, has bytes it
/// executes and bytes it writes; mapping the whole of it both ways would
/// give the one program that holds every capability of the machine a
/// writable text segment. The kernel therefore reads the same format the
/// root task itself reads for its children, out of the crate all three of
/// the loader, the kernel, and the userland share (D-90).
fn map_program<E: Environment>(
    environment: &mut E,
    root: PhysFrame,
    program: &[u8],
) -> Result<(), Error> {
    let image =
        audhsos_elf::parse(program, ROOT_TASK_CONSTRAINTS).map_err(|_| Error::InvalidArgument)?;
    if image.entry != ROOT_TASK_BASE {
        return Err(Error::InvalidArgument);
    }
    let mut previous_end = 0u64;
    for segment in image.segments() {
        let start = segment
            .vaddr
            .wrapping_sub(segment.vaddr.wrapping_rem(PAGE_SIZE));
        let end = segment
            .vaddr
            .saturating_add(segment.mem_size)
            .next_multiple_of(PAGE_SIZE);
        // Two segments in one page would need one set of permissions for
        // both, and the linker script of the root task puts every section
        // on a page of its own so that this cannot happen.
        if start < previous_end {
            return Err(Error::AddressInUse);
        }
        previous_end = end;
        map_segment(environment, root, program, &segment, start, end)?;
    }
    Ok(())
}

/// The constraints the root task is read under: from where it is linked up
/// to the page below the buffers of its own threads.
const ROOT_TASK_CONSTRAINTS: audhsos_elf::Constraints = audhsos_elf::Constraints {
    lowest_vaddr: ROOT_TASK_BASE,
    highest_vaddr: match ipc_buffer_address(THREADS_PER_PROCESS - 1) {
        Some(lowest) => lowest.saturating_sub(1),
        None => ROOT_TASK_BASE,
    },
};

/// Maps the pages of one segment, with the bytes of the file in them and
/// zeros behind those bytes.
fn map_segment<E: Environment>(
    environment: &mut E,
    root: PhysFrame,
    program: &[u8],
    segment: &audhsos_elf::Segment,
    start: u64,
    end: u64,
) -> Result<(), Error> {
    let leading = segment.vaddr.wrapping_sub(start);
    let from = usize::try_from(segment.file_offset).map_err(|_| Error::InvalidArgument)?;
    let to = from.saturating_add(usize::try_from(segment.file_size).unwrap_or(0));
    let content = program.get(from..to).ok_or(Error::InvalidArgument)?;
    let permissions = Permissions {
        write: segment.write,
        execute: segment.execute,
        user: true,
    };
    let pages = end.saturating_sub(start).wrapping_div(PAGE_SIZE);
    for index in 0..pages {
        let offset = index.wrapping_mul(PAGE_SIZE);
        let frame = environment.allocate_frame()?;
        // A frame is a page and an IPC buffer is a page, so the seam that
        // reaches the bytes of one reaches the bytes of the other. What the
        // file does not fill stays as the frame came: the frames of the
        // reserve are zero, which is what `.bss` needs.
        environment.with_buffer(frame, |bytes| {
            copy_page(bytes, content, leading, offset);
        })?;
        let address = VirtAddr::new(start.checked_add(offset).ok_or(Error::InvalidArgument)?)
            .map_err(|_| Error::InvalidArgument)?;
        let page = Page::from_start(address).map_err(|_| Error::Unaligned)?;
        environment.map(root, page, frame, permissions, CachePolicy::WriteBack)?;
    }
    Ok(())
}

/// Copies the part of `content` that belongs on the page at `offset` of a
/// segment whose bytes begin `leading` bytes into its first page.
fn copy_page(
    bytes: &mut [u8; audhsos_abi::ipc_buffer::SIZE],
    content: &[u8],
    leading: u64,
    offset: u64,
) {
    let page = usize::try_from(PAGE_SIZE).unwrap_or(4096);
    let at = usize::try_from(leading).unwrap_or(0);
    let skipped = usize::try_from(offset).unwrap_or(0);
    // Where in the segment's bytes this page begins, and where in the page
    // they land.
    let (source_from, target_from) = if skipped == 0 {
        (0usize, at)
    } else {
        (skipped.saturating_sub(at), 0usize)
    };
    let room = page.saturating_sub(target_from);
    let source = content.get(source_from..).unwrap_or(&[]);
    let take = source.len().min(room);
    if let (Some(slot), Some(from)) = (
        bytes.get_mut(target_from..target_from.saturating_add(take)),
        source.get(..take),
    ) {
        slot.copy_from_slice(from);
    }
}

/// Maps the stack below the program, read and write.
fn map_stack<E: Environment>(environment: &mut E, root: PhysFrame) -> Result<(), Error> {
    for index in 0..STACK_PAGES {
        let frame = environment.allocate_frame()?;
        let offset = index
            .checked_add(1)
            .and_then(|step| step.checked_mul(PAGE_SIZE))
            .ok_or(Error::InvalidArgument)?;
        let address = VirtAddr::new(
            STACK_TOP
                .checked_sub(offset)
                .ok_or(Error::InvalidArgument)?,
        )
        .map_err(|_| Error::InvalidArgument)?;
        let page = Page::from_start(address).map_err(|_| Error::Unaligned)?;
        environment.map(
            root,
            page,
            frame,
            Permissions::READ_WRITE.for_user(),
            CachePolicy::WriteBack,
        )?;
    }
    Ok(())
}

/// Installs the capabilities the root task starts with and writes the
/// message that says which handle is which.
fn give<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    process: ProcessId,
    grants: &Grants<'_>,
    environment: &mut E,
    buffer_frame: PhysFrame,
) -> Result<(), Error> {
    let mut given: [Option<(Role, audhsos_abi::Handle)>; ROLES] = [None; ROLES];
    let mut count = 0usize;

    let own = install(
        objects,
        process,
        AnyObjectId::of(process),
        ObjectType::Process,
    )?;
    put(&mut given, &mut count, Role::OwnProcess, own)?;

    let system = install(
        objects,
        process,
        AnyObjectId::of(SystemControl::ID),
        ObjectType::SystemControl,
    )?;
    put(&mut given, &mut count, Role::SystemControl, system)?;

    let image = memory_object(objects, process, grants.boot_image)?;
    put(&mut given, &mut count, Role::BootImage, image)?;

    for region in grants.ram {
        let handle = memory_object(objects, process, *region)?;
        put(&mut given, &mut count, Role::Ram, handle)?;
    }

    environment.with_buffer(buffer_frame, |bytes| {
        let mut buffer = audhsos_abi::BufferMut::new(bytes);
        let mut writer = Writer::new();
        for (role, handle) in given.iter().take(count).flatten() {
            let _written = writer.give(&mut buffer, *role, *handle);
        }
        let _finished = writer.finish(&mut buffer);
    })?;
    Ok(())
}

/// How many roles the message can carry: everything the kernel gives, which
/// is three fixed handles and one memory object per region of memory.
const ROLES: usize = audhsos_abi::layout::MAX_BOOT_REGIONS + 3;

/// Records one pair for the startup message.
fn put(
    given: &mut [Option<(Role, audhsos_abi::Handle)>; ROLES],
    count: &mut usize,
    role: Role,
    handle: audhsos_abi::Handle,
) -> Result<(), Error> {
    let slot = given.get_mut(*count).ok_or(Error::OutOfHandles)?;
    *slot = Some((role, handle));
    *count = count.wrapping_add(1);
    Ok(())
}

/// Installs a handle to `object` with every right its type accepts.
fn install<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    process: ProcessId,
    object: AnyObjectId,
    kind: ObjectType,
) -> Result<audhsos_abi::Handle, Error> {
    let handle = objects.install_handle(process, Entry::new(object, kind.rights_mask()))?;
    objects.retain(object)?;
    Ok(handle)
}

/// Makes a `Ram` memory object over `frames` and installs a handle to it.
fn memory_object<const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    objects: &mut Objects<NP, NT, NM, NH>,
    process: ProcessId,
    frames: PhysFrameRange,
) -> Result<audhsos_abi::Handle, Error> {
    let id = objects.memory.allocate(MemoryObject::new(
        frames,
        MemoryKind::Ram,
        CachePolicy::WriteBack,
    ))?;
    objects.install_handle(
        process,
        Entry::new(AnyObjectId::of(id), ObjectType::MemoryObject.rights_mask()),
    )
}
