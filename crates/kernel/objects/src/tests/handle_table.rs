// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::handle_table`, covering the handle items of the
//! catalog 6.6.6.

use audhsos_abi::{Error, Handle, ObjectType, Rights};

use crate::handle_table::{Entry, HandleArena, HandleList, Link};
use crate::object::{AnyObjectId, MemoryObject, Process, ProcessId, Thread};
use crate::pool::ObjectId;

/// The process every test installs handles for.
fn owner() -> ProcessId {
    ObjectId::new(0, 1)
}

/// Another process, to check that the arena keeps them apart.
fn other() -> ProcessId {
    ObjectId::new(1, 1)
}

/// An entry naming a thread, with every right a thread accepts.
fn thread_entry(index: u32) -> Entry {
    Entry::new(
        AnyObjectId::of(ObjectId::<Thread>::new(index, 1)),
        ObjectType::Thread.rights_mask(),
    )
}

/// An entry naming a memory object, with the rights given.
fn memory_entry(index: u32, rights: Rights) -> Entry {
    Entry::new(
        AnyObjectId::of(ObjectId::<MemoryObject>::new(index, 1)),
        rights,
    )
}

#[test]
fn an_empty_arena_is_a_constant() {
    // The arena of the kernel is a `static` of this type and reaches the
    // `.bss` only because the empty arena is a constant (D-66).
    const ARENA: HandleArena<8> = HandleArena::new();
    let arena = ARENA;
    assert!(arena.is_empty());
    assert_eq!(arena.live(), 0);
    assert_eq!(arena.capacity(), 8);
}

#[test]
fn a_handle_names_what_was_installed() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    let entry = thread_entry(3);
    let handle = arena.insert(owner(), &mut list, entry).unwrap();
    assert_eq!(arena.lookup(owner(), handle), Ok(&entry));
    assert_eq!(list.count(), 1);
    assert_eq!(arena.live(), 1);
    assert_eq!(handle.generation(), 1, "the first generation is one");
}

#[test]
fn a_handle_of_one_process_is_no_handle_of_another() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut mine = HandleList::with_capacity(4);
    let handle = arena.insert(owner(), &mut mine, thread_entry(1)).unwrap();
    assert_eq!(arena.lookup(other(), handle), Err(Error::InvalidHandle));
    assert_eq!(
        arena.lookup_mut(other(), handle),
        Err(Error::InvalidHandle),
        "not even for modification"
    );
    assert!(arena.lookup(owner(), handle).is_ok());
}

#[test]
fn a_handle_with_an_index_beyond_the_arena_is_invalid() {
    let arena: HandleArena<4> = HandleArena::new();
    let handle = Handle::new(9, 1).unwrap();
    assert_eq!(arena.lookup(owner(), handle), Err(Error::InvalidHandle));
    let highest = Handle::new(u32::MAX, 1).unwrap();
    assert_eq!(arena.lookup(owner(), highest), Err(Error::InvalidHandle));
}

#[test]
fn a_handle_of_a_slot_that_was_never_used_is_invalid() {
    let arena: HandleArena<4> = HandleArena::new();
    for index in 0..4 {
        let handle = Handle::new(index, 1).unwrap();
        assert_eq!(arena.lookup(owner(), handle), Err(Error::InvalidHandle));
    }
}

#[test]
fn a_stale_generation_is_rejected_after_the_slot_is_reused() {
    let mut arena: HandleArena<2> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    let first = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    assert!(arena.close(owner(), &mut list, first).is_ok());
    let second = arena.insert(owner(), &mut list, thread_entry(2)).unwrap();
    assert_eq!(second.index(), first.index(), "the slot came back");
    assert_ne!(second.generation(), first.generation());
    assert_eq!(arena.lookup(owner(), first), Err(Error::InvalidHandle));
    assert!(arena.lookup(owner(), second).is_ok());
}

#[test]
fn the_generation_wraps_and_never_becomes_zero() {
    let mut arena: HandleArena<1> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    assert!(arena.set_generation(0, u32::MAX - 1));
    let highest = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    assert_eq!(highest.generation(), u32::MAX);
    assert!(arena.close(owner(), &mut list, highest).is_ok());
    assert_eq!(arena.generation_of(0), Some(u32::MAX));
    let wrapped = arena.insert(owner(), &mut list, thread_entry(2)).unwrap();
    assert_eq!(wrapped.generation(), 1, "zero is skipped");
    assert_eq!(arena.lookup(owner(), highest), Err(Error::InvalidHandle));
    assert!(
        !arena.set_generation(0, 5),
        "an occupied slot is not changed"
    );
}

#[test]
fn closing_twice_fails_the_second_time() {
    let mut arena: HandleArena<4> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    let handle = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    assert_eq!(arena.close(owner(), &mut list, handle), Ok(thread_entry(1)));
    assert_eq!(
        arena.close(owner(), &mut list, handle),
        Err(Error::InvalidHandle)
    );
    assert_eq!(list.count(), 0);
    assert!(arena.is_empty());
}

