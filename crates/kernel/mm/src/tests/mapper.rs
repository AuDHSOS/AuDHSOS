// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mapper`, covering the walker items of the catalog
//! 6.6.4.

#![allow(clippy::arithmetic_side_effects)]

use std::collections::HashMap;

use audhsos_abi::layout::{KERNEL_SPACE_START, MAX_PAGES_PER_CALL, PAGE_SIZE, USER_SPACE_END};
use kernel_hal_api::doubles::{CountingFrameSource, Flush, MemoryFrameAccess, RecordingTlb};
use kernel_hal_api::paging::FrameAccess;
use kernel_types::{Page, PageRange, PhysFrame, PhysFrameRange, VirtAddr};
use test_support::generators::{BoxGen, Generator, one_of, pair, range};
use test_support::model::{ModelTest, run_model_test};

use crate::mapper::{MapError, Mapper, Progress};
use crate::page_table::{
    CachePolicy, EntryError, EntryFormat, PageTable, Permissions, X86_GLOBAL, X86_PRESENT,
    X86_RESERVED_MASK, X86Entry,
};
use crate::strategies::any_permissions;

/// First frame of the memory the double materializes tables in.
const RAM_START: u64 = 0x100;

/// Number of frames of that memory.
const RAM_FRAMES: u64 = 4096;

/// First frame the frame source hands out.
const POOL_START: u64 = 0x200;

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

fn page(address: u64) -> Page {
    VirtAddr::new(address).unwrap().page()
}

/// A mapper with its three doubles.
struct Fixture {
    access: MemoryFrameAccess<PageTable<X86Entry>>,
    tlb: RecordingTlb,
    frames: CountingFrameSource,
    root: PhysFrame,
}

impl Fixture {
    fn new() -> Self {
        Self::with_limit(None)
    }

    fn with_limit(limit: Option<u64>) -> Self {
        let ram = PhysFrameRange::new(frame(RAM_START), RAM_FRAMES).unwrap();
        let mut access = MemoryFrameAccess::with_lazy_tables(ram);
        let root = frame(RAM_START);
        access.insert(root, PageTable::default());
        Fixture {
            access,
            tlb: RecordingTlb::new(),
            frames: CountingFrameSource::new(frame(POOL_START), limit),
            root,
        }
    }

    fn mapper(
        &mut self,
    ) -> Mapper<
        '_,
        X86Entry,
        MemoryFrameAccess<PageTable<X86Entry>>,
        RecordingTlb,
        CountingFrameSource,
    > {
        Mapper::new(self.root, &mut self.access, &mut self.tlb, &mut self.frames)
    }

    fn flushes(&self) -> &[Flush] {
        self.tlb.flushes()
    }

    fn clear_flushes(&mut self) {
        self.tlb.clear();
    }

    fn allocated(&self) -> usize {
        self.frames.allocated().len()
    }

    fn outstanding(&self) -> usize {
        self.frames.outstanding()
    }

    /// The raw entry a page selects at `level`, following the tables.
    fn entry_at(&self, page: Page, level: usize) -> Option<X86Entry> {
        let mut current = X86Entry::LEVELS - 1;
        let mut table_frame = self.root;
        while current > level {
            let entry = self
                .access
                .table(table_frame)?
                .entry(X86Entry::index(current, page));
            table_frame = entry.frame()?;
            current -= 1;
        }
        Some(
            self.access
                .table(table_frame)?
                .entry(X86Entry::index(level, page)),
        )
    }
}

