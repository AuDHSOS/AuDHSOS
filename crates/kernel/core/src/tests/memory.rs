// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::memory`, covering the memory items of the catalog
//! 6.6.21 on the host: the same bring-up runs against the doubles here and
//! against the machine in QEMU.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::boot_image::{BOOT_IMAGE_HEADER_LEN, BootImageHeader};
use audhsos_abi::layout::{
    BOOT_INFO_VADDR, BOOT_STACK_PAGES, BOOT_STACK_TOP, KERNEL_BASE, KERNEL_STACK_PAGES,
    KERNEL_STACK_SLOTS, KERNEL_STACKS_BASE, MAX_PHYS_WINDOW_BYTES, PAGE_SIZE, PHYS_WINDOW_BASE,
    USER_SPACE_END,
};
use kernel_hal_api::doubles::{
    CountingFrameSource, MemoryFrameAccess, RecordingConsole, RecordingTlb, ScriptedPlatform,
};
use kernel_hal_api::platform::MemoryRegionKind;
use kernel_mm::address_space::RegionError;
use kernel_mm::frame_allocator::FrameError;
use kernel_mm::mapper::{MapError, Mapper};
use kernel_mm::memory_map::MapError as MemoryMapError;
use kernel_mm::page_table::{CachePolicy, PageTable, Permissions, X86Entry};
use kernel_types::{Page, PageRange, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

use crate::config::KERNEL_REGIONS;
use crate::memory::{
    Adopted, KernelBacking, MemoryError, adopt, backing_name, boot_image, bring_up, drop_identity,
    identity_range, report, requested_reserve, reserve, reserve_override,
};

const MIB: u64 = 1024 * 1024;

/// The memory of the machine under test.
const RAM_BYTES: u64 = 8 * MIB;

/// The pages the window covers on that machine.
const RAM_PAGES: u64 = RAM_BYTES / PAGE_SIZE;

/// Where the kernel image sits physically, and how long it is.
const KERNEL_PHYS: u64 = 6 * MIB;
const KERNEL_LEN: u64 = 64 * 1024;

/// Where the boot image sits physically, and how long it is.
const IMAGE_PHYS: u64 = 6 * MIB + 512 * 1024;
const IMAGE_LEN: u64 = 2 * PAGE_SIZE;

/// Where the loader's page tables sit physically, and how long they are.
const TABLES_PHYS: u64 = 7 * MIB;
const TABLES_LEN: u64 = 64 * 1024;

/// Where the boot stack sits physically.
const STACK_PHYS: u64 = 7 * MIB + 512 * 1024;

/// Where the boot information page sits physically.
const INFO_PHYS: u64 = 7 * MIB + 768 * 1024;

/// The pages of the kernel image the loader mapped.
const KERNEL_PAGES: u64 = KERNEL_LEN / PAGE_SIZE;

/// The first frame the loader's own tables use, well above the memory of
/// the machine so that such a table frame is never a reserve frame.
const TABLE_START: u64 = 0x1_0000;

/// The number of frames above the machine the loader's own tables use.
const TABLE_FRAMES: u64 = 512;

type Access = MemoryFrameAccess<PageTable<X86Entry>>;
type TestMapper<'a> = Mapper<'a, X86Entry, Access, RecordingTlb, CountingFrameSource>;

fn frame(number: u64) -> PhysFrame {
    PhysFrame::from_number(number).unwrap()
}

fn address(raw: u64) -> PhysAddr {
    PhysAddr::new(raw).unwrap()
}

fn page(raw: u64) -> Page {
    VirtAddr::new(raw).unwrap().page()
}

fn stack_bottom() -> u64 {
    BOOT_STACK_TOP - BOOT_STACK_PAGES * PAGE_SIZE
}

/// The machine the loader would have left behind.
pub(super) fn platform() -> ScriptedPlatform {
    ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap())
        .region(address(0), RAM_BYTES, MemoryRegionKind::Usable)
        .region(address(KERNEL_PHYS), KERNEL_LEN, MemoryRegionKind::Kernel)
        .region(address(IMAGE_PHYS), IMAGE_LEN, MemoryRegionKind::BootImage)
        .region(
            address(TABLES_PHYS),
            TABLES_LEN,
            MemoryRegionKind::PageTables,
        )
        .region(
            address(STACK_PHYS),
            BOOT_STACK_PAGES * PAGE_SIZE,
            MemoryRegionKind::BootStack,
        )
        .region(address(INFO_PHYS), PAGE_SIZE, MemoryRegionKind::BootInfo)
}

