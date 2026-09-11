// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The kernel every Phase 5 test image is: the memory bring-up, the
//! machine that holds the objects, a process built by hand the way the
//! root task will be built in Phase 7, the switch into ring three, the
//! system call gate, and the trap handler that tells a fault of the kernel
//! from a fault of a user thread.
//!
//! Four images share it — `user`, `isolation`, `concurrency`, and
//! `syscalls` — because what they differ in is what they watch, not what
//! they run on.
//!
//! Invariants: the bring-up happens once per image, whichever test asks
//! for it first; the thread the kernel switched away from finds its
//! context in its own pool entry, so that a thread which is resumed
//! carries on where it stopped; a fault at ring three stops one thread and
//! a fault at ring zero fails the image.

#![expect(dead_code, reason = "each image uses the part of the kernel it needs")]

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use audhsos_sync::{Global, UncontendedToken};

use audhsos_abi::layout::{
    BOOT_STACK_TOP, KERNEL_STACK_PAGES, KERNEL_STACK_SLOT_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE,
    TICKS_PER_SECOND,
};
use audhsos_abi::{Handle, Rights, ThreadState, ipc_buffer};
use kernel_core::machine::with_machine;
use kernel_core::memory::{self, with_memory};
use kernel_core::state::KernelState;
use kernel_core::syscall::{KernelEnvironment, handle_syscall, reap, schedule, store_context};
use kernel_core::trap::{Exception, Response};
use kernel_hal_api::paging::FrameAccess;
use kernel_hal_x86_64::console::SerialConsole;
use kernel_hal_x86_64::paging::{AddressSpaces, LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::ports::DeviceAccess;
use kernel_hal_x86_64::traps::TrapReport;
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_hal_x86_64::{context, descriptors, instructions, interrupts, testing, traps, vectors};
use kernel_mm::page_table::{CachePolicy, PageTable, Permissions};
use kernel_objects::handle_table::{Entry, HandleList};
use kernel_objects::object::{
    AnyObjectId, Endpoint, EndpointId, MemoryKind, MemoryObject, MemoryObjectId, Process,
    ProcessId, Thread, ThreadId,
};
use kernel_objects::quota::Quota;
use kernel_syscall::environment::{Environment, KernelStack};
use kernel_syscall::fault;
use kernel_types::{Alignment, Page, PhysFrame, PhysFrameRange, VirtAddr};

/// Where a program is linked and mapped. The linker script of the user
/// test programs and the xtask both name this address.
pub(crate) const USER_BASE: u64 = 0x40_0000;

/// Where the stack of the first thread of a process ends.
pub(crate) const USER_STACK_TOP: u64 = 0x80_0000;

/// How many pages a user stack has.
pub(crate) const USER_STACK_PAGES: u64 = 2;

/// The page a process shares between its threads, where a program that
/// runs in more than one thread keeps what both of them touch.
pub(crate) const SHARED_BASE: u64 = 0x90_0000;

/// The priority a thread gets when a test does not say otherwise.
pub(crate) const DEFAULT_PRIORITY: u8 = 4;

/// The highest priority a thread of a test image may reach.
pub(crate) const MAX_PRIORITY: u8 = 8;

/// The slot the idle thread carries: it stands on the boot stack, which
/// the loader made and the stack area does not hold.
const BOOT_SLOT: u32 = u32::MAX;

/// How many user threads have been stopped by a fault since the image
/// started.
static FAULTS: AtomicU32 = AtomicU32::new(0);

/// What the processor reported for the last fault of a user thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct LastFault {
    /// The vector the processor entered; [`NO_VECTOR`] before any fault.
    pub(crate) vector: u32,
    /// The error code it pushed, or zero.
    pub(crate) error_code: u64,
    /// The address a page fault named.
    pub(crate) address: u64,
    /// Where the thread stood.
    pub(crate) ip: u64,
}

/// The vector of [`LastFault`] before a user thread has faulted. No vector
/// is that wide, so it can never be one.
pub(crate) const NO_VECTOR: u32 = u32::MAX;

static LAST_VECTOR: AtomicU32 = AtomicU32::new(NO_VECTOR);
static LAST_ERROR: AtomicU64 = AtomicU64::new(0);
static LAST_ADDRESS: AtomicU64 = AtomicU64::new(0);
static LAST_IP: AtomicU64 = AtomicU64::new(0);

/// Whether the bring-up has run.
static READY: AtomicU32 = AtomicU32::new(0);

/// Whether the timer has been started.
static TIMER: AtomicU32 = AtomicU32::new(0);

/// How many turns of the processor the log holds. A test that needs more
/// than this to see the scheduler go round is asking the wrong question.
pub(crate) const TURNS: usize = 32;

/// The index of no thread at all, which is what an unwritten turn holds.
pub(crate) const NO_THREAD: u32 = u32::MAX;

/// The threads the timer found on the processor, one entry per turn: a
/// tick that finds the same thread as the tick before it adds nothing, so
/// what the log holds is the order the processor went round and not how
/// long each turn was.
static TURN_OF: [AtomicU32; TURNS] = [const { AtomicU32::new(NO_THREAD) }; TURNS];

/// How many turns have been recorded, which may be more than the log
/// holds.
static TURNS_SEEN: AtomicU32 = AtomicU32::new(0);

/// What an image does on every timer tick, beyond what the scheduler does
/// with the time slice of the running thread. `true` asks for a switch.
pub(crate) type TickHook = fn(u64) -> bool;

/// The hook of the running image. The borrow is copied out before the
/// hook runs, as every other hook of this system is, so that a hook which
/// switches threads leaves no borrow alive on the stack it leaves.
static TICK_HOOK: Global<TickHook> = Global::new();

/// What an image does at the entry of every system call a user thread
/// makes, before the kernel answers it. The argument is the call number
/// the thread wrote into its buffer.
///
/// This is what a measuring image is: the entry of a call is the one point
/// of the kernel that a round trip passes through exactly once, so the
/// difference between two entries of the same call is one round trip, and
/// nothing else of the kernel has to know that anybody is counting.
pub(crate) type CallHook = fn(u64);

/// The call hook of the running image, if it has one.
static CALL_HOOK: Global<CallHook> = Global::new();

/// Registers what the image does at the entry of every system call. An
/// image sets it once; a second call leaves the first hook in place.
pub(crate) fn set_call_hook(hook: CallHook) {
    let _ = CALL_HOOK.init(hook);
}

/// The window over physical memory. The tables the loader built stay
/// active for the whole run, so the window maps every physical frame read
/// and write, and this image is the only writer.
pub(crate) fn window() -> PhysicalWindow {
    // SAFETY: the loader's tables stay active and the image runs on one
    // processor with nothing else touching kernel memory.
    unsafe { PhysicalWindow::kernel() }
}

/// The serial console, for the running commentary of an image.
pub(crate) fn console() -> SerialConsole {
    // SAFETY: the first serial controller belongs to the kernel while the
    // debug console is on; no userland driver exists in a test image.
    unsafe { SerialConsole::new(0x3F8) }
}

/// Writes one line of commentary.
macro_rules! say {
    ($($argument:tt)*) => {{
        use kernel_hal_api::console::DebugConsole;
        let mut console = $crate::support::console();
        console.write_bytes(b"[user] ");
        let _ = core::fmt::Write::write_fmt(
            &mut $crate::support::Writer(&mut console),
            format_args!($($argument)*),
        );
        console.write_bytes(b"\n");
    }};
}