#[test]
fn mapping_the_first_page_creates_a_table_on_every_level() {
    let mut fixture = Fixture::new();
    let target = page(0x1000);
    assert_eq!(
        fixture.mapper().map(
            target,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Ok(())
    );
    assert_eq!(fixture.allocated(), 3, "one table per level below the root");
    assert_eq!(fixture.flushes(), &[Flush::Page(target)]);
    assert_eq!(
        fixture.mapper().translate(target),
        Some((frame(0x40), Permissions::READ_WRITE))
    );

    fixture.clear_flushes();
    let neighbor = page(0x2000);
    assert_eq!(
        fixture.mapper().map(
            neighbor,
            frame(0x41),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Ok(())
    );
    assert_eq!(fixture.allocated(), 3, "the neighbor needs no new table");
    assert_eq!(fixture.flushes(), &[Flush::Page(neighbor)]);
}

#[test]
fn mapping_the_same_page_twice_changes_nothing() {
    let mut fixture = Fixture::new();
    let target = page(0x1000);
    fixture
        .mapper()
        .map(
            target,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    fixture.clear_flushes();
    let before = fixture.entry_at(target, 0);
    assert_eq!(
        fixture.mapper().map(
            target,
            frame(0x77),
            Permissions::READ_EXECUTE,
            CachePolicy::Uncached
        ),
        Err(MapError::AlreadyMapped)
    );
    assert_eq!(fixture.entry_at(target, 0), before);
    assert!(fixture.flushes().is_empty(), "a failed map flushes nothing");
    assert_eq!(
        fixture.mapper().translate(target),
        Some((frame(0x40), Permissions::READ_WRITE))
    );
}

#[test]
fn unmapping_a_page_without_a_translation_is_reported() {
    let mut fixture = Fixture::new();
    assert_eq!(
        fixture.mapper().unmap(page(0x1000)),
        Err(MapError::NotMapped),
        "no table exists at all"
    );
    assert!(fixture.flushes().is_empty());
    fixture
        .mapper()
        .map(
            page(0x1000),
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    fixture.clear_flushes();
    assert_eq!(
        fixture.mapper().unmap(page(0x2000)),
        Err(MapError::NotMapped),
        "the tables exist but the leaf is empty"
    );
    assert!(fixture.flushes().is_empty());
    assert_eq!(
        fixture
            .mapper()
            .protect(page(0x2000), Permissions::READ_ONLY),
        Err(MapError::NotMapped)
    );
    assert!(fixture.flushes().is_empty());
}

#[test]
fn unmapping_the_last_page_of_a_table_frees_it_and_otherwise_does_not() {
    let mut fixture = Fixture::new();
    let first = page(0x1000);
    let second = page(0x2000);
    for (target, number) in [(first, 0x40), (second, 0x41)] {
        fixture
            .mapper()
            .map(
                target,
                frame(number),
                Permissions::READ_WRITE,
                CachePolicy::WriteBack,
            )
            .unwrap();
    }
    assert_eq!(fixture.outstanding(), 3);
    fixture.clear_flushes();
    assert_eq!(fixture.mapper().unmap(first), Ok(frame(0x40)));
    assert_eq!(
        fixture.outstanding(),
        3,
        "the table still holds the second page"
    );
    assert_eq!(fixture.flushes(), &[Flush::Page(first)]);
    assert_eq!(fixture.mapper().unmap(second), Ok(frame(0x41)));
    assert_eq!(
        fixture.outstanding(),
        0,
        "the empty tables of all three levels are freed"
    );
    assert_eq!(fixture.mapper().translate(first), None);
}

#[test]
fn a_neighboring_branch_keeps_its_tables_when_another_one_is_emptied() {
    let mut fixture = Fixture::new();
    let low = page(0x1000);
    let high = page(1 << 30);
    fixture
        .mapper()
        .map(
            low,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    fixture
        .mapper()
        .map(
            high,
            frame(0x41),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    assert_eq!(fixture.outstanding(), 5, "the branches share the top table");
    fixture.mapper().unmap(low).unwrap();
    assert_eq!(fixture.outstanding(), 3);
    assert_eq!(
        fixture.mapper().translate(high),
        Some((frame(0x41), Permissions::READ_WRITE))
    );
}

#[test]
fn the_highest_user_page_and_the_lowest_kernel_page_can_be_mapped() {
    let mut fixture = Fixture::new();
    let highest_user = page(USER_SPACE_END - PAGE_SIZE);
    let lowest_kernel = page(KERNEL_SPACE_START);
    assert!(highest_user.is_user() && !lowest_kernel.is_user());
    fixture
        .mapper()
        .map(
            highest_user,
            frame(0x40),
            Permissions::READ_WRITE.for_user(),
            CachePolicy::WriteBack,
        )
        .unwrap();
    fixture
        .mapper()
        .map(
            lowest_kernel,
            frame(0x41),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    assert_eq!(
        fixture.entry_at(highest_user, 0).map(|e| e.has(X86_GLOBAL)),
        Some(false),
        "user pages are not global"
    );
    assert_eq!(
        fixture
            .entry_at(lowest_kernel, 0)
            .map(|e| e.has(X86_GLOBAL)),
        Some(true),
        "kernel pages are global"
    );
    assert!(fixture.mapper().translate(highest_user).is_some());
    assert!(fixture.mapper().translate(lowest_kernel).is_some());
}

#[test]
fn a_range_across_the_canonical_hole_cannot_be_built() {
    let last_user = page(USER_SPACE_END - PAGE_SIZE);
    assert_eq!(
        PageRange::new(last_user, 2).err(),
        Some(kernel_types::Error::CrossesCanonicalHole)
    );
    assert_eq!(
        PageRange::from_addresses(
            VirtAddr::new(0x1000).unwrap(),
            VirtAddr::new(KERNEL_SPACE_START).unwrap()
        )
        .err(),
        Some(kernel_types::Error::CrossesCanonicalHole)
    );
}

#[test]
fn every_permission_combination_reaches_the_entry_untouched() {
    let mut fixture = Fixture::new();
    let combinations = [
        Permissions::READ_ONLY,
        Permissions::READ_WRITE,
        Permissions::READ_EXECUTE,
        Permissions::READ_ONLY.for_user(),
        Permissions::READ_WRITE.for_user(),
        Permissions::READ_EXECUTE.for_user(),
        Permissions {
            write: true,
            execute: true,
            user: true,
        },
    ];
    for (index, perms) in combinations.into_iter().enumerate() {
        let target = page(0x1000 + u64::try_from(index).unwrap() * PAGE_SIZE);
        fixture
            .mapper()
            .map(target, frame(0x50), perms, CachePolicy::WriteBack)
            .unwrap();
        assert_eq!(
            fixture.mapper().translate(target).map(|pair| pair.1),
            Some(perms)
        );
        assert_eq!(
            fixture
                .mapper()
                .translate(target)
                .is_some_and(|pair| pair.1.is_write_execute()),
            perms.is_write_execute(),
            "write and execute appear only where they were asked for"
        );
    }
}

#[test]
fn protect_changes_only_the_flags_and_flushes_once() {
    let mut fixture = Fixture::new();
    let target = page(0x1000);
    fixture
        .mapper()
        .map(
            target,
            frame(0x40),
            Permissions::READ_ONLY,
            CachePolicy::Uncached,
        )
        .unwrap();
    fixture.clear_flushes();
    let before = fixture.entry_at(target, 0).unwrap();
    assert_eq!(
        fixture.mapper().protect(target, Permissions::READ_WRITE),
        Ok(())
    );
    assert_eq!(fixture.flushes(), &[Flush::Page(target)]);
    let after = fixture.entry_at(target, 0).unwrap();
    assert_eq!(after.frame(), before.frame());
    assert_eq!(after.permissions(), Permissions::READ_WRITE);
    assert_eq!(
        after.as_u64() & !(before.as_u64() ^ after.as_u64()),
        before.as_u64() & !(before.as_u64() ^ after.as_u64()),
        "only the permission bits differ"
    );
}

#[test]
fn a_range_operation_stops_at_the_budget_and_can_be_resumed() {
    let mut fixture = Fixture::new();
    let pages = PageRange::new(page(0x1000), 5).unwrap();
    assert_eq!(
        fixture.mapper().map_range(
            pages,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
            2
        ),
        Ok(Progress::Partial(2))
    );
    assert!(fixture.mapper().translate(page(0x2000)).is_some());
    assert_eq!(fixture.mapper().translate(page(0x3000)), None);
    let rest = PageRange::new(page(0x3000), 3).unwrap();
    assert_eq!(
        fixture.mapper().map_range(
            rest,
            frame(0x42),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
            MAX_PAGES_PER_CALL
        ),
        Ok(Progress::Done)
    );
    for index in 0..5u64 {
        assert_eq!(
            fixture
                .mapper()
                .translate(page(0x1000 + index * PAGE_SIZE))
                .map(|pair| pair.0),
            Some(frame(0x40 + index))
        );
    }
    assert_eq!(
        fixture.mapper().unmap_range(pages, 2),
        Ok(Progress::Partial(2))
    );
    assert_eq!(fixture.mapper().translate(page(0x1000)), None);
    assert_eq!(
        fixture.mapper().unmap_range(rest, MAX_PAGES_PER_CALL),
        Ok(Progress::Done)
    );
    assert_eq!(fixture.outstanding(), 0);
}

#[test]
fn an_empty_range_is_done_at_once() {
    let mut fixture = Fixture::new();
    let empty = PageRange::new(page(0x1000), 0).unwrap();
    assert_eq!(
        fixture.mapper().map_range(
            empty,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
            0
        ),
        Ok(Progress::Done)
    );
    assert_eq!(fixture.mapper().unmap_range(empty, 0), Ok(Progress::Done));
    assert!(fixture.flushes().is_empty());
}

#[test]
fn a_budget_of_zero_makes_no_progress() {
    let mut fixture = Fixture::new();
    let pages = PageRange::new(page(0x1000), 2).unwrap();
    assert_eq!(
        fixture.mapper().map_range(
            pages,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
            0
        ),
        Ok(Progress::Partial(0))
    );
    assert!(fixture.flushes().is_empty());
}

#[test]
fn frame_exhaustion_leaves_no_half_created_table_behind() {
    for limit in [0u64, 1, 2] {
        let mut fixture = Fixture::with_limit(Some(limit));
        assert_eq!(
            fixture.mapper().map(
                page(0x1000),
                frame(0x40),
                Permissions::READ_WRITE,
                CachePolicy::WriteBack
            ),
            Err(MapError::OutOfKernelMemory),
            "a limit of {limit} is not enough for three tables"
        );
        assert_eq!(
            fixture.outstanding(),
            0,
            "every frame taken for a table was given back"
        );
        assert!(fixture.flushes().is_empty());
        assert_eq!(fixture.mapper().translate(page(0x1000)), None);
        let table = fixture.access.table(fixture.root).unwrap();
        assert!(table.is_unused(), "the root table is untouched");
    }
    let mut enough = Fixture::with_limit(Some(3));
    assert_eq!(
        enough.mapper().map(
            page(0x1000),
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Ok(())
    );
}

#[test]
fn an_unreachable_table_frame_is_reported() {
    let ram = PhysFrameRange::new(frame(RAM_START), RAM_FRAMES).unwrap();
    let mut access: MemoryFrameAccess<PageTable<X86Entry>> = MemoryFrameAccess::new();
    let mut tlb = RecordingTlb::new();
    let mut frames = CountingFrameSource::new(frame(POOL_START), None);
    assert!(ram.contains(frame(POOL_START)));
    let root = frame(RAM_START);
    let mut mapper = Mapper::new(root, &mut access, &mut tlb, &mut frames);
    assert_eq!(mapper.root(), root);
    assert_eq!(
        mapper.map(
            page(0x1000),
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Err(MapError::UnreachableFrame),
        "the root table is not reachable"
    );
    assert_eq!(mapper.translate(page(0x1000)), None);
}

#[test]
fn an_unreachable_frame_for_a_new_table_is_reported_and_given_back() {
    let mut access: MemoryFrameAccess<PageTable<X86Entry>> = MemoryFrameAccess::new();
    let root = frame(RAM_START);
    access.insert(root, PageTable::default());
    let mut tlb = RecordingTlb::new();
    let mut frames = CountingFrameSource::new(frame(POOL_START), None);
    let mut mapper = Mapper::new(root, &mut access, &mut tlb, &mut frames);
    assert_eq!(
        mapper.map(
            page(0x1000),
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Err(MapError::UnreachableFrame)
    );
    assert_eq!(frames.outstanding(), 0);
    assert!(tlb.flushes().is_empty());
}

#[test]
fn an_entry_with_reserved_bits_stops_the_walk() {
    let mut fixture = Fixture::new();
    let target = page(0x1000);
    let broken = X86Entry::from_raw(X86_PRESENT | (1 << 60));
    let index = X86Entry::index(X86Entry::LEVELS - 1, target);
    fixture
        .access
        .table_mut(fixture.root)
        .unwrap()
        .set_entry(index, broken);
    assert_eq!(
        fixture.mapper().map(
            target,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack
        ),
        Err(MapError::Entry(EntryError::ReservedBits(
            X86_PRESENT | (1 << 60)
        )))
    );
    assert_eq!(fixture.mapper().translate(target), None);
    assert_eq!(broken.as_u64() & X86_RESERVED_MASK, 1 << 60);
}

#[test]
fn the_loader_can_map_one_frame_at_two_addresses_with_different_permissions() {
    let mut fixture = Fixture::new();
    let identity = page(0x40 * PAGE_SIZE);
    let window = page(KERNEL_SPACE_START + 0x40 * PAGE_SIZE);
    fixture
        .mapper()
        .map(
            identity,
            frame(0x40),
            Permissions::READ_EXECUTE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    fixture
        .mapper()
        .map(
            window,
            frame(0x40),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
        )
        .unwrap();
    assert_eq!(
        fixture.mapper().translate(identity),
        Some((frame(0x40), Permissions::READ_EXECUTE))
    );
    assert_eq!(
        fixture.mapper().translate(window),
        Some((frame(0x40), Permissions::READ_WRITE))
    );
}

#[test]
fn a_kernel_image_maps_its_segments_with_their_own_permissions() {
    let mut fixture = Fixture::new();
    let base = KERNEL_SPACE_START + (1 << 30);
    let segments = [
        (0u64, 2u64, Permissions::READ_EXECUTE),
        (2, 1, Permissions::READ_ONLY),
        (3, 3, Permissions::READ_WRITE),
    ];
    for (offset, count, perms) in segments {
        let pages = PageRange::new(page(base + offset * PAGE_SIZE), count).unwrap();
        assert_eq!(
            fixture.mapper().map_range(
                pages,
                frame(0x800 + offset),
                perms,
                CachePolicy::WriteBack,
                MAX_PAGES_PER_CALL
            ),
            Ok(Progress::Done)
        );
    }
    for (offset, count, perms) in segments {
        for index in 0..count {
            assert_eq!(
                fixture
                    .mapper()
                    .translate(page(base + (offset + index) * PAGE_SIZE))
                    .map(|pair| pair.1),
                Some(perms)
            );
        }
    }
}

#[test]
fn errors_render_a_message_and_map_to_the_abi() {
    let cases = [
        (MapError::AlreadyMapped, audhsos_abi::Error::AlreadyMapped),
        (MapError::NotMapped, audhsos_abi::Error::NotMapped),
        (
            MapError::OutOfKernelMemory,
            audhsos_abi::Error::OutOfKernelMemory,
        ),
        (
            MapError::UnreachableFrame,
            audhsos_abi::Error::InvalidArgument,
        ),
        (
            MapError::Entry(EntryError::HugePage),
            audhsos_abi::Error::InvalidArgument,
        ),
    ];
    for (error, expected) in cases {
        assert!(!format!("{error}").is_empty());
        assert_eq!(audhsos_abi::Error::from(error), expected);
    }
    assert_ne!(Progress::Done, Progress::Partial(0));
}

/// One operation of the mapper model test.
#[derive(Clone, Copy, Debug)]
enum Op {
    Map(u64, u64, Permissions),
    Unmap(u64),
    Protect(u64, Permissions),
    Translate(u64),
}

struct MapperModel;

impl ModelTest for MapperModel {
    type Op = Op;
    type Sut = Fixture;
    type Model = HashMap<Page, (PhysFrame, Permissions)>;

    fn generator(&self) -> BoxGen<Op> {
        let addresses = one_of((0..12u64).map(|index| 0x1000 + index * PAGE_SIZE).collect());
        pair(
            range(0u32..=3),
            pair(pair(addresses, range(0x40u64..=0x48)), any_permissions()),
        )
        .map(|(choice, ((address, target), perms))| match choice {
            1 => Op::Unmap(address),
            2 => Op::Protect(address, perms),
            3 => Op::Translate(address),
            _ => Op::Map(address, target, perms),
        })
        .boxed()
    }

    fn new_sut(&self) -> Fixture {
        Fixture::new()
    }

    fn new_model(&self) -> Self::Model {
        HashMap::new()
    }

    fn step(&self, sut: &mut Fixture, model: &mut Self::Model, op: &Op) -> Result<(), String> {
        match *op {
            Op::Map(address, target, perms) => {
                let target_page = page(address);
                let target_frame = frame(target);
                let result =
                    sut.mapper()
                        .map(target_page, target_frame, perms, CachePolicy::WriteBack);
                match (result, model.contains_key(&target_page)) {
                    (Ok(()), false) => {
                        model.insert(target_page, (target_frame, perms));
                    }
                    (Err(MapError::AlreadyMapped), true) => {}
                    (result, mapped) => {
                        return Err(format!("map: {result:?} with mapped={mapped}"));
                    }
                }
            }
            Op::Unmap(address) => {
                let target_page = page(address);
                let expected = model.get(&target_page).copied();
                match (sut.mapper().unmap(target_page), expected) {
                    (Ok(got), Some((want, _))) if got == want => {
                        model.remove(&target_page);
                    }
                    (Err(MapError::NotMapped), None) => {}
                    (result, expected) => {
                        return Err(format!("unmap: {result:?} against {expected:?}"));
                    }
                }
            }
            Op::Protect(address, perms) => {
                let target_page = page(address);
                let expected = model.get(&target_page).copied();
                match (sut.mapper().protect(target_page, perms), expected) {
                    (Ok(()), Some((target_frame, _))) => {
                        model.insert(target_page, (target_frame, perms));
                    }
                    (Err(MapError::NotMapped), None) => {}
                    (result, expected) => {
                        return Err(format!("protect: {result:?} against {expected:?}"));
                    }
                }
            }
            Op::Translate(address) => {
                let target_page = page(address);
                if sut.mapper().translate(target_page) != model.get(&target_page).copied() {
                    return Err("translate disagrees with the model".to_owned());
                }
            }
        }
        for index in 0..12u64 {
            let touched = page(0x1000 + index * PAGE_SIZE);
            if sut.mapper().translate(touched) != model.get(&touched).copied() {
                return Err(format!("{touched:?} disagrees with the model"));
            }
        }
        if model.is_empty() && sut.outstanding() != 0 {
            return Err("tables were kept although nothing is mapped".to_owned());
        }
        Ok(())
    }
}

#[test]
fn model_map_unmap_and_protect_agree_with_a_map_of_pages() {
    run_model_test("mapper_model", &MapperModel, 48);
}
