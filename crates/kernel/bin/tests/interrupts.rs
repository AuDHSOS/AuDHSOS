// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The interrupt hardware of the machine: the tables say where it is, the
//! timer delivers, the end-of-interrupt lets the next one through, a
//! masked timer delivers nothing, and a vector raised from software
//! reaches the handler the plan gives it.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

// The memory test image uses this crate; the other test images do not.
use kernel_mm as _;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use audhsos_abi::layout::TICKS_PER_SECOND;
use kernel_core::memory;
use kernel_hal_api::interrupt::{InterruptController, InterruptLine, Vector};
use kernel_hal_x86_64::bootinfo::X86Platform;
use kernel_hal_x86_64::interrupts;
use kernel_hal_x86_64::paging::{LocalTlb, X86Entry, active_root};
use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::window::PhysicalWindow;
use kernel_hal_x86_64::{instructions, traps, vectors};
use kernel_types::{PhysFrame, PhysFrameRange, VirtAddr};

kernel_hal_x86_64::test_kernel!();

/// The value of [`LAST_VECTOR`] before any interrupt has arrived. No
/// vector is that wide, so it can never be one.
const NO_VECTOR: u32 = u32::MAX;

/// The vector of the interrupt that arrived last.
static LAST_VECTOR: AtomicU32 = AtomicU32::new(NO_VECTOR);

/// Set while the image raises a vector itself, so that the handler does
/// not acknowledge an interrupt the hardware never delivered.
static FROM_SOFTWARE: AtomicBool = AtomicBool::new(false);

/// Whether the machine has been brought up.
static READY: AtomicBool = AtomicBool::new(false);

/// How long a wait for a tick spins before it gives up. The timer runs at
/// [`TICKS_PER_SECOND`], so a tick is a millisecond away and this is far
/// more time than one needs.
const SPIN_LIMIT: u64 = 200_000_000;

/// How long the image watches a masked timer before it believes it.
const QUIET_SPINS: u64 = 20_000_000;