/// The tables the loader would have built, and the doubles that reach them.
pub(super) struct Machine {
    pub(super) access: Access,
    tlb: RecordingTlb,
    pub(super) root: PhysFrame,
}

impl Machine {
    fn new(identity_pages: u64) -> Self {
        // The window reaches every frame of the machine, so a table may
        // live in any of them: the frames of the reserve carry the tables
        // a later mapping needs, not only the frames above the machine
        // that this fixture starts the loader's tables in.
        let ram = PhysFrameRange::from_numbers(0, TABLE_START + TABLE_FRAMES).unwrap();
        let mut access = MemoryFrameAccess::with_lazy_tables(ram);
        let root = frame(TABLE_START);
        access.insert(root, PageTable::default());
        let mut tlb = RecordingTlb::new();
        let mut frames = CountingFrameSource::new(frame(TABLE_START + 1), None);
        {
            let mut mapper = TestMapper::new(root, &mut access, &mut tlb, &mut frames);
            map(&mut mapper, 0, identity_pages, 0);
            map(&mut mapper, PHYS_WINDOW_BASE, RAM_PAGES, 0);
            map(
                &mut mapper,
                KERNEL_BASE,
                KERNEL_PAGES,
                KERNEL_PHYS / PAGE_SIZE,
            );
            map(
                &mut mapper,
                stack_bottom(),
                BOOT_STACK_PAGES,
                STACK_PHYS / PAGE_SIZE,
            );
            map(&mut mapper, BOOT_INFO_VADDR, 1, INFO_PHYS / PAGE_SIZE);
        }
        Machine { access, tlb, root }
    }

    pub(super) fn full() -> Self {
        Self::new(RAM_PAGES)
    }

    fn mapper<'a>(&'a mut self, frames: &'a mut CountingFrameSource) -> TestMapper<'a> {
        TestMapper::new(self.root, &mut self.access, &mut self.tlb, frames)
    }
}

/// Maps `count` pages at `base` to the frames starting at `first_frame`.
fn map(mapper: &mut TestMapper<'_>, base: u64, count: u64, first_frame: u64) {
    if count == 0 {
        return;
    }
    let pages = PageRange::new(page(base), count).unwrap();
    mapper
        .map_range(
            pages,
            frame(first_frame),
            Permissions::READ_WRITE,
            CachePolicy::WriteBack,
            u64::MAX,
        )
        .unwrap();
}

/// A frame source for a walk that creates no table.
fn no_frames() -> CountingFrameSource {
    CountingFrameSource::new(frame(TABLE_START + TABLE_FRAMES), Some(0))
}

#[test]
fn the_reserve_is_a_sixteenth_of_the_usable_memory_by_default() {
    let taken = reserve(&platform(), 0).unwrap();
    assert_eq!(taken.window_pages, RAM_PAGES);
    // Eight mebibytes give a sixteenth of half a mebibyte, which the
    // minimum of four mebibytes replaces.
    assert_eq!(taken.frames.capacity(), 4 * MIB / PAGE_SIZE);
    assert_eq!(taken.frames.free_count(), taken.frames.capacity());
    assert!(taken.free.total_bytes() > 0);
    assert!(taken.free.is_sorted_and_disjoint());
}

#[test]
fn an_override_names_the_size_of_the_reserve() {
    let taken = reserve(&platform(), 5 * MIB).unwrap();
    assert_eq!(taken.frames.capacity(), 5 * MIB / PAGE_SIZE);
}

