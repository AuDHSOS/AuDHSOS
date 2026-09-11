// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::syscall`.

use crate::layout::MAX_SYSCALL_ARGUMENTS;
use crate::syscall::{FirstArgument, Syscall};
use crate::{Error, ObjectType};
use std::collections::HashSet;

#[test]
fn every_call_round_trips_through_its_number() {
    for &call in Syscall::ALL {
        assert_eq!(Syscall::from_number(call.number()), Some(call));
        assert_eq!(Syscall::try_from(call.number()), Ok(call));
        assert_eq!(Syscall::from_word(u64::from(call.number())), Ok(call));
    }
}

#[test]
fn numbers_are_unique_dense_and_non_zero() {
    let mut numbers: Vec<u32> = Syscall::ALL.iter().map(|c| c.number()).collect();
    let unique: HashSet<u32> = numbers.iter().copied().collect();
    assert_eq!(unique.len(), Syscall::ALL.len());
    numbers.sort_unstable();
    let expected: Vec<u32> = (1..=u32::try_from(Syscall::ALL.len()).unwrap()).collect();
    assert_eq!(numbers, expected);
    assert_eq!(Syscall::from_number(0), None);
}

#[test]
fn the_table_holds_every_call_of_the_interface() {
    assert_eq!(Syscall::ALL.len(), 50);
    assert_eq!(Syscall::ProcessCreate.number(), 1);
    assert_eq!(Syscall::DebugLog.number(), 41);
    assert_eq!(Syscall::MemoryMerge.number(), 42);
    assert_eq!(Syscall::ProcessWatch.number(), 43);
    assert_eq!(Syscall::IoPortWriteString.number(), 46);
    assert_eq!(Syscall::ClockNow.number(), 47);
    assert_eq!(Syscall::NotificationWaitUntil.number(), 48);
    assert_eq!(Syscall::RandomBytes.number(), 49);
    assert_eq!(Syscall::InterruptCreateMsi.number(), 50);
}

#[test]
fn a_number_above_the_table_is_unknown() {
    let highest = Syscall::ALL.iter().map(|c| c.number()).max().unwrap();
    assert_eq!(Syscall::from_number(highest + 1), None);
    assert_eq!(
        Syscall::try_from(highest + 1),
        Err(Error::UnknownSyscall),
        "a number above the table is not an argument error"
    );
    assert_eq!(Syscall::from_number(u32::MAX), None);
}

#[test]
fn a_word_above_the_number_width_is_unknown() {
    assert_eq!(Syscall::from_word(0), Err(Error::UnknownSyscall));
    assert_eq!(
        Syscall::from_word(0x1_0000_0000),
        Err(Error::UnknownSyscall)
    );
    assert_eq!(Syscall::from_word(u64::MAX), Err(Error::UnknownSyscall));
    // The low half naming a valid call does not make a wide word valid.
    assert_eq!(
        Syscall::from_word(0x1_0000_0001),
        Err(Error::UnknownSyscall)
    );
}

#[test]
fn names_are_unique_and_lower_case() {
    let names: HashSet<&str> = Syscall::ALL.iter().map(|c| c.name()).collect();
    assert_eq!(names.len(), Syscall::ALL.len());
    for &call in Syscall::ALL {
        let name = call.name();
        assert!(!name.is_empty());
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{name}"
        );
    }
}

#[test]
fn no_call_reads_more_arguments_than_the_buffer_holds() {
    for &call in Syscall::ALL {
        assert!(
            usize::from(call.argument_count()) <= MAX_SYSCALL_ARGUMENTS,
            "{}",
            call.name()
        );
    }
}

#[test]
fn a_call_without_a_handle_takes_no_object_type() {
    for &call in Syscall::ALL {
        match call.first_argument() {
            FirstArgument::Nothing => {
                assert!(!call.takes_handle(), "{}", call.name());
                assert_eq!(call.object_type(), None);
                assert_eq!(call.argument_count(), 0, "{}", call.name());
            }
            FirstArgument::Any => {
                assert!(call.takes_handle(), "{}", call.name());
                assert_eq!(call.object_type(), None);
            }
            FirstArgument::Object(ty) => {
                assert!(call.takes_handle(), "{}", call.name());
                assert_eq!(call.object_type(), Some(ty));
            }
        }
    }
}

