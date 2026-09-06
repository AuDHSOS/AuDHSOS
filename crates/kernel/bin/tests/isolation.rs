// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a user thread cannot do, and what happens to it when it tries.
//!
//! Two programs go looking for the boundary: one reads an address of the
//! kernel half, which its page tables carry but not for ring three, and
//! one runs `hlt`, which only ring zero may. The processor stops both, the
//! kernel stops the thread, and the machine goes on — which is the whole
//! point, and what every test here checks after the fault: another thread
//! is started, runs, and ends in the same machine.
//!
//! Phase 6 turns the same two faults into a message on the fault handler
//! endpoint of the process. The tests at the end of this file are that
//! item of [6.6.21](../../../docs/06-testing-strategy.md): a handler in a
//! process of its own receives the message with the reserved label, the
//! kind, and the address, and what it does with the fault decides what
//! becomes of the thread. Answering the fault of a thread that reads a
//! kernel address resumes it at the instruction it faulted on, where it
//! faults again — so the handler gives up on the second one and kills the
//! process. A thread that wrote where nothing was mapped is the other
//! case: the handler maps a page there, answers, and the thread runs on.
//!
//! The tests before them are the item that names no handler.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ipc_buffer::{fault_kind_of, is_kernel_label};
use audhsos_abi::{FaultKind, Rights, ThreadState};
use kernel_hal_x86_64::testing;
use kernel_objects::object::{AnyObjectId, EndpointId, ThreadId};
use kernel_types::PhysFrame;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that reads the first byte of the kernel image.
static READ_KERNEL_MEMORY: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/read_kernel_memory.bin"
));

/// The program that halts, which a user thread may not do.
static HLT_IN_USER: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/hlt_in_user.bin"));

/// The program that ends itself and nothing else, which is what runs after
/// a fault to show that the machine still runs threads.
static THREAD_EXIT: &[u8] =
    include_bytes!(concat!(env!("AUDHSOS_USER_TESTS_DIR"), "/thread_exit.bin"));

/// The vector of a page fault.
const PAGE_FAULT: u32 = 14;

/// The vector of a general protection fault.
const GENERAL_PROTECTION: u32 = 13;

/// Runs `program` in a thread of its own until it faults, and returns that
/// thread. Fails the image when the thread came back without faulting.
fn until_it_faults(program: &[u8], what: &str) -> ThreadId {
    support::bring_up();
    let before = support::faults();
    let spawned = support::spawn(program);
    say!("switching into a thread that {what}");
    support::run_threads(None);
    say!("back in the kernel");
    if support::faults() != before.saturating_add(1) {
        testing::fail(format_args!(
            "the thread that {what} raised no fault: {} before, {} after",
            before,
            support::faults()
        ));
    }
    spawned.thread
}

/// The thread stopped where a faulted thread stops, and it still holds
/// everything it held.
fn assert_faulted(thread: ThreadId, what: &str) {
    match support::state_of(thread) {
        Some(ThreadState::Faulted) => {}
        Some(state) => testing::fail(format_args!(
            "the thread that {what} is {state:?}, not Faulted"
        )),
        None => testing::fail(format_args!(
            "the thread that {what} was cleared away; a faulted thread keeps its slot"
        )),
    }
    say!("the thread that {what} stopped in Faulted and kept its slot");
}

/// The machine runs threads after a fault: a new process starts, runs, and
/// ends.
fn assert_the_machine_runs_on() {
    let after = support::spawn(THREAD_EXIT);
    support::run_threads(None);
    if support::state_of(after.thread).is_some() {
        testing::fail(format_args!(
            "the thread started after the fault did not end"
        ));
    }
    say!("a thread started after the fault ran and ended");
}