#[test]
fn the_reserve_never_overlaps_the_memory_that_is_left() {
    let taken = reserve(&platform(), 0).unwrap();
    let range = taken.frames.range();
    for free in taken.free.iter() {
        assert!(!free.overlaps(range), "{free:?} overlaps the reserve");
    }
}

#[test]
fn an_unaligned_override_is_refused() {
    let error = reserve(&platform(), 4 * MIB + 1).unwrap_err();
    assert!(matches!(error, MemoryError::Map(_)));
    assert!(!format!("{error}").is_empty());
}

#[test]
fn a_machine_without_usable_memory_is_refused() {
    let empty = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap());
    assert!(matches!(
        reserve(&empty, 0).unwrap_err(),
        MemoryError::Map(MemoryMapError::NoUsableMemory)
    ));
}

#[test]
fn a_machine_with_more_memory_than_the_window_maps_is_refused() {
    let huge = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap()).region(
        address(0),
        MAX_PHYS_WINDOW_BYTES + PAGE_SIZE,
        MemoryRegionKind::Usable,
    );
    let error = reserve(&huge, 0).unwrap_err();
    assert!(matches!(error, MemoryError::WindowTooLarge(_)));
    assert!(format!("{error}").contains("more than the window maps"));
}

#[test]
fn a_device_aperture_does_not_widen_the_window() {
    let with_device = platform().region(
        address(MAX_PHYS_WINDOW_BYTES),
        PAGE_SIZE,
        MemoryRegionKind::MmioReserved,
    );
    assert_eq!(reserve(&with_device, 0).unwrap().window_pages, RAM_PAGES);
}

#[test]
fn a_reserve_larger_than_the_bitmap_is_refused() {
    let big = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap()).region(
        address(0),
        1024 * MIB,
        MemoryRegionKind::Usable,
    );
    let error = reserve(&big, 128 * MIB).unwrap_err();
    assert!(matches!(
        error,
        MemoryError::Frames(FrameError::InvalidCount)
    ));
}

#[test]
fn the_walk_registers_the_four_fixed_ranges_of_the_kernel_half() {
    let mut machine = Machine::full();
    let mut frames = no_frames();
    let mapper = machine.mapper(&mut frames);
    let Adopted { regions, boot_info } = adopt(&mapper, RAM_PAGES).unwrap();
    assert_eq!(regions.len(), 4);
    assert!(regions.check_invariants());
    let found: Vec<_> = regions
        .iter()
        .map(|region| {
            (
                region.backing,
                region.pages.start().start().as_u64(),
                region.pages.count(),
            )
        })
        .collect();
    assert!(found.contains(&(KernelBacking::Image, KERNEL_BASE, KERNEL_PAGES)));
    assert!(found.contains(&(KernelBacking::Window, PHYS_WINDOW_BASE, RAM_PAGES)));
    assert!(found.contains(&(KernelBacking::BootStack, stack_bottom(), BOOT_STACK_PAGES)));
    assert!(found.contains(&(KernelBacking::BootInfo, BOOT_INFO_VADDR, 1)));
    assert_eq!(boot_info, frame(INFO_PHYS / PAGE_SIZE));
}

#[test]
fn a_boot_information_page_without_a_translation_is_reported() {
    let mut machine = Machine::full();
    let mut frames = no_frames();
    {
        let mut mapper = machine.mapper(&mut frames);
        mapper.unmap(page(BOOT_INFO_VADDR)).unwrap();
    }
    let mut frames = no_frames();
    let mapper = machine.mapper(&mut frames);
    let error = adopt(&mapper, RAM_PAGES).unwrap_err();
    assert_eq!(error, MemoryError::NoBootInfo);
    assert!(format!("{error}").contains("nothing is mapped"));
}