pub(crate) use say;

/// Turns a console into something `format_args!` can write to.
pub(crate) struct Writer<'a>(pub &'a mut SerialConsole);

impl core::fmt::Write for Writer<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        use kernel_hal_api::console::DebugConsole;
        self.0.write_bytes(text.as_bytes());
        Ok(())
    }
}

/// Brings the memory of the kernel up and arms the two gates a user thread
/// can reach the kernel through: the system call vector and the trap
/// handler. Runs once per image, whichever test asks for it first.
pub(crate) fn bring_up() {
    if READY.swap(1, Ordering::SeqCst) == 1 {
        return;
    }
    let Ok(root) = active_root() else {
        testing::fail(format_args!("the page table root is not addressable"));
    };
    let mut tables = window();
    let mut tlb = LocalTlb;
    let brought_up = testing::with_platform(|platform| {
        memory::initialize::<X86Entry, _, _, _>(platform, root, &mut tables, &mut tlb, 0)
    });
    match brought_up {
        Some(Ok(())) => {}
        Some(Err(error)) => testing::fail(format_args!("the memory bring-up failed: {error}")),
        None => testing::fail(format_args!("the platform is not reachable")),
    }
    traps::set_syscall_handler(on_syscall);
    testing::set_trap_hook(on_trap);
    say!("memory is up, the system call gate is armed, and faults are caught");
}

/// The kernel's own process and the idle thread, which is this image
/// itself: the thread the kernel switches back into when no user thread
/// can run. Its context is written by the first switch away from it.
pub(crate) fn idle_thread() -> ThreadId {
    let root = with_memory(|memory| memory.root()).unwrap_or_else(|| {
        testing::fail(format_args!("the kernel memory is not reachable"));
    });
    with_machine(|machine| {
        if let Some(idle) = machine.scheduler.idle() {
            return idle;
        }
        let process = machine
            .objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(4),
                Quota::new(64),
                Quota::new(64),
            ))
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the kernel process")));
        let thread = machine
            .objects
            .threads
            .allocate(
                Thread::new(process, 0, 0, BOOT_SLOT, root)
                    .unwrap_or_else(|_| testing::fail(format_args!("the idle thread is not one"))),
            )
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the idle thread")));
        machine.scheduler.set_idle(thread);
        // The kernel is that thread: it is what the first switch leaves.
        machine.scheduler.adopt(thread);
        thread
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")))
}

/// A user process: an address space that carries the kernel half, with a
/// program mapped read and execute at [`USER_BASE`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct UserProcess {
    /// The process object.
    pub(crate) id: ProcessId,
    /// The frame its page tables are rooted in.
    pub(crate) root: PhysFrame,
    /// How many threads it holds, which is what decides where the stack of
    /// the next one lies.
    threads: u64,
}

/// A thread of such a process, and what a test needs to watch it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spawned {
    /// The process it belongs to.
    pub(crate) process: ProcessId,
    /// The thread.
    pub(crate) thread: ThreadId,
    /// The frame holding its IPC buffer.
    pub(crate) buffer: PhysFrame,
}

