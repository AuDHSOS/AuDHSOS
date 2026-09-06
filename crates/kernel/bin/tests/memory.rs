// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The kernel owns its memory after boot: the reserve hands out and takes
//! back every frame it holds, the loader's identity mapping is gone, the
//! boot information page is reported with the physical address the walk of
//! the loader's tables gives, and a frame the kernel maps itself is
//! written through the window and read through the mapping.
//!
//! That the mapping stops working once it is removed is a fault, which
//! ends the machine, so it has an image of its own in `memory_fault.rs`.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The user test image uses these; this one does not.
use audhsos_sync as _;
use kernel_ipc as _;
use kernel_objects as _;
use kernel_syscall as _;

use audhsos_abi::layout::{BOOT_INFO_VADDR, ROOT_TASK_BASE};
use kernel_core::memory;
use kernel_hal_api::paging::FrameSource;
use kernel_hal_api::platform::{MemoryRegionKind, Platform};
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_mm::mapper::Mapper;
use kernel_mm::page_table::{CachePolicy, Permissions};
use kernel_types::{Page, PhysFrame, VirtAddr};

kernel_hal_x86_64::test_kernel!();

/// The byte the test writes through the window and reads back through the
/// mapping.
const PATTERN: u8 = 0xA5;

/// The user address the test maps a frame at.
const PROBE: u64 = ROOT_TASK_BASE;

/// The address the loader's identity mapping covered and nothing covers
/// afterwards.
const IDENTITY_PROBE: u64 = 0x1000;

/// The page starting at `address`, or a failed test.
fn page_of(address: u64) -> Page {
    match VirtAddr::new(address).and_then(Page::from_start) {
        Ok(page) => page,
        Err(_) => testing::fail(format_args!("{address:#x} is not a page start")),
    }
}

/// The frame the tables are rooted in, or a failed test.
fn root() -> PhysFrame {
    match active_root() {
        Ok(root) => root,
        Err(_) => testing::fail(format_args!("the page table root is not addressable")),
    }
}

/// A window over physical memory.
///
/// # Safety
///
/// The tables the loader built are active for the whole run of a test
/// image, so the window maps every physical frame read and write, and the
/// image is the only writer.
fn window() -> PhysicalWindow {
    // SAFETY: the loader's tables stay active and the image runs on one
    // processor with nothing else touching kernel memory.
    unsafe { PhysicalWindow::kernel() }
}

/// Runs the memory bring-up once. A second call finds it done and returns.
fn bring_up() {
    if memory::with_memory(|_| ()).is_some() {
        return;
    }
    let mut access = window();
    let mut tlb = LocalTlb::new();
    let outcome = testing::with_platform(|platform| {
        memory::initialize::<X86Entry, _, _, _>(platform, root(), &mut access, &mut tlb, 0)
    });
    match outcome {
        Some(Ok(())) => {}
        Some(Err(error)) => testing::fail(format_args!("the memory bring-up failed: {error}")),
        None => testing::fail(format_args!("the platform is not reachable")),
    }
}

/// The bring-up takes a reserve and hands the allocator over it back with
/// every frame free.
#[test_case]
fn the_bring_up_leaves_every_reserve_frame_free() {
    bring_up();
    let reported = memory::with_memory(|kernel| {
        let frames = kernel.frames();
        (frames.capacity(), frames.free_count())
    });
    let Some((capacity, free)) = reported else {
        testing::fail(format_args!("the kernel memory is not reachable"));
    };
    if capacity == 0 {
        testing::fail(format_args!("the reserve holds no frame"));
    }
    if free != capacity {
        testing::fail(format_args!(
            "{free} of {capacity} reserve frames are free, not all of them"
        ));
    }
}

/// Every frame of the reserve is handed out once and taken back.
#[test_case]
fn every_reserve_frame_is_handed_out_and_taken_back() {
    bring_up();
    let outcome = memory::with_memory(|kernel| {
        let frames = kernel.frames_mut();
        let capacity = frames.capacity();
        let mut taken = 0u64;
        while frames.allocate().is_ok() {
            taken = taken.saturating_add(1);
        }
        let range = frames.range();
        for frame in range {
            let _ = frames.free(frame);
        }
        (capacity, taken, frames.free_count())
    });
    let Some((capacity, taken, free)) = outcome else {
        testing::fail(format_args!("the kernel memory is not reachable"));
    };
    if taken != capacity {
        testing::fail(format_args!(
            "{taken} frames came out of a reserve of {capacity}"
        ));
    }
    if free != capacity {
        testing::fail(format_args!(
            "{free} of {capacity} frames are free after every one was given back"
        ));
    }
}