#[test]
fn a_call_that_takes_a_handle_reads_at_least_one_argument() {
    for &call in Syscall::ALL {
        if call.takes_handle() {
            assert!(call.argument_count() >= 1, "{}", call.name());
        }
    }
}

#[test]
fn the_table_matches_the_interface_documentation() {
    let expected: &[(Syscall, &str, u8, FirstArgument)] = &[
        (
            Syscall::ProcessCreate,
            "process_create",
            5,
            FirstArgument::Object(ObjectType::Process),
        ),
        (
            Syscall::ProcessSetFaultHandler,
            "process_set_fault_handler",
            2,
            FirstArgument::Object(ObjectType::Process),
        ),
        (
            Syscall::ThreadCreate,
            "thread_create",
            6,
            FirstArgument::Object(ObjectType::Process),
        ),
        (
            Syscall::ThreadExit,
            "thread_exit",
            0,
            FirstArgument::Nothing,
        ),
        (
            Syscall::MemoryMap,
            "memory_map",
            6,
            FirstArgument::Object(ObjectType::Process),
        ),
        (
            Syscall::MemoryInfo,
            "memory_info",
            1,
            FirstArgument::Object(ObjectType::MemoryObject),
        ),
        (
            Syscall::HandleDuplicate,
            "handle_duplicate",
            2,
            FirstArgument::Any,
        ),
        (Syscall::HandleClose, "handle_close", 1, FirstArgument::Any),
        (
            Syscall::IpcReplyRecv,
            "ipc_reply_recv",
            2,
            FirstArgument::Object(ObjectType::Reply),
        ),
        (
            Syscall::InterruptCreate,
            "interrupt_create",
            2,
            FirstArgument::Object(ObjectType::SystemControl),
        ),
        (Syscall::DebugLog, "debug_log", 0, FirstArgument::Nothing),
        (Syscall::ClockNow, "clock_now", 0, FirstArgument::Nothing),
        (
            Syscall::NotificationWaitUntil,
            "notification_wait_until",
            2,
            FirstArgument::Object(ObjectType::Notification),
        ),
        (
            Syscall::RandomBytes,
            "random_bytes",
            0,
            FirstArgument::Nothing,
        ),
        (
            Syscall::InterruptCreateMsi,
            "interrupt_create_msi",
            1,
            FirstArgument::Object(ObjectType::SystemControl),
        ),
    ];
    for &(call, name, args, first) in expected {
        assert_eq!(call.name(), name);
        assert_eq!(call.argument_count(), args, "{name}");
        assert_eq!(call.first_argument(), first, "{name}");
    }
}

#[test]
fn the_memory_calls_name_the_process_they_map_into() {
    // `memory_map`, `memory_unmap`, and `memory_protect` take the process
    // first and the memory object in a later argument, because a mapping
    // belongs to an address space.
    for call in [
        Syscall::MemoryMap,
        Syscall::MemoryUnmap,
        Syscall::MemoryProtect,
    ] {
        assert_eq!(
            call.object_type(),
            Some(ObjectType::Process),
            "{}",
            call.name()
        );
    }
    for call in [Syscall::MemorySplit, Syscall::MemoryInfo] {
        assert_eq!(
            call.object_type(),
            Some(ObjectType::MemoryObject),
            "{}",
            call.name()
        );
    }
}

#[test]
fn first_argument_answers_both_questions_for_every_object_type() {
    for &ty in ObjectType::ALL {
        let first = FirstArgument::Object(ty);
        assert!(first.is_handle());
        assert_eq!(first.object_type(), Some(ty));
    }
    assert!(!FirstArgument::Nothing.is_handle());
    assert_eq!(FirstArgument::Nothing.object_type(), None);
    assert!(FirstArgument::Any.is_handle());
    assert_eq!(FirstArgument::Any.object_type(), None);
}
