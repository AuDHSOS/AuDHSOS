// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The three capabilities of Phase 12 as a user thread reaches them: the
//! clock, a wait that ends at its deadline, entropy, and a message
//! interrupt.
//!
//! The host tests reach every path of all four calls against a recording
//! double. What they cannot show is that the clock advances with the
//! hardware, that a thread which nobody signals comes back at the
//! microsecond it named, that `RDSEED` answers on the reference machine,
//! and that a vector nothing routed arrives as the bit it was bound to.
//! This image shows that.
//!
//! Catalog 6.6.59 and 6.6.60.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::layout::TICKS_PER_SECOND;
use audhsos_abi::{Error, Rights};
use kernel_hal_x86_64::testing;
use kernel_hal_x86_64::vectors;
use kernel_objects::object::{AnyObjectId, SystemControl};
use kernel_types::PhysFrame;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that reads the clock around a wait of fifty milliseconds.
static CLOCK_AND_WAIT: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/clock_and_wait.bin"
));

/// The program that draws two seeds.
static ENTROPY: &[u8] = include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/entropy.bin"));

/// The driver that takes a message interrupt.
static MSI_VECTOR: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/msi_vector.bin"));

/// The words `clock_and_wait` writes.
const CLOCK_BEFORE: usize = 0;
const CLOCK_AFTER: usize = 1;
const CLOCK_ELAPSED: usize = 2;
const CLOCK_BITS: usize = 3;
const CLOCK_STATUS: usize = 4;
const CLOCK_DONE: usize = 5;

/// The value `clock_and_wait` writes when it has finished.
const CLOCK_FINISHED: u64 = 0x0C_10CC;

/// How long that program waits, in microseconds.
const WAIT: u64 = 50_000;

/// The microseconds one tick is, which is the resolution of the clock and
/// therefore the tolerance of every reading.
const TICK: u64 = 1_000_000 / TICKS_PER_SECOND as u64;

/// The words `entropy` writes.
const ENTROPY_FIRST_STATUS: usize = 0;
const ENTROPY_SECOND_STATUS: usize = 1;
const ENTROPY_COUNT: usize = 2;
const ENTROPY_DIFFER: usize = 3;
const ENTROPY_DONE: usize = 5;

/// The value `entropy` writes when it has finished.
const ENTROPY_FINISHED: u64 = 0x0E_71D0;

/// The words `msi_vector` writes.
const MSI_CREATE: usize = 0;
const MSI_BIND: usize = 1;
const MSI_ADDRESS: usize = 2;
const MSI_DATA: usize = 3;
const MSI_READY: usize = 4;
const MSI_WORD: usize = 5;
const MSI_DONE: usize = 6;

/// The value `msi_vector` writes once it is about to wait.
const MSI_ARMED: u64 = 0x0A_2110;

/// The value it writes when it has finished.
const MSI_FINISHED: u64 = 0x0D_5170;

/// The bit the driver binds its vector to.
const MSI_BIT: u64 = 5;

/// The payload word the handle of the system control capability is in.
const CONTROL_WORD: usize = 0;

/// The vector the driver's interrupt gets: it is the only caller of
/// `interrupt_create_msi` in this image, so it takes the first of the
/// space.
const MSI: u8 = vectors::MSI_BASE;

/// The rights of a system control handle a driver holds.
const ROOT_AUTHORITY: Rights = Rights::MANAGE
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// Runs `program` to its end and answers the page it wrote into. The
/// caller says which word carries the mark and what the mark is.
fn run(program: &[u8], done: usize, finished: u64, control: bool) -> PhysFrame {
    let mut process = support::create_process(program);
    let page = support::share_page(&process);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    if control {
        let handle = support::install(
            process.id,
            AnyObjectId::of(SystemControl::ID),
            ROOT_AUTHORITY,
        );
        support::set_buffer_word(spawned.buffer, CONTROL_WORD, handle.raw());
    }
    support::start(spawned.thread);
    support::idle_until(|| support::page_word(page, done) == finished);
    page
}

