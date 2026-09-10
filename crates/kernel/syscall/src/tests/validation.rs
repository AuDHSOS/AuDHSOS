// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::dispatch`, covering the decoding and dispatch items of
//! the catalog 6.6.9.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, Status};
use audhsos_abi::layout::MAX_SYSCALL_ARGUMENTS;
use audhsos_abi::{Error, Handle, ObjectType, Rights, Syscall};
use kernel_objects::object::AnyObjectId;
use kernel_sched::Outcome;

use crate::calls::UNIMPLEMENTED;
use crate::dispatch::{decode, dispatch, required_rights};
use crate::tests::double::{Fixture, call, error_of, request};

#[test]
fn a_number_that_names_no_call_is_unknown() {
    let mut fixture = Fixture::new();
    let mut buffer = [0; SIZE];
    BufferMut::new(&mut buffer).set_syscall_number(0);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::UnknownSyscall));

    let mut buffer = [0; SIZE];
    BufferMut::new(&mut buffer).set_syscall_number(u64::MAX);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::UnknownSyscall));

    let above = u64::from(u32::try_from(Syscall::ALL.len()).unwrap() + 1);
    let mut buffer = [0; SIZE];
    BufferMut::new(&mut buffer).set_syscall_number(above);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::UnknownSyscall));
}

#[test]
fn an_argument_above_what_the_call_reads_is_rejected_before_the_handle() {
    let mut fixture = Fixture::new();
    // `thread_start` reads one argument. A second one is a caller that
    // does not match the table, and it is refused before the handle in the
    // first word is even looked at — which is why the handle here is one
    // that names nothing.
    let mut buffer = request(Syscall::ThreadStart, &[0xDEAD_BEEF]);
    assert!(BufferMut::new(&mut buffer).set_argument(1, 1));
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::ArgumentCount));

    let mut buffer = request(Syscall::ThreadYield, &[]);
    assert!(BufferMut::new(&mut buffer).set_argument(MAX_SYSCALL_ARGUMENTS - 1, 1));
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::ArgumentCount));
}

#[test]
fn the_arguments_a_call_reads_may_hold_anything() {
    let mut fixture = Fixture::new();
    let handle = fixture.own_thread.raw();
    let buffer = request(Syscall::ThreadSetPriority, &[handle, 7]);
    assert_eq!(error_of(&mut fixture, buffer), None);
}

#[test]
fn an_invalid_handle_beats_the_wrong_type_and_the_missing_right() {
    let mut fixture = Fixture::new();
    // A handle nobody holds, on a call that wants a process with `MANAGE`.
    let ghost = Handle::new(31, 9).unwrap();
    let buffer = request(Syscall::ProcessKill, &[ghost.raw()]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::InvalidHandle));

    // A raw word that is no handle at all.
    let buffer = request(Syscall::ProcessKill, &[1]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::InvalidHandle));
}

#[test]
fn the_wrong_type_beats_the_missing_right() {
    let mut fixture = Fixture::new();
    // A handle to a thread, with no rights at all, on a call that wants a
    // process with `MANAGE`: the type is reported, not the rights.
    let thread = AnyObjectId::of(fixture.thread);
    let handle = fixture.install(thread, Rights::EMPTY);
    let buffer = request(Syscall::ProcessKill, &[handle.raw()]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::WrongObjectType));
}

#[test]
fn the_missing_right_beats_the_invalid_argument() {
    let mut fixture = Fixture::new();
    // A thread handle without `MANAGE`, and a priority nobody may have:
    // the right is reported, because it is checked first.
    let thread = AnyObjectId::of(fixture.thread);
    let handle = fixture.install(thread, Rights::DUPLICATE);
    let buffer = request(Syscall::ThreadSetPriority, &[handle.raw(), 999]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::AccessDenied));

    // With the right, the argument is what fails.
    let buffer = request(Syscall::ThreadSetPriority, &[fixture.own_thread.raw(), 999]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::InvalidArgument));
}

#[test]
fn a_handle_of_another_process_is_no_handle_of_the_caller() {
    let mut fixture = Fixture::new();
    let other = fixture
        .objects
        .processes
        .allocate(*fixture.objects.processes.get(fixture.process).unwrap())
        .unwrap();
    let mut list = fixture.objects.processes.get(other).unwrap().handles;
    let theirs = fixture
        .objects
        .handles
        .insert(
            other,
            &mut list,
            kernel_objects::handle_table::Entry::new(
                AnyObjectId::of(fixture.thread),
                Rights::MANAGE,
            ),
        )
        .unwrap();
    fixture.objects.processes.get_mut(other).unwrap().handles = list;

    let buffer = request(Syscall::ThreadStart, &[theirs.raw()]);
    assert_eq!(error_of(&mut fixture, buffer), Some(Error::InvalidHandle));
}

