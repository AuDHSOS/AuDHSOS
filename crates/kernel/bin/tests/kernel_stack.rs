// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A kernel stack is real memory and its guard page is a real hole: every
//! page of an allocated stack carries what the kernel writes into it, a
//! released stack is gone, the slot comes back, and a write below the
//! lowest page of a stack raises a page fault at the guard address.
//!
//! The host tests of `kernel-mm::stack` show the bookkeeping against a
//! table of pages; only the machine shows that the guard page faults. The
//! fault ends the machine, so this image runs one test that does the whole
//! sequence in order.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The memory test image uses this crate; this one does not.
use kernel_hal_api as _;

use audhsos_abi::layout::{KERNEL_STACK_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE};
use kernel_core::memory;
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::traps::TrapReport;
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_mm::mapper::Mapper;
use kernel_mm::stack::KernelStack;
use kernel_types::PhysFrame;

kernel_hal_x86_64::test_kernel!();

/// The vector a missing translation raises.
const PAGE_FAULT: u8 = 14;

/// The byte the test writes into every page of the stack.
const PATTERN: u8 = 0x5A;

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

/// Runs the memory bring-up, without which there is no reserve to take a
/// stack out of.
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

/// Takes a stack out of the pool, or fails the test.
fn allocate() -> KernelStack {
    let mut access = window();
    let mut tlb = LocalTlb::new();
    let outcome = memory::with_memory(|kernel| {
        kernel.allocate_stack::<X86Entry, _, _>(&mut access, &mut tlb)
    });
    match outcome {
        Some(Ok(stack)) => stack,
        Some(Err(error)) => testing::fail(format_args!("no kernel stack: {error}")),
        None => testing::fail(format_args!("the kernel memory is not reachable")),
    }
}

/// Gives a stack back, or fails the test.
fn release(stack: KernelStack) {
    let mut access = window();
    let mut tlb = LocalTlb::new();
    let outcome = memory::with_memory(|kernel| {
        kernel.release_stack::<X86Entry, _, _>(&mut access, &mut tlb, stack)
    });
    match outcome {
        Some(Ok(())) => {}
        Some(Err(error)) => testing::fail(format_args!("the stack stayed: {error}")),
        None => testing::fail(format_args!("the kernel memory is not reachable")),
    }
}

/// `true` if every page of `stack` has a translation.
fn is_mapped(stack: KernelStack) -> bool {
    let mut access = window();
    let mut tlb = LocalTlb::new();
    memory::with_memory(|kernel| {
        let root = kernel.root();
        let mapper =
            Mapper::<'_, X86Entry, _, _, _>::new(root, &mut access, &mut tlb, kernel.frames_mut());
        stack
            .pages()
            .into_iter()
            .all(|page| mapper.translate(page).is_some())
    })
    .unwrap_or(false)
}

/// The address of the guard page of `stack`, or a failed test.
fn guard_address(stack: KernelStack) -> u64 {
    match stack.guard() {
        Some(page) => page.start().as_u64(),
        None => testing::fail(format_args!("the stack has no guard page")),
    }
}

/// What the test expects the processor to report for the guard page. The
/// test takes the first slot of the area, whose guard page is the base of
/// the area, so the hook needs no state of its own.
fn on_guard_fault(report: TrapReport) -> ! {
    let guard = KERNEL_STACKS_BASE;
    if report.vector != PAGE_FAULT {
        testing::fail(format_args!(
            "trap {} instead of a page fault",
            report.vector
        ));
    }
    if report.fault_address != guard {
        testing::fail(format_args!(
            "the page fault names {:#x}, not the guard page {guard:#x}",
            report.fault_address
        ));
    }
    testing::pass();
    testing::finish()
}

/// The whole life of a kernel stack, ending in the fault its guard page
/// raises.
#[test_case]
fn a_kernel_stack_carries_writes_and_its_guard_page_faults() {
    bring_up();

    let stack = allocate();
    if stack.pages().count() != KERNEL_STACK_PAGES {
        testing::fail(format_args!(
            "the stack holds {} pages, not {KERNEL_STACK_PAGES}",
            stack.pages().count()
        ));
    }
    let Some(top) = stack.top() else {
        testing::fail(format_args!("the stack has no top"));
    };
    let lowest = stack.pages().start().start().as_u64();
    if top.as_u64() != lowest.wrapping_add(KERNEL_STACK_PAGES.wrapping_mul(PAGE_SIZE)) {
        testing::fail(format_args!("the top of the stack is not above its pages"));
    }
    if !is_mapped(stack) {
        testing::fail(format_args!("the stack is not mapped"));
    }

    // The first and the last byte of every page, so that a mapping that
    // covers only part of the stack is caught.
    for page in stack.pages() {
        let first = page.start().as_u64();
        let last = page.last_address().as_u64();
        testing::write_byte(first, PATTERN);
        testing::write_byte(last, PATTERN);
        let (read_first, read_last) = (testing::read_byte(first), testing::read_byte(last));
        if read_first != PATTERN || read_last != PATTERN {
            testing::fail(format_args!(
                "{first:#x} reads {read_first:#x} and {last:#x} reads {read_last:#x}"
            ));
        }
    }

    let guard = guard_address(stack);
    if guard != lowest.wrapping_sub(PAGE_SIZE) || guard != KERNEL_STACKS_BASE {
        testing::fail(format_args!(
            "the guard page {guard:#x} is not below the stack at {lowest:#x}"
        ));
    }

    release(stack);
    if is_mapped(stack) {
        testing::fail(format_args!("the released stack is still mapped"));
    }

    // The slot comes back, and with it the same pages.
    let again = allocate();
    if again.index() != stack.index() || again.pages() != stack.pages() {
        testing::fail(format_args!("the released slot was not handed out again"));
    }
    if !is_mapped(again) {
        testing::fail(format_args!("the second stack is not mapped"));
    }

    testing::set_trap_hook(|report| on_guard_fault(report));
    testing::write_byte(guard, PATTERN);
    testing::fail(format_args!(
        "the write to the guard page {guard:#x} did not fault"
    ));
}
