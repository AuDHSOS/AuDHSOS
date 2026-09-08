// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the memory calls.

use audhsos_abi::layout::{MAX_PAGES_PER_CALL, PAGE_SIZE};
use audhsos_abi::{Error, Handle, Rights, Syscall};
use kernel_objects::object::{AnyObjectId, MemoryObject};

use crate::tests::double::{Call, Fixture, call, error_of, request, value_of};

/// Where the tests map things.
const ADDRESS: u64 = 0x40_0000;

/// The rights a memory object of these tests carries.
fn full() -> Rights {
    Rights::READ | Rights::WRITE | Rights::EXECUTE | Rights::MAP | Rights::INFO
}

/// The permission bits: writable, executable.
const READ_ONLY: u64 = 0;
const WRITABLE: u64 = 0b1;
const EXECUTABLE: u64 = 0b10;

#[test]
fn mapping_puts_the_pages_into_the_address_space_and_the_region_table() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, full());
    let own = fixture.own_process.raw();
    let root = fixture.objects.processes.get(fixture.process).unwrap().root;
    let mapped = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, WRITABLE],
        ),
    );
    assert_eq!(mapped, 4);
    let pages = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Map(top, _, _) if *top == root))
        .count();
    assert_eq!(pages, 4);

    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    let region = holder.regions.iter().next().expect("a region");
    assert_eq!(region.pages.count(), 4);
    assert_eq!(region.pages.start().start().as_u64(), ADDRESS);
    assert_eq!(region.backing, object);
    assert!(region.perms.write);
    assert!(!region.perms.execute);
    assert!(region.perms.user, "a user thread has to reach it");
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        2,
        "the mapping holds a reference of its own"
    );
}

#[test]
fn mapping_writable_without_the_write_right_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, Rights::READ | Rights::MAP);
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, WRITABLE]
            )
        ),
        Some(Error::AccessDenied)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, EXECUTABLE]
            )
        ),
        Some(Error::AccessDenied)
    );
    assert_eq!(fixture.environment.calls.len(), 0, "nothing was mapped");
}

#[test]
fn mapping_an_object_without_the_map_right_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, Rights::READ);
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn permission_bits_that_name_nothing_are_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, 0b100]
            )
        ),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn an_offset_that_is_not_a_page_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 4, full());
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 8, PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::Unaligned)
    );
}

#[test]
fn a_range_beyond_what_the_object_holds_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, 3 * PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::InvalidArgument)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[
                    own,
                    handle.raw(),
                    ADDRESS,
                    2 * PAGE_SIZE,
                    PAGE_SIZE,
                    READ_ONLY
                ]
            )
        ),
        Some(Error::InvalidArgument),
        "an offset at the end of the object"
    );
}

#[test]
fn an_address_outside_user_space_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    let own = fixture.own_process.raw();
    for address in [0_u64, 0xFFFF_8000_0000_0000] {
        assert_eq!(
            error_of(
                &mut fixture,
                request(
                    Syscall::MemoryMap,
                    &[own, handle.raw(), address, 0, PAGE_SIZE, READ_ONLY]
                )
            ),
            Some(Error::InvalidArgument),
            "{address:#x}"
        );
    }
}

#[test]
fn mapping_where_something_is_mapped_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 4, full());
    let own = fixture.own_process.raw();
    assert!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, READ_ONLY]
            )
        )
        .is_none()
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::AddressInUse)
    );
}

#[test]
fn a_mapping_that_fails_halfway_takes_back_what_it_had_made() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 4, full());
    let own = fixture.own_process.raw();
    fixture.environment.no_mapping = true;
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::OutOfKernelMemory)
    );
    assert!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .regions
            .is_empty(),
        "no region was recorded"
    );
}

#[test]
fn a_mapping_longer_than_a_call_may_do_reports_how_far_it_came() {
    let mut fixture = Fixture::new();
    let frames = MAX_PAGES_PER_CALL + 8;
    let (_, handle) = fixture.memory(0x1000, frames, full());
    let own = fixture.own_process.raw();
    let mut buffer = request(
        Syscall::MemoryMap,
        &[
            own,
            handle.raw(),
            ADDRESS,
            0,
            frames.saturating_mul(PAGE_SIZE),
            READ_ONLY,
        ],
    );
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert!(status.is_success());
    assert!(status.is_partial(), "it did part of the work");
    assert_eq!(values[0], MAX_PAGES_PER_CALL);
    let region = fixture
        .objects
        .processes
        .get(fixture.process)
        .unwrap()
        .regions
        .iter()
        .next()
        .expect("a region")
        .pages
        .count();
    assert_eq!(region, MAX_PAGES_PER_CALL);
}