/// Builds a process with `program` mapped at [`USER_BASE`].
pub(crate) fn create_process(program: &[u8]) -> UserProcess {
    let mut tables = window();
    let mut tlb = LocalTlb;
    let root = with_memory(|memory| {
        let mut environment = environment(memory, &mut tables, &mut tlb);
        let root = environment
            .create_address_space()
            .unwrap_or_else(|error| testing::fail(format_args!("no address space: {error}")));
        map_program(&mut environment, root, program);
        root
    })
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")));
    let id = with_machine(|machine| {
        machine
            .objects
            .processes
            .allocate(Process::new(
                root,
                // Room for the handles the root task will install and the
                // objects a program of this system makes for itself.
                HandleList::with_capacity(64),
                Quota::new(32),
                Quota::new(64),
            ))
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the process")))
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")));
    UserProcess {
        id,
        root,
        threads: 0,
    }
}

/// Adds a thread to `process` at `priority`, with a stack of its own. The
/// thread is `Inactive` until [`start`] puts it in a run queue. Every
/// thread of a process runs the same program: the kernel maps one binary,
/// and where a thread begins is the entry it is given.
pub(crate) fn add_thread(process: &mut UserProcess, priority: u8) -> Spawned {
    let index = process.threads;
    process.threads = process.threads.saturating_add(1);
    let stack_top = stack_top_for(index);
    let mut tables = window();
    let mut tlb = LocalTlb;
    let (stack, buffer) = with_memory(|memory| {
        let mut environment = environment(memory, &mut tables, &mut tlb);
        map_stack(&mut environment, process.root, stack_top);
        let stack = environment
            .allocate_kernel_stack()
            .unwrap_or_else(|error| testing::fail(format_args!("no kernel stack: {error}")));
        let buffer = environment
            .allocate_frame()
            .unwrap_or_else(|error| testing::fail(format_args!("no buffer frame: {error}")));
        (stack, buffer)
    })
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")));
    build(process, priority, stack, buffer, stack_top)
}

/// Starts `thread` and says whether it should take the processor from
/// whoever holds it: a thread of higher priority does.
pub(crate) fn start(thread: ThreadId) -> bool {
    with_machine(|machine| {
        machine
            .scheduler
            .start(&mut machine.objects.threads, thread)
            .unwrap_or_else(|error| {
                testing::fail(format_args!("the thread does not start: {error}"))
            })
            .reschedule
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")))
}

/// The same, for a caller that cannot fail an image: a tick hook runs
/// inside an interrupt, and an interrupt that arrives while the kernel
/// holds the machine finds it busy.
///
/// `None` says that and nothing else, so that a caller may try again on
/// the next tick without ever trying forever: a thread the scheduler
/// refuses — one that has already started, or that the pool no longer
/// holds — is `Some(false)`, which is an answer and not a reason to come
/// back.
pub(crate) fn try_start(thread: ThreadId) -> Option<bool> {
    with_machine(|machine| {
        machine
            .scheduler
            .start(&mut machine.objects.threads, thread)
            .is_ok_and(|outcome| outcome.reschedule)
    })
}

/// Ends `thread` wherever it is, for a caller that cannot fail an image.
/// `None` says the machine was busy, as in [`try_start`].
pub(crate) fn try_kill(thread: ThreadId) -> Option<bool> {
    with_machine(|machine| {
        machine
            .scheduler
            .exit(&mut machine.objects.threads, thread)
            .is_ok_and(|outcome| outcome.reschedule)
    })
}

/// A thread id as one word, so that a tick hook can keep one in a `static`
/// without a cell of its own. [`NO_THREAD_WORD`] is no thread.
pub(crate) fn pack(thread: ThreadId) -> u64 {
    (u64::from(thread.index()) << 32) | u64::from(thread.generation())
}

/// The thread `word` names, or `None` for [`NO_THREAD_WORD`].
pub(crate) fn unpack(word: u64) -> Option<ThreadId> {
    if word == NO_THREAD_WORD {
        return None;
    }
    let index = u32::try_from(word >> 32).unwrap_or(0);
    let generation = u32::try_from(word & 0xFFFF_FFFF).unwrap_or(0);
    Some(ThreadId::new(index, generation))
}

/// The word that names no thread. No id packs to it: it would need a slot
/// index and a generation that are both the widest a `u32` holds.
pub(crate) const NO_THREAD_WORD: u64 = u64::MAX;

/// The word `index` of the IPC buffer of `thread`, or zero when the pool
/// does not hold it or the machine is busy.
pub(crate) fn thread_word(thread: ThreadId, index: usize) -> u64 {
    let frame = with_machine(|machine| {
        machine
            .objects
            .threads
            .get(thread)
            .ok()
            .map(|entry| entry.ipc_buffer)
    })
    .flatten();
    frame.map_or(0, |frame| buffer_word(frame, index))
}

/// How many ticks the timer has delivered.
pub(crate) fn ticks() -> u64 {
    interrupts::ticks()
}

/// Empties the log of turns, so that what a run records is its own.
pub(crate) fn forget_turns() {
    for slot in &TURN_OF {
        slot.store(NO_THREAD, Ordering::SeqCst);
    }
    TURNS_SEEN.store(0, Ordering::SeqCst);
}

/// A process with `program` and one running thread of `priority`.
pub(crate) fn spawn_at(program: &[u8], priority: u8) -> Spawned {
    let mut process = create_process(program);
    let spawned = add_thread(&mut process, priority);
    start(spawned.thread);
    spawned
}

/// A process with `program` and one running thread of the default
/// priority: the shortest way to a running user thread.
pub(crate) fn spawn(program: &[u8]) -> Spawned {
    spawn_at(program, DEFAULT_PRIORITY)
}

/// Installs a handle to `object` with `rights` in the table of `process`,
/// the way the root task will install the handles of what it starts.
pub(crate) fn install(process: ProcessId, object: AnyObjectId, rights: Rights) -> Handle {
    with_machine(|machine| {
        let Ok(mut list) = machine
            .objects
            .processes
            .get(process)
            .map(|held| held.handles)
        else {
            testing::fail(format_args!("the process holds no handle list"));
        };
        let handle = machine
            .objects
            .handles
            .insert(process, &mut list, Entry::new(object, rights))
            .unwrap_or_else(|error| testing::fail(format_args!("no handle slot: {error}")));
        if let Ok(held) = machine.objects.processes.get_mut(process) {
            held.handles = list;
        }
        // A handle is a reference: the root task will take one the same way.
        machine
            .objects
            .retain(object)
            .unwrap_or_else(|error| testing::fail(format_args!("no reference: {error}")));
        handle
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")))
}

/// An endpoint of the machine, which the root task will make with
/// `endpoint_create` once it runs.
pub(crate) fn endpoint() -> EndpointId {
    with_machine(|machine| {
        machine
            .objects
            .endpoints
            .allocate(Endpoint::new())
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the endpoint")))
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")))
}

/// Installs a handle to `object` with `rights` and `badge` in the table of
/// `process`, which is what `endpoint_badge` makes of a capability.
pub(crate) fn install_badged(
    process: ProcessId,
    object: AnyObjectId,
    rights: Rights,
    badge: u64,
) -> Handle {
    let handle = install(process, object, rights);
    with_machine(|machine| {
        if let Ok(entry) = machine.objects.handles.lookup_mut(process, handle) {
            entry.badge = badge;
        }
    });
    handle
}

/// Maps every frame of `object` read and write at `address` of `process`,
/// which is what the root task does for a program it hands memory to.
pub(crate) fn map_memory(process: &UserProcess, object: MemoryObjectId, address: u64) {
    let frames = with_machine(|machine| {
        machine
            .objects
            .memory
            .get(object)
            .map(|held| held.frames)
            .unwrap_or_else(|_| testing::fail(format_args!("the memory object is gone")))
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")));
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        let mut environment = environment(memory, &mut tables, &mut tlb);
        for index in 0..frames.count() {
            let frame = frames
                .start()
                .checked_add(index)
                .unwrap_or_else(|| testing::fail(format_args!("the object runs past memory")));
            let at = VirtAddr::new(address.saturating_add(index.saturating_mul(PAGE_SIZE)))
                .unwrap_or_else(|_| {
                    testing::fail(format_args!("the mapping is outside user space"))
                });
            let page = Page::from_start(at)
                .unwrap_or_else(|_| testing::fail(format_args!("the mapping is not aligned")));
            environment
                .map(
                    process.root,
                    page,
                    frame,
                    Permissions::READ_WRITE.for_user(),
                    CachePolicy::WriteBack,
                )
                .unwrap_or_else(|error| {
                    testing::fail(format_args!("the object is not mapped: {error}"))
                });
        }
    });
}

/// Names `endpoint` as the endpoint the faults of `process` are reported on,
/// which `process_set_fault_handler` does once a program runs.
pub(crate) fn set_fault_handler(process: ProcessId, endpoint: EndpointId) {
    with_machine(|machine| {
        machine
            .objects
            .retain(AnyObjectId::of(endpoint))
            .unwrap_or_else(|error| testing::fail(format_args!("no reference: {error}")));
        machine
            .objects
            .processes
            .with(process, |held| held.fault_handler = Some((endpoint, 0)));
    });
}

/// Drops one reference to `object` and wakes whoever waited on it if that
/// was the last one, which is what the close of a handle does.
pub(crate) fn release(object: AnyObjectId) -> bool {
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        with_machine(|machine| {
            let mut environment = environment(memory, &mut tables, &mut tlb);
            let mut syscall = kernel_syscall::dispatch::Machine {
                objects: &mut machine.objects,
                scheduler: &mut machine.scheduler,
                environment: &mut environment,
            };
            kernel_syscall::lifetime::release(&mut syscall, object).unwrap_or(false)
        })
    })
    .flatten()
    .unwrap_or(false)
}

/// Ends `thread` wherever it is, taking it out of whatever it waits on
/// first, which is what `thread_kill` does.
pub(crate) fn kill(thread: ThreadId) -> bool {
    with_machine(|machine| {
        kernel_ipc::cancel(&mut machine.objects, thread);
        machine
            .scheduler
            .exit(&mut machine.objects.threads, thread)
            .is_ok_and(|outcome| outcome.reschedule)
    })
    .unwrap_or(false)
}

/// Gives the processor to whoever can run, over and over, until `done` says
/// the run is over. An image without a timer comes back here every time a
/// thread blocks or ends.
pub(crate) fn run_until(done: impl Fn() -> bool) {
    for _ in 0..RUN_LIMIT {
        if done() {
            return;
        }
        run_threads(None);
    }
    if !done() {
        testing::fail(format_args!(
            "the run did not end in {RUN_LIMIT} turns of the processor"
        ));
    }
}

/// How many turns of the processor a run of [`run_until`] may take.
const RUN_LIMIT: usize = 64;

/// The status word a thread found in its buffer.
pub(crate) fn thread_status(frame: PhysFrame) -> u64 {
    let mut window = window();
    window
        .frame_bytes_mut(frame)
        .and_then(|bytes| ipc_buffer::Buffer::new(bytes).status().ok())
        .map_or(u64::MAX, audhsos_abi::Status::raw)
}

/// A memory object over `frames` contiguous frames of the reserve, the way
/// the root task will hand memory out once it owns the boot regions.
pub(crate) fn memory_object(frames: u64) -> MemoryObjectId {
    let range = with_memory(|memory| {
        memory
            .frames_mut()
            .allocate_contiguous(frames, Alignment::PAGE)
            .unwrap_or_else(|error| {
                testing::fail(format_args!("no run of {frames} frames: {error}"))
            })
    })
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")));
    with_machine(|machine| {
        machine
            .objects
            .memory
            .allocate(MemoryObject::new(
                range,
                MemoryKind::Ram,
                CachePolicy::WriteBack,
            ))
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the memory object")))
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")))
}

/// Maps one page of memory read and write at [`SHARED_BASE`] of `process`
/// and returns the frame behind it, so that a test can read what the
/// threads of the process wrote there.
pub(crate) fn share_page(process: &UserProcess) -> PhysFrame {
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        let mut environment = environment(memory, &mut tables, &mut tlb);
        let frame = environment
            .allocate_frame()
            .unwrap_or_else(|error| testing::fail(format_args!("no frame to share: {error}")));
        let address = VirtAddr::new(SHARED_BASE).unwrap_or_else(|_| {
            testing::fail(format_args!("the shared page is outside user space"))
        });
        let page = Page::from_start(address)
            .unwrap_or_else(|_| testing::fail(format_args!("the shared page is not aligned")));
        environment
            .map(
                process.root,
                page,
                frame,
                Permissions::READ_WRITE.for_user(),
                CachePolicy::WriteBack,
            )
            .unwrap_or_else(|error| {
                testing::fail(format_args!("the shared page is not mapped: {error}"))
            });
        frame
    })
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")))
}

/// The environment the system call layer reaches the machine through.
fn environment<'a>(
    memory: &'a mut kernel_core::memory::KernelMemory,
    tables: &'a mut PhysicalWindow,
    tlb: &'a mut LocalTlb,
) -> KernelEnvironment<'a, X86Entry, PhysicalWindow, LocalTlb, SerialConsole, DeviceAccess<'a>> {
    KernelEnvironment::new(
        memory,
        tables,
        tlb,
        None,
        None,
        acpi_pointer(),
        context::prepare_user,
    )
    .at(now_micros())
}

/// The clock the system call layer answers with: the ticks the timer has
/// counted, in microseconds.
pub(crate) fn now_micros() -> u64 {
    kernel_core::tick::micros(interrupts::ticks())
}

/// The address of the root system description pointer, or zero when the
/// platform named none. `system_info` reports it.
pub(crate) fn acpi_pointer() -> u64 {
    testing::with_platform(|platform| {
        use kernel_hal_api::platform::Platform;
        platform.acpi_rsdp().map_or(0, |address| address.as_u64())
    })
    .unwrap_or(0)
}

/// Where the stack of thread `index` of a process ends. One unmapped page
/// lies between two stacks, so that a thread that runs off the bottom of
/// its own stack faults instead of writing into the one below.
fn stack_top_for(index: u64) -> u64 {
    let span = USER_STACK_PAGES.saturating_add(1).saturating_mul(PAGE_SIZE);
    USER_STACK_TOP.saturating_sub(index.saturating_mul(span))
}

/// Copies the program into frames of the reserve and maps them read and
/// execute at [`USER_BASE`].
fn map_program<A>(
    environment: &mut KernelEnvironment<'_, X86Entry, A, LocalTlb, SerialConsole, DeviceAccess<'_>>,
    root: PhysFrame,
    program: &[u8],
) where
    A: FrameAccess<PageTable<X86Entry>> + kernel_hal_api::paging::FrameBytes,
{
    let pages = u64::try_from(program.len().div_ceil(4096))
        .unwrap_or(0)
        .max(1);
    for index in 0..pages {
        let frame = environment.allocate_frame().unwrap_or_else(|error| {
            testing::fail(format_args!("no frame for the program: {error}"))
        });
        copy_page(frame, program, index);
        let address = VirtAddr::new(USER_BASE.saturating_add(index.saturating_mul(PAGE_SIZE)))
            .unwrap_or_else(|_| testing::fail(format_args!("the program does not fit user space")));
        let page = Page::from_start(address)
            .unwrap_or_else(|_| testing::fail(format_args!("the program is not page aligned")));
        environment
            .map(
                root,
                page,
                frame,
                Permissions::READ_EXECUTE.for_user(),
                CachePolicy::WriteBack,
            )
            .unwrap_or_else(|error| {
                testing::fail(format_args!("the program is not mapped: {error}"))
            });
    }
    say!("program mapped at {USER_BASE:#x}, {pages} page(s)");
}

/// Copies page `index` of `program` into `frame`.
fn copy_page(frame: PhysFrame, program: &[u8], index: u64) {
    let mut window = window();
    let Some(bytes) = window.frame_bytes_mut(frame) else {
        testing::fail(format_args!("a frame of the reserve is not reachable"));
    };
    let start = usize::try_from(index.saturating_mul(PAGE_SIZE)).unwrap_or(0);
    let end = start.saturating_add(4096).min(program.len());
    let Some(source) = program.get(start..end) else {
        return;
    };
    let Some(target) = bytes.get_mut(..source.len()) else {
        testing::fail(format_args!("a page of the program does not fit a frame"));
    };
    target.copy_from_slice(source);
}

/// Maps the stack that ends at `top`, read and write.
fn map_stack<A>(
    environment: &mut KernelEnvironment<'_, X86Entry, A, LocalTlb, SerialConsole, DeviceAccess<'_>>,
    root: PhysFrame,
    top: u64,
) where
    A: FrameAccess<PageTable<X86Entry>> + kernel_hal_api::paging::FrameBytes,
{
    for index in 0..USER_STACK_PAGES {
        let frame = environment
            .allocate_frame()
            .unwrap_or_else(|error| testing::fail(format_args!("no frame for the stack: {error}")));
        let offset = index.saturating_add(1).saturating_mul(PAGE_SIZE);
        let address = VirtAddr::new(top.saturating_sub(offset))
            .unwrap_or_else(|_| testing::fail(format_args!("the stack is outside user space")));
        let page = Page::from_start(address)
            .unwrap_or_else(|_| testing::fail(format_args!("the stack is not page aligned")));
        environment
            .map(
                root,
                page,
                frame,
                Permissions::READ_WRITE.for_user(),
                CachePolicy::WriteBack,
            )
            .unwrap_or_else(|error| {
                testing::fail(format_args!("the stack is not mapped: {error}"))
            });
    }
}

/// Puts the thread objects together, maps its IPC buffer, writes the frame
/// the first switch returns through, and starts it.
fn build(
    process: &UserProcess,
    priority: u8,
    stack: KernelStack,
    buffer: PhysFrame,
    stack_top: u64,
) -> Spawned {
    let idle = idle_thread();
    let entry = VirtAddr::new(USER_BASE).unwrap_or(VirtAddr::ZERO);
    let user_stack = VirtAddr::new(stack_top).unwrap_or(VirtAddr::ZERO);
    let (thread, buffer_address) = with_machine(|machine| {
        let thread = machine
            .objects
            .threads
            .allocate(
                Thread::new(process.id, priority, MAX_PRIORITY, stack.slot, buffer)
                    .unwrap_or_else(|_| testing::fail(format_args!("the thread is not one"))),
            )
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the thread")));
        let slot = machine
            .objects
            .processes
            .get_mut(process.id)
            .ok()
            .and_then(|holder| holder.add_thread(thread).ok())
            .unwrap_or_else(|| testing::fail(format_args!("the process holds no thread")));
        let raw = audhsos_abi::layout::ipc_buffer_address(slot).unwrap_or(0);
        let address = VirtAddr::new(raw).unwrap_or(VirtAddr::ZERO);
        if let Ok(entry_of) = machine.objects.threads.get_mut(thread) {
            *entry_of = entry_of.starting_at(entry, user_stack, address);
        }
        (thread, address)
    })
    .unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")));
    let _ = idle;

    // The buffer of the thread, at the top of its own address space.
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        let mut environment = environment(memory, &mut tables, &mut tlb);
        let page = Page::from_start(buffer_address)
            .unwrap_or_else(|_| testing::fail(format_args!("the buffer is not page aligned")));
        environment
            .map(
                process.root,
                page,
                buffer,
                Permissions::READ_WRITE.for_user(),
                CachePolicy::WriteBack,
            )
            .unwrap_or_else(|error| {
                testing::fail(format_args!("the buffer is not mapped: {error}"))
            });
    });

    // The frame the first switch returns through, written into the top of
    // the kernel stack of the thread.
    let top_frame = stack_frame_below(stack.top);
    let mut stack_window = window();
    let context = context::prepare_user(
        &mut stack_window,
        top_frame,
        stack.top,
        entry,
        user_stack,
        buffer_address,
    )
    .unwrap_or_else(|| testing::fail(format_args!("the kernel stack has no room for the frame")));
    with_machine(|machine| store_context(&mut machine.objects, thread, context));
    say!(
        "thread ready: entry {:#x}, stack {:#x}, buffer {:#x}, priority {priority}",
        entry.as_u64(),
        user_stack.as_u64(),
        buffer_address.as_u64()
    );
    Spawned {
        process: process.id,
        thread,
        buffer,
    }
}

/// The frame the page below `top` is mapped to, which is the page the
/// frame of a new thread is written into. The geometry of a slot is the
/// business of `kernel-mm`; this asks the page tables where the stack
/// really is.
fn stack_frame_below(top: VirtAddr) -> PhysFrame {
    let address = top
        .checked_sub(PAGE_SIZE)
        .unwrap_or_else(|| testing::fail(format_args!("the kernel stack starts at zero")));
    let page = Page::from_start(address)
        .unwrap_or_else(|_| testing::fail(format_args!("the kernel stack is not page aligned")));
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        let root = memory.root();
        let mapper = kernel_mm::mapper::Mapper::<'_, X86Entry, _, _, _>::new(
            root,
            &mut tables,
            &mut tlb,
            memory.frames_mut(),
        );
        mapper
            .translate(page)
            .map(|(frame, _)| frame)
            .unwrap_or_else(|| testing::fail(format_args!("the kernel stack is not mapped")))
    })
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")))
}