/// A user thread that reads a kernel address faults, stops in `Faulted`,
/// and leaves the rest of the system running.
#[test_case]
fn a_thread_that_reads_kernel_memory_faults_and_the_machine_runs_on() {
    let thread = until_it_faults(READ_KERNEL_MEMORY, "reads the kernel half");
    assert_faulted(thread, "reads the kernel half");
    assert_the_machine_runs_on();
}

/// A user thread that executes `hlt` faults the same way.
#[test_case]
fn a_thread_that_halts_faults_and_the_machine_runs_on() {
    let thread = until_it_faults(HLT_IN_USER, "halts");
    assert_faulted(thread, "halts");
    assert_the_machine_runs_on();
}

/// The two faults are the two the processor raises for them: a page fault
/// for the read of a page that is mapped but not for ring three, a general
/// protection fault for the instruction that ring three may not run.
#[test_case]
fn the_two_faults_are_the_two_vectors_the_processor_raises() {
    support::bring_up();
    let read = support::spawn(READ_KERNEL_MEMORY);
    support::run_threads(None);
    let read_vector = support::last_fault().vector;

    let halt = support::spawn(HLT_IN_USER);
    support::run_threads(None);
    let halt_vector = support::last_fault().vector;

    if read_vector != PAGE_FAULT {
        testing::fail(format_args!(
            "reading the kernel half raised vector {read_vector}, not {PAGE_FAULT}"
        ));
    }
    if halt_vector != GENERAL_PROTECTION {
        testing::fail(format_args!(
            "halting raised vector {halt_vector}, not {GENERAL_PROTECTION}"
        ));
    }
    say!("the read faulted with vector {read_vector}, the halt with vector {halt_vector}");
    let _ = (read.thread, halt.thread);
}

/// Two threads that fault one after the other are both stopped: the kernel
/// reports the second fault as readily as the first, and neither of them
/// takes the machine with it.
#[test_case]
fn a_second_fault_is_handled_like_the_first() {
    support::bring_up();
    let first = support::spawn(READ_KERNEL_MEMORY);
    support::run_threads(None);
    let second = support::spawn(HLT_IN_USER);
    support::run_threads(None);

    assert_faulted(first.thread, "read the kernel half first");
    assert_faulted(second.thread, "halted second");
    assert_the_machine_runs_on();
}

/// A faulted thread is not a thread the reaper takes: its kernel stack,
/// its buffer, and its slot stay where they are, because whoever created
/// it may still want to look at it.
#[test_case]
fn a_faulted_thread_keeps_what_it_holds() {
    let thread = until_it_faults(HLT_IN_USER, "halts");
    support::sweep();
    assert_faulted(thread, "halts");
}

/// The handler that takes the fault messages of another process.
static FAULT_HANDLER: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/fault_handler.bin"
));

/// The program that writes where nothing is mapped.
static WRITE_UNMAPPED: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/write_unmapped.bin"
));

/// What the handler is told to do with a fault.
const ANSWER: u64 = 0;
const KILL: u64 = 1;
const REPAIR: u64 = 2;

/// The words of the handler's own buffer the kernel fills in.
const ENDPOINT_WORD: usize = 0;
const TARGET_WORD: usize = 1;
const MEMORY_WORD: usize = 2;
const ACTION_WORD: usize = 3;
const FAULTS_WORD: usize = 4;
const FINAL_WORD: usize = 5;

/// The words of the handler's shared page.
const TAKEN: usize = 0;
const LABEL: usize = 1;
const KIND: usize = 2;
const ADDRESS: usize = 3;
const POINTER: usize = 4;
const HANDLER_DONE: usize = 5;

/// The value the handler writes into [`HANDLER_DONE`] when it has finished.
const HANDLER_FINISHED: u64 = 0x0FA0_17ED;

/// The words of the shared page of `write_unmapped`.
const WROTE_WORD: usize = 0;
const WRITER_DONE: usize = 1;

/// What that program writes where nothing was mapped, and the marker after.
const WROTE: u64 = 0x0A11_0CA7;
const WRITER_FINISHED: u64 = 0x0DEA_1000;

