// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A mapping the kernel removes stops working: the read after the unmap
//! raises a page fault that names the address. The fault ends the machine,
//! which is why this is an image of its own.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::layout::ROOT_TASK_BASE;
use kernel_core::memory;
use kernel_hal_api::paging::FrameSource;
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_mm::mapper::Mapper;
use kernel_mm::page_table::{CachePolicy, Permissions};
use kernel_types::{Page, PhysFrame, VirtAddr};

kernel_hal_x86_64::test_kernel!();

/// The vector a missing translation raises.
const PAGE_FAULT: u8 = 14;

/// The user address the test maps a frame at and then unmaps.
const PROBE: u64 = ROOT_TASK_BASE;

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

/// A window over physical memory. The tables the loader built stay active
/// for the whole run of a test image, so the window maps every physical
/// frame read and write, and the image is the only writer.
fn window() -> PhysicalWindow {
    // SAFETY: the loader's tables stay active and the image runs on one
    // processor with nothing else touching kernel memory.
    unsafe { PhysicalWindow::kernel() }
}

/// Runs the memory bring-up, without which nothing is mapped or unmapped.
fn bring_up() {
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

/// What the test expects the processor to report.
fn on_page_fault(report: TrapReport) -> ! {
    if report.vector != PAGE_FAULT {
        testing::fail(format_args!(
            "trap {} instead of a page fault",
            report.vector
        ));
    }
    if report.fault_address != PROBE {
        testing::fail(format_args!(
            "the page fault names {:#x}, not {PROBE:#x}",
            report.fault_address
        ));
    }
    testing::pass();
    testing::finish()
}

/// The read after the unmap faults at the address the mapping had.
#[test_case]
fn a_read_after_the_unmap_faults_at_the_address_of_the_mapping() {
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
        Some(())
    });
    if mapped != Some(Some(())) {
        testing::fail(format_args!("the frame could not be mapped at {PROBE:#x}"));
    }
    let _ = testing::read_byte(PROBE);

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

    testing::set_trap_hook(|report| on_page_fault(report));
    let _ = testing::read_byte(PROBE);
    testing::fail(format_args!(
        "the read from {PROBE:#x} did not fault after the unmap"
    ));
}