/// Whether the thread that would leave the processor is holding neither
/// the memory nor the machine.
///
/// This image's own thread is not inside a gate while it works: once
/// [`start_timer`] has run, it builds processes and threads with
/// interrupts on, and `create_process`, `share_page` and `add_thread`
/// take the memory out of its cell for the length of real work. A switch
/// from there leaves the cell borrowed by a thread that is no longer
/// running, and the first system call of whoever got the processor finds
/// it gone — [`answer`] can then do nothing at all, so a `thread_exit`
/// never ends its thread, the thread never gives the processor up, and
/// the image never comes back. That is sixty seconds of silence with no
/// summary line, and it is what the `ipc` image did about one run in six.
///
/// A tick that finds a cell held switches nobody and lets the next tick
/// try, a millisecond later. The tick hook of `preemption.rs` already
/// leaves work for the next tick on the same grounds.
fn nothing_is_held() -> bool {
    with_memory(|_| ()).is_some() && with_machine(|_| ()).is_some()
}

/// Gives the processor to whichever thread should run, and returns when
/// the kernel is back on the stack it was called on.
///
/// One switch and no more. What comes after the switch runs when somebody
/// switches back to this thread, which is a whole turn of the processor
/// later; asking the scheduler a second time before returning would take
/// the processor away from the thread that just got it, and two threads of
/// equal priority would trade it back and forth without either of them
/// ever reaching user mode again.
///
/// A call that finds nothing to switch to returns at once, which is how
/// the idle thread learns that the run is over. So does one made while
/// the thread that would leave holds the memory or the machine — see
/// [`nothing_is_held`].
///
/// `standing_on` names the thread whose kernel stack the processor stands
/// on when the scheduler has already let go of it, which is what a thread
/// that faulted looks like. Its context belongs in its own pool entry all
/// the same, so that resuming the thread returns here.
pub(crate) fn run_threads(standing_on: Option<ThreadId>) {
    if !nothing_is_held() {
        return;
    }
    let Some((from, to, top)) = next_switch(standing_on) else {
        return;
    };
    // A thread that has ended has nowhere to keep a context any more, and
    // nobody will ask for it: the switch writes it here, on the stack that
    // goes with it.
    let mut discarded = VirtAddr::ZERO;
    let from = from.unwrap_or(&raw mut discarded);
    if descriptors::set_kernel_stack(top.as_u64()).is_err() {
        testing::fail(format_args!("the task state segment is not reachable"));
    }
    // SAFETY: `from` points at the context word of the thread that is
    // running, which lives in the `static` cell of the machine and stays
    // where it is; `to` is a context the kernel wrote, of a thread whose
    // kernel stack is mapped in every address space.
    unsafe {
        context::switch_to(from, to);
    }
    // Back on this stack, a turn of the processor later: whatever ended
    // while we were away can go.
    sweep();
}

