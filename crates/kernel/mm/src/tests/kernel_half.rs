// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::kernel_half`.

use audhsos_abi::layout::{
    BOOT_INFO_VADDR, BOOT_STACK_TOP, KERNEL_BASE, KERNEL_SPACE_START, KERNEL_STACKS_BASE,
    PHYS_WINDOW_BASE, USER_SPACE_START,
};
use kernel_hal_api::doubles::MemoryFrameAccess;
use kernel_hal_api::paging::FrameAccess;
use kernel_types::{PhysAddr, PhysFrame};

use crate::kernel_half::{
    ENTRIES, FIRST_KERNEL_ENTRY, ShareError, entry_of, kernel_entries, share, shares_kernel_half,
};
use crate::page_table::{EntryFormat, PageTable, Permissions, X86Entry};

/// The tables the tests work on.
type Tables = MemoryFrameAccess<PageTable<X86Entry>>;

/// The frame with the given number.
fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

/// An entry that points at `target`.
fn entry(target: u64) -> X86Entry {
    X86Entry::table(frame(target))
}

/// A pair of roots: a kernel root with the entries the kernel occupies,
/// and an empty one.
fn roots() -> (Tables, PhysFrame, PhysFrame) {
    let mut tables = Tables::default();
    let kernel = frame(1);
    let fresh = frame(2);
    tables.insert(kernel, PageTable::new());
    tables.insert(fresh, PageTable::new());
    let table = tables.table_mut(kernel).unwrap();
    table.set_entry(entry_of(PHYS_WINDOW_BASE), entry(10));
    table.set_entry(entry_of(KERNEL_BASE), entry(11));
    (tables, kernel, fresh)
}

#[test]
fn the_kernel_of_this_system_lives_in_two_top_level_entries() {
    // Everything the kernel maps falls into entry 256 or entry 511, which
    // is what makes sharing the upper half two words (D-65).
    assert_eq!(entry_of(PHYS_WINDOW_BASE), 256);
    assert_eq!(entry_of(KERNEL_STACKS_BASE), 511);
    assert_eq!(entry_of(BOOT_INFO_VADDR), 511);
    assert_eq!(entry_of(BOOT_STACK_TOP), 511);
    assert_eq!(entry_of(KERNEL_BASE), 511);
    assert_eq!(entry_of(KERNEL_SPACE_START), FIRST_KERNEL_ENTRY);
    assert_eq!(entry_of(USER_SPACE_START), 0);
}

#[test]
fn sharing_copies_every_kernel_entry_and_nothing_else() {
    let (mut tables, kernel, fresh) = roots();
    assert_eq!(share(&mut tables, kernel, fresh), Ok(2));
    assert_eq!(shares_kernel_half(&tables, kernel, fresh), Ok(true));

    let table = tables.table(fresh).unwrap();
    assert!(table.entry(entry_of(PHYS_WINDOW_BASE)).is_present());
    assert!(table.entry(entry_of(KERNEL_BASE)).is_present());
    for index in 0..FIRST_KERNEL_ENTRY {
        assert!(
            !table.entry(index).is_present(),
            "the user half stays empty at {index}"
        );
    }
}

#[test]
fn a_user_entry_of_the_kernel_root_is_not_copied() {
    let (mut tables, kernel, fresh) = roots();
    tables
        .table_mut(kernel)
        .unwrap()
        .set_entry(entry_of(USER_SPACE_START), entry(20));
    assert_eq!(share(&mut tables, kernel, fresh), Ok(2));
    assert!(
        !tables
            .table(fresh)
            .unwrap()
            .entry(entry_of(USER_SPACE_START))
            .is_present(),
        "what the kernel maps in the lower half is its own business"
    );
}

#[test]
fn what_the_target_had_in_the_lower_half_survives_the_sharing() {
    let (mut tables, kernel, fresh) = roots();
    tables
        .table_mut(fresh)
        .unwrap()
        .set_entry(entry_of(USER_SPACE_START), entry(30));
    assert_eq!(share(&mut tables, kernel, fresh), Ok(2));
    assert_eq!(
        tables
            .table(fresh)
            .unwrap()
            .entry(entry_of(USER_SPACE_START))
            .frame(),
        Some(frame(30))
    );
}

#[test]
fn a_kernel_mapping_made_afterwards_is_reachable_because_the_tables_are_shared() {
    // The entries are copied, the tables below them are not: a kernel
    // stack mapped after the root was built changes a table the two roots
    // both point at, so it is there in both.
    let (mut tables, kernel, fresh) = roots();
    assert_eq!(share(&mut tables, kernel, fresh), Ok(2));
    let stacks = entry_of(KERNEL_STACKS_BASE);
    assert_eq!(
        tables.table(fresh).unwrap().entry(stacks).frame(),
        tables.table(kernel).unwrap().entry(stacks).frame(),
        "both roots name the same table below the entry"
    );
}

#[test]
fn a_root_that_gains_a_kernel_entry_afterwards_is_no_longer_shared() {
    // The case the QEMU test rules out for the running kernel: an entry
    // that appears after a root was built is missing in that root.
    let (mut tables, kernel, fresh) = roots();
    assert_eq!(share(&mut tables, kernel, fresh), Ok(2));
    tables.table_mut(kernel).unwrap().set_entry(300, entry(40));
    assert_eq!(shares_kernel_half(&tables, kernel, fresh), Ok(false));
}

