// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::syscall`: the environment the calls reach the machine
//! through, and the two entry points the architecture layer calls.

use audhsos_abi::ipc_buffer::{BufferMut, SIZE};
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Syscall};
use kernel_hal_api::doubles::{
    RecordingAddressSpaces, RecordingConsole, RecordingDevices, RecordingTlb,
};
use kernel_hal_api::paging::FrameAccess;
use kernel_mm::kernel_half::{entry_of, shares_kernel_half};
use kernel_mm::page_table::{EntryFormat, PageTable, Permissions, X86Entry};
use kernel_objects::handle_table::HandleList;
use kernel_objects::object::{Process, Thread};
use kernel_objects::quota::Quota;
use kernel_objects::store::Objects;
use kernel_sched::Scheduler;
use kernel_syscall::environment::Environment;
use kernel_types::{CachePolicy, Page, PhysAddr, PhysFrame, VirtAddr};

use crate::syscall::{KernelEnvironment, process_of, schedule, store_context};

/// A machine of a handful of slots.
type Small = Objects<4, 8, 8, 32>;

/// The frame with the given number.
fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

/// An address in the kernel half.
fn kernel_address(raw: u64) -> VirtAddr {
    VirtAddr::new(raw).unwrap()
}

/// The kernel memory of these tests, brought up the way the memory tests
/// bring it up, and the tables it lives in.
fn kernel_memory() -> (super::memory::Machine, crate::memory::KernelMemory) {
    let mut machine = super::memory::Machine::full();
    let mut tlb = RecordingTlb::new();
    let memory = crate::memory::bring_up::<X86Entry, _, _, _>(
        &super::memory::platform(),
        machine.root,
        &mut machine.access,
        &mut tlb,
        0,
    )
    .unwrap();
    (machine, memory)
}

#[test]
fn a_fresh_address_space_carries_the_kernel_half_and_nothing_else() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let kernel_root = memory.root();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );

    let root = environment.create_address_space().unwrap();
    assert_ne!(root, kernel_root);
    assert_eq!(
        shares_kernel_half(&machine.access, kernel_root, root),
        Ok(true)
    );
    let table = machine.access.table(root).unwrap();
    for index in 0..kernel_mm::kernel_half::FIRST_KERNEL_ENTRY {
        assert!(
            !table.entry(index).is_present(),
            "the user half of a fresh address space is empty at {index}"
        );
    }
    assert!(
        table
            .entry(entry_of(audhsos_abi::layout::KERNEL_BASE))
            .is_present(),
        "the kernel image is reachable through it"
    );
}

#[test]
fn an_address_space_without_a_frame_is_refused() {
    let (mut machine, mut memory) = kernel_memory();
    // Take every frame of the reserve, so that nothing is left for a root.
    while memory.frames_mut().allocate().is_ok() {}
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    assert_eq!(
        environment.create_address_space(),
        Err(Error::OutOfKernelMemory)
    );
}

#[test]
fn an_address_space_that_is_taken_apart_gives_its_tables_back() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let kernel_root = memory.root();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    let free = environment.memory.frames().free_count();
    let root = environment.create_address_space().unwrap();
    let page = Page::containing(kernel_address(0x40_0000));
    environment
        .map(
            root,
            page,
            frame(600),
            Permissions::READ_WRITE.for_user(),
            CachePolicy::WriteBack,
        )
        .unwrap();
    assert!(environment.memory.frames().free_count() < free);

    environment.destroy_address_space(root);
    assert_eq!(
        environment.memory.frames().free_count(),
        free,
        "every table of the address space went back into the reserve"
    );
    // The kernel keeps its own.
    environment.destroy_address_space(kernel_root);
    assert_eq!(environment.memory.root(), kernel_root);
}

#[test]
fn a_mapping_of_a_user_page_is_what_the_page_tables_hold_afterwards() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    let root = environment.create_address_space().unwrap();
    let page = Page::containing(kernel_address(0x40_0000));
    let target = frame(700);
    let perms = Permissions::READ_WRITE.for_user();
    environment
        .map(root, page, target, perms, CachePolicy::WriteBack)
        .unwrap();
    assert_eq!(
        environment.map(root, page, target, perms, CachePolicy::WriteBack),
        Err(Error::AlreadyMapped)
    );

    environment
        .protect(root, page, Permissions::READ_ONLY.for_user())
        .unwrap();
    environment.unmap(root, page).unwrap();
    assert_eq!(environment.unmap(root, page), Err(Error::NotMapped));
    assert_eq!(
        environment.protect(root, page, perms),
        Err(Error::NotMapped)
    );
}