/// Gives back what every thread that has ended held.
pub(crate) fn sweep() {
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        with_machine(|machine| {
            let mut environment =
                KernelEnvironment::<X86Entry, _, _, SerialConsole, DeviceAccess<'_>>::new(
                    memory,
                    &mut tables,
                    &mut tlb,
                    None,
                    None,
                    acpi_pointer(),
                    context::prepare_user,
                );
            // The kernel stands on the stack of the thread it just
            // switched to, which is the one the scheduler now calls
            // current.
            let running = machine.scheduler.current();
            reap(
                &mut machine.objects,
                &mut machine.scheduler,
                &mut environment,
                running,
            )
        })
    });
}

/// The next switch, or `None` when the thread that should run is the one
/// that is running. The first element is where the context of the thread
/// that leaves belongs, and `None` when it has ended.
fn next_switch(
    standing_on: Option<ThreadId>,
) -> Option<(Option<*mut VirtAddr>, VirtAddr, VirtAddr)> {
    let mut spaces = AddressSpaces::current().ok()?;
    with_machine(|machine| {
        let next = schedule(
            &mut machine.objects,
            &mut machine.scheduler,
            &mut spaces,
            stack_top_of,
        );
        let switch = next.switch?;
        let leaving = switch.from.or(standing_on);
        let pointer = leaving.and_then(|from| {
            machine
                .objects
                .threads
                .get_mut(from)
                .ok()
                .map(|thread| &raw mut thread.context)
        });
        Some((pointer, switch.context, switch.kernel_stack_top))
    })
    .flatten()
}

/// The top of the kernel stack in `slot`: the slot begins with a guard
/// page and holds [`KERNEL_STACK_PAGES`] mapped pages above it. The idle
/// thread is the exception: it is this image, standing on the boot stack.
fn stack_top_of(slot: u32) -> VirtAddr {
    if slot == BOOT_SLOT {
        return VirtAddr::new(BOOT_STACK_TOP).unwrap_or(VirtAddr::ZERO);
    }
    let base = KERNEL_STACKS_BASE
        .saturating_add(u64::from(slot).saturating_mul(KERNEL_STACK_SLOT_PAGES * PAGE_SIZE));
    let top = base
        .saturating_add(PAGE_SIZE)
        .saturating_add(KERNEL_STACK_PAGES.saturating_mul(PAGE_SIZE));
    VirtAddr::new(top).unwrap_or(VirtAddr::ZERO)
}

