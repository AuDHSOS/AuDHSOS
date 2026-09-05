// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::stack`.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::{
    KERNEL_STACK_PAGES, KERNEL_STACK_SLOT_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE,
};
use kernel_hal_api::doubles::{CountingFrameSource, MemoryFrameAccess, RecordingTlb};
use kernel_types::{PhysFrame, PhysFrameRange};

use crate::mapper::Mapper;
use crate::page_table::{PageTable, X86Entry};
use crate::stack::{KernelStack, StackError, StackPool, slot_pages};

/// First frame of the memory the double materializes tables in.
const RAM_START: u64 = 0x100;

/// Number of frames of that memory.
const RAM_FRAMES: u64 = 4096;

/// Number of bitmap words of the pool under test: two slots per word is
/// enough to exhaust it in a test.
const WORDS: usize = 1;

/// Number of slots such a pool manages.
const SLOTS: u32 = 64;

type Access = MemoryFrameAccess<PageTable<X86Entry>>;

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

/// A mapper with its three doubles.
struct Fixture {
    access: Access,
    tlb: RecordingTlb,
    frames: CountingFrameSource,
    root: PhysFrame,
}

impl Fixture {
    fn with_limit(limit: Option<u64>) -> Self {
        let ram = PhysFrameRange::new(frame(RAM_START), RAM_FRAMES).unwrap();
        let mut access = MemoryFrameAccess::with_lazy_tables(ram);
        let root = frame(RAM_START);
        access.insert(root, PageTable::default());
        Fixture {
            access,
            tlb: RecordingTlb::new(),
            frames: CountingFrameSource::new(frame(RAM_START + 1), limit),
            root,
        }
    }

    fn new() -> Self {
        Self::with_limit(None)
    }

    fn mapper(&mut self) -> Mapper<'_, X86Entry, Access, RecordingTlb, CountingFrameSource> {
        Mapper::new(self.root, &mut self.access, &mut self.tlb, &mut self.frames)
    }
}

fn pool() -> StackPool<WORDS> {
    StackPool::new()
}

#[test]
fn a_fresh_pool_manages_its_slots_and_hands_none_out() {
    let pool = pool();
    assert_eq!(pool.capacity(), SLOTS);
    assert_eq!(pool.live(), 0);
    assert!(pool.is_empty());
    assert!(!pool.is_used(0));
    assert_eq!(StackPool::<WORDS>::default().live(), 0);
}

#[test]
fn slot_geometry_leaves_a_guard_page_below_every_stack() {
    let first = slot_pages(0, SLOTS).unwrap();
    assert_eq!(first.count(), KERNEL_STACK_PAGES);
    assert_eq!(
        first.start().start().as_u64(),
        KERNEL_STACKS_BASE + PAGE_SIZE
    );
    let second = slot_pages(1, SLOTS).unwrap();
    assert_eq!(
        second.start().start().as_u64() - first.start().start().as_u64(),
        KERNEL_STACK_SLOT_PAGES * PAGE_SIZE
    );
    let gap = second.start().number() - (first.start().number() + first.count());
    assert_eq!(gap, 1, "one guard page between two stacks");
}

#[test]
fn a_slot_at_or_above_the_capacity_is_not_allocated() {
    assert_eq!(slot_pages(SLOTS, SLOTS), Err(StackError::NotAllocated));
    assert_eq!(slot_pages(0, 0), Err(StackError::NotAllocated));
}

#[test]
fn an_allocated_stack_is_mapped_and_names_its_top_and_its_guard_page() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let stack = pool.allocate(&mut fixture.mapper()).unwrap();
    assert_eq!(stack.index(), 0);
    assert_eq!(stack.pages(), slot_pages(0, SLOTS).unwrap());
    assert_eq!(
        stack.top().unwrap().as_u64(),
        stack.pages().start().start().as_u64() + KERNEL_STACK_PAGES * PAGE_SIZE
    );
    assert_eq!(stack.guard().unwrap().start().as_u64(), KERNEL_STACKS_BASE);
    assert!(pool.is_used(0));
    assert_eq!(pool.live(), 1);
    let mapper = fixture.mapper();
    for page in stack.pages() {
        assert!(mapper.translate(page).is_some(), "{page:?} is mapped");
    }
    assert!(
        mapper.translate(stack.guard().unwrap()).is_none(),
        "the guard page stays unmapped"
    );
}