#[test]
fn slots_come_back_in_the_order_they_were_closed() {
    let mut arena: HandleArena<4> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    let a = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    let b = arena.insert(owner(), &mut list, thread_entry(2)).unwrap();
    assert!(arena.close(owner(), &mut list, a).is_ok());
    assert!(arena.close(owner(), &mut list, b).is_ok());
    let next = arena.insert(owner(), &mut list, thread_entry(3)).unwrap();
    assert_eq!(next.index(), a.index(), "the one closed first comes first");
    let after = arena.insert(owner(), &mut list, thread_entry(4)).unwrap();
    assert_eq!(after.index(), b.index());
}

#[test]
fn a_full_arena_refuses_and_installs_nothing() {
    let mut arena: HandleArena<2> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    assert!(arena.insert(owner(), &mut list, thread_entry(1)).is_ok());
    assert!(arena.insert(owner(), &mut list, thread_entry(2)).is_ok());
    assert_eq!(
        arena.insert(owner(), &mut list, thread_entry(3)),
        Err(Error::OutOfHandles)
    );
    assert_eq!(list.count(), 2, "the refused handle left no trace");
    assert_eq!(arena.live(), 2);
}

#[test]
fn a_process_may_not_exceed_the_capacity_its_creator_granted() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(2);
    assert!(arena.insert(owner(), &mut list, thread_entry(1)).is_ok());
    assert!(arena.insert(owner(), &mut list, thread_entry(2)).is_ok());
    assert_eq!(
        arena.insert(owner(), &mut list, thread_entry(3)),
        Err(Error::QuotaExceeded)
    );
    assert_eq!(list.count(), 2);
    assert_eq!(list.free_slots(), 0);
    assert_eq!(arena.live(), 2, "the arena kept its slots for others");
}

#[test]
fn a_list_of_capacity_zero_holds_nothing() {
    let mut arena: HandleArena<4> = HandleArena::new();
    let mut list = HandleList::default();
    assert_eq!(list.capacity(), 0);
    assert!(list.is_empty());
    assert_eq!(
        arena.insert(owner(), &mut list, thread_entry(1)),
        Err(Error::QuotaExceeded)
    );
}

#[test]
fn duplicating_keeps_the_rights_or_fewer() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    let rights = Rights::READ | Rights::WRITE | Rights::DUPLICATE;
    let original = arena
        .insert(owner(), &mut list, memory_entry(1, rights))
        .unwrap();

    let same = arena
        .duplicate(owner(), &mut list, original, rights)
        .unwrap();
    assert_eq!(arena.lookup(owner(), same).unwrap().rights, rights);

    let fewer = Rights::READ | Rights::DUPLICATE;
    let narrowed = arena
        .duplicate(owner(), &mut list, original, fewer)
        .unwrap();
    assert_eq!(arena.lookup(owner(), narrowed).unwrap().rights, fewer);

    let more = rights | Rights::EXECUTE;
    assert_eq!(
        arena.duplicate(owner(), &mut list, original, more),
        Err(Error::AccessDenied)
    );
    assert_eq!(
        arena.lookup(owner(), original).unwrap().rights,
        rights,
        "the original is unchanged in every case"
    );
}

#[test]
fn duplicating_without_the_right_fails() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    let rights = Rights::READ | Rights::WRITE;
    let original = arena
        .insert(owner(), &mut list, memory_entry(1, rights))
        .unwrap();
    assert_eq!(
        arena.duplicate(owner(), &mut list, original, Rights::READ),
        Err(Error::AccessDenied)
    );
    assert_eq!(list.count(), 1, "nothing was installed");
    assert_eq!(arena.lookup(owner(), original).unwrap().rights, rights);
}

#[test]
fn a_duplicate_keeps_the_badge_and_the_object() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    let entry = memory_entry(2, Rights::READ | Rights::DUPLICATE).with_badge(0xBEEF);
    let original = arena.insert(owner(), &mut list, entry).unwrap();
    let copy = arena
        .duplicate(
            owner(),
            &mut list,
            original,
            Rights::READ | Rights::DUPLICATE,
        )
        .unwrap();
    let copied = *arena.lookup(owner(), copy).unwrap();
    assert_eq!(copied.badge, 0xBEEF);
    assert_eq!(copied.object, entry.object);
    assert_eq!(copied.object_type(), ObjectType::MemoryObject);
}

#[test]
fn duplicating_a_handle_of_another_process_fails() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut mine = HandleList::with_capacity(8);
    let mut theirs = HandleList::with_capacity(8);
    let handle = arena
        .insert(owner(), &mut mine, memory_entry(1, Rights::ALL))
        .unwrap();
    assert_eq!(
        arena.duplicate(other(), &mut theirs, handle, Rights::READ),
        Err(Error::InvalidHandle)
    );
    assert_eq!(theirs.count(), 0);
}