/// What the kernel does with a system call: read the buffer of the thread
/// that made it, answer, clear away what ended, and switch if the answer
/// asks for it.
fn on_syscall() {
    let caller = with_machine(|machine| machine.scheduler.current()).flatten();
    let Some(caller) = caller else {
        return;
    };
    let frame = with_machine(|machine| {
        machine
            .objects
            .threads
            .get(caller)
            .ok()
            .map(|thread| thread.ipc_buffer)
    })
    .flatten();
    let Some(frame) = frame else {
        return;
    };

    // Two views of the window: one for the buffer of the thread, one for
    // the page tables. They never name the same frame, and this image is
    // the only writer of either.
    let mut buffer_window = window();
    let Some(bytes) = buffer_window.frame_bytes_mut(frame) else {
        return;
    };
    let number = ipc_buffer::Buffer::new(bytes).syscall_number();
    watch(number);
    // The hook is copied out before it runs, as every other hook of this
    // image is, so that no borrow of the cell is alive while it works.
    let hook = CALL_HOOK.borrow(&UncontendedToken).ok().map(|hook| *hook);
    if let Some(hook) = hook {
        hook(number);
    }
    // The interrupt controller and the ports, in an image that brought them
    // up. Holding the controller turns interrupts off for the length of the
    // call, which is what the calls that touch it need.
    let reschedule = if DEVICES.load(Ordering::SeqCst) == 0 {
        answer(caller, bytes, None)
    } else {
        interrupts::with_controller(|apics| {
            let mut devices = DeviceAccess::new(apics).with_entropy();
            answer(caller, bytes, Some(&mut devices))
        })
        .unwrap_or(false)
    };
    if reschedule {
        run_threads(Some(caller));
    }
}

/// The fault an exception names, or `None` for a vector that stops a thread
/// without a message. This is the mapping of `kernel_core::trap`.
fn of(exception: Exception) -> audhsos_abi::Fault {
    exception.fault().unwrap_or(audhsos_abi::Fault {
        kind: audhsos_abi::FaultKind::GeneralProtection,
        address: exception.ip,
        instruction_pointer: exception.ip,
        error_code: exception.error_code,
    })
}

/// Answers the system call of `caller` and clears away what ended.
fn answer(
    caller: ThreadId,
    bytes: &mut [u8; ipc_buffer::SIZE],
    devices: Option<&mut DeviceAccess<'_>>,
) -> bool {
    let mut tables = window();
    let mut tlb = LocalTlb;
    let mut devices = devices;
    let outcome = with_memory(|memory| {
        with_machine(|machine| {
            let mut environment =
                KernelEnvironment::<X86Entry, _, _, SerialConsole, DeviceAccess<'_>>::new(
                    memory,
                    &mut tables,
                    &mut tlb,
                    None,
                    devices.as_deref_mut(),
                    acpi_pointer(),
                    context::prepare_user,
                )
                .at(now_micros());
            let reschedule = handle_syscall(
                &mut machine.objects,
                &mut machine.scheduler,
                &mut environment,
                caller,
                bytes,
            );
            reap(
                &mut machine.objects,
                &mut machine.scheduler,
                &mut environment,
                Some(caller),
            );
            reschedule
        })
    });
    // A call that found either cell borrowed used to answer `false` and
    // change nothing whatever: no status in the caller's buffer, no
    // object touched, no thread ended. Silence is the worst of the
    // answers a system call can give, and the hang it caused was read as
    // a crash of the machine. `run_threads` keeps a switch from leaving a
    // thread that holds either cell, so this cannot arise; if it ever
    // does, the run says which cell it was and stops there.
    match outcome {
        Some(Some(reschedule)) => reschedule,
        Some(None) => testing::fail(format_args!(
            "a system call found the machine borrowed and would have done nothing"
        )),
        None => testing::fail(format_args!(
            "a system call found the memory borrowed and would have done nothing"
        )),
    }
}

/// How many system call numbers [`watch_syscalls`] remembers. More than
/// the table holds, so that a run which made an extra call shows it
/// instead of losing it off the end.
pub(crate) const WATCH_CAPACITY: usize = 64;

/// The numbers the kernel saw, in the order the calls arrived.
static WATCHED: [AtomicU32; WATCH_CAPACITY] = [const { AtomicU32::new(0) }; WATCH_CAPACITY];

/// How many of [`WATCHED`] hold a number.
static WATCHED_LEN: AtomicU32 = AtomicU32::new(0);

/// Whether the numbers are being remembered.
static WATCHING: AtomicU32 = AtomicU32::new(0);

/// Remembers the number every later system call of a user thread arrives
/// under.
///
/// This is what makes a wrapper of `user-sys-x86_64` checkable: the number
/// is what the wrapper wrote into the buffer, so a run of the wrappers in
/// the order of the table has to arrive here in the order of the table.
pub(crate) fn watch_syscalls() {
    WATCHED_LEN.store(0, Ordering::SeqCst);
    WATCHING.store(1, Ordering::SeqCst);
}

/// Copies the numbers that were seen into `into` and answers with how many
/// there were, which may be more than `into` holds.
pub(crate) fn watched_syscalls(into: &mut [u32]) -> usize {
    let len = usize::try_from(WATCHED_LEN.load(Ordering::SeqCst)).unwrap_or(0);
    for (slot, seen) in into.iter_mut().zip(WATCHED.iter().take(len)) {
        *slot = seen.load(Ordering::SeqCst);
    }
    len
}

/// Remembers `number`, the call the buffer of the caller stands on.
fn watch(number: u64) {
    if WATCHING.load(Ordering::SeqCst) == 0 {
        return;
    }
    let at = usize::try_from(WATCHED_LEN.load(Ordering::SeqCst)).unwrap_or(usize::MAX);
    if let Some(slot) = WATCHED.get(at) {
        slot.store(u32::try_from(number).unwrap_or(u32::MAX), Ordering::SeqCst);
    }
    // The length counts every call, whether there was room for it or not,
    // so a run that made more calls than the watch holds is a run the
    // reader can see was too long.
    WATCHED_LEN.store(
        u32::try_from(at.saturating_add(1)).unwrap_or(u32::MAX),
        Ordering::SeqCst,
    );
}

/// Whether the interrupt hardware of the machine is in place, so that the
/// calls which touch a line or a port can reach it.
static DEVICES: AtomicU32 = AtomicU32::new(0);

/// Takes the interrupt hardware over without starting the timer: an image
/// that wants the port and interrupt calls but no preemption asks for this,
/// and one that wants ticks calls [`start_timer`], which does it as well.
pub(crate) fn bring_up_devices() {
    if DEVICES.swap(1, Ordering::SeqCst) == 1 {
        return;
    }
    let brought_up = testing::with_platform(|platform| {
        // SAFETY: the loader's tables are active, the descriptor tables of
        // the image carry a handler for every vector of the plan, interrupts
        // are off, and this runs once on the boot processor.
        unsafe { interrupts::bring_up(platform, map_device) }
    });
    match brought_up {
        Some(Ok(())) => say!("the interrupt hardware is in place"),
        Some(Err(error)) => testing::fail(format_args!("the interrupt bring-up failed: {error}")),
        None => testing::fail(format_args!("the platform is not reachable")),
    }
}