#[test]
fn two_stacks_take_two_slots_and_share_no_page() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let first = pool.allocate(&mut fixture.mapper()).unwrap();
    let second = pool.allocate(&mut fixture.mapper()).unwrap();
    assert_eq!((first.index(), second.index()), (0, 1));
    assert!(!first.pages().overlaps(second.pages()));
    assert_eq!(pool.live(), 2);
}

#[test]
fn a_released_stack_frees_its_slot_and_its_pages() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let stack = pool.allocate(&mut fixture.mapper()).unwrap();
    let outstanding = fixture.frames.outstanding();
    pool.release(&mut fixture.mapper(), stack).unwrap();
    assert!(!pool.is_used(0));
    assert!(pool.is_empty());
    assert!(fixture.frames.outstanding() < outstanding);
    let mapper = fixture.mapper();
    for page in stack.pages() {
        assert!(mapper.translate(page).is_none(), "{page:?} is unmapped");
    }
}

#[test]
fn the_slot_of_a_released_stack_is_handed_out_again() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let first = pool.allocate(&mut fixture.mapper()).unwrap();
    pool.release(&mut fixture.mapper(), first).unwrap();
    let again = pool.allocate(&mut fixture.mapper()).unwrap();
    assert_eq!(again.index(), first.index());
    assert_eq!(again.pages(), first.pages());
}

#[test]
fn releasing_a_stack_twice_is_refused() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let stack = pool.allocate(&mut fixture.mapper()).unwrap();
    pool.release(&mut fixture.mapper(), stack).unwrap();
    assert_eq!(
        pool.release(&mut fixture.mapper(), stack),
        Err(StackError::NotAllocated)
    );
}

#[test]
fn a_pool_without_a_free_slot_is_exhausted() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    for _ in 0..SLOTS {
        pool.allocate(&mut fixture.mapper()).unwrap();
    }
    assert_eq!(pool.live(), SLOTS);
    assert_eq!(
        pool.allocate(&mut fixture.mapper()).unwrap_err(),
        StackError::Exhausted
    );
}

#[test]
fn an_allocation_without_frames_leaves_no_slot_taken_and_no_page_mapped() {
    // Two frames are enough for the tables of the first page but not for
    // the four pages of a stack.
    let mut fixture = Fixture::with_limit(Some(4));
    let mut pool = pool();
    assert_eq!(
        pool.allocate(&mut fixture.mapper()).unwrap_err(),
        StackError::OutOfKernelMemory
    );
    assert!(pool.is_empty());
    assert!(!pool.is_used(0));
    let pages = slot_pages(0, SLOTS).unwrap();
    let mapper = fixture.mapper();
    for page in pages {
        assert!(mapper.translate(page).is_none(), "{page:?} stays unmapped");
    }
}

#[test]
fn every_error_reads_as_a_sentence_and_maps_to_an_abi_error() {
    let errors = [
        StackError::Exhausted,
        StackError::NotAllocated,
        StackError::OutOfKernelMemory,
        StackError::Address,
        StackError::Map(crate::mapper::MapError::NotMapped),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
        let _: audhsos_abi::Error = error.into();
    }
    assert_eq!(
        audhsos_abi::Error::from(StackError::Exhausted),
        audhsos_abi::Error::QuotaExceeded
    );
    assert_eq!(
        audhsos_abi::Error::from(StackError::NotAllocated),
        audhsos_abi::Error::NotMapped
    );
    assert_eq!(
        audhsos_abi::Error::from(StackError::OutOfKernelMemory),
        audhsos_abi::Error::OutOfKernelMemory
    );
    assert_eq!(
        audhsos_abi::Error::from(StackError::Address),
        audhsos_abi::Error::InvalidArgument
    );
    assert_eq!(
        StackError::from(crate::mapper::MapError::AlreadyMapped),
        StackError::Map(crate::mapper::MapError::AlreadyMapped)
    );
}

#[test]
fn a_stack_is_copied_and_compared_by_value() {
    let mut fixture = Fixture::new();
    let mut pool = pool();
    let stack = pool.allocate(&mut fixture.mapper()).unwrap();
    let copy: KernelStack = stack;
    assert_eq!(copy, stack);
    assert!(!format!("{stack:?}").is_empty());
}