#[test]
fn a_kernel_stack_and_a_frame_come_out_of_the_reserve_and_go_back() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    let free = environment.memory.frames().free_count();

    let stack = environment.allocate_kernel_stack().unwrap();
    assert!(stack.top.as_u64() > audhsos_abi::layout::KERNEL_STACKS_BASE);
    let buffer = environment.allocate_frame().unwrap();
    assert!(environment.memory.frames().free_count() < free);

    environment.release_frame(buffer);
    environment.release_kernel_stack(stack.slot);
    assert_eq!(environment.memory.frames().free_count(), free);
    // A slot nobody took is no stack, and giving it back changes nothing.
    environment.release_kernel_stack(9999);
    assert_eq!(environment.memory.frames().free_count(), free);
}

#[test]
fn a_frame_for_an_ipc_buffer_is_zeroed_before_it_is_handed_out() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    let frame = environment.allocate_frame().unwrap();
    // What a frame of the reserve looks like through the window is a page
    // table in this double; every word of it is zero.
    let table = environment.access.table(frame).unwrap();
    assert!(
        (0..512).all(|index| !table.entry(index).is_present()),
        "nothing of what stood there before reaches the thread"
    );
    assert_eq!(*table, PageTable::new());
}

#[test]
fn what_debug_log_writes_reaches_the_console_and_a_build_without_one_drops_it() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    {
        let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            Some(&mut console),
            None,
            0,
            no_frame,
        );
        environment.log(b"hello");
    }
    assert_eq!(console.output(), b"hello");

    let mut environment =
        KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            None,
            None,
            0,
            no_frame,
        );
    environment.log(b"nowhere");
}

/// A machine with two processes, each with one thread of the priority
/// given, and the roots of the two address spaces.
fn two_processes(first: u8, second: u8) -> (Small, Scheduler, [PhysFrame; 2]) {
    let mut objects = Small::new();
    let mut scheduler = Scheduler::new();
    let mut roots = [frame(1), frame(2)];
    for (index, priority) in [first, second].into_iter().enumerate() {
        let root = frame(u64::try_from(index).unwrap().saturating_add(1));
        *roots.get_mut(index).unwrap() = root;
        let process = objects
            .processes
            .allocate(Process::new(
                root,
                HandleList::with_capacity(4),
                Quota::new(8),
                Quota::new(8),
            ))
            .unwrap();
        let buffer = PhysFrame::containing(PhysAddr::new(0x10_0000).unwrap());
        let thread = objects
            .threads
            .allocate(
                Thread::new(process, priority, 31, u32::try_from(index).unwrap(), buffer).unwrap(),
            )
            .unwrap();
        objects
            .processes
            .get_mut(process)
            .unwrap()
            .add_thread(thread)
            .unwrap();
        scheduler.start(&mut objects.threads, thread).unwrap();
    }
    (objects, scheduler, roots)
}

/// The top of the kernel stack of a slot, as the architecture layer
/// computes it.
fn stack_top(slot: u32) -> VirtAddr {
    let slot_bytes = u64::from(slot)
        .saturating_mul(audhsos_abi::layout::KERNEL_STACK_SLOT_PAGES)
        .saturating_mul(PAGE_SIZE);
    kernel_address(audhsos_abi::layout::KERNEL_STACKS_BASE.saturating_add(slot_bytes))
}

#[test]
fn a_switch_between_processes_loads_the_root_and_one_inside_a_process_does_not() {
    let (mut objects, mut scheduler, roots) = two_processes(5, 5);
    let mut spaces = RecordingAddressSpaces::new(roots[0]);

    // The first switch takes the thread of the first process, whose root
    // is the one already loaded.
    let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    let switch = next.switch.expect("a switch");
    assert_eq!(switch.from, None);
    assert_eq!(switch.address_space, None, "its root is already loaded");
    assert_eq!(spaces.switches(), 0);

    // The next takes the thread of the second process.
    let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    let switch = next.switch.expect("a switch");
    assert_eq!(switch.address_space, Some(roots[1]));
    assert_eq!(spaces.loaded(), [roots[1]]);

    // And the one after that goes back to the first.
    let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    let switch = next.switch.expect("a switch");
    assert_eq!(switch.address_space, Some(roots[0]));
    assert_eq!(spaces.switches(), 2);
}

