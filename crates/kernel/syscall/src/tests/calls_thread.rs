// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the thread calls: one success and every error of each, which
//! is what the catalog 6.6.9 asks for.

use audhsos_abi::layout::PRIORITY_COUNT;
use audhsos_abi::{Error, Handle, Rights, Syscall, ThreadState};
use kernel_objects::object::AnyObjectId;
use kernel_sched::Outcome;

use crate::tests::double::{Fixture, call, error_of, request, value_of};

/// An address a user thread may start at.
const ENTRY: u64 = 0x40_0000;

/// An address a user thread may stand on.
const STACK: u64 = 0x50_0000;

/// The arguments of a `thread_create` that works.
fn creation(fixture: &Fixture) -> [u64; 6] {
    [fixture.own_process.raw(), ENTRY, STACK, 4, 8, 0]
}

#[test]
fn creating_a_thread_hands_out_a_handle_and_takes_a_stack_and_a_frame() {
    let mut fixture = Fixture::new();
    let before = fixture.objects.threads.live();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    let handle = Handle::from_raw(raw).expect("a handle");
    assert_eq!(fixture.objects.threads.live(), before + 1);
    assert_eq!(fixture.environment.stacks_out(), 1);
    assert_eq!(fixture.environment.frames_out(), 1);

    let entry = fixture.objects.entry(fixture.process, handle).unwrap();
    assert_eq!(entry.object.object_type(), audhsos_abi::ObjectType::Thread);
    let id = entry
        .object
        .typed::<kernel_objects::object::Thread>()
        .unwrap();
    let thread = fixture.objects.threads.get(id).unwrap();
    assert_eq!(thread.state, ThreadState::Inactive);
    assert_eq!(thread.priority, 4);
    assert_eq!(thread.max_priority, 8);
    assert_eq!(thread.entry.as_u64(), ENTRY);
    assert_eq!(thread.user_stack.as_u64(), STACK);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .thread_count(),
        2
    );
}

#[test]
fn a_thread_may_not_be_created_above_the_maximum_of_the_thread_creating_it() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[4] = 9; // the caller's maximum is 8
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::InvalidArgument)
    );
    assert_eq!(fixture.environment.stacks_out(), 0, "nothing was taken");
}

#[test]
fn a_priority_above_the_maximum_or_outside_the_table_is_refused() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[3] = 6;
    arguments[4] = 5;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::InvalidArgument)
    );

    let mut arguments = creation(&fixture);
    arguments[3] = u64::from(PRIORITY_COUNT);
    arguments[4] = u64::from(PRIORITY_COUNT);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::InvalidArgument)
    );

    let mut arguments = creation(&fixture);
    arguments[3] = u64::from(u32::MAX);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::InvalidArgument),
        "a priority that is not even a byte"
    );
}

#[test]
fn an_entry_or_a_stack_outside_user_space_is_refused() {
    let mut fixture = Fixture::new();
    for index in 1..=2 {
        for address in [0_u64, 0x800, 0xFFFF_8000_0000_0000, u64::MAX] {
            let mut arguments = creation(&fixture);
            arguments[index] = address;
            assert_eq!(
                error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
                Some(Error::InvalidArgument),
                "argument {index} = {address:#x}"
            );
        }
    }
    assert_eq!(fixture.environment.stacks_out(), 0);
}

#[test]
fn the_reserved_argument_of_thread_create_must_be_zero() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[5] = 1;
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn a_thread_create_without_a_kernel_stack_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    fixture.environment.no_stack = true;
    let quota = fixture
        .objects
        .processes
        .get(fixture.process)
        .unwrap()
        .kernel_object_quota;
    assert_eq!(
        error_of(&mut fixture, creation),
        Some(Error::OutOfKernelMemory)
    );
    assert_eq!(fixture.objects.threads.live(), 1);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .kernel_object_quota,
        quota,
        "the quota is where it was"
    );
}

#[test]
fn a_thread_create_without_a_buffer_frame_gives_the_stack_back() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    fixture.environment.no_frame = true;
    assert_eq!(
        error_of(&mut fixture, creation),
        Some(Error::OutOfKernelMemory)
    );
    assert_eq!(fixture.environment.stacks_out(), 0, "the stack went back");
    assert_eq!(fixture.objects.threads.live(), 1);
}

#[test]
fn a_thread_create_beyond_the_object_quota_is_refused() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let quota = &mut fixture
        .objects
        .processes
        .get_mut(fixture.process)
        .unwrap()
        .kernel_object_quota;
    let left = quota.remaining();
    quota.charge(left).unwrap();
    assert_eq!(error_of(&mut fixture, creation), Some(Error::QuotaExceeded));
    assert_eq!(fixture.environment.stacks_out(), 0);
}