/// What the kernel does with a processor exception.
///
/// A fault at ring zero is the kernel falling over, and the image fails
/// with it. A fault at ring three is one thread falling over: it stops in
/// [`ThreadState::Faulted`], keeps everything it holds, and the processor
/// goes to whoever is next. Nothing else of the machine notices.
fn on_trap(report: TrapReport) {
    let exception = Exception {
        vector: report.vector,
        error_code: report.error_code,
        ip: report.ip,
        sp: report.sp,
        cr2: report.fault_address,
        user: report.from_user(),
    };
    if exception.response() == Response::StopMachine {
        testing::fail(format_args!(
            "the kernel itself faulted: {} (vector {}) at ip {:#x} sp {:#x} error {:#x} address {:#x}",
            exception.name(),
            exception.vector,
            exception.ip,
            exception.sp,
            exception.error_code,
            exception.cr2
        ));
    }
    let mut state = KernelState::new();
    let mut reported = console();
    kernel_core::trap::on_user_fault(exception, &mut state, &mut reported);
    LAST_VECTOR.store(u32::from(exception.vector), Ordering::SeqCst);
    LAST_ERROR.store(exception.error_code, Ordering::SeqCst);
    LAST_ADDRESS.store(exception.cr2, Ordering::SeqCst);
    LAST_IP.store(exception.ip, Ordering::SeqCst);

    let Some(faulted) = with_machine(|machine| machine.scheduler.current()).flatten() else {
        testing::fail(format_args!(
            "a user thread faulted and the kernel holds none"
        ));
    };
    // The message a fault handler receives is built in the buffer of the
    // thread that faulted, which the kernel reaches the way the system call
    // gate does.
    let frame = with_machine(|machine| {
        machine
            .objects
            .threads
            .get(faulted)
            .ok()
            .map(|thread| thread.ipc_buffer)
    })
    .flatten();
    let mut buffer_window = window();
    let mut tables = window();
    let mut tlb = LocalTlb;
    let bytes = frame.and_then(|frame| buffer_window.frame_bytes_mut(frame));
    let Some(bytes) = bytes else {
        testing::fail(format_args!("the buffer of a faulting thread is gone"));
    };
    let reschedule = with_memory(|memory| {
        with_machine(|machine| {
            let mut environment =
                KernelEnvironment::<X86Entry, _, _, SerialConsole, DeviceAccess<'_>>::new(
                    memory,
                    &mut tables,
                    &mut tlb,
                    None,
                    None,
                    acpi_pointer(),
                    context::prepare_user,
                );
            let mut syscall = kernel_syscall::dispatch::Machine {
                objects: &mut machine.objects,
                scheduler: &mut machine.scheduler,
                environment: &mut environment,
            };
            fault::deliver(&mut syscall, faulted, of(exception), bytes).reschedule
        })
    })
    .flatten()
    .unwrap_or(false);
    FAULTS.fetch_add(1, Ordering::SeqCst);
    if reschedule {
        run_threads(Some(faulted));
    }
}

/// Starts the interval timer, once for the whole image, and lets
/// interrupts through. From here on the scheduler can take the processor
/// away from a thread that asks for nothing.
///
/// The image is the idle thread, and the timer has to keep reaching it:
/// [`run_until_idle`] is what a test calls instead of [`run_threads`] once
/// this has run.
pub(crate) fn start_timer(hook: TickHook) {
    if TIMER.swap(1, Ordering::SeqCst) == 1 {
        return;
    }
    let _ = TICK_HOOK.init(hook);
    traps::set_interrupt_handler(on_interrupt);
    bring_up_devices();
    // SAFETY: the interval timer belongs to the kernel, interrupts are
    // still off, and this is the processor the controller belongs to.
    if let Err(error) = unsafe { interrupts::start_timer(TICKS_PER_SECOND) } {
        testing::fail(format_args!("the timer did not start: {error}"));
    }
    let ticks = interrupts::ticks();
    say!("the timer runs, {ticks} ticks so far");
    enable_interrupts();
}

/// Lets interrupts through again. The kernel comes back to the idle thread
/// through a switch out of an interrupt handler, where interrupts are off,
/// and the flags of a switch are not saved: the thread that stands here
/// has to turn them back on itself.
fn enable_interrupts() {
    if TIMER.load(Ordering::SeqCst) == 0 {
        return;
    }
    // SAFETY: the descriptor table is loaded and every vector the hardware
    // can raise has a handler.
    unsafe {
        instructions::enable_interrupts();
    }
}

/// Turns interrupts off for as long as the kernel is deciding who runs.
///
/// Every other caller of [`run_threads`] is already inside an interrupt
/// gate and has them off; the idle thread is the one that does not. A tick
/// between the decision and the switch would find a thread the scheduler
/// already calls current standing nowhere, and the switch it made from
/// here would write the context of the idle thread into that thread's
/// entry. The window is a handful of instructions against a tick a
/// millisecond wide, which is exactly the kind of race that passes a
/// hundred runs and fails the hundred and first.
fn disable_interrupts() {
    if TIMER.load(Ordering::SeqCst) == 0 {
        return;
    }
    // SAFETY: nothing between here and the switch needs an interrupt, and
    // the thread that takes the processor turns them back on itself: a
    // user thread through the flags of its interrupt return, the idle
    // thread through `enable_interrupts` below.
    unsafe {
        instructions::disable_interrupts();
    }
}

/// Runs whatever is runnable and comes back when nothing is, with the
/// timer still reaching this thread. This is the loop of the idle thread
/// in an image that has a timer.
pub(crate) fn run_until_idle() {
    disable_interrupts();
    run_threads(None);
    enable_interrupts();
}

/// Waits, halting, until `done` says the run is over, giving the processor
/// to whoever the timer makes ready in between.
pub(crate) fn idle_until(done: impl Fn() -> bool) {
    let mut spins: u64 = 0;
    while !done() {
        run_until_idle();
        if done() {
            break;
        }
        if spins >= IDLE_LIMIT {
            testing::fail(format_args!(
                "the idle thread waited {IDLE_LIMIT} turns and the run did not end"
            ));
        }
        spins = spins.saturating_add(1);
        instructions::halt();
    }
    run_until_idle();
}

/// How long the idle thread waits before it believes the run will never
/// end. The timer ticks a thousand times a second, so this is minutes.
const IDLE_LIMIT: u64 = 200_000;

/// Maps the frames of a device register window into the physical window,
/// uncached, out of the address space the memory bring-up left.
fn map_device(frames: PhysFrameRange) -> Option<VirtAddr> {
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|kernel| {
        kernel
            .map_device::<X86Entry, _, _>(&mut tables, &mut tlb, frames)
            .ok()
    })
    .flatten()
}

/// What the kernel does with a device interrupt: acknowledge it at the
/// hardware first, so that the next one can arrive whatever happens next;
/// then give the image its turn, charge the running thread's time slice,
/// and switch when either asks for it.
fn on_interrupt(vector: u8) {
    if vector != vectors::TIMER {
        forward_interrupt(vector);
        return;
    }
    interrupts::acknowledge(vector);
    record_turn();
    let ticks = interrupts::ticks();
    let hook = TICK_HOOK.borrow(&UncontendedToken).ok().map(|hook| *hook);
    let asked = hook.is_some_and(|hook| hook(ticks));
    let woken = wake_deadlines();
    let expired = with_machine(|machine| {
        machine
            .scheduler
            .tick(&mut machine.objects.threads)
            .reschedule
    })
    .unwrap_or(false);
    if asked || woken || expired {
        // The thread that leaves is the one the scheduler still calls
        // current, so the switch finds where to write its context by
        // itself; a thread the hook ended is not one to come back to.
        run_threads(None);
    }
}