#[test]
fn installing_into_another_process_makes_a_handle_of_that_process() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mine = HandleList::with_capacity(8);
    let mut theirs = HandleList::with_capacity(8);
    let entry = memory_entry(1, Rights::READ);
    let installed = arena.insert(other(), &mut theirs, entry).unwrap();
    assert_eq!(arena.lookup(other(), installed), Ok(&entry));
    assert_eq!(arena.lookup(owner(), installed), Err(Error::InvalidHandle));
    assert_eq!(mine.count(), 0);
    assert_eq!(theirs.count(), 1);
}

#[test]
fn the_list_holds_every_handle_of_its_process() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    let first = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    let second = arena.insert(owner(), &mut list, thread_entry(2)).unwrap();
    let third = arena.insert(owner(), &mut list, thread_entry(3)).unwrap();
    let held: Vec<u32> = arena
        .handles_of(list)
        .map(|(handle, _)| handle.index())
        .collect();
    assert_eq!(
        held,
        vec![third.index(), second.index(), first.index()],
        "the chain runs from the newest to the oldest"
    );
}

#[test]
fn closing_from_the_middle_keeps_the_chain_whole() {
    let mut arena: HandleArena<8> = HandleArena::new();
    let mut list = HandleList::with_capacity(8);
    let first = arena.insert(owner(), &mut list, thread_entry(1)).unwrap();
    let second = arena.insert(owner(), &mut list, thread_entry(2)).unwrap();
    let third = arena.insert(owner(), &mut list, thread_entry(3)).unwrap();
    assert!(arena.close(owner(), &mut list, second).is_ok());
    let held: Vec<u32> = arena
        .handles_of(list)
        .map(|(handle, _)| handle.index())
        .collect();
    assert_eq!(held, vec![third.index(), first.index()]);
    assert_eq!(list.count(), 2);

    assert!(arena.close(owner(), &mut list, third).is_ok());
    let held: Vec<u32> = arena
        .handles_of(list)
        .map(|(handle, _)| handle.index())
        .collect();
    assert_eq!(held, vec![first.index()], "closing the head works too");
}

#[test]
fn closing_all_walks_the_chain_and_frees_every_slot() {
    let mut arena: HandleArena<16> = HandleArena::new();
    let mut mine = HandleList::with_capacity(16);
    let mut theirs = HandleList::with_capacity(16);
    for index in 0..5 {
        assert!(
            arena
                .insert(owner(), &mut mine, thread_entry(index))
                .is_ok()
        );
    }
    let kept = arena.insert(other(), &mut theirs, thread_entry(9)).unwrap();

    assert_eq!(arena.close_all(&mut mine), 5);
    assert!(mine.is_empty());
    assert_eq!(mine.count(), 0);
    assert_eq!(arena.handles_of(mine).count(), 0);
    assert_eq!(arena.live(), 1, "the other process keeps its handle");
    assert!(arena.lookup(other(), kept).is_ok());
    assert_eq!(
        arena.close_all(&mut mine),
        0,
        "a second sweep finds nothing"
    );
}

#[test]
fn every_slot_of_a_closed_process_is_handed_out_again() {
    let mut arena: HandleArena<4> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    for index in 0..4 {
        assert!(
            arena
                .insert(owner(), &mut list, thread_entry(index))
                .is_ok()
        );
    }
    assert_eq!(
        arena.insert(owner(), &mut list, thread_entry(4)),
        Err(Error::QuotaExceeded)
    );
    assert_eq!(arena.close_all(&mut list), 4);
    assert!(arena.is_empty());
    let mut second = HandleList::with_capacity(4);
    for index in 0..4 {
        assert!(
            arena
                .insert(other(), &mut second, thread_entry(index))
                .is_ok(),
            "slot {index} came back"
        );
    }
}

#[test]
fn a_modified_entry_is_what_the_next_lookup_sees() {
    let mut arena: HandleArena<4> = HandleArena::new();
    let mut list = HandleList::with_capacity(4);
    let handle = arena
        .insert(owner(), &mut list, memory_entry(1, Rights::READ))
        .unwrap();
    arena.lookup_mut(owner(), handle).unwrap().badge = 7;
    assert_eq!(arena.lookup(owner(), handle).unwrap().badge, 7);
}

#[test]
fn a_link_is_a_one_based_index_whose_zero_is_none() {
    assert!(Link::NONE.is_none());
    assert_eq!(Link::NONE.index(), None);
    assert_eq!(Link::default(), Link::NONE);
    assert_eq!(Link::to(0).index(), Some(0));
    assert_eq!(Link::to(41).index(), Some(41));
    assert!(!Link::to(0).is_none());
    assert_eq!(
        Link::to(u32::MAX),
        Link::NONE,
        "an index that cannot be stored links to nothing"
    );
}

#[test]
fn an_entry_carries_its_object_type() {
    let entry = Entry::new(
        AnyObjectId::of(ObjectId::<Process>::new(1, 1)),
        Rights::MANAGE,
    );
    assert_eq!(entry.object_type(), ObjectType::Process);
    assert_eq!(entry.badge, 0);
    assert_eq!(entry.with_badge(3).badge, 3);
    assert_eq!(entry.with_badge(3).rights, Rights::MANAGE);
}