#[test]
fn a_thread_create_with_a_full_pool_is_refused_and_gives_everything_back() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    // The pool of the fixture holds eight threads and one is taken.
    for index in 0..7 {
        assert!(error_of(&mut fixture, creation).is_none(), "thread {index}");
    }
    let out = fixture.environment.stacks_out();
    assert_eq!(error_of(&mut fixture, creation), Some(Error::PoolExhausted));
    assert_eq!(
        fixture.environment.stacks_out(),
        out,
        "the refused thread gave its stack back"
    );
    assert_eq!(fixture.objects.threads.live(), 8);
}

#[test]
fn starting_a_thread_makes_it_ready_and_starting_it_twice_fails() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    let mut buffer = request(Syscall::ThreadStart, &[raw]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(
        outcome,
        Outcome::NOTHING,
        "priority four does not preempt the running thread of priority four"
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadStart, &[raw])),
        Some(Error::InvalidState)
    );
}

#[test]
fn a_thread_of_higher_priority_takes_the_processor_when_it_starts() {
    let mut fixture = Fixture::new();
    let mut arguments = creation(&fixture);
    arguments[3] = 7;
    let raw = value_of(&mut fixture, request(Syscall::ThreadCreate, &arguments));
    let mut buffer = request(Syscall::ThreadStart, &[raw]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(outcome, Outcome::RESCHEDULE);
}

#[test]
fn suspending_and_resuming_a_thread_walks_the_state_table() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadResume, &[raw])),
        Some(Error::InvalidState),
        "a thread that never ran does not resume"
    );
    assert!(error_of(&mut fixture, request(Syscall::ThreadSuspend, &[raw])).is_none());
    assert_eq!(
        state_of(&mut fixture, raw),
        ThreadState::Suspended.code().into()
    );
    assert!(error_of(&mut fixture, request(Syscall::ThreadResume, &[raw])).is_none());
    assert_eq!(
        state_of(&mut fixture, raw),
        ThreadState::Ready.code().into()
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadSuspend, &[raw])),
        None,
        "a ready thread suspends again"
    );
}

/// The state `thread_info` reports for the thread `raw` names.
fn state_of(fixture: &mut Fixture, raw: u64) -> u64 {
    let mut buffer = request(Syscall::ThreadInfo, &[raw]);
    let (status, values, _) = call(fixture, &mut buffer);
    assert_eq!(status.error(), None);
    values[0]
}

#[test]
fn thread_info_reports_the_state_and_whether_it_faulted() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadInfo, &[fixture.own_thread.raw()]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], u64::from(ThreadState::Running.code()));
    assert_eq!(values[1], 0);

    // A thread that carries a fault reports its kind in the second return
    // word and the three words of it in the message area.
    let id = fixture.thread;
    let fault = audhsos_abi::Fault {
        kind: audhsos_abi::FaultKind::PageFault,
        address: 0x1234,
        instruction_pointer: 0x40_0000,
        error_code: 0b110,
    };
    fixture
        .objects
        .threads
        .with(id, |thread| thread.fault = Some(fault));
    fixture
        .scheduler
        .fault(&mut fixture.objects.threads, id)
        .unwrap();
    let mut buffer = request(Syscall::ThreadInfo, &[fixture.own_thread.raw()]);
    let (_, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(values[0], u64::from(ThreadState::Faulted.code()));
    assert_eq!(
        values[1],
        u64::from(audhsos_abi::FaultKind::PageFault.code())
    );
    let view = audhsos_abi::ipc_buffer::Buffer::new(&buffer);
    let message = view.message().unwrap();
    assert_eq!(message.label, 0, "a result carries no label");
    assert_eq!(message.word_count, 3);
    assert_eq!(message.handle_count, 0);
    assert_eq!(view.word(0), Some(0x1234));
    assert_eq!(view.word(1), Some(0x40_0000));
    assert_eq!(view.word(2), Some(0b110));
}

#[test]
fn thread_info_of_a_thread_that_did_not_fault_reports_no_words() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadInfo, &[fixture.own_thread.raw()]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[1], 0);
    let message = audhsos_abi::ipc_buffer::Buffer::new(&buffer)
        .message()
        .unwrap();
    assert_eq!(message.word_count, 0);
}