#[test]
fn the_bring_up_drops_the_identity_mapping_and_keeps_the_kernel_half() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    assert_eq!(memory.identity_pages(), RAM_PAGES);
    assert_eq!(memory.root(), machine.root);
    assert_eq!(memory.boot_info(), frame(INFO_PHYS / PAGE_SIZE));
    assert_eq!(memory.regions().len(), 4);
    assert_eq!(memory.frames().free_count(), memory.frames().capacity());
    assert!(memory.free().total_bytes() > 0);

    let mut frames = no_frames();
    let mapper = machine.mapper(&mut frames);
    assert!(mapper.translate(page(0x1000)).is_none());
    assert!(mapper.translate(page(0)).is_none());
    assert!(mapper.translate(page(RAM_BYTES - PAGE_SIZE)).is_none());
    assert!(mapper.translate(page(PHYS_WINDOW_BASE)).is_some());
    assert!(mapper.translate(page(KERNEL_BASE)).is_some());
    assert!(mapper.translate(page(BOOT_INFO_VADDR)).is_some());
    assert!(mapper.translate(page(stack_bottom())).is_some());
}

#[test]
fn the_bring_up_gives_no_frame_of_the_identity_mapping_back() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    // Every frame of the reserve is still free: the identity mapping named
    // frames the window maps, and none of them went into the allocator.
    assert_eq!(memory.frames().free_count(), memory.frames().capacity());
}

#[test]
fn dropping_an_identity_mapping_with_holes_removes_what_is_there() {
    let mut machine = Machine::full();
    let mut frames = no_frames();
    {
        let mut mapper = machine.mapper(&mut frames);
        mapper.unmap(page(4 * PAGE_SIZE)).unwrap();
        mapper.unmap(page(5 * PAGE_SIZE)).unwrap();
    }
    let mut frames = no_frames();
    let mut mapper = machine.mapper(&mut frames);
    let pages = identity_range(RAM_PAGES).unwrap();
    let removed = drop_identity(&mut mapper, pages).unwrap();
    assert_eq!(removed, RAM_PAGES - 2);
    for offset in 0..RAM_PAGES {
        assert!(mapper.translate(page(offset * PAGE_SIZE)).is_none());
    }
}

#[test]
fn dropping_an_identity_mapping_that_is_not_there_removes_nothing() {
    let mut machine = Machine::new(0);
    let mut frames = no_frames();
    let mut mapper = machine.mapper(&mut frames);
    let pages = identity_range(RAM_PAGES).unwrap();
    assert_eq!(drop_identity(&mut mapper, pages).unwrap(), 0);
}

#[test]
fn the_identity_range_never_leaves_the_user_half() {
    let bounded = identity_range(u64::MAX).unwrap();
    assert_eq!(bounded.start().start().as_u64(), 0);
    assert_eq!(bounded.count(), USER_SPACE_END / PAGE_SIZE);
    assert_eq!(identity_range(RAM_PAGES).unwrap().count(), RAM_PAGES);
    assert!(identity_range(0).unwrap().is_empty());
}

#[test]
fn the_boot_image_is_found_by_its_kind() {
    assert_eq!(
        boot_image(&platform()),
        Some((address(IMAGE_PHYS), IMAGE_LEN))
    );
    let without = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap());
    assert_eq!(boot_image(&without), None);
}

/// The first bytes of a boot image whose header asks for `bytes`.
fn image_header(bytes: u64) -> [u8; BOOT_IMAGE_HEADER_LEN] {
    BootImageHeader {
        root_task_offset: PAGE_SIZE,
        root_task_len: PAGE_SIZE,
        archive_offset: PAGE_SIZE,
        archive_len: 0,
        kernel_reserve_size: bytes,
    }
    .to_bytes()
}

#[test]
fn the_header_of_the_boot_image_names_the_reserve_the_kernel_takes() {
    let header = image_header(6 * MIB);
    assert_eq!(
        reserve_override(&header, IMAGE_LEN, RAM_BYTES),
        Some(6 * MIB)
    );
    assert_eq!(requested_reserve(&platform(), &header), 6 * MIB);
}

#[test]
fn a_boot_image_header_the_kernel_cannot_read_asks_for_the_default() {
    assert_eq!(reserve_override(&[0u8; 8], IMAGE_LEN, RAM_BYTES), None);
    assert_eq!(requested_reserve(&platform(), &[0u8; 8]), 0);
    let without = ScriptedPlatform::new(VirtAddr::new(PHYS_WINDOW_BASE).unwrap());
    assert_eq!(requested_reserve(&without, &image_header(6 * MIB)), 0);
}