#[test]
fn two_threads_of_one_process_switch_without_touching_the_root() {
    let mut objects = Small::new();
    let mut scheduler = Scheduler::new();
    let root = frame(3);
    let process = objects
        .processes
        .allocate(Process::new(
            root,
            HandleList::with_capacity(4),
            Quota::new(8),
            Quota::new(8),
        ))
        .unwrap();
    let buffer = PhysFrame::containing(PhysAddr::new(0x10_0000).unwrap());
    for slot in 0..2 {
        let thread = objects
            .threads
            .allocate(Thread::new(process, 4, 31, slot, buffer).unwrap())
            .unwrap();
        objects
            .processes
            .get_mut(process)
            .unwrap()
            .add_thread(thread)
            .unwrap();
        scheduler.start(&mut objects.threads, thread).unwrap();
    }
    let mut spaces = RecordingAddressSpaces::new(root);
    for _ in 0..4 {
        let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
        assert_eq!(next.switch.expect("a switch").address_space, None);
    }
    assert_eq!(spaces.switches(), 0, "the root was never written");
}

#[test]
fn a_switch_names_the_stack_and_the_context_of_the_thread_that_takes_over() {
    let (mut objects, mut scheduler, _) = two_processes(5, 5);
    let mut spaces = RecordingAddressSpaces::new(frame(1));
    let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    let switch = next.switch.expect("a switch");
    let thread = objects.threads.get(switch.to).unwrap();
    assert_eq!(switch.context, thread.context);
    assert_eq!(switch.kernel_stack_top, stack_top(thread.kernel_stack));

    // What the architecture layer writes back is what the next switch
    // hands out.
    store_context(
        &mut objects,
        switch.to,
        kernel_address(0xFFFF_8000_1234_5000),
    );
    assert_eq!(
        objects.threads.get(switch.to).unwrap().context.as_u64(),
        0xFFFF_8000_1234_5000
    );
}

#[test]
fn a_machine_with_nobody_to_run_asks_for_no_switch() {
    let mut objects = Small::new();
    let mut scheduler = Scheduler::new();
    let mut spaces = RecordingAddressSpaces::new(frame(1));
    let next = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    assert_eq!(next.switch, None);
    assert_eq!(spaces.switches(), 0);
}

#[test]
fn a_scheduler_that_picks_the_thread_that_already_runs_asks_for_no_switch() {
    let (mut objects, mut scheduler, _) = two_processes(5, 1);
    let mut spaces = RecordingAddressSpaces::new(frame(1));
    // The thread of priority five runs and nobody outranks it, so the
    // second call picks it again.
    assert!(
        schedule(&mut objects, &mut scheduler, &mut spaces, stack_top)
            .switch
            .is_some()
    );
    let second = schedule(&mut objects, &mut scheduler, &mut spaces, stack_top);
    assert_eq!(second.switch, None, "the same thread keeps the processor");
}

#[test]
fn the_process_of_a_thread_is_what_it_was_created_with() {
    let (objects, _, _) = two_processes(1, 1);
    let thread = objects.threads.ids().next().unwrap();
    let process = process_of(&objects, thread).expect("a process");
    assert_eq!(objects.threads.get(thread).unwrap().process, process);
    let ghost = kernel_objects::pool::ObjectId::new(7, 1);
    assert_eq!(process_of(&objects, ghost), None);
}

#[test]
fn a_system_call_of_a_thread_goes_through_the_kernel_environment() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let (mut objects, mut scheduler, _) = two_processes(5, 5);
    let thread = objects.threads.ids().next().unwrap();
    scheduler.pick_next(&mut objects.threads).unwrap();

    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    let mut buffer = [0_u8; SIZE];
    BufferMut::new(&mut buffer).set_syscall_number(u64::from(Syscall::ThreadYield.number()));
    let reschedule = crate::syscall::handle_syscall(
        &mut objects,
        &mut scheduler,
        &mut environment,
        thread,
        &mut buffer,
    );
    assert!(reschedule, "a thread that yields asks for a switch");
    assert_eq!(
        objects.threads.get(thread).unwrap().state,
        audhsos_abi::ThreadState::Ready
    );

    // Nothing ended, so there is nothing to clear away.
    assert_eq!(
        crate::syscall::reap(&mut objects, &mut scheduler, &mut environment, Some(thread)),
        0
    );
}