#[test]
fn the_entries_of_a_root_are_reported() {
    let (tables, kernel, fresh) = roots();
    let present = kernel_entries(&tables, kernel).unwrap();
    assert_eq!(present.len(), ENTRIES);
    let occupied: Vec<usize> = present
        .iter()
        .enumerate()
        .filter(|(_, present)| **present)
        .map(|(index, _)| index)
        .collect();
    assert_eq!(occupied, vec![256, 511]);
    assert!(kernel_entries(&tables, fresh).unwrap().iter().all(|p| !p));
}

#[test]
fn a_root_that_is_not_reachable_is_reported_and_nothing_is_written() {
    let (mut tables, kernel, _) = roots();
    let missing = frame(99);
    assert_eq!(
        share(&mut tables, missing, kernel),
        Err(ShareError::Unreachable(missing))
    );
    assert_eq!(
        share(&mut tables, kernel, missing),
        Err(ShareError::Unreachable(missing))
    );
    assert_eq!(
        kernel_entries(&tables, missing),
        Err(ShareError::Unreachable(missing))
    );
    assert_eq!(
        shares_kernel_half(&tables, kernel, missing),
        Err(ShareError::Unreachable(missing))
    );
    assert_eq!(
        shares_kernel_half(&tables, missing, kernel),
        Err(ShareError::Unreachable(missing))
    );
    assert_eq!(
        audhsos_abi::Error::from(ShareError::Unreachable(missing)),
        audhsos_abi::Error::OutOfKernelMemory
    );
}

#[test]
fn sharing_an_empty_kernel_root_shares_nothing() {
    let mut tables = Tables::default();
    let empty = frame(1);
    let fresh = frame(2);
    tables.insert(empty, PageTable::new());
    tables.insert(fresh, PageTable::new());
    assert_eq!(share(&mut tables, empty, fresh), Ok(0));
    assert_eq!(shares_kernel_half(&tables, empty, fresh), Ok(true));
}

#[test]
fn a_leaf_entry_of_the_kernel_half_is_shared_like_a_table_entry() {
    let (mut tables, kernel, fresh) = roots();
    let leaf = X86Entry::leaf(
        frame(50),
        Permissions::READ_WRITE,
        kernel_half_cache(),
        true,
    );
    tables.table_mut(kernel).unwrap().set_entry(300, leaf);
    assert_eq!(share(&mut tables, kernel, fresh), Ok(3));
    assert_eq!(tables.table(fresh).unwrap().entry(300), leaf);
}

/// The policy a kernel mapping uses.
fn kernel_half_cache() -> crate::page_table::CachePolicy {
    crate::page_table::CachePolicy::WriteBack
}

/// A physical address for a frame number, for the reader of the test.
#[test]
fn a_frame_number_is_an_address() {
    assert_eq!(frame(1).start(), PhysAddr::new(4096).unwrap());
}

/// A frame source that hands out frames in order and records what came
/// back.
#[derive(Debug, Default)]
struct Frames {
    next: u64,
    released: Vec<PhysFrame>,
}

impl kernel_hal_api::paging::FrameSource for Frames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.next = self.next.saturating_add(1);
        PhysFrame::from_number(self.next).ok()
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        self.released.push(frame);
    }
}

#[test]
fn freeing_the_user_half_gives_back_every_table_and_no_leaf() {
    use crate::kernel_half::free_user_half;
    use crate::mapper::Mapper;
    use kernel_hal_api::doubles::RecordingTlb;
    use kernel_types::{Page, VirtAddr};

    // Every frame the mapper takes for a table appears in the map on its
    // first access, the way a frame of the reserve does through the window.
    let mut tables =
        Tables::with_lazy_tables(kernel_types::PhysFrameRange::new(frame(1), 64).unwrap());
    let mut frames = Frames::default();
    let mut tlb = RecordingTlb::default();
    let root = frame(1);
    tables.insert(root, PageTable::new());

    let leaf = frame(500);
    {
        let mut mapper =
            Mapper::<'_, X86Entry, _, _, _>::new(root, &mut tables, &mut tlb, &mut frames);
        let page = Page::containing(VirtAddr::new(USER_SPACE_START).unwrap());
        mapper
            .map(
                page,
                leaf,
                Permissions::READ_WRITE.for_user(),
                crate::page_table::CachePolicy::WriteBack,
            )
            .unwrap();
    }
    // Three tables below the root for one page, plus the root itself.
    let freed = free_user_half(&mut tables, &mut frames, root);
    assert_eq!(freed, 4);
    assert!(
        !frames.released.contains(&leaf),
        "the frame of the mapping belongs to a memory object and stays"
    );
    assert!(frames.released.contains(&root));
    assert_eq!(frames.released.len(), 4);
}

#[test]
fn freeing_the_user_half_leaves_the_kernel_half_alone() {
    use crate::kernel_half::free_user_half;

    let (mut tables, kernel, fresh) = roots();
    let mut frames = Frames::default();
    share(&mut tables, kernel, fresh).unwrap();
    let freed = free_user_half(&mut tables, &mut frames, fresh);
    assert_eq!(freed, 1, "only the root, which had no user table");
    assert_eq!(frames.released, vec![fresh]);
    // The tables the kernel entries point at were never followed.
    assert!(
        tables
            .table(kernel)
            .unwrap()
            .entry(entry_of(KERNEL_BASE))
            .is_present(),
        "the kernel keeps its own tables"
    );
}

#[test]
fn freeing_a_root_that_is_not_reachable_gives_back_the_root_alone() {
    use crate::kernel_half::free_user_half;

    let mut tables = Tables::default();
    let mut frames = Frames::default();
    let missing = frame(77);
    assert_eq!(free_user_half(&mut tables, &mut frames, missing), 1);
    assert_eq!(frames.released, vec![missing]);
}