#[test]
fn unmapping_takes_the_pages_out_and_gives_the_object_back() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, full());
    let own = fixture.own_process.raw();
    let _ = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, READ_ONLY],
        ),
    );
    let removed = value_of(
        &mut fixture,
        request(Syscall::MemoryUnmap, &[own, ADDRESS, 4 * PAGE_SIZE]),
    );
    assert_eq!(removed, 4);
    assert!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .regions
            .is_empty()
    );
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        1,
        "the reference of the mapping is gone"
    );
}

#[test]
fn unmapping_what_is_not_mapped_is_refused() {
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryUnmap, &[own, ADDRESS, PAGE_SIZE])
        ),
        Some(Error::NotMapped)
    );
}

#[test]
fn protecting_a_mapped_range_changes_what_it_allows() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    let own = fixture.own_process.raw();
    let _ = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 2 * PAGE_SIZE, WRITABLE],
        ),
    );
    let done = value_of(
        &mut fixture,
        request(
            Syscall::MemoryProtect,
            &[own, ADDRESS, 2 * PAGE_SIZE, READ_ONLY],
        ),
    );
    assert_eq!(done, 2);
    let region = *fixture
        .objects
        .processes
        .get(fixture.process)
        .unwrap()
        .regions
        .iter()
        .next()
        .expect("a region");
    assert!(!region.perms.write);
    let changed = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Protect(_, _, _)))
        .count();
    assert_eq!(changed, 2);
}

#[test]
fn protecting_what_is_not_mapped_is_refused() {
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryProtect,
                &[own, ADDRESS, PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::NotMapped)
    );
}

#[test]
fn splitting_an_object_makes_two_of_it() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 6, full());
    let raw = value_of(
        &mut fixture,
        request(Syscall::MemorySplit, &[handle.raw(), 2 * PAGE_SIZE]),
    );
    let second = Handle::from_raw(raw).expect("a handle");
    let tail = fixture
        .objects
        .entry(fixture.process, second)
        .unwrap()
        .object
        .typed::<MemoryObject>()
        .unwrap();
    assert_eq!(fixture.objects.memory.get(object).unwrap().frame_count(), 2);
    assert_eq!(fixture.objects.memory.get(tail).unwrap().frame_count(), 4);
    assert_eq!(
        fixture
            .objects
            .memory
            .get(tail)
            .unwrap()
            .frames
            .start()
            .number(),
        0x102,
        "the second part starts where the first ends"
    );
    assert_eq!(
        fixture
            .objects
            .entry(fixture.process, second)
            .unwrap()
            .rights,
        full(),
        "the parts carry the rights of the whole"
    );
}

#[test]
fn splitting_at_an_offset_that_is_no_split_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 4, full());
    for offset in [0, 4 * PAGE_SIZE, 8 * PAGE_SIZE] {
        assert_eq!(
            error_of(
                &mut fixture,
                request(Syscall::MemorySplit, &[handle.raw(), offset])
            ),
            Some(Error::InvalidArgument),
            "offset {offset}"
        );
    }
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemorySplit, &[handle.raw(), 100])
        ),
        Some(Error::Unaligned)
    );
}

#[test]
fn splitting_beyond_the_pool_or_the_quota_is_refused_and_changes_nothing() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 8, full());
    let quota = &mut fixture
        .objects
        .processes
        .get_mut(fixture.process)
        .unwrap()
        .kernel_object_quota;
    let left = quota.remaining();
    quota.charge(left).unwrap();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemorySplit, &[handle.raw(), PAGE_SIZE])
        ),
        Some(Error::QuotaExceeded)
    );
    assert_eq!(
        fixture.objects.memory.get(object).unwrap().frame_count(),
        8,
        "the object is whole"
    );
}