#[test]
fn what_a_thread_that_ended_held_goes_back_through_the_environment() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut console = RecordingConsole::new();
    let (mut objects, mut scheduler, _) = two_processes(5, 5);
    let thread = objects.threads.ids().next().unwrap();

    let mut environment = KernelEnvironment::<X86Entry, _, _, _, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
        None,
        0,
        no_frame,
    );
    // A stack of the reserve for the thread, so that the sweep has
    // something to give back.
    let stack = environment.allocate_kernel_stack().unwrap();
    objects.threads.get_mut(thread).unwrap().kernel_stack = stack.slot;
    let free = environment.memory.frames().free_count();

    scheduler.exit(&mut objects.threads, thread).unwrap();
    // The kernel is no longer standing on the stack of that thread, so
    // the sweep may take it.
    assert_eq!(
        crate::syscall::reap(&mut objects, &mut scheduler, &mut environment, None),
        1
    );
    assert!(
        environment.memory.frames().free_count() > free,
        "the stack of the thread went back into the reserve"
    );
    assert!(objects.threads.get(thread).is_err(), "the slot came back");
}

#[test]
fn the_buffer_of_a_thread_is_reached_through_the_window() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut environment =
        KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            None,
            None,
            0,
            no_frame,
        );
    let held = environment.allocate_frame().unwrap();
    let written = environment
        .with_buffer(held, |bytes| {
            let mut writer = BufferMut::new(bytes);
            writer.set_word(0, 0x00C0_FFEE);
            42
        })
        .unwrap();
    assert_eq!(written, 42, "the closure answers the caller");
    let read = environment
        .with_buffer(held, |bytes| {
            audhsos_abi::ipc_buffer::Buffer::new(bytes).word(0)
        })
        .unwrap();
    assert_eq!(read, Some(0x00C0_FFEE));
    // A frame the window does not reach is no buffer.
    let beyond = PhysFrame::from_number(0xF_FFFF).unwrap();
    assert_eq!(
        environment.with_buffer(beyond, |_| ()),
        Err(Error::InvalidArgument)
    );
}

#[test]
fn a_kernel_without_devices_refuses_the_calls_that_need_them() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut environment =
        KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            None,
            None,
            0,
            no_frame,
        );
    assert_eq!(environment.read_port(0x40, 1), Err(Error::Unsupported));
    assert_eq!(environment.write_port(0x40, 1, 0), Err(Error::Unsupported));
    assert_eq!(environment.interrupt_vector(0), None);
    assert_eq!(
        environment.route_interrupt(0, 0x40),
        Err(Error::Unsupported)
    );
    // The two that answer nothing are no-operations without devices.
    environment.mask_interrupt(0);
    environment.unmask_interrupt(0);
}

#[test]
fn a_port_is_read_and_written_at_every_width_the_interface_allows() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut devices = RecordingDevices::new(24);
    devices.ports.script_read(0x40, 0xAB);
    devices.ports.script_read(0x42, 0xBEEF);
    devices.ports.script_read(0x44, 0xDEAD_BEEF);
    let mut environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        Some(&mut devices),
        0,
        no_frame,
    );
    assert_eq!(environment.read_port(0x40, 1), Ok(0xAB));
    assert_eq!(environment.read_port(0x42, 2), Ok(0xBEEF));
    assert_eq!(environment.read_port(0x44, 4), Ok(0xDEAD_BEEF));
    assert_eq!(environment.read_port(0x40, 3), Err(Error::InvalidArgument));
    assert!(environment.write_port(0x50, 1, 0x1FF).is_ok());
    assert!(environment.write_port(0x52, 2, 0x1_FFFF).is_ok());
    assert!(environment.write_port(0x54, 4, 0x1_FFFF_FFFF).is_ok());
    assert_eq!(
        environment.write_port(0x50, 8, 0),
        Err(Error::InvalidArgument)
    );
    let _ = &environment;
    assert_eq!(
        devices.ports.writes_to(0x50),
        vec![0xFF],
        "a write of one byte carries one byte"
    );
    assert_eq!(devices.ports.writes_to(0x52), vec![0xFFFF]);
    assert_eq!(devices.ports.writes_to(0x54), vec![0xFFFF_FFFF]);
}