/// The clock advances with the timer, and a thread that nobody signals
/// comes back at the deadline it named with no bits.
#[test_case]
fn the_clock_measures_a_wait_that_ends_at_its_deadline() {
    support::bring_up();
    support::start_timer(|_ticks| false);
    let page = run(CLOCK_AND_WAIT, CLOCK_DONE, CLOCK_FINISHED, false);

    let status = support::page_word(page, CLOCK_STATUS);
    if status != Status::OK.raw() {
        testing::fail(format_args!("the wait answered {status:#x}"));
    }
    let bits = support::page_word(page, CLOCK_BITS);
    if bits != 0 {
        testing::fail(format_args!(
            "the wait answered {bits:#x}, and nothing signalled"
        ));
    }
    let before = support::page_word(page, CLOCK_BEFORE);
    let after = support::page_word(page, CLOCK_AFTER);
    let elapsed = support::page_word(page, CLOCK_ELAPSED);
    if after < before {
        testing::fail(format_args!(
            "the clock went backwards: {before} to {after}"
        ));
    }
    // The wake is at tick resolution, and a deadline between two ticks
    // waits for the later one: the difference is the wait, to within a
    // tick on either side.
    if elapsed < WAIT.saturating_sub(TICK) {
        testing::fail(format_args!(
            "the wait of {WAIT} microseconds took only {elapsed}"
        ));
    }
    if elapsed > WAIT.saturating_add(TICK.saturating_mul(4)) {
        testing::fail(format_args!(
            "the wait of {WAIT} microseconds took {elapsed}"
        ));
    }
    say!("the clock read {before} and {after}, {elapsed} microseconds apart");
}

/// `random_bytes` answers four words on the reference machine, and two
/// draws differ.
#[test_case]
fn two_seeds_out_of_the_hardware_differ() {
    support::bring_up();
    let page = run(ENTROPY, ENTROPY_DONE, ENTROPY_FINISHED, false);

    for (word, which) in [
        (ENTROPY_FIRST_STATUS, "first"),
        (ENTROPY_SECOND_STATUS, "second"),
    ] {
        let status = support::page_word(page, word);
        if status == Status::failed(Error::Unavailable).raw() {
            testing::fail(format_args!(
                "the {which} draw found no source; the machine wants -cpu qemu64,+rdrand,+rdseed"
            ));
        }
        if status != Status::OK.raw() {
            testing::fail(format_args!("the {which} draw answered {status:#x}"));
        }
    }
    let count = support::page_word(page, ENTROPY_COUNT);
    if count != 4 {
        testing::fail(format_args!("a seed is {count} words, not four"));
    }
    if support::page_word(page, ENTROPY_DIFFER) != 1 {
        testing::fail(format_args!("two draws answered the same thirty-two bytes"));
    }
    say!("two seeds of four words each, and they differ");
}

/// A message interrupt the root task created and bound arrives as the bit
/// it names, and its acknowledgement touches no controller.
#[test_case]
fn a_message_vector_arrives_as_the_bit_it_was_bound_to() {
    support::bring_up();
    support::start_timer(|_ticks| false);

    let mut process = support::create_process(MSI_VECTOR);
    let page = support::share_page(&process);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    let control = support::install(
        process.id,
        AnyObjectId::of(SystemControl::ID),
        ROOT_AUTHORITY,
    );
    support::set_buffer_word(spawned.buffer, CONTROL_WORD, control.raw());
    support::start(spawned.thread);
    support::idle_until(|| support::page_word(page, MSI_READY) == MSI_ARMED);

    let create = support::page_word(page, MSI_CREATE);
    if create != Status::OK.raw() {
        testing::fail(format_args!("the vector was refused: {create:#x}"));
    }
    let bind = support::page_word(page, MSI_BIND);
    if bind != Status::OK.raw() {
        testing::fail(format_args!("the binding was refused: {bind:#x}"));
    }
    let address = support::page_word(page, MSI_ADDRESS);
    let data = support::page_word(page, MSI_DATA);
    if address != kernel_hal_x86_64::apic::MSI_ADDRESS_BASE
        && address & 0xFFF0_0000 != kernel_hal_x86_64::apic::MSI_ADDRESS_BASE
    {
        testing::fail(format_args!(
            "the message address {address:#x} is not in the local APIC's region"
        ));
    }
    if data != u64::from(MSI) {
        testing::fail(format_args!(
            "the message data is {data:#x}, and the first vector of the space is {MSI:#x}"
        ));
    }

    // The device: nothing of the reference machine writes a message yet, so
    // the image raises the vector itself. It arrives through the same
    // handler an MSI-X write would reach.
    say!("raising vector {MSI:#x}, which nothing routed");
    testing::raise_interrupt::<MSI>();
    support::idle_until(|| support::page_word(page, MSI_DONE) == MSI_FINISHED);

    let word = support::page_word(page, MSI_WORD);
    let expected = 1_u64 << MSI_BIT;
    if word != expected {
        testing::fail(format_args!(
            "the driver woke with {word:#x}, and it bound bit {MSI_BIT}"
        ));
    }
    say!("a message vector reached ring three as bit {MSI_BIT}");
}