#[test]
fn memory_info_reports_where_the_object_lies() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 3, full());
    let mut buffer = request(Syscall::MemoryInfo, &[handle.raw()]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert_eq!(values[0], 0x100 * PAGE_SIZE);
    assert_eq!(values[1], 3 * PAGE_SIZE);
}

#[test]
fn memory_reference_count_includes_mappings_and_transferred_handles() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 1, full() | Rights::DUPLICATE | Rights::TRANSFER);
    let own = fixture.own_process.raw();
    assert_eq!(
        value_of(
            &mut fixture,
            request(Syscall::MemoryReferences, &[handle.raw()])
        ),
        1
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, WRITABLE],
        ),
    );
    assert_eq!(
        value_of(
            &mut fixture,
            request(Syscall::MemoryReferences, &[handle.raw()])
        ),
        2
    );
    let duplicate = value_of(
        &mut fixture,
        request(
            Syscall::HandleDuplicate,
            &[handle.raw(), u64::from(Rights::INFO.bits())],
        ),
    );
    assert_eq!(
        value_of(
            &mut fixture,
            request(Syscall::MemoryReferences, &[handle.raw()])
        ),
        3
    );
    value_of(&mut fixture, request(Syscall::HandleClose, &[duplicate]));
    assert_eq!(
        value_of(
            &mut fixture,
            request(Syscall::MemoryReferences, &[handle.raw()])
        ),
        2
    );
    value_of(
        &mut fixture,
        request(Syscall::MemoryUnmap, &[own, ADDRESS, PAGE_SIZE]),
    );
    assert_eq!(fixture.objects.memory.references(object), Ok(1));
    let without = fixture.install(AnyObjectId::of(object), Rights::READ);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryReferences, &[without.raw()])
        ),
        Some(Error::AccessDenied)
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::MemoryReferences, &[0])),
        Some(Error::InvalidHandle)
    );
}

#[test]
fn memory_info_without_the_info_right_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 3, Rights::READ | Rights::MAP);
    assert_eq!(
        error_of(&mut fixture, request(Syscall::MemoryInfo, &[handle.raw()])),
        Some(Error::AccessDenied)
    );
}

#[test]
fn a_memory_call_on_a_handle_of_the_wrong_type_is_refused() {
    let mut fixture = Fixture::new();
    let thread = fixture
        .install(AnyObjectId::of(fixture.thread), Rights::ALL)
        .raw();
    assert_eq!(
        error_of(&mut fixture, request(Syscall::MemoryInfo, &[thread])),
        Some(Error::WrongObjectType)
    );
    assert_eq!(
        error_of(&mut fixture, request(Syscall::MemorySplit, &[thread, 0])),
        Some(Error::WrongObjectType)
    );
}

#[test]
fn unmapping_more_than_a_call_may_do_reports_how_far_it_came() {
    let mut fixture = Fixture::new();
    let frames = MAX_PAGES_PER_CALL + 4;
    let (_, handle) = fixture.memory(0x1000, frames, full());
    let own = fixture.own_process.raw();
    let length = frames.saturating_mul(PAGE_SIZE);
    let _ = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, length, READ_ONLY],
        ),
    );
    // The mapping stopped at the bound, so the range that is mapped is
    // exactly what one call may take out again.
    let mut buffer = request(
        Syscall::MemoryUnmap,
        &[own, ADDRESS, MAX_PAGES_PER_CALL.saturating_mul(PAGE_SIZE)],
    );
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert!(status.is_success());
    assert!(!status.is_partial());
    assert_eq!(values[0], MAX_PAGES_PER_CALL);
}

#[test]
fn protecting_more_than_a_call_may_do_reports_how_far_it_came() {
    let mut fixture = Fixture::new();
    let frames = MAX_PAGES_PER_CALL + 4;
    let (_, handle) = fixture.memory(0x1000, frames, full());
    let own = fixture.own_process.raw();
    let length = frames.saturating_mul(PAGE_SIZE);
    let _ = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, length, READ_ONLY],
        ),
    );
    let mut buffer = request(Syscall::MemoryProtect, &[own, ADDRESS, length, WRITABLE]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert!(status.is_success());
    assert!(status.is_partial());
    assert_eq!(values[0], MAX_PAGES_PER_CALL);
}

