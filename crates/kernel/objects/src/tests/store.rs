// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::store`.

use audhsos_abi::{Error, ObjectType, Rights};

use crate::config::{HANDLE_ENTRIES, MEMORY_OBJECTS, PROCESSES, THREADS};
use crate::handle_table::{Entry, HandleList};
use crate::object::{AnyObjectId, MemoryObject, Process, ProcessId, Thread};
use crate::pool::ObjectId;
use crate::quota::Quota;
use crate::store::{MachineObjects, Objects};
use kernel_types::{PhysAddr, PhysFrame};

/// A machine of a handful of slots, which is what a test can hold on its
/// stack; the machine of the kernel is a `static` and never travels.
type Small = Objects<2, 4, 4, 8>;

#[test]
fn what_the_machine_costs_the_image() {
    // The whole structure is one `static` in the `.bss`, so its size is
    // what the kernel image grows by. The bound is what the numbers of
    // `config` add up to today, with room for the padding a change of a
    // field would add; a jump past it means a number was raised without
    // anyone looking at the image.
    // It measures 1_224_280 bytes today. The bound leaves room for the
    // padding a new field would add; a jump past it means a number in
    // `config` was raised without anyone looking at the image.
    let size = size_of::<MachineObjects>();
    assert!(size < 2 * 1024 * 1024, "the machine is {size} bytes");
    assert!(
        size > 512 * 1024,
        "the pools are there at all: {size} bytes"
    );
}

#[test]
fn the_configuration_sizes_the_pools() {
    // The type, not a value: building one here would put a mebibyte on the
    // stack of the test, which is the whole reason the kernel keeps it in
    // a `static` (D-66).
    assert_eq!(PROCESSES, 64);
    assert_eq!(THREADS, 256);
    assert_eq!(MEMORY_OBJECTS, 4096);
    assert_eq!(HANDLE_ENTRIES, 16384);
    let small = Small::new();
    assert_eq!(small.processes.capacity(), 2);
    assert_eq!(small.threads.capacity(), 4);
    assert_eq!(small.memory.capacity(), 4);
    assert_eq!(small.handles.capacity(), 8);
}

/// A process to install handles for.
fn owner() -> ProcessId {
    ObjectId::new(0, 1)
}

/// A frame for the objects that need one.
fn frame(address: u64) -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(address).unwrap())
}

#[test]
fn a_handle_resolves_to_the_object_it_names() {
    let mut objects = Small::new();
    let mut list = HandleList::with_capacity(8);
    let thread = ObjectId::<Thread>::new(2, 1);
    let entry = Entry::new(AnyObjectId::of(thread), Rights::MANAGE | Rights::DUPLICATE);
    let handle = objects.handles.insert(owner(), &mut list, entry).unwrap();

    assert_eq!(objects.entry(owner(), handle), Ok(entry));
    assert_eq!(objects.object_type(owner(), handle), Ok(ObjectType::Thread));
    assert_eq!(
        objects.resolve::<Thread>(owner(), handle, Rights::MANAGE),
        Ok((thread, entry.rights))
    );
}

#[test]
fn resolving_checks_the_handle_then_the_type_then_the_rights() {
    let mut objects = Small::new();
    let mut list = HandleList::with_capacity(8);
    let memory = ObjectId::<MemoryObject>::new(1, 1);
    let entry = Entry::new(AnyObjectId::of(memory), Rights::READ);
    let handle = objects.handles.insert(owner(), &mut list, entry).unwrap();

    // A handle of nobody: the first check.
    let stranger = ObjectId::new(9, 1);
    assert_eq!(
        objects.resolve::<MemoryObject>(stranger, handle, Rights::READ),
        Err(Error::InvalidHandle)
    );
    // The wrong type beats the missing right: the entry carries neither
    // `MANAGE` nor a process.
    assert_eq!(
        objects.resolve::<Process>(owner(), handle, Rights::MANAGE),
        Err(Error::WrongObjectType)
    );
    // The right type, a right it does not carry.
    assert_eq!(
        objects.resolve::<MemoryObject>(owner(), handle, Rights::WRITE),
        Err(Error::AccessDenied)
    );
    // Asking for nothing is allowed.
    assert!(
        objects
            .resolve::<MemoryObject>(owner(), handle, Rights::EMPTY)
            .is_ok()
    );
}

#[test]
fn the_machine_starts_empty_and_counts_what_it_holds() {
    let mut objects = Small::new();
    assert_eq!(objects.counts(), [0, 0, 0, 0]);
    assert!(!objects.holds_process(owner()));

    let process = objects
        .processes
        .allocate(Process::new(
            frame(0x1000),
            HandleList::with_capacity(4),
            Quota::new(8),
            Quota::new(8),
        ))
        .unwrap();
    assert!(objects.holds_process(process));
    assert_eq!(objects.counts(), [1, 0, 0, 0]);

    let thread = objects
        .threads
        .allocate(Thread::new(process, 0, 0, 0, frame(0x2000)).unwrap())
        .unwrap();
    assert!(objects.holds_thread(thread));
    assert_eq!(objects.counts(), [1, 1, 0, 0]);
}
