// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::syscall`: the environment the calls reach the machine
//! through, and the two entry points the architecture layer calls.

use audhsos_abi::ipc_buffer::{BufferMut, SIZE};
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Syscall};
use kernel_hal_api::doubles::{RecordingAddressSpaces, RecordingConsole, RecordingTlb};
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
        let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
            &mut memory,
            &mut machine.access,
            &mut tlb,
            Some(&mut console),
        );
        environment.log(b"hello");
    }
    assert_eq!(console.output(), b"hello");

    let mut environment = KernelEnvironment::<X86Entry, _, _, RecordingConsole>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        None,
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

    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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

    let mut environment = KernelEnvironment::<X86Entry, _, _, _>::new(
        &mut memory,
        &mut machine.access,
        &mut tlb,
        Some(&mut console),
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