#[test]
fn a_header_that_asks_for_nothing_leaves_the_default() {
    let header = image_header(0);
    assert_eq!(reserve_override(&header, IMAGE_LEN, RAM_BYTES), Some(0));
    assert_eq!(requested_reserve(&platform(), &header), 0);
}

#[test]
fn the_report_names_the_reserve_every_region_and_the_boot_information() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let mut console = RecordingConsole::new();
    report(&memory, &mut console);
    let text = console.text();
    assert!(text.contains("reserve"), "{text}");
    assert!(
        text.contains("identity mapping dropped, 2048 pages"),
        "{text}"
    );
    assert!(text.contains("4 kernel regions"), "{text}");
    for name in ["kernel-image", "physical-window", "boot-stack", "boot-info"] {
        assert!(text.contains(name), "{text} has no {name}");
    }
    assert!(text.contains("boot information page at"), "{text}");
}

#[test]
fn every_backing_has_a_name() {
    let backings = [
        KernelBacking::Image,
        KernelBacking::Window,
        KernelBacking::BootStack,
        KernelBacking::BootInfo,
    ];
    for backing in backings {
        assert!(!backing_name(backing).is_empty());
        assert!(!format!("{backing:?}").is_empty());
    }
}

#[test]
fn every_error_reads_as_a_sentence() {
    let errors = [
        MemoryError::Map(MemoryMapError::NoUsableMemory),
        MemoryError::Frames(FrameError::OutOfFrames),
        MemoryError::Region(RegionError::QuotaExceeded),
        MemoryError::Paging(MapError::NotMapped),
        MemoryError::Address,
        MemoryError::WindowTooLarge(1),
        MemoryError::NoBootInfo,
        MemoryError::AlreadyDone,
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
    assert_eq!(
        MemoryError::from(MemoryMapError::NoUsableMemory),
        MemoryError::Map(MemoryMapError::NoUsableMemory)
    );
    assert_eq!(
        MemoryError::from(FrameError::OutOfFrames),
        MemoryError::Frames(FrameError::OutOfFrames)
    );
    assert_eq!(
        MemoryError::from(RegionError::NotMapped),
        MemoryError::Region(RegionError::NotMapped)
    );
    assert_eq!(
        MemoryError::from(MapError::AlreadyMapped),
        MemoryError::Paging(MapError::AlreadyMapped)
    );
}

#[test]
fn the_kernel_memory_is_reachable_once_the_bring_up_has_stored_it() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    assert!(crate::memory::with_memory(|_| ()).is_none());
    crate::memory::initialize::<X86Entry, _, _, _>(
        &platform(),
        machine.root,
        &mut machine.access,
        &mut tlb,
        0,
    )
    .unwrap();
    let regions = crate::memory::with_memory(|memory| {
        assert_eq!(memory.stacks_mut().live(), 0);
        assert_eq!(memory.regions_mut().len(), 4);
        assert!(memory.frames_mut().allocate().is_ok());
        memory.regions().len()
    });
    assert_eq!(regions, Some(4));

    let mut second = Machine::full();
    let mut tlb = RecordingTlb::new();
    assert_eq!(
        crate::memory::initialize::<X86Entry, _, _, _>(
            &platform(),
            second.root,
            &mut second.access,
            &mut tlb,
            0,
        ),
        Err(MemoryError::AlreadyDone)
    );
}

