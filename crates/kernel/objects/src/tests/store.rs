// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::store`.

use audhsos_abi::{Error, ObjectType, Rights};

use crate::config::{HANDLE_ENTRIES, MEMORY_OBJECTS, PROCESSES, THREADS};
use crate::handle_table::{Entry, HandleList};
use crate::object::{
    AnyObjectId, Endpoint, Interrupt, IoPortRange, MemoryObject, Notification, Process, ProcessId,
    Reply, SystemControl, Thread, ThreadId,
};
use crate::pool::ObjectId;
use crate::quota::Quota;
use crate::store::{Destroyed, MachineObjects, Objects};
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
    assert_eq!(objects.counts(), [0; 9]);
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
    assert_eq!(objects.counts(), [1, 0, 0, 0, 0, 0, 0, 0, 0]);

    let thread = objects
        .threads
        .allocate(Thread::new(process, 0, 0, 0, frame(0x2000)).unwrap())
        .unwrap();
    assert!(objects.holds_thread(thread));
    assert_eq!(objects.counts(), [1, 1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn reaching_an_object_that_is_there_and_one_that_is_not() {
    let mut objects = Small::new();
    let process = objects
        .processes
        .allocate(Process::new(
            frame(0x1000),
            HandleList::with_capacity(4),
            Quota::new(8),
            Quota::new(8),
        ))
        .unwrap();
    let thread = objects
        .threads
        .allocate(Thread::new(process, 0, 0, 0, frame(0x2000)).unwrap())
        .unwrap();

    assert!(objects.with_process(process, |holder| {
        holder.kernel_object_quota.charge(1).unwrap();
    }));
    assert_eq!(
        objects
            .processes
            .get(process)
            .unwrap()
            .kernel_object_quota
            .used(),
        1
    );
    assert!(objects.with_thread(thread, |entry| {
        entry.priority = 3;
    }));
    assert_eq!(objects.threads.get(thread).unwrap().priority, 3);

    // An id the machine does not hold reaches nothing, and the body is
    // not run: the counter it would raise stays where it is.
    let mut touched = 0_u32;
    let ghost_process = ObjectId::new(3, 9);
    let ghost_thread = ObjectId::new(7, 9);
    assert!(!objects.with_process(ghost_process, |_| touched += 1));
    assert!(!objects.with_thread(ghost_thread, |_| touched += 1));
    assert_eq!(touched, 0);
}

/// A machine with every pool of Phase 6 filled with one object each, and
/// the ids of what it holds.
struct Filled {
    objects: Small,
    endpoint: crate::object::EndpointId,
    notification: crate::object::NotificationId,
    reply: crate::object::ReplyId,
    interrupt: crate::object::InterruptId,
    ports: crate::object::IoPortRangeId,
    caller: ThreadId,
}

fn filled() -> Filled {
    let mut objects = Small::new();
    let caller = objects
        .threads
        .allocate(Thread::new(owner(), 4, 8, 0, frame(0x20_0000)).unwrap())
        .unwrap();
    let endpoint = objects.endpoints.allocate(Endpoint::new()).unwrap();
    let notification = objects.notifications.allocate(Notification::new()).unwrap();
    let reply = objects.replies.allocate(Reply::new(caller)).unwrap();
    let interrupt = objects
        .interrupts
        .allocate(Interrupt::new(0, 0x40))
        .unwrap();
    let ports = objects
        .ports
        .allocate(IoPortRange::new(0x40, 4).unwrap())
        .unwrap();
    Filled {
        objects,
        endpoint,
        notification,
        reply,
        interrupt,
        ports,
        caller,
    }
}

#[test]
fn the_eight_pools_and_the_arena_report_their_capacity_and_their_count() {
    let small = Small::new();
    let capacities = small.capacities();
    assert_eq!(capacities.len(), 9);
    assert_eq!(
        capacities,
        [
            2,
            4,
            4,
            u32::try_from(crate::config::ENDPOINTS).unwrap(),
            u32::try_from(crate::config::NOTIFICATIONS).unwrap(),
            u32::try_from(crate::config::REPLIES).unwrap(),
            u32::try_from(crate::config::INTERRUPTS).unwrap(),
            u32::try_from(crate::config::IO_PORT_RANGES).unwrap(),
            8,
        ],
        "the five pools of Phase 6 take their size from the configuration"
    );
    assert_eq!(small.counts(), [0; 9]);
}

#[test]
fn every_pool_of_the_phase_counts_what_it_holds() {
    let held = filled();
    assert_eq!(held.objects.counts(), [0, 1, 0, 1, 1, 1, 1, 1, 0]);
}

#[test]
fn retaining_and_dropping_a_reference_meet_in_the_middle() {
    let mut held = filled();
    let id = AnyObjectId::of(held.endpoint);
    held.objects.retain(id).unwrap();
    assert_eq!(held.objects.endpoints.references(held.endpoint), Ok(2));
    assert_eq!(held.objects.destroy(id), None, "one reference is left");
    assert!(held.objects.endpoints.get(held.endpoint).is_ok());
}

#[test]
fn a_reference_can_be_taken_of_an_object_of_every_pool() {
    let mut held = filled();
    for id in [
        AnyObjectId::of(held.caller),
        AnyObjectId::of(held.endpoint),
        AnyObjectId::of(held.notification),
        AnyObjectId::of(held.reply),
        AnyObjectId::of(held.interrupt),
        AnyObjectId::of(held.ports),
    ] {
        assert!(held.objects.retain(id).is_ok(), "{id:?}");
        assert_eq!(held.objects.destroy(id), None, "{id:?}");
    }
    let process = held
        .objects
        .processes
        .allocate(Process::new(
            frame(0x10_0000),
            HandleList::with_capacity(4),
            Quota::new(4),
            Quota::new(4),
        ))
        .unwrap();
    let memory = held
        .objects
        .memory
        .allocate(MemoryObject::new(
            kernel_types::PhysFrameRange::new(frame(0x30_0000), 1).unwrap(),
            crate::object::MemoryKind::Ram,
            kernel_types::CachePolicy::WriteBack,
        ))
        .unwrap();
    for id in [AnyObjectId::of(process), AnyObjectId::of(memory)] {
        assert!(held.objects.retain(id).is_ok());
        assert_eq!(held.objects.destroy(id), None);
    }
}

#[test]
fn a_reference_to_the_system_control_capability_is_counted_nowhere() {
    let mut objects = Small::new();
    let id = AnyObjectId::of(SystemControl::ID);
    assert!(objects.retain(id).is_ok(), "there is nothing to count");
    assert_eq!(objects.destroy(id), None, "and nothing to destroy");
}

#[test]
fn retaining_an_object_that_is_gone_is_refused() {
    let mut objects = Small::new();
    let stale = AnyObjectId::of(ObjectId::<Endpoint>::new(3, 7));
    assert_eq!(objects.retain(stale), Err(Error::InvalidHandle));
}

#[test]
fn destroying_an_endpoint_hands_back_the_queues_it_held() {
    let mut held = filled();
    let mut endpoint = *held.objects.endpoints.get(held.endpoint).unwrap();
    endpoint
        .senders
        .enqueue(&mut held.objects.threads, held.caller)
        .unwrap();
    *held.objects.endpoints.get_mut(held.endpoint).unwrap() = endpoint;
    let gone = held.objects.destroy(AnyObjectId::of(held.endpoint));
    match gone {
        Some(Destroyed::Endpoint(id, held_endpoint)) => {
            assert_eq!(id, held.endpoint);
            assert_eq!(held_endpoint.senders.head(), Some(held.caller));
        }
        other => panic!("the endpoint came back as {other:?}"),
    }
    assert!(held.objects.endpoints.get(held.endpoint).is_err());
    assert_eq!(
        gone.as_ref().map(Destroyed::id),
        Some(AnyObjectId::of(held.endpoint))
    );
}

#[test]
fn destroying_a_notification_hands_back_its_waiter() {
    let mut held = filled();
    held.objects
        .notifications
        .get_mut(held.notification)
        .unwrap()
        .waiter = Some(held.caller);
    match held.objects.destroy(AnyObjectId::of(held.notification)) {
        Some(Destroyed::Notification(id, notification)) => {
            assert_eq!(id, held.notification);
            assert_eq!(notification.waiter, Some(held.caller));
        }
        other => panic!("the notification came back as {other:?}"),
    }
}

#[test]
fn destroying_a_reply_object_hands_back_the_caller_that_waits() {
    let mut held = filled();
    match held.objects.destroy(AnyObjectId::of(held.reply)) {
        Some(Destroyed::Reply(id, reply)) => {
            assert_eq!(id, held.reply);
            assert_eq!(reply.caller, held.caller);
            assert!(!reply.consumed);
        }
        other => panic!("the reply object came back as {other:?}"),
    }
}

#[test]
fn an_object_nobody_can_wait_on_is_destroyed_quietly() {
    let mut held = filled();
    for id in [
        AnyObjectId::of(held.interrupt),
        AnyObjectId::of(held.ports),
        AnyObjectId::of(held.caller),
    ] {
        assert_eq!(held.objects.destroy(id), Some(Destroyed::Quietly(id)));
    }
}

#[test]
fn destroying_an_object_that_is_already_gone_says_so() {
    let mut held = filled();
    let id = AnyObjectId::of(held.reply);
    assert!(held.objects.destroy(id).is_some());
    assert_eq!(
        held.objects.destroy(id),
        None,
        "a second time finds nothing"
    );
    let stale = AnyObjectId::of(ObjectId::<Notification>::new(9, 9));
    assert_eq!(held.objects.destroy(stale), None);
    let stale = AnyObjectId::of(ObjectId::<Endpoint>::new(9, 9));
    assert_eq!(held.objects.destroy(stale), None);
    let stale = AnyObjectId::of(ObjectId::<Interrupt>::new(9, 9));
    assert_eq!(held.objects.destroy(stale), None);
}