/// Records the vector and acknowledges it, unless the image raised it
/// itself: an end-of-interrupt for an interrupt that was never delivered
/// would acknowledge whatever is actually in service.
fn on_interrupt(vector: u8) {
    LAST_VECTOR.store(u32::from(vector), Ordering::SeqCst);
    if FROM_SOFTWARE.load(Ordering::SeqCst) {
        return;
    }
    interrupts::acknowledge(vector);
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

/// Maps the frames of a device register window, uncached, out of the
/// address space the memory bring-up left.
fn map_device(frames: PhysFrameRange) -> Option<VirtAddr> {
    let mut access = window();
    let mut tlb = LocalTlb::new();
    memory::with_memory(|kernel| {
        kernel
            .map_device::<X86Entry, _, _>(&mut access, &mut tlb, frames)
            .ok()
    })
    .flatten()
}

/// Runs the memory bring-up, without which nothing can be mapped.
fn bring_up_memory() {
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

/// Brings the interrupt hardware up and starts the timer, once for the
/// whole image: the order of the tests is not fixed, so every one of them
/// starts by making sure this has happened.
fn ensure_ready() {
    if READY.swap(true, Ordering::SeqCst) {
        return;
    }
    bring_up_memory();
    traps::set_interrupt_handler(on_interrupt);
    let brought_up = testing::with_platform(|platform: &X86Platform| {
        // SAFETY: the loader's tables are active, the descriptor tables of
        // the test image carry a handler for every vector of the plan,
        // interrupts are off, and this runs once on the boot processor.
        unsafe { interrupts::bring_up(platform, map_device) }
    });
    match brought_up {
        Some(Ok(())) => {}
        Some(Err(error)) => testing::fail(format_args!("the interrupt bring-up failed: {error}")),
        None => testing::fail(format_args!("the platform is not reachable")),
    }
    // SAFETY: the interval timer belongs to the kernel, interrupts are
    // still off, and this is the processor the controller belongs to.
    if let Err(error) = unsafe { interrupts::start_timer(TICKS_PER_SECOND) } {
        testing::fail(format_args!("the timer did not start: {error}"));
    }
    // SAFETY: the descriptor table is loaded and every vector the hardware
    // can raise has a handler.
    unsafe {
        instructions::enable_interrupts();
    }
}

/// Waits until the timer has delivered `wanted` ticks. `false` if it never
/// did.
fn wait_for(wanted: u64) -> bool {
    let mut spins = 0u64;
    while interrupts::ticks() < wanted {
        if spins >= SPIN_LIMIT {
            return false;
        }
        spins = spins.saturating_add(1);
        core::hint::spin_loop();
    }
    true
}

/// Spins for `count` turns without waiting for anything.
fn spin(count: u64) {
    let mut spins = 0u64;
    while spins < count {
        spins = spins.saturating_add(1);
        core::hint::spin_loop();
    }
}

/// Runs `body` with the timer masked, and unmasks it afterwards.
fn with_masked_timer<R>(body: impl FnOnce() -> R) -> R {
    interrupts::with_controller(|apics| apics.local_mut().mask_timer(true));
    let outcome = body();
    interrupts::with_controller(|apics| apics.local_mut().mask_timer(false));
    outcome
}

/// Raises `VECTOR` from software with the hardware kept quiet, and reports
/// which vector the handler saw.
fn raise<const VECTOR: u8>() -> u32 {
    LAST_VECTOR.store(NO_VECTOR, Ordering::SeqCst);
    // SAFETY: interrupts go back on at the end of this function, and
    // nothing in between needs one.
    unsafe {
        instructions::disable_interrupts();
    }
    FROM_SOFTWARE.store(true, Ordering::SeqCst);
    testing::raise_interrupt::<VECTOR>();
    FROM_SOFTWARE.store(false, Ordering::SeqCst);
    // SAFETY: the descriptor table is loaded and every vector the hardware
    // can raise has a handler.
    unsafe {
        instructions::enable_interrupts();
    }
    LAST_VECTOR.load(Ordering::SeqCst)
}

/// The tables of the machine name a local APIC and at least one I/O APIC,
/// and the unit is on once the bring-up has run.
#[test_case]
fn the_bring_up_finds_the_interrupt_hardware_and_turns_it_on() {
    ensure_ready();
    let counts = interrupts::with_madt(|madt| (madt.io_apic_count(), madt.processors));
    let Some((io_apics, processors)) = counts else {
        testing::fail(format_args!("the controller is not reachable"));
    };
    if io_apics == 0 {
        testing::fail(format_args!("the machine reports no i/o apic"));
    }
    if processors == 0 {
        testing::fail(format_args!("the machine reports no processor"));
    }
    let enabled = interrupts::with_controller(|apics| apics.local().is_enabled());
    assert_eq!(enabled, Some(true));
}

/// The timer delivers, and it keeps delivering: the second tick only
/// arrives because the handler acknowledged the first.
#[test_case]
fn a_second_tick_arrives_after_the_end_of_interrupt() {
    ensure_ready();
    let start = interrupts::ticks();
    if !wait_for(start.saturating_add(1)) {
        testing::fail(format_args!("the first tick never arrived"));
    }
    if !wait_for(start.saturating_add(2)) {
        testing::fail(format_args!(
            "the second tick never arrived, so the end-of-interrupt did not take"
        ));
    }
    assert!(interrupts::ticks() >= start + 2);
}

/// The tick counter grows on its own, without anything asking it to.
#[test_case]
fn the_tick_counter_grows_while_the_kernel_does_nothing() {
    ensure_ready();
    let start = interrupts::ticks();
    if !wait_for(start.saturating_add(5)) {
        testing::fail(format_args!("the timer delivered fewer than five ticks"));
    }
}

/// A masked timer delivers nothing, and unmasking starts it again.
#[test_case]
fn masking_the_timer_stops_it_and_unmasking_starts_it_again() {
    ensure_ready();
    let quiet = with_masked_timer(|| {
        let before = interrupts::ticks();
        spin(QUIET_SPINS);
        interrupts::ticks() == before
    });
    if !quiet {
        testing::fail(format_args!("a masked timer still delivered"));
    }
    let after = interrupts::ticks();
    if !wait_for(after.saturating_add(1)) {
        testing::fail(format_args!("the timer did not start again"));
    }
}

/// A vector raised from software reaches the handler of exactly that
/// vector, for the timer and for the spurious vector of the local APIC.
#[test_case]
fn a_vector_raised_from_software_reaches_the_handler_of_that_vector() {
    ensure_ready();
    assert_eq!(raise::<{ vectors::TIMER }>(), u32::from(vectors::TIMER));
    assert_eq!(
        raise::<{ vectors::SPURIOUS }>(),
        u32::from(vectors::SPURIOUS)
    );
    assert_eq!(
        raise::<{ vectors::IOAPIC_BASE }>(),
        u32::from(vectors::IOAPIC_BASE)
    );
}

/// The controller routes an ISA line to a vector, masks it, and refuses to
/// route it twice.
#[test_case]
fn an_isa_line_is_routed_once_and_stays_masked() {
    ensure_ready();
    let line = InterruptLine::new(4);
    let outcome = interrupts::with_controller(|apics| {
        let gsi = apics.gsi_of(line);
        let vector = vectors::for_gsi(gsi).and_then(|number| Vector::new(number).ok())?;
        let first = apics.route(line, vector);
        let second = apics.route(line, vector);
        Some((first, second))
    });
    match outcome {
        Some(Some((Ok(()), Err(_)))) => {}
        Some(Some((first, second))) => testing::fail(format_args!(
            "routing the line gave {first:?} and then {second:?}"
        )),
        Some(None) => testing::fail(format_args!("the line has no vector in the plan")),
        None => testing::fail(format_args!("the controller is not reachable")),
    }
}