#[test]
fn a_kernel_stack_comes_out_of_the_reserve_and_goes_back_into_it() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let mut memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let free = memory.frames().free_count();

    let stack = memory
        .allocate_stack::<X86Entry, _, _>(&mut machine.access, &mut tlb)
        .unwrap();
    assert_eq!(stack.pages().count(), KERNEL_STACK_PAGES);
    assert_eq!(stack.guard().unwrap().start().as_u64(), KERNEL_STACKS_BASE);
    // Four pages plus the tables the walk had to create.
    assert!(memory.frames().free_count() <= free - KERNEL_STACK_PAGES);
    assert_eq!(memory.stacks_mut().live(), 1);

    let mut frames = no_frames();
    {
        let mapper = machine.mapper(&mut frames);
        for page in stack.pages() {
            assert!(mapper.translate(page).is_some(), "{page:?} is mapped");
        }
        assert!(mapper.translate(stack.guard().unwrap()).is_none());
    }

    memory
        .release_stack::<X86Entry, _, _>(&mut machine.access, &mut tlb, stack)
        .unwrap();
    assert_eq!(memory.stacks_mut().live(), 0);
    assert_eq!(memory.frames().free_count(), free);
    let mut frames = no_frames();
    let mapper = machine.mapper(&mut frames);
    for page in stack.pages() {
        assert!(mapper.translate(page).is_none(), "{page:?} is unmapped");
    }
}

#[test]
fn a_kernel_stack_is_not_a_region_of_the_region_table() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let mut memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let before = memory.regions().len();
    memory
        .allocate_stack::<X86Entry, _, _>(&mut machine.access, &mut tlb)
        .unwrap();
    // The area is one constant range and the pool says which slots are
    // taken; a row per stack would need more rows than the table has.
    assert_eq!(memory.regions().len(), before);
    assert!(u64::try_from(KERNEL_REGIONS).unwrap() < KERNEL_STACK_SLOTS);
}

/// The address the machine puts a local APIC at, which is above the memory
/// the window covers.
const APIC_PHYS: u64 = 0xFEE0_0000;

#[test]
fn a_device_window_is_mapped_uncached_where_the_physical_window_would_put_it() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let mut memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let before = memory.regions().len();
    let frames = PhysFrameRange::new(frame(APIC_PHYS / PAGE_SIZE), 1).unwrap();
    let base = memory
        .map_device::<X86Entry, _, _>(&mut machine.access, &mut tlb, frames)
        .unwrap();
    assert_eq!(base.as_u64(), PHYS_WINDOW_BASE + APIC_PHYS);
    assert_eq!(memory.regions().len(), before + 1);

    let mut sources = no_frames();
    let mapper = machine.mapper(&mut sources);
    let (aperture, perms) = mapper.translate(page(base.as_u64())).unwrap();
    assert_eq!(aperture, frame(APIC_PHYS / PAGE_SIZE));
    assert_eq!(perms, Permissions::READ_WRITE);
}

#[test]
fn a_device_window_of_two_frames_maps_both_of_them() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let mut memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let frames = PhysFrameRange::new(frame(APIC_PHYS / PAGE_SIZE), 2).unwrap();
    let base = memory
        .map_device::<X86Entry, _, _>(&mut machine.access, &mut tlb, frames)
        .unwrap();
    let mut sources = no_frames();
    let mapper = machine.mapper(&mut sources);
    assert!(mapper.translate(page(base.as_u64())).is_some());
    assert!(mapper.translate(page(base.as_u64() + PAGE_SIZE)).is_some());
}

#[test]
fn a_device_window_that_is_already_mapped_is_refused() {
    let mut machine = Machine::full();
    let mut tlb = RecordingTlb::new();
    let mut memory =
        bring_up::<X86Entry, _, _, _>(&platform(), machine.root, &mut machine.access, &mut tlb, 0)
            .unwrap();
    let frames = PhysFrameRange::new(frame(APIC_PHYS / PAGE_SIZE), 1).unwrap();
    memory
        .map_device::<X86Entry, _, _>(&mut machine.access, &mut tlb, frames)
        .unwrap();
    assert_eq!(
        memory.map_device::<X86Entry, _, _>(&mut machine.access, &mut tlb, frames),
        Err(MemoryError::Paging(MapError::AlreadyMapped))
    );
}

#[test]
fn a_device_window_is_reported_under_its_own_name() {
    assert_eq!(backing_name(KernelBacking::Device), "device");
}