#[test]
fn a_line_is_routed_once_and_the_plan_says_which_vector_it_reaches() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut devices = RecordingDevices::new(4);
    let mut environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        Some(&mut devices),
        0,
        no_frame,
    );
    assert_eq!(environment.interrupt_vector(0), Some(32));
    assert_eq!(environment.interrupt_vector(9), None, "no such line");
    assert!(environment.route_interrupt(0, 32).is_ok());
    assert_eq!(
        environment.route_interrupt(0, 32),
        Err(Error::AlreadyExists)
    );
    assert_eq!(
        environment.route_interrupt(1, 3),
        Err(Error::InvalidArgument),
        "a vector the processor keeps for its exceptions"
    );
    assert_eq!(
        environment.route_interrupt(9, 41),
        Err(Error::InvalidArgument),
        "a line the controller does not have"
    );
    environment.mask_interrupt(0);
    environment.unmask_interrupt(0);
    let _ = &environment;
    let line = kernel_hal_api::interrupt::InterruptLine::new(0);
    assert!(
        !devices.interrupts.is_masked(line),
        "the last word was the unmask"
    );
    assert_eq!(devices.interrupts.events().len(), 3);
}

#[test]
fn a_device_range_that_meets_memory_of_the_machine_is_recognised() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let reserve = memory.frames().range();
    let free = memory.free().iter().next().unwrap();
    let environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        None,
        0x1234,
        no_frame,
    );
    assert!(
        environment.meets_ram(reserve),
        "the reserve is memory of the machine too"
    );
    assert!(environment.meets_ram(free));
    let aperture = kernel_types::PhysFrameRange::new(frame(0xF_0000), 4).unwrap();
    assert!(
        !environment.meets_ram(aperture),
        "an aperture above every region meets nothing"
    );
    assert_eq!(environment.acpi_pointer(), 0x1234);
}

/// The frame a new thread returns through, for tests that do not switch
/// into one: the word below the top of the stack, which is where a real
/// frame leaves the pointer.
fn no_frame<A>(
    _access: &mut A,
    _frame: kernel_types::PhysFrame,
    stack_top: kernel_types::VirtAddr,
    _entry: kernel_types::VirtAddr,
    _user_stack: kernel_types::VirtAddr,
    _buffer: kernel_types::VirtAddr,
) -> Option<kernel_types::VirtAddr> {
    stack_top.checked_sub(8)
}

#[test]
fn the_clock_the_environment_answers_is_the_one_it_was_given() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        None,
        0,
        no_frame,
    );
    assert_eq!(environment.now_micros(), 0, "a machine that has not ticked");
    let environment = environment.at(50_000);
    assert_eq!(environment.now_micros(), 50_000);
}

#[test]
fn a_seed_comes_from_the_devices_and_a_machine_without_them_has_none() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut devices = RecordingDevices::new(4).seeding(Ok([1, 2, 3, 4]));
    let mut environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        Some(&mut devices),
        0,
        no_frame,
    );
    assert_eq!(environment.random_seed(), Ok([1, 2, 3, 4]));
    assert_eq!(
        environment.random_seed(),
        Err(Error::Unavailable),
        "the script is empty"
    );
    let _ = &environment;

    let mut environment =
        KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            None,
            None,
            0,
            no_frame,
        );
    assert_eq!(
        environment.random_seed(),
        Err(Error::Unavailable),
        "a machine whose devices are not up has no source"
    );
}

#[test]
fn a_message_vector_is_handed_out_taken_back_and_runs_out() {
    let (mut machine, mut memory) = kernel_memory();
    let mut tlb = RecordingTlb::new();
    let mut devices = RecordingDevices {
        interrupts: kernel_hal_api::doubles::FakeInterruptController::new(4)
            .with_message_vectors(1),
        ..RecordingDevices::new(4)
    };
    let mut environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
        Some(&mut devices),
        0,
        no_frame,
    );
    let (vector, address, data) = environment.allocate_message_vector().unwrap();
    assert_eq!(u64::from(data), u64::from(vector));
    assert_ne!(address, 0);
    assert_eq!(
        environment.allocate_message_vector(),
        Err(Error::NoVector),
        "the space of one is exhausted"
    );
    environment.release_message_vector(vector);
    assert_eq!(
        environment.allocate_message_vector(),
        Ok((vector, address, data)),
        "and the vector came back"
    );
    // A vector below the first the processor lets a device raise is no
    // vector, and giving one back changes nothing.
    environment.release_message_vector(0);
    assert_eq!(environment.allocate_message_vector(), Err(Error::NoVector));
    let _ = &environment;

    let mut environment =
        KernelEnvironment::<X86Entry, _, _, RecordingConsole, RecordingDevices>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            None,
            None,
            0,
            no_frame,
        );
    assert_eq!(
        environment.allocate_message_vector(),
        Err(Error::Unsupported),
        "a machine whose controller is not up routes nothing"
    );
    environment.release_message_vector(0x58);
}