#[test]
fn an_unmapping_the_page_tables_refuse_is_reported() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    let own = fixture.own_process.raw();
    let _ = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 2 * PAGE_SIZE, READ_ONLY],
        ),
    );
    fixture.environment.not_mapped = true;
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryUnmap, &[own, ADDRESS, 2 * PAGE_SIZE])
        ),
        Some(Error::NotMapped)
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryProtect,
                &[own, ADDRESS, 2 * PAGE_SIZE, WRITABLE]
            )
        ),
        Some(Error::NotMapped)
    );
}

#[test]
fn mapping_with_a_full_region_table_is_refused_and_takes_its_pages_back() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 1, full());
    let own = fixture.own_process.raw();
    // Fill the region table past the accessors, so that the call meets a
    // table with no room rather than sixty-four mappings.
    let holder = fixture.objects.processes.get_mut(fixture.process).unwrap();
    let mut page = 0x1000_u64;
    while holder.regions.free_slots() > 0 {
        let pages = kernel_types::PageRange::new(
            kernel_types::Page::containing(kernel_types::VirtAddr::new(page * PAGE_SIZE).unwrap()),
            1,
        )
        .unwrap();
        holder
            .regions
            .insert(kernel_mm::address_space::Region {
                pages,
                backing: kernel_objects::pool::ObjectId::new(0, 1),
                offset: 0,
                perms: kernel_mm::page_table::Permissions::READ_ONLY.for_user(),
            })
            .unwrap();
        page += 2;
    }
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, handle.raw(), ADDRESS, 0, PAGE_SIZE, READ_ONLY]
            )
        ),
        Some(Error::QuotaExceeded)
    );
    let unmapped = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Unmap(_, _)))
        .count();
    assert_eq!(unmapped, 1, "the page it had mapped went back");
}

#[test]
fn merging_two_neighbours_makes_one_object_of_them() {
    let mut fixture = Fixture::new();
    let (lower, low) = fixture.memory(0x100, 2, full());
    let (upper, high) = fixture.memory(0x102, 4, full());
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        None
    );
    let joined = *fixture.objects.memory.get(lower).unwrap();
    assert_eq!(joined.frames.start().number(), 0x100);
    assert_eq!(joined.frame_count(), 6, "the two ranges, end to end");
    assert!(
        fixture.objects.memory.get(upper).is_err(),
        "the upper object is gone"
    );
    assert!(
        fixture.objects.entry(fixture.process, high).is_err(),
        "and so is the handle that named it"
    );
}

#[test]
fn what_a_split_made_a_merge_undoes() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x200, 8, full());
    let raw = value_of(
        &mut fixture,
        request(Syscall::MemorySplit, &[handle.raw(), 3 * PAGE_SIZE]),
    );
    let second = Handle::from_raw(raw).expect("a handle");
    assert_eq!(fixture.objects.memory.get(object).unwrap().frame_count(), 3);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[handle.raw(), second.raw()])
        ),
        None
    );
    let back = *fixture.objects.memory.get(object).unwrap();
    assert_eq!(back.frames.start().number(), 0x200);
    assert_eq!(back.frame_count(), 8, "the object it was before the split");
}

#[test]
fn merging_two_that_do_not_touch_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    let (_, high) = fixture.memory(0x110, 2, full());
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn merging_them_the_wrong_way_round_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    let (_, high) = fixture.memory(0x102, 2, full());
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[high.raw(), low.raw()])
        ),
        Some(Error::InvalidArgument),
        "the first argument is the lower part"
    );
}

#[test]
fn merging_an_object_with_itself_is_refused() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x100, 2, full());
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[handle.raw(), handle.raw()])
        ),
        Some(Error::InvalidArgument)
    );
}

#[test]
fn merging_two_that_carry_different_rights_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    let (_, high) = fixture.memory(0x102, 2, Rights::MAP | Rights::READ);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn merging_without_the_map_right_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, Rights::READ);
    let (_, high) = fixture.memory(0x102, 2, Rights::READ);
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        Some(Error::AccessDenied)
    );
}