/// Nothing is mapped at the addresses the loader's identity mapping
/// covered.
#[test_case]
fn the_identity_mapping_is_gone() {
    bring_up();
    let mut access = window();
    let mut tlb = LocalTlb::new();
    let outcome = memory::with_memory(|kernel| {
        let root = kernel.root();
        let mapper =
            Mapper::<'_, X86Entry, _, _, _>::new(root, &mut access, &mut tlb, kernel.frames_mut());
        mapper.translate(page_of(IDENTITY_PROBE)).is_some()
    });
    match outcome {
        Some(false) => {}
        Some(true) => testing::fail(format_args!("{IDENTITY_PROBE:#x} is still mapped")),
        None => testing::fail(format_args!("the kernel memory is not reachable")),
    }
}

/// The boot information page is reported as a region of its own, with the
/// physical address the walk of the loader's tables gives.
#[test_case]
fn the_boot_information_page_is_reported_with_its_physical_address() {
    bring_up();
    let Some(expected) = memory::with_memory(|kernel| kernel.boot_info()) else {
        testing::fail(format_args!("the kernel memory is not reachable"));
    };
    let reported = testing::with_platform(|platform| {
        let mut found = None;
        let mut count = 0u32;
        for region in platform.memory_regions() {
            if region.kind == MemoryRegionKind::BootInfo {
                found = Some(region.start);
                count = count.saturating_add(1);
            }
        }
        (found, count)
    });
    let Some((found, count)) = reported else {
        testing::fail(format_args!("the platform is not reachable"));
    };
    if count != 1 {
        testing::fail(format_args!("{count} boot information regions, not one"));
    }
    if found != Some(expected.start()) {
        testing::fail(format_args!(
            "the boot information region is not at {}",
            expected.start()
        ));
    }
    let translated = {
        // SAFETY: the loader's tables are active, so the window maps every
        // physical frame read and write.
        unsafe { kernel_hal_x86_64::memory::translate(BOOT_INFO_VADDR) }
    };
    if translated != Some(expected) {
        testing::fail(format_args!(
            "the walk of {BOOT_INFO_VADDR:#x} names another frame"
        ));
    }
}

/// A frame the kernel maps itself is written through the window and read
/// back through the mapping.
#[test_case]
fn a_frame_mapped_by_the_kernel_carries_what_the_window_wrote() {
    bring_up();
    let page = page_of(PROBE);
    let mut access = window();
    let mut tlb = LocalTlb::new();
    let mapped = memory::with_memory(|kernel| {
        let root = kernel.root();
        let mut mapper =
            Mapper::<'_, X86Entry, _, _, _>::new(root, &mut access, &mut tlb, kernel.frames_mut());
        let frame = mapper.frames_mut().allocate().ok()?;
        mapper
            .map(page, frame, Permissions::READ_WRITE, CachePolicy::WriteBack)
            .ok()?;
        Some(frame)
    });
    let Some(Some(frame)) = mapped else {
        testing::fail(format_args!("the frame could not be mapped at {PROBE:#x}"));
    };

    let mut bytes = window();
    match bytes.frame_bytes_mut(frame) {
        Some(page_bytes) => {
            if let Some(slot) = page_bytes.first_mut() {
                *slot = PATTERN;
            }
        }
        None => testing::fail(format_args!(
            "the frame is not reachable through the window"
        )),
    }

    let read = testing::read_byte(PROBE);
    if read != PATTERN {
        testing::fail(format_args!(
            "the mapping reads {read:#x}, not {PATTERN:#x}"
        ));
    }

    let removed = memory::with_memory(|kernel| {
        let root = kernel.root();
        let mut mapper =
            Mapper::<'_, X86Entry, _, _, _>::new(root, &mut access, &mut tlb, kernel.frames_mut());
        match mapper.unmap(page) {
            Ok(frame) => {
                mapper.frames_mut().release_frame(frame);
                true
            }
            Err(_) => false,
        }
    });
    if removed != Some(true) {
        testing::fail(format_args!(
            "the mapping at {PROBE:#x} could not be removed"
        ));
    }
}