#[test]
fn setting_a_priority_within_the_maximum_works_and_above_it_does_not() {
    let mut fixture = Fixture::new();
    let own = fixture.own_thread.raw();
    assert!(error_of(&mut fixture, request(Syscall::ThreadSetPriority, &[own, 8])).is_none());
    assert_eq!(
        fixture
            .objects
            .threads
            .get(fixture.thread)
            .unwrap()
            .priority,
        8
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadSetPriority, &[own, 9])),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn killing_a_thread_ends_it_and_gives_back_what_it_held() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    let raw = value_of(&mut fixture, creation);
    assert_eq!(fixture.environment.stacks_out(), 1);
    let mut buffer = request(Syscall::ThreadKill, &[raw]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(outcome, Outcome::NOTHING, "it was not the running thread");
    assert_eq!(
        state_of(&mut fixture, raw),
        ThreadState::Exited.code().into()
    );
    assert_eq!(
        fixture.environment.stacks_out(),
        1,
        "what it held stays until the kernel clears it away"
    );
    assert_eq!(crate::reaper::reap(&mut fixture.machine(), None), 1);
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .thread_count(),
        1
    );
}

#[test]
fn a_thread_that_exits_leaves_the_processor() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadExit, &[]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(outcome, Outcome::RESCHEDULE);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Exited
    );
    assert_eq!(fixture.scheduler.current(), None);
}

#[test]
fn yielding_puts_the_caller_back_into_its_queue() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadYield, &[]);
    let (status, _, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(outcome, Outcome::RESCHEDULE);
    assert_eq!(
        fixture.objects.threads.get(fixture.thread).unwrap().state,
        ThreadState::Ready
    );
}

#[test]
fn a_thread_call_on_a_handle_without_manage_is_refused() {
    let mut fixture = Fixture::new();
    let weak = fixture.install(AnyObjectId::of(fixture.thread), Rights::DUPLICATE);
    for call in [
        Syscall::ThreadStart,
        Syscall::ThreadSuspend,
        Syscall::ThreadResume,
        Syscall::ThreadKill,
        Syscall::ThreadInfo,
    ] {
        assert_eq!(
            error_of(&mut fixture, request(call, &[weak.raw()])),
            Some(Error::AccessDenied),
            "{}",
            call.name()
        );
    }
}

#[test]
fn creating_a_thread_of_a_process_the_caller_may_not_manage_is_refused() {
    let mut fixture = Fixture::new();
    let weak = fixture.install(AnyObjectId::of(fixture.process), Rights::MAP);
    let mut arguments = creation(&fixture);
    arguments[0] = weak.raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::ThreadCreate, &arguments)),
        Some(Error::AccessDenied)
    );
}

#[test]
fn a_thread_create_without_a_handle_slot_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    // The list of the caller is full, so the handle of the new thread has
    // nowhere to go and everything the call took goes back.
    fixture.fill_handles();
    assert_eq!(error_of(&mut fixture, creation), Some(Error::QuotaExceeded));
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(fixture.objects.threads.live(), 1);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .thread_count(),
        1,
        "the process does not hold a thread that has no handle"
    );
}

#[test]
fn every_slot_of_a_process_has_a_page_for_its_buffer_and_nothing_beyond() {
    use audhsos_abi::layout::{PAGE_SIZE, THREADS_PER_PROCESS};

    let first = crate::calls::thread::ipc_page(0).expect("a page");
    let last = crate::calls::thread::ipc_page(THREADS_PER_PROCESS - 1).expect("a page");
    assert!(first.start().is_user());
    assert!(last.start().is_user());
    assert_eq!(
        first.start().as_u64() - last.start().as_u64(),
        u64::try_from(THREADS_PER_PROCESS - 1).unwrap() * PAGE_SIZE,
        "the buffers of one process are pages below one another"
    );
    assert_eq!(
        crate::calls::thread::ipc_page(THREADS_PER_PROCESS),
        None,
        "a slot no process has"
    );
}

#[test]
fn a_thread_create_whose_buffer_cannot_be_mapped_leaves_nothing_behind() {
    let mut fixture = Fixture::new();
    let creation = request(Syscall::ThreadCreate, &creation(&fixture));
    fixture.environment.no_mapping = true;
    assert_eq!(
        error_of(&mut fixture, creation),
        Some(Error::OutOfKernelMemory)
    );
    assert_eq!(fixture.environment.stacks_out(), 0);
    assert_eq!(fixture.environment.frames_out(), 0);
    assert_eq!(fixture.objects.threads.live(), 1);
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .thread_count(),
        1
    );
}