#[test]
fn merging_an_object_that_something_else_holds_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    let (upper, high) = fixture.memory(0x102, 2, full());
    // A second handle to the upper part, which is a second reference.
    let second = fixture.install(AnyObjectId::of(upper), full());
    fixture.objects.retain(AnyObjectId::of(upper)).unwrap();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        Some(Error::Busy)
    );
    assert!(
        fixture.objects.entry(fixture.process, second).is_ok(),
        "the refused merge took nothing away"
    );
}

#[test]
fn merging_a_mapped_object_is_refused() {
    let mut fixture = Fixture::new();
    let own = fixture.own_process.raw();
    let (_, low) = fixture.memory(0x100, 2, full());
    let (_, high) = fixture.memory(0x102, 2, full());
    assert_eq!(
        error_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[own, high.raw(), ADDRESS, 0, PAGE_SIZE, WRITABLE]
            )
        ),
        None
    );
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), high.raw()])
        ),
        Some(Error::Busy),
        "a mapping is a reference"
    );
}

#[test]
fn merging_a_handle_the_caller_does_not_hold_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    assert_eq!(
        error_of(&mut fixture, request(Syscall::MemoryMerge, &[low.raw(), 0])),
        Some(Error::InvalidHandle),
        "a second argument that is no handle"
    );
    let stranger = Handle::new(4000, 1).unwrap();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), stranger.raw()])
        ),
        Some(Error::InvalidHandle)
    );
}

#[test]
fn merging_something_that_is_no_memory_object_is_refused() {
    let mut fixture = Fixture::new();
    let (_, low) = fixture.memory(0x100, 2, full());
    let thread = fixture.own_thread.raw();
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[low.raw(), thread])
        ),
        Some(Error::WrongObjectType)
    );
}

#[test]
fn a_merge_gives_the_kernel_object_back_to_the_quota() {
    let mut fixture = Fixture::new();
    let (_, handle) = fixture.memory(0x300, 4, full());
    let before = fixture
        .objects
        .processes
        .get(fixture.process)
        .unwrap()
        .kernel_object_quota
        .used();
    let raw = value_of(
        &mut fixture,
        request(Syscall::MemorySplit, &[handle.raw(), 2 * PAGE_SIZE]),
    );
    let second = Handle::from_raw(raw).expect("a handle");
    assert_eq!(
        error_of(
            &mut fixture,
            request(Syscall::MemoryMerge, &[handle.raw(), second.raw()])
        ),
        None
    );
    let after = fixture
        .objects
        .processes
        .get(fixture.process)
        .unwrap()
        .kernel_object_quota
        .used();
    assert_eq!(
        after, before,
        "the split charged one and the merge gave it back"
    );
}

#[test]
fn a_second_mapping_that_continues_the_first_is_one_region_and_one_reference() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 8, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    let mapped = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, WRITABLE],
        ),
    );
    assert_eq!(mapped, 4);
    let mapped = value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[
                own,
                handle.raw(),
                ADDRESS + 4 * PAGE_SIZE,
                4 * PAGE_SIZE,
                4 * PAGE_SIZE,
                WRITABLE,
            ],
        ),
    );
    assert_eq!(mapped, 4);
    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(holder.regions.len(), 1, "the two calls made one region");
    let region = holder.regions.iter().next().unwrap();
    assert_eq!(region.pages.count(), 8);
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        2,
        "one reference for the region, one for the handle"
    );
}

#[test]
fn unmapping_part_of_a_region_keeps_the_reference_the_rest_of_it_holds() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, WRITABLE],
        ),
    );
    assert_eq!(fixture.objects.memory.references(object).unwrap(), 2);
    value_of(
        &mut fixture,
        request(Syscall::MemoryUnmap, &[own, ADDRESS, 2 * PAGE_SIZE]),
    );
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        2,
        "half the region is still mapped"
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryUnmap,
            &[own, ADDRESS + 2 * PAGE_SIZE, 2 * PAGE_SIZE],
        ),
    );
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        1,
        "the region is gone and so is its reference"
    );
}

