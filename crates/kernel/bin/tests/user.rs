// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The first user thread of this system: a program of its own, in an
//! address space of its own, at privilege level three, asking the kernel
//! for something through the one gate it may enter.
//!
//! The image builds a process by hand, the way the root task will be built
//! in Phase 7: an address space that carries the kernel half, the flat
//! binary of a test program mapped read and execute, a stack, and a thread
//! whose kernel stack carries the frame the first switch returns through.
//! Then it switches into it and waits to be switched back.
//!
//! What the tests watch is what came back: the state of the thread, what
//! it left in its own IPC buffer, and what the kernel did with the call it
//! made. Nothing here reads a register.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::layout::{
    BOOT_STACK_TOP, KERNEL_STACK_PAGES, KERNEL_STACK_SLOT_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE,
};
use audhsos_abi::{ThreadState, ipc_buffer};
use kernel_core::machine::with_machine;
use kernel_core::memory::{self, with_memory};
use kernel_core::syscall::{KernelEnvironment, handle_syscall, reap, schedule, store_context};
use kernel_hal_x86_64::console::SerialConsole;
use kernel_hal_x86_64::paging::{AddressSpaces, LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_hal_x86_64::{context, descriptors, testing, traps};
use kernel_mm::page_table::{CachePolicy, Permissions};
use kernel_objects::handle_table::HandleList;
use kernel_objects::object::{Process, ProcessId, Thread, ThreadId};
use kernel_objects::quota::Quota;
use kernel_syscall::environment::Environment;
use kernel_types::{Page, PhysFrame, VirtAddr};

kernel_hal_x86_64::test_kernel!();

/// The program that writes a mark into its buffer, gives up the processor
/// once, and ends itself.
static COUNT_AND_EXIT: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/count_and_exit.bin"
));

/// The program that ends itself and nothing else.
static THREAD_EXIT: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/thread_exit.bin"));

/// What `count_and_exit` leaves in the first payload word of its buffer.
const MARK: u64 = 0x5EED_0001;

/// Where a program is linked and mapped.
const USER_BASE: u64 = 0x40_0000;

/// Where the stack of a user thread ends.
const USER_STACK_TOP: u64 = 0x80_0000;

/// How many pages that stack has.
const USER_STACK_PAGES: u64 = 2;

/// The window over physical memory. The tables the loader built stay
/// active for the whole run, so the window maps every physical frame read
/// and write, and this image is the only writer.
fn window() -> PhysicalWindow {
    // SAFETY: the loader's tables stay active and the image runs on one
    // processor with nothing else touching kernel memory.
    unsafe { PhysicalWindow::kernel() }
}

/// The serial console, for the running commentary of this image.
fn console() -> SerialConsole {
    // SAFETY: the first serial controller belongs to the kernel while the
    // debug console is on; no userland driver exists in this image.
    unsafe { SerialConsole::new(0x3F8) }
}

/// Writes one line of commentary.
macro_rules! say {
    ($($argument:tt)*) => {{
        use kernel_hal_api::console::DebugConsole;
        let mut console = console();
        console.write_bytes(b"[user] ");
        let _ = core::fmt::Write::write_fmt(&mut Writer(&mut console), format_args!($($argument)*));
        console.write_bytes(b"\n");
    }};
}

/// Turns a console into something `format_args!` can write to.
struct Writer<'a>(&'a mut SerialConsole);

impl core::fmt::Write for Writer<'_> {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        use kernel_hal_api::console::DebugConsole;
        self.0.write_bytes(text.as_bytes());
        Ok(())
    }
}

/// Brings the memory of the kernel up, once for the whole image.
fn bring_up() {
    if with_memory(|_| ()).is_some() {
        return;
    }
    let Ok(root) = active_root() else {
        testing::fail(format_args!("the page table root is not addressable"));
    };
    let mut window = window();
    let mut tlb = LocalTlb;
    let brought_up = testing::with_platform(|platform| {
        memory::initialize::<X86Entry, _, _, _>(platform, root, &mut window, &mut tlb, 0)
    });
    match brought_up {
        Some(Ok(())) => {}
        Some(Err(error)) => testing::fail(format_args!("the memory bring-up failed: {error}")),
        None => testing::fail(format_args!("the platform is not reachable")),
    }
    traps::set_syscall_handler(on_syscall);
    say!("memory is up and the system call gate is armed");
}