#[test]
fn no_call_of_the_table_answers_that_it_does_not_exist() {
    // The table is complete: nothing is left to a later phase, and
    // `UNIMPLEMENTED` is therefore empty. What a call refuses it refuses for
    // a reason of its own, never because the kernel does not have it.
    assert!(
        UNIMPLEMENTED.is_empty(),
        "{UNIMPLEMENTED:?} are still missing"
    );
    for &call in Syscall::ALL {
        // A fixture per call: some of them end the caller or its process,
        // and what this asks of each is only what it answers first.
        let mut fixture = Fixture::new();
        let arguments: Vec<u64> = (0..call.argument_count())
            .map(|index| {
                if index == 0 && call.takes_handle() {
                    handle_for(&mut fixture, call)
                } else {
                    0
                }
            })
            .collect();
        let buffer = request(call, &arguments);
        assert_ne!(
            error_of(&mut fixture, buffer),
            Some(Error::Unsupported),
            "{}",
            call.name()
        );
    }
}

/// A handle of the type `call` expects, with the rights it requires.
fn handle_for(fixture: &mut Fixture, call: Syscall) -> u64 {
    let rights = required_rights(call) | Rights::DUPLICATE;
    match call.object_type() {
        Some(ObjectType::Process) => fixture
            .install(AnyObjectId::of(fixture.process), rights)
            .raw(),
        Some(ObjectType::Thread) => fixture
            .install(AnyObjectId::of(fixture.thread), rights)
            .raw(),
        // Every other type is handed a thread: the call answers about the
        // type before it does anything, which is what this asks of it.
        _ => fixture.own_thread.raw(),
    }
}

#[test]
fn every_call_of_the_table_is_implemented() {
    assert_eq!(
        Syscall::ALL.len() - UNIMPLEMENTED.len(),
        46,
        "the whole table, including memory_references and ioport_write_string"
    );
    assert!(UNIMPLEMENTED.is_empty());
}

#[test]
fn the_rights_table_asks_for_a_right_the_object_type_accepts() {
    for &call in Syscall::ALL {
        let required = required_rights(call);
        let Some(object_type) = call.object_type() else {
            continue;
        };
        assert!(
            required.is_subset_of(object_type.rights_mask()),
            "{} wants {:?} of a {}",
            call.name(),
            required,
            object_type.name()
        );
    }
}

#[test]
fn decoding_reads_the_call_the_handle_and_the_arguments() {
    let mut buffer = request(
        Syscall::ThreadSetPriority,
        &[Handle::new(1, 1).unwrap().raw(), 5],
    );
    let request = decode(&Buffer::new(&buffer)).unwrap();
    assert_eq!(request.call, Syscall::ThreadSetPriority);
    assert_eq!(request.handle, Handle::new(1, 1));
    assert_eq!(request.argument(1), 5);
    assert_eq!(request.argument(5), 0);
    assert_eq!(request.argument(99), 0, "an index beyond the area is zero");

    BufferMut::new(&mut buffer).set_syscall_number(u64::from(Syscall::ThreadYield.number()));
    // `thread_yield` reads nothing, so the words of the call before it are
    // now arguments above its count.
    assert_eq!(decode(&Buffer::new(&buffer)), Err(Error::ArgumentCount));
}

#[test]
fn a_call_without_a_handle_decodes_without_one() {
    let buffer = request(Syscall::ThreadExit, &[]);
    let request = decode(&Buffer::new(&buffer)).unwrap();
    assert_eq!(request.handle, None);
    assert_eq!(request.call, Syscall::ThreadExit);
}

#[test]
fn a_successful_call_leaves_a_success_status_and_its_values() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadInfo, &[fixture.own_thread.raw()]);
    let (status, values, outcome) = call(&mut fixture, &mut buffer);
    assert_eq!(status, Status::OK);
    assert!(status.is_success());
    assert_eq!(
        values[0],
        u64::from(audhsos_abi::ThreadState::Running.code())
    );
    assert_eq!(values[1], 0, "it did not fault");
    assert_eq!(outcome, Outcome::NOTHING);
}

#[test]
fn a_failed_call_writes_no_return_values() {
    let mut fixture = Fixture::new();
    let mut buffer = request(Syscall::ThreadInfo, &[fixture.own_thread.raw()]);
    // A value left over from an earlier call is cleared before the status
    // is written, so a caller never reads the result of the call before.
    assert!(BufferMut::new(&mut buffer).set_return_word(0, 0xFFFF));
    BufferMut::new(&mut buffer).set_syscall_number(0);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), Some(Error::UnknownSyscall));
    assert_eq!(values, [0, 0]);
}

#[test]
fn a_call_by_a_thread_the_machine_does_not_hold_fails() {
    let mut fixture = Fixture::new();
    let ghost = kernel_objects::pool::ObjectId::new(7, 1);
    let mut buffer = request(Syscall::ThreadYield, &[]);
    let outcome = dispatch(&mut fixture.machine(), ghost, &mut buffer);
    assert_eq!(outcome, Outcome::NOTHING);
    assert_eq!(
        Buffer::new(&buffer).status().unwrap().error(),
        Some(Error::InvalidHandle)
    );
}
