// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::object`.

use audhsos_abi::layout::{PAGE_SIZE, PRIORITY_COUNT, THREADS_PER_PROCESS};
use audhsos_abi::{Error, ObjectType, ThreadState};
use kernel_types::{CachePolicy, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

use crate::handle_table::HandleList;
use crate::object::{
    AnyObjectId, Endpoint, Interrupt, IoPortRange, Links, MemoryKind, MemoryObject, Notification,
    Object, Process, ProcessId, Queue, Reply, SystemControl, Thread, ThreadId, Wait,
};
use crate::pool::ObjectId;
use crate::quota::Quota;

/// The frame a test process is rooted at.
fn root() -> PhysFrame {
    PhysFrame::containing(PhysAddr::new(0x10_0000).unwrap())
}

/// A process with room for `threads` threads and `handles` handles.
fn process(handles: u32) -> Process {
    Process::new(
        root(),
        HandleList::with_capacity(handles),
        Quota::new(64),
        Quota::new(16),
    )
}

fn thread_id(index: u32) -> ThreadId {
    ObjectId::new(index, 1)
}

fn process_id(index: u32) -> ProcessId {
    ObjectId::new(index, 1)
}

#[test]
fn every_object_names_its_type() {
    assert_eq!(Process::TYPE, ObjectType::Process);
    assert_eq!(Thread::TYPE, ObjectType::Thread);
    assert_eq!(MemoryObject::TYPE, ObjectType::MemoryObject);
}

#[test]
fn an_untyped_id_keeps_the_type_and_the_slot() {
    let id = thread_id(7);
    let any = AnyObjectId::of(id);
    assert_eq!(any.object_type(), ObjectType::Thread);
    assert_eq!(any.index(), 7);
    assert_eq!(any.generation(), 1);
    assert_eq!(any.typed::<Thread>(), Ok(id));
}

#[test]
fn an_untyped_id_of_the_wrong_type_is_refused() {
    let any = AnyObjectId::of(thread_id(1));
    assert_eq!(any.typed::<Process>(), Err(Error::WrongObjectType));
    assert_eq!(any.typed::<MemoryObject>(), Err(Error::WrongObjectType));
    assert_eq!(
        AnyObjectId::of(process_id(1)).typed::<Thread>(),
        Err(Error::WrongObjectType)
    );
}

#[test]
fn two_ids_of_different_types_are_different_untyped_ids() {
    let thread = AnyObjectId::of(ObjectId::<Thread>::new(3, 2));
    let memory = AnyObjectId::of(ObjectId::<MemoryObject>::new(3, 2));
    assert_ne!(thread, memory, "the slot is the same, the object is not");
    assert_eq!(thread.index(), memory.index());
}

#[test]
fn a_new_process_holds_nothing() {
    let process = process(4);
    assert_eq!(process.thread_count(), 0);
    assert_eq!(process.threads().count(), 0);
    assert_eq!(process.handles.count(), 0);
    assert_eq!(process.handles.capacity(), 4);
    assert_eq!(process.root, root());
    assert_eq!(process.fault_handler, None);
    assert!(process.regions.is_empty());
    assert!(process.has_region_room(1));
}

#[test]
fn a_process_holds_its_threads_and_forgets_them_again() {
    let mut process = process(4);
    let first = thread_id(1);
    let second = thread_id(2);
    assert_eq!(process.add_thread(first), Ok(0));
    assert_eq!(process.add_thread(second), Ok(1));
    assert_eq!(process.thread_count(), 2);
    assert_eq!(process.threads().collect::<Vec<_>>(), vec![first, second]);

    assert!(process.remove_thread(first));
    assert_eq!(process.threads().collect::<Vec<_>>(), vec![second]);
    assert!(
        !process.remove_thread(first),
        "a thread it does not hold is not removed twice"
    );
    assert!(!process.remove_thread(thread_id(9)));
}

#[test]
fn a_process_takes_no_more_threads_than_it_may_hold() {
    let mut process = process(4);
    for index in 0..THREADS_PER_PROCESS {
        let id = thread_id(u32::try_from(index).unwrap());
        assert_eq!(process.add_thread(id), Ok(index), "thread {index}");
    }
    assert_eq!(process.thread_count(), THREADS_PER_PROCESS);
    assert_eq!(
        process.add_thread(thread_id(999)),
        Err(Error::QuotaExceeded)
    );
    // A slot that comes free takes the next thread.
    assert!(process.remove_thread(thread_id(0)));
    assert_eq!(
        process.add_thread(thread_id(999)),
        Ok(0),
        "the slot that came free"
    );
}

#[test]
fn a_freed_thread_slot_is_used_again_before_the_process_is_full() {
    let mut process = process(4);
    assert_eq!(process.add_thread(thread_id(1)), Ok(0));
    assert_eq!(process.add_thread(thread_id(2)), Ok(1));
    assert!(process.remove_thread(thread_id(1)));
    assert_eq!(process.add_thread(thread_id(3)), Ok(0));
    let held: Vec<ThreadId> = process.threads().collect();
    assert_eq!(held, vec![thread_id(3), thread_id(2)]);
}

#[test]
fn the_region_room_of_a_process_is_what_its_table_has_left() {
    let process = process(4);
    let free = process.regions.free_slots();
    assert!(process.has_region_room(free));
    assert!(!process.has_region_room(free + 1));
}

#[test]
fn a_new_thread_starts_inactive_with_no_context() {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    let thread = Thread::new(process_id(0), 3, 7, 11, frame).unwrap();
    assert_eq!(thread.state, ThreadState::Inactive);
    assert!(!thread.is_runnable());
    assert_eq!(thread.priority, 3);
    assert_eq!(thread.max_priority, 7);
    assert_eq!(thread.kernel_stack, 11);
    assert_eq!(thread.ipc_buffer, frame);
    assert_eq!(thread.time_slice, 0);
    assert_eq!(thread.context, VirtAddr::ZERO);
    assert!(thread.queue_links.is_unlinked());
}

#[test]
fn a_thread_may_not_start_above_its_maximum_priority() {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    assert_eq!(
        Thread::new(process_id(0), 8, 7, 0, frame),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        Thread::new(process_id(0), 0, PRIORITY_COUNT, 0, frame),
        Err(Error::InvalidArgument),
        "a maximum outside the priorities of this system"
    );
    assert!(Thread::new(process_id(0), 0, PRIORITY_COUNT - 1, 0, frame).is_ok());
    assert!(Thread::new(process_id(0), 7, 7, 0, frame).is_ok());
}

#[test]
fn a_running_thread_is_runnable() {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    let mut thread = Thread::new(process_id(0), 0, 0, 0, frame).unwrap();
    for state in [ThreadState::Ready, ThreadState::Running] {
        thread.state = state;
        assert!(thread.is_runnable(), "{}", state.name());
    }
    for state in [
        ThreadState::Inactive,
        ThreadState::Suspended,
        ThreadState::Faulted,
        ThreadState::Exited,
    ] {
        thread.state = state;
        assert!(!thread.is_runnable(), "{}", state.name());
    }
}

#[test]
fn unlinked_links_are_the_default() {
    assert_eq!(Links::default(), Links::UNLINKED);
    assert!(Links::UNLINKED.is_unlinked());
    let linked = Links {
        next: Some(thread_id(1)),
        previous: None,
    };
    assert!(!linked.is_unlinked());
}

#[test]
fn a_memory_object_covers_its_frames() {
    let start = PhysFrame::containing(PhysAddr::new(0x100_0000).unwrap());
    let frames = PhysFrameRange::new(start, 8).unwrap();
    let object = MemoryObject::new(frames, MemoryKind::Ram, CachePolicy::WriteBack);
    assert_eq!(object.frame_count(), 8);
    assert_eq!(object.frames.count() * PAGE_SIZE, 8 * PAGE_SIZE);
    assert_eq!(object.kind, MemoryKind::Ram);
    assert_eq!(object.kind.name(), "Ram");
    assert!(!object.cache.is_uncached());

    let device = MemoryObject::new(frames, MemoryKind::Device, CachePolicy::Uncached);
    assert_eq!(device.kind.name(), "Device");
    assert!(device.cache.is_uncached());
    assert_eq!(MemoryKind::default(), MemoryKind::Ram);
}

#[test]
fn a_fresh_thread_waits_on_nothing_and_carries_no_fault() {
    let frame = PhysFrame::containing(PhysAddr::new(0x20_0000).unwrap());
    let thread = Thread::new(process_id(0), 3, 7, 11, frame).unwrap();
    assert_eq!(thread.wait, Wait::Nothing);
    assert!(thread.wait.is_nothing());
    assert!(thread.wait_links.is_unlinked());
    assert!(thread.queue_links.is_unlinked());
    assert_eq!(thread.fault, None);
}

#[test]
fn a_fresh_endpoint_has_two_empty_queues() {
    let endpoint = Endpoint::new();
    assert_eq!(endpoint, Endpoint::EMPTY);
    assert_eq!(endpoint, Endpoint::default());
    assert!(endpoint.is_quiet());
    assert!(endpoint.senders.is_empty());
    assert!(endpoint.receivers.is_empty());
    assert_eq!(Endpoint::TYPE, ObjectType::Endpoint);
}

#[test]
fn a_reply_object_names_its_caller_and_starts_unconsumed() {
    let caller: ThreadId = ObjectId::new(2, 5);
    let reply = Reply::new(caller);
    assert_eq!(reply.caller, caller);
    assert!(!reply.consumed);
    assert_eq!(Reply::TYPE, ObjectType::Reply);
}

#[test]
fn a_notification_merges_signals_and_clears_when_consumed() {
    let mut notification = Notification::new();
    assert_eq!(notification, Notification::EMPTY);
    assert_eq!(notification, Notification::default());
    assert_eq!(notification.waiter, None);
    notification.word |= 0b0010;
    notification.word |= 0b1000;
    assert_eq!(notification.consume(), 0b1010, "two signals are merged");
    assert_eq!(notification.word, 0);
    assert_eq!(notification.consume(), 0, "and nothing is left");
    assert_eq!(Notification::TYPE, ObjectType::Notification);
}

#[test]
fn an_interrupt_object_starts_bound_to_nothing_and_unmasked() {
    let interrupt = Interrupt::new(0, 0x40);
    assert_eq!(interrupt.line, Some(0));
    assert_eq!(interrupt.vector, 0x40);
    assert_eq!(interrupt.notification, None);
    assert!(!interrupt.masked);
    assert_eq!(Interrupt::TYPE, ObjectType::Interrupt);
}

#[test]
fn a_port_range_covers_what_it_says_and_nothing_beside_it() {
    let range = IoPortRange::new(0x40, 4).unwrap();
    assert_eq!(range.first, 0x40);
    assert_eq!(range.count, 4);
    assert_eq!(range.last(), 0x43);
    assert!(range.contains(0x40));
    assert!(range.contains(0x43));
    assert!(!range.contains(0x3F));
    assert!(!range.contains(0x44));
    assert_eq!(IoPortRange::TYPE, ObjectType::IoPortRange);
}

#[test]
fn a_port_range_of_no_ports_and_one_past_the_end_are_refused() {
    assert_eq!(IoPortRange::new(0, 0), Err(Error::InvalidArgument));
    assert_eq!(IoPortRange::new(0xFFFF, 2), Err(Error::InvalidArgument));
    assert!(IoPortRange::new(0xFFFF, 1).is_ok());
    assert!(IoPortRange::new(0, 0xFFFF).is_ok());
}

#[test]
fn an_access_has_to_fit_inside_the_range_it_is_made_through() {
    let range = IoPortRange::new(0x40, 4).unwrap();
    assert!(range.holds(0x40, 1));
    assert!(range.holds(0x40, 4));
    assert!(range.holds(0x42, 2));
    assert!(!range.holds(0x42, 4), "it would run past the end");
    assert!(!range.holds(0x44, 1), "outside the range altogether");
    let edge = IoPortRange::new(0xFFFF, 1).unwrap();
    assert!(edge.holds(0xFFFF, 1));
    assert!(!edge.holds(0xFFFF, 2), "the addition would wrap");
}

#[test]
fn two_port_ranges_overlap_exactly_when_they_share_a_port() {
    let range = IoPortRange::new(0x40, 4).unwrap();
    assert!(range.overlaps(&IoPortRange::new(0x43, 1).unwrap()));
    assert!(range.overlaps(&IoPortRange::new(0x3F, 2).unwrap()));
    assert!(range.overlaps(&IoPortRange::new(0x00, 0x100).unwrap()));
    assert!(!range.overlaps(&IoPortRange::new(0x44, 1).unwrap()));
    assert!(!range.overlaps(&IoPortRange::new(0x3E, 2).unwrap()));
}

#[test]
fn the_system_control_capability_holds_nothing_and_lives_in_no_pool() {
    assert_eq!(size_of::<SystemControl>(), 0);
    assert_eq!(SystemControl::TYPE, ObjectType::SystemControl);
    let id = AnyObjectId::of(SystemControl::ID);
    assert_eq!(id.object_type(), ObjectType::SystemControl);
    assert_eq!(id.generation(), 1, "never an id that was never handed out");
}

#[test]
fn the_wait_record_names_the_queue_and_what_it_asked_for() {
    assert!(Queue::Senders.is_sender());
    assert!(Queue::Callers.is_sender());
    assert!(!Queue::Receivers.is_sender());
    assert!(!Queue::Senders.wants_reply());
    assert!(Queue::Callers.wants_reply());
    assert!(!Queue::Receivers.wants_reply());
    assert_eq!(Wait::default(), Wait::Nothing);
    let waiting = Wait::Endpoint {
        endpoint: ObjectId::new(1, 1),
        queue: Queue::Callers,
        badge: 7,
    };
    assert!(!waiting.is_nothing());
}

#[test]
fn a_process_reports_faults_on_an_endpoint_or_on_nothing() {
    let mut holder = process(4);
    assert_eq!(holder.fault_handler, None);
    holder.fault_handler = Some((ObjectId::new(3, 2), 0x5EED));
    assert_eq!(holder.fault_handler, Some((ObjectId::new(3, 2), 0x5EED)));
}