#[test]
fn protecting_the_middle_of_a_mapping_and_unmapping_all_of_it_holds_the_count() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, WRITABLE],
        ),
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryProtect,
            &[own, ADDRESS + PAGE_SIZE, 2 * PAGE_SIZE, READ_ONLY],
        ),
    );
    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(holder.regions.len(), 3, "head, middle, and tail");
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        4,
        "one reference per region, and one for the handle"
    );
    let mut left = 4 * PAGE_SIZE;
    let mut at = ADDRESS;
    while left > 0 {
        let done = value_of(
            &mut fixture,
            request(Syscall::MemoryUnmap, &[own, at, left]),
        );
        assert!(done > 0, "the unmap made no progress");
        at += done * PAGE_SIZE;
        left -= done * PAGE_SIZE;
    }
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        1,
        "every region gave its reference back, and the handle kept its own"
    );
}

#[test]
fn unmapping_the_middle_of_a_mapping_and_then_the_rest_holds_the_count() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 4 * PAGE_SIZE, WRITABLE],
        ),
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryUnmap,
            &[own, ADDRESS + PAGE_SIZE, 2 * PAGE_SIZE],
        ),
    );
    let holder = fixture.objects.processes.get(fixture.process).unwrap();
    assert_eq!(holder.regions.len(), 2, "the region was divided in two");
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        3,
        "two regions and the handle"
    );
    value_of(
        &mut fixture,
        request(Syscall::MemoryUnmap, &[own, ADDRESS, PAGE_SIZE]),
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryUnmap,
            &[own, ADDRESS + 3 * PAGE_SIZE, PAGE_SIZE],
        ),
    );
    assert_eq!(fixture.objects.memory.references(object).unwrap(), 1);
}

#[test]
fn unmapping_a_range_that_reaches_over_a_gap_takes_what_is_mapped_and_nothing_else() {
    let mut fixture = Fixture::new();
    let (object, handle) = fixture.memory(0x100, 4, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    // Two mappings of two pages each, with two unmapped pages between them.
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[own, handle.raw(), ADDRESS, 0, 2 * PAGE_SIZE, WRITABLE],
        ),
    );
    value_of(
        &mut fixture,
        request(
            Syscall::MemoryMap,
            &[
                own,
                handle.raw(),
                ADDRESS + 4 * PAGE_SIZE,
                2 * PAGE_SIZE,
                2 * PAGE_SIZE,
                WRITABLE,
            ],
        ),
    );
    assert_eq!(fixture.objects.memory.references(object).unwrap(), 3);
    let done = value_of(
        &mut fixture,
        request(Syscall::MemoryUnmap, &[own, ADDRESS, 6 * PAGE_SIZE]),
    );
    assert_eq!(done, 6, "the whole request was answered");
    assert!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .regions
            .is_empty()
    );
    assert_eq!(
        fixture.objects.memory.references(object).unwrap(),
        1,
        "both mappings gave their reference back"
    );
    let unmapped = fixture
        .environment
        .calls
        .iter()
        .filter(|call| matches!(call, Call::Unmap(_, _)))
        .count();
    assert_eq!(unmapped, 4, "the pages of the gap were never unmapped");
}

#[test]
fn a_range_over_more_mappings_than_one_call_takes_comes_back_with_the_rest() {
    let mut fixture = Fixture::new();
    let (_object, handle) = fixture.memory(0x100, 4, Rights::READ | Rights::WRITE | Rights::MAP);
    let own = fixture.own_process.raw();
    for index in 0..3 {
        value_of(
            &mut fixture,
            request(
                Syscall::MemoryMap,
                &[
                    own,
                    handle.raw(),
                    ADDRESS + index * 2 * PAGE_SIZE,
                    index * PAGE_SIZE,
                    PAGE_SIZE,
                    WRITABLE,
                ],
            ),
        );
    }
    let mut buffer = request(Syscall::MemoryUnmap, &[own, ADDRESS, 6 * PAGE_SIZE]);
    let (status, values, _) = call(&mut fixture, &mut buffer);
    assert_eq!(status.error(), None);
    assert!(status.is_partial(), "two of the three mappings were taken");
    assert_eq!(
        values[0], 4,
        "up to the page the mapping it did not reach begins at"
    );
    assert_eq!(
        fixture
            .objects
            .processes
            .get(fixture.process)
            .unwrap()
            .regions
            .len(),
        1
    );
}