/// Wakes every thread whose deadline has passed, and writes into each one's
/// buffer the zero bits it woke with. Returns whether one of them should
/// take the processor.
fn wake_deadlines() -> bool {
    let now = now_micros();
    let mut switch = false;
    loop {
        let woken = with_machine(|machine| {
            kernel_ipc::expire(&mut machine.objects, &mut machine.scheduler, now)
        });
        let Some(Some(outcome)) = woken else {
            break;
        };
        if let Some(wakeup) = outcome.wakeup {
            write_wakeup(wakeup);
        }
        switch |= outcome.reschedule;
    }
    switch
}

/// Forwards a device interrupt to whoever holds the interrupt object of its
/// vector, in the order 2.7 gives: mask the line at the I/O APIC, end the
/// interrupt at the local APIC, then signal the notification.
///
/// A vector no interrupt object names is acknowledged and nothing else.
fn forward_interrupt(vector: u8) {
    let line = with_machine(|machine| {
        kernel_ipc::interrupt_for(&machine.objects, vector).and_then(|id| {
            machine
                .objects
                .interrupts
                .get(id)
                .ok()
                .and_then(|held| held.line)
        })
    })
    .flatten();
    if let Some(line) = line {
        interrupts::with_controller(|apics| {
            use kernel_hal_api::interrupt::InterruptController;
            apics.mask(kernel_hal_api::interrupt::InterruptLine::new(line));
        });
    }
    interrupts::acknowledge(vector);
    let outcome = with_machine(|machine| {
        kernel_ipc::deliver(&mut machine.objects, &mut machine.scheduler, vector)
    })
    .flatten();
    let Some(outcome) = outcome else {
        return;
    };
    if let Some(wakeup) = outcome.wakeup {
        write_wakeup(wakeup);
    }
    if outcome.reschedule {
        run_threads(None);
    }
}

/// Writes the status word and the return words a thread that a device
/// interrupt woke finds in its buffer.
fn write_wakeup(wakeup: kernel_ipc::Wakeup) {
    let frame = with_machine(|machine| {
        machine
            .objects
            .threads
            .get(wakeup.thread)
            .ok()
            .map(|thread| thread.ipc_buffer)
    })
    .flatten();
    let Some(frame) = frame else {
        return;
    };
    let mut buffer_window = window();
    let Some(bytes) = buffer_window.frame_bytes_mut(frame) else {
        return;
    };
    let mut writer = ipc_buffer::BufferMut::new(bytes);
    writer.clear_result();
    writer.set_status(wakeup.status);
    for (index, value) in wakeup.values.iter().enumerate() {
        writer.set_return_word(index, *value);
    }
}

/// Writes down which thread the timer found on the processor, unless it is
/// the one the tick before it found.
fn record_turn() {
    // A tick that arrives while the kernel holds the machine says nothing
    // about whose turn it is: it arrived inside a system call of the
    // thread whose turn it already was.
    let Some(running) = with_machine(|machine| machine.scheduler.current()) else {
        return;
    };
    let current = running.map_or(NO_THREAD, |thread| thread.index());
    let seen = TURNS_SEEN.load(Ordering::SeqCst);
    let last = seen
        .checked_sub(1)
        .and_then(|index| TURN_OF.get(usize::try_from(index).unwrap_or(TURNS)))
        .map(|slot| slot.load(Ordering::SeqCst));
    if last == Some(current) {
        return;
    }
    if let Some(slot) = TURN_OF.get(usize::try_from(seen).unwrap_or(TURNS)) {
        slot.store(current, Ordering::SeqCst);
    }
    TURNS_SEEN.store(seen.saturating_add(1), Ordering::SeqCst);
}

/// How many turns of the processor the timer saw. More than [`TURNS`]
/// means the log holds only the first ones.
pub(crate) fn turns_seen() -> u32 {
    TURNS_SEEN.load(Ordering::SeqCst)
}

/// The thread that held turn `index`, as its pool index; [`NO_THREAD`] for
/// the idle thread and for a turn the log does not hold.
pub(crate) fn turn(index: usize) -> u32 {
    TURN_OF
        .get(index)
        .map_or(NO_THREAD, |slot| slot.load(Ordering::SeqCst))
}

/// `true` when the timer found `thread` on the processor at least once.
pub(crate) fn took_a_turn(thread: ThreadId) -> bool {
    let wanted = thread.index();
    let seen = usize::try_from(turns_seen()).unwrap_or(TURNS).min(TURNS);
    (0..seen).any(|index| turn(index) == wanted)
}

/// How many user threads a fault has stopped since the image started.
pub(crate) fn faults() -> u32 {
    FAULTS.load(Ordering::SeqCst)
}

/// What the processor reported for the last one of them.
pub(crate) fn last_fault() -> LastFault {
    LastFault {
        vector: LAST_VECTOR.load(Ordering::SeqCst),
        error_code: LAST_ERROR.load(Ordering::SeqCst),
        address: LAST_ADDRESS.load(Ordering::SeqCst),
        ip: LAST_IP.load(Ordering::SeqCst),
    }
}

/// The word `index` of the IPC buffer in `frame`.
pub(crate) fn buffer_word(frame: PhysFrame, index: usize) -> u64 {
    let mut window = window();
    window
        .frame_bytes_mut(frame)
        .and_then(|bytes| ipc_buffer::Buffer::new(bytes).word(index))
        .unwrap_or(0)
}

/// Writes the word `index` of the IPC buffer in `frame`. This is how the
/// kernel of a test image tells a thread something before it starts: the
/// program reads the word its own kind agreed on, and the buffer is the
/// only thing both of them can reach.
pub(crate) fn set_buffer_word(frame: PhysFrame, index: usize, value: u64) {
    let mut window = window();
    let Some(bytes) = window.frame_bytes_mut(frame) else {
        testing::fail(format_args!("the buffer of a thread is not reachable"));
    };
    if !ipc_buffer::BufferMut::new(bytes).set_word(index, value) {
        testing::fail(format_args!("word {index} is beyond the buffer"));
    }
}

/// The word at `offset` of `frame`, as a page a program writes into holds
/// it. A shared page carries no IPC buffer layout: it is what the program
/// makes of it.
pub(crate) fn page_word(frame: PhysFrame, index: usize) -> u64 {
    let mut window = window();
    let Some(bytes) = window.frame_bytes_mut(frame) else {
        return 0;
    };
    let start = index.saturating_mul(8);
    let end = start.saturating_add(8);
    let Some(word) = bytes.get(start..end) else {
        return 0;
    };
    let mut value = [0_u8; 8];
    value.copy_from_slice(word);
    u64::from_le_bytes(value)
}

/// The state of a thread, or `None` when the pool no longer holds it.
pub(crate) fn state_of(thread: ThreadId) -> Option<ThreadState> {
    with_machine(|machine| machine.objects.threads.get(thread).ok().map(|t| t.state)).flatten()
}