/// The kernel's own process and the idle thread, which is this image
/// itself: the thread the kernel switches back into when no user thread
/// can run. Its context is written by the first switch away from it.
fn idle_thread() -> ThreadId {
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

/// A user process with `program` mapped at [`USER_BASE`], a stack, and one
/// thread ready to run.
struct Spawned {
    /// The process.
    process: ProcessId,
    /// Its thread.
    thread: ThreadId,
    /// The frame holding the IPC buffer of that thread.
    buffer: PhysFrame,
}

/// Builds that process and starts its thread.
fn spawn(program: &[u8]) -> Spawned {
    let mut tables = window();
    let mut tlb = LocalTlb;
    let (process, thread, buffer) = with_memory(|memory| {
        let mut environment = KernelEnvironment::<X86Entry, _, _, SerialConsole>::new(
            memory,
            &mut tables,
            &mut tlb,
            None,
        );
        let root = environment
            .create_address_space()
            .unwrap_or_else(|error| testing::fail(format_args!("no address space: {error}")));
        map_program(&mut environment, root, program);
        map_stack(&mut environment, root);
        let stack = environment
            .allocate_kernel_stack()
            .unwrap_or_else(|error| testing::fail(format_args!("no kernel stack: {error}")));
        let buffer = environment
            .allocate_frame()
            .unwrap_or_else(|error| testing::fail(format_args!("no buffer frame: {error}")));
        (root, stack, buffer)
    })
    .map(|(root, stack, buffer)| build(root, stack, buffer))
    .unwrap_or_else(|| testing::fail(format_args!("the kernel memory is not reachable")));
    Spawned {
        process,
        thread,
        buffer,
    }
}

/// Copies the program into frames of the reserve and maps them read and
/// execute at [`USER_BASE`].
fn map_program<A>(
    environment: &mut KernelEnvironment<'_, X86Entry, A, LocalTlb, SerialConsole>,
    root: PhysFrame,
    program: &[u8],
) where
    A: kernel_hal_api::paging::FrameAccess<kernel_mm::page_table::PageTable<X86Entry>>,
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

/// Maps the stack of the user thread, read and write.
fn map_stack<A>(
    environment: &mut KernelEnvironment<'_, X86Entry, A, LocalTlb, SerialConsole>,
    root: PhysFrame,
) where
    A: kernel_hal_api::paging::FrameAccess<kernel_mm::page_table::PageTable<X86Entry>>,
{
    for index in 0..USER_STACK_PAGES {
        let frame = environment
            .allocate_frame()
            .unwrap_or_else(|error| testing::fail(format_args!("no frame for the stack: {error}")));
        let offset = index.saturating_add(1).saturating_mul(PAGE_SIZE);
        let address = VirtAddr::new(USER_STACK_TOP.saturating_sub(offset))
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

/// Puts the objects of the process together and starts its thread.
fn build(
    root: PhysFrame,
    stack: kernel_syscall::environment::KernelStack,
    buffer: PhysFrame,
) -> (ProcessId, ThreadId, PhysFrame) {
    let idle = idle_thread();
    let entry = VirtAddr::new(USER_BASE).unwrap_or(VirtAddr::ZERO);
    let user_stack = VirtAddr::new(USER_STACK_TOP).unwrap_or(VirtAddr::ZERO);
    let result = with_machine(|machine| {
        let process = machine
            .objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(8),
                Quota::new(32),
                Quota::new(32),
            ))
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the process")));
        let thread = machine
            .objects
            .threads
            .allocate(
                Thread::new(process, 4, 8, stack.slot, buffer)
                    .unwrap_or_else(|_| testing::fail(format_args!("the thread is not one"))),
            )
            .unwrap_or_else(|_| testing::fail(format_args!("no slot for the thread")));
        let slot = machine
            .objects
            .processes
            .get_mut(process)
            .ok()
            .and_then(|holder| holder.add_thread(thread).ok())
            .unwrap_or_else(|| testing::fail(format_args!("the process holds no thread")));
        let raw = audhsos_abi::layout::ipc_buffer_address(slot).unwrap_or(0);
        let address = VirtAddr::new(raw).unwrap_or(VirtAddr::ZERO);
        if let Ok(entry_of) = machine.objects.threads.get_mut(thread) {
            *entry_of = entry_of.starting_at(entry, user_stack, address);
        }
        let _ = machine
            .scheduler
            .start(&mut machine.objects.threads, thread);
        (process, thread, address, idle)
    });
    let (process, thread, buffer_address, _) =
        result.unwrap_or_else(|| testing::fail(format_args!("the machine is not reachable")));

    // The buffer of the thread, at the top of its own address space.
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        let mut environment = KernelEnvironment::<X86Entry, _, _, SerialConsole>::new(
            memory,
            &mut tables,
            &mut tlb,
            None,
        );
        let page = Page::from_start(buffer_address)
            .unwrap_or_else(|_| testing::fail(format_args!("the buffer is not page aligned")));
        environment
            .map(
                root,
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
        "thread ready: entry {:#x}, stack {:#x}, buffer {:#x}",
        entry.as_u64(),
        user_stack.as_u64(),
        buffer_address.as_u64()
    );
    (process, thread, buffer)
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

/// Switches into whichever thread should run, and returns when the kernel
/// is back on its own stack with nothing left to run.
fn run_threads() {
    loop {
        let Some(next) = next_switch() else {
            return;
        };
        let (from, to, top) = next;
        // A thread that has ended has nowhere to keep a context any more,
        // and nobody will ask for it: the switch writes it here, on the
        // stack that goes with it.
        let mut discarded = VirtAddr::ZERO;
        let from = from.unwrap_or(&raw mut discarded);
        if descriptors::set_kernel_stack(top.as_u64()).is_err() {
            testing::fail(format_args!("the task state segment is not reachable"));
        }
        // SAFETY: `from` points at the context word of the thread that is
        // running, which lives in the `static` cell of the machine and
        // stays where it is; `to` is a context the kernel wrote, of a
        // thread whose kernel stack is mapped in every address space.
        unsafe {
            context::switch_to(from, to);
        }
        // Back on this stack: whatever ended while we were away can go.
        sweep();
    }
}

/// Gives back what every thread that has ended held.
fn sweep() {
    let mut tables = window();
    let mut tlb = LocalTlb;
    with_memory(|memory| {
        with_machine(|machine| {
            let mut environment = KernelEnvironment::<X86Entry, _, _, SerialConsole>::new(
                memory,
                &mut tables,
                &mut tlb,
                None,
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
fn next_switch() -> Option<(Option<*mut VirtAddr>, VirtAddr, VirtAddr)> {
    let mut spaces = AddressSpaces::current().ok()?;
    with_machine(|machine| {
        let next = schedule(
            &mut machine.objects,
            &mut machine.scheduler,
            &mut spaces,
            |slot| stack_top_of(slot),
        );
        let switch = next.switch?;
        let pointer = switch.from.and_then(|from| {
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

/// The slot the idle thread carries: it stands on the boot stack, which
/// the loader made and the stack area does not hold.
const BOOT_SLOT: u32 = u32::MAX;

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
    let mut tables = window();
    let mut tlb = LocalTlb;
    let Some(bytes) = buffer_window.frame_bytes_mut(frame) else {
        return;
    };
    let number = ipc_buffer::Buffer::new(bytes).syscall_number();
    let reschedule = with_memory(|memory| {
        with_machine(|machine| {
            let mut environment = KernelEnvironment::<X86Entry, _, _, SerialConsole>::new(
                memory,
                &mut tables,
                &mut tlb,
                None,
            );
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
    })
    .flatten()
    .unwrap_or(false);
    say!("system call {number} from a user thread, reschedule: {reschedule}");
    if reschedule {
        run_threads();
    }
}

/// The word `index` of the IPC buffer in `frame`.
fn buffer_word(frame: PhysFrame, index: usize) -> u64 {
    let mut window = window();
    window
        .frame_bytes_mut(frame)
        .and_then(|bytes| ipc_buffer::Buffer::new(bytes).word(index))
        .unwrap_or(0)
}

/// The state of a thread.
fn state_of(thread: ThreadId) -> Option<ThreadState> {
    with_machine(|machine| machine.objects.threads.get(thread).ok().map(|t| t.state)).flatten()
}

/// The first user thread of this system runs its own program in its own
/// address space, writes into its own buffer, and ends itself.
#[test_case]
fn a_user_thread_runs_its_own_program_and_ends_itself() {
    bring_up();
    let spawned = spawn(COUNT_AND_EXIT);
    say!("switching into user mode");
    run_threads();
    say!("back in the kernel");

    let mark = buffer_word(spawned.buffer, 0);
    if mark != MARK {
        testing::fail(format_args!(
            "the thread left {mark:#x} in its buffer, not {MARK:#x}"
        ));
    }
    say!("the thread wrote {mark:#x} into its own buffer");
    let state = state_of(spawned.thread);
    if state.is_some() {
        testing::fail(format_args!("the thread is still in the pool: {state:?}"));
    }
    say!("the thread ended and the kernel cleared it away");
    let _ = spawned.process;
}

/// A second process runs in the same machine after the first one ended.
#[test_case]
fn a_second_thread_runs_after_the_first_one_ended() {
    bring_up();
    let spawned = spawn(THREAD_EXIT);
    run_threads();
    if state_of(spawned.thread).is_some() {
        testing::fail(format_args!("the second thread did not end"));
    }
    say!("a second process ran and ended in the same machine");
}