/// The rights a handler needs over the process whose faults it takes: to end
/// it, and to map memory into it.
const OVER_THE_TARGET: Rights = Rights::MANAGE
    .union(Rights::MAP)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// What one run of a program with a handler left behind.
struct Handled {
    handler: PhysFrame,
    target: PhysFrame,
    thread: ThreadId,
}

/// Runs `program` with a handler in a process of its own, which takes
/// `faults` of its faults, does `action` with every one but the last and
/// `last` with that one.
fn with_a_handler(program: &[u8], faults: u64, action: u64, last: u64) -> Handled {
    support::bring_up();
    let endpoint = support::endpoint();

    // The faulting process, which reports on the endpoint.
    let mut target = support::create_process(program);
    let target_page = support::share_page(&target);
    let faulting = support::add_thread(&mut target, support::DEFAULT_PRIORITY);
    support::set_fault_handler(target.id, endpoint);

    // The handler, which holds the endpoint, the process, and a page to map.
    let mut handler = support::create_process(FAULT_HANDLER);
    let handler_page = support::share_page(&handler);
    let taking = support::add_thread(&mut handler, support::MAX_PRIORITY);
    let handler_endpoint = support::install(
        handler.id,
        AnyObjectId::of(endpoint),
        Rights::RECV | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let over = support::install(handler.id, AnyObjectId::of(target.id), OVER_THE_TARGET);
    let memory = support::memory_object(1);
    let spare = support::install(
        handler.id,
        AnyObjectId::of(memory),
        Rights::READ | Rights::WRITE | Rights::MAP | Rights::DUPLICATE | Rights::TRANSFER,
    );
    support::set_buffer_word(taking.buffer, ENDPOINT_WORD, handler_endpoint.raw());
    support::set_buffer_word(taking.buffer, TARGET_WORD, over.raw());
    support::set_buffer_word(taking.buffer, MEMORY_WORD, spare.raw());
    support::set_buffer_word(taking.buffer, ACTION_WORD, action);
    support::set_buffer_word(taking.buffer, FAULTS_WORD, faults);
    support::set_buffer_word(taking.buffer, FINAL_WORD, last);

    // The handler waits first, at the higher priority, so that the fault it
    // takes finds it in the receivers queue.
    support::start(taking.thread);
    support::run_threads(None);
    support::start(faulting.thread);
    support::run_until(|| support::page_word(handler_page, HANDLER_DONE) == HANDLER_FINISHED);
    Handled {
        handler: handler_page,
        target: target_page,
        thread: faulting.thread,
    }
}

/// The fault handler of the process receives the message of a thread that
/// read a kernel address, with the label the kernel keeps, the kind, and the
/// address; a reply resumes the thread, which faults again, and the handler
/// ends the process.
#[test_case]
fn the_fault_handler_receives_the_message_of_a_read_of_the_kernel_half() {
    let run = with_a_handler(READ_KERNEL_MEMORY, 2, ANSWER, KILL);
    let taken = support::page_word(run.handler, TAKEN);
    if taken != 2 {
        testing::fail(format_args!(
            "the handler took {taken} faults, not the two a reply makes"
        ));
    }
    let label = support::page_word(run.handler, LABEL);
    if !is_kernel_label(label) {
        testing::fail(format_args!("the label {label:#x} is not the kernel's"));
    }
    if fault_kind_of(label) != Some(FaultKind::PageFault) {
        testing::fail(format_args!("the label {label:#x} names no page fault"));
    }
    let kind = support::page_word(run.handler, KIND);
    if kind != u64::from(FaultKind::PageFault.code()) {
        testing::fail(format_args!("the handler read kind {kind}"));
    }
    let address = support::page_word(run.handler, ADDRESS);
    if address < audhsos_abi::layout::KERNEL_SPACE_START {
        testing::fail(format_args!(
            "the fault named {address:#x}, which is no kernel address"
        ));
    }
    let pointer = support::page_word(run.handler, POINTER);
    if pointer < support::USER_BASE {
        testing::fail(format_args!(
            "the instruction pointer {pointer:#x} is not in the program"
        ));
    }
    if support::state_of(run.thread).is_some() {
        testing::fail(format_args!(
            "the handler killed the process and the thread is still there"
        ));
    }
    say!("the handler saw the label, the kind, the address, and the pointer");
}

/// The same for a thread that runs a privileged instruction: the kind is the
/// one the processor raises for it, and the reply resumes it into the same
/// instruction.
#[test_case]
fn the_fault_handler_receives_the_message_of_a_privileged_instruction() {
    let run = with_a_handler(HLT_IN_USER, 2, ANSWER, KILL);
    let taken = support::page_word(run.handler, TAKEN);
    if taken != 2 {
        testing::fail(format_args!("the handler took {taken} faults, not two"));
    }
    let kind = support::page_word(run.handler, KIND);
    if kind != u64::from(FaultKind::GeneralProtection.code()) {
        testing::fail(format_args!(
            "the handler read kind {kind}, not a general protection fault"
        ));
    }
    if support::state_of(run.thread).is_some() {
        testing::fail(format_args!("the process was not ended"));
    }
    say!("a privileged instruction reached the handler as its own kind");
}

/// A fault the handler can do something about: it maps a page at the address
/// the fault names, answers, and the thread writes through the mapping and
/// ends itself.
#[test_case]
fn a_fault_the_handler_repairs_lets_the_thread_run_on() {
    let run = with_a_handler(WRITE_UNMAPPED, 1, ANSWER, REPAIR);
    let taken = support::page_word(run.handler, TAKEN);
    if taken != 1 {
        testing::fail(format_args!(
            "the handler took {taken} faults, and one repair is all it takes"
        ));
    }
    let kind = support::page_word(run.handler, KIND);
    if kind != u64::from(FaultKind::PageFault.code()) {
        testing::fail(format_args!("the handler read kind {kind}"));
    }
    support::run_until(|| support::page_word(run.target, WRITER_DONE) == WRITER_FINISHED);
    let wrote = support::page_word(run.target, WROTE_WORD);
    if wrote != WROTE {
        testing::fail(format_args!(
            "the thread read {wrote:#x} back through the mapping, not {WROTE:#x}"
        ));
    }
    if support::state_of(run.thread).is_some() {
        testing::fail(format_args!("the thread did not end after it ran on"));
    }
    say!("the handler mapped what the thread fell over and the thread ran on");
}

/// A process whose handler endpoint is gone stops the thread where a fault
/// nobody takes stops it, and the machine runs on.
#[test_case]
fn a_handler_endpoint_that_is_gone_stops_the_thread_as_no_handler_would() {
    support::bring_up();
    let endpoint = support::endpoint();
    let mut process = support::create_process(HLT_IN_USER);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    support::set_fault_handler(process.id, endpoint);
    // The reference the handler holds is the last one, so naming it and
    // taking it away leaves the process pointing at nothing.
    take_apart(endpoint);
    support::start(spawned.thread);
    support::run_threads(None);
    assert_faulted(spawned.thread, "halts with a handler that is gone");
    assert_the_machine_runs_on();
}

/// Drops every reference to `endpoint`, so that the machine no longer holds
/// it.
fn take_apart(endpoint: EndpointId) {
    for _ in 0..4 {
        if !lives(endpoint) {
            return;
        }
        support::release(AnyObjectId::of(endpoint));
    }
    testing::fail(format_args!("the endpoint outlived its last reference"));
}

/// `true` while the machine still holds `endpoint`.
fn lives(endpoint: EndpointId) -> bool {
    kernel_core::machine::with_machine(|machine| machine.objects.endpoints.get(endpoint).is_ok())
        .unwrap_or(false)
}
