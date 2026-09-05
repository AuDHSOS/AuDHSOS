// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every system call of this kernel, made from ring three.
//!
//! The host tests of `kernel-syscall` reach every path of every call with
//! a recording double. What they cannot show is that a user thread reaches
//! them at all: that the gate at vector `0x80` is the only one it may
//! enter, that the buffer the kernel reads is the one the thread wrote,
//! and that the status word comes back to the thread that asked. This
//! image shows that, once, for the whole table.
//!
//! `every_syscall` walks the table and writes down a pair per call — the
//! number and the status word — so the tests here read the log rather than
//! the order the program went in:
//!
//! - every call of the table was made;
//! - each of the twenty this phase implements answered a success once and
//!   an error once;
//! - each of the twenty-one it does not answered exactly the refusal the
//!   table of the interface asks for: `Unsupported` where the call is
//!   reachable, `WrongObjectType` where its first argument names an object
//!   type of a later phase.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, FirstArgument, Handle, ObjectType, Rights, Syscall};
use kernel_hal_x86_64::testing;
use kernel_objects::object::AnyObjectId;
use kernel_syscall::calls::UNIMPLEMENTED;

use crate::support::say;

mod support;

kernel_hal_x86_64::test_kernel!();

/// The program that walks the whole table.
static EVERY_SYSCALL: &[u8] = include_bytes!(concat!(
    env!("AUDHSOS_USER_TESTS_DIR"),
    "/every_syscall.bin"
));

/// The payload words the kernel leaves the handles in, which
/// `every_syscall.rs` names the same way.
const PROCESS_WORD: usize = 0;
const THREAD_WORD: usize = 1;
const MEMORY_WORD: usize = 2;
const BAD_WORD: usize = 3;

/// The payload word the first pair is in.
const FIRST_RESULT: usize = 8;

/// The payload word past the last pair.
const RESULTS_END: usize = 400;

/// How many frames the memory object the program is given covers: enough
/// to map one page of and still split in two.
const MEMORY_FRAMES: u64 = 4;

/// A handle that names nothing: its index lies beyond the arena, so no
/// generation could make it name anything.
const BEYOND_THE_ARENA: u32 = 0x00FF_FFFF;

/// One pair of the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Answer {
    /// The call that was made.
    call: Syscall,
    /// What the kernel answered.
    status: Status,
}

/// The log of one run, read out of the buffer of the thread that wrote it.
struct Log {
    answers: [Option<Answer>; Log::CAPACITY],
    len: usize,
}

impl Log {
    /// How many pairs the log holds; the buffer has room for fewer.
    const CAPACITY: usize = (RESULTS_END - FIRST_RESULT) / 2;

    /// Every answer the log holds for `call`.
    fn statuses(&self, call: Syscall) -> impl Iterator<Item = Status> + '_ {
        self.answers
            .iter()
            .take(self.len)
            .flatten()
            .filter(move |answer| answer.call == call)
            .map(|answer| answer.status)
    }

    /// `true` when `call` was answered without an error at least once.
    fn succeeded(&self, call: Syscall) -> bool {
        self.statuses(call).any(|status| status.error().is_none())
    }

    /// The errors `call` was answered with.
    fn errors(&self, call: Syscall) -> impl Iterator<Item = Error> + '_ {
        self.statuses(call).filter_map(Status::error)
    }

    /// How many answers the log holds for `call`.
    fn count(&self, call: Syscall) -> usize {
        self.statuses(call).count()
    }
}

/// Whether the run has been made; a second test reads the same log.
static DONE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The log of the run, once it has been made.
static LOG: audhsos_sync::Global<Log> = audhsos_sync::Global::new();

/// Builds the process, hands it its handles, runs it, and reads the log.
/// Runs once; every test reads what it left.
fn run_once() {
    if DONE.swap(1, core::sync::atomic::Ordering::SeqCst) == 1 {
        return;
    }
    support::bring_up();
    let mut process = support::create_process(EVERY_SYSCALL);
    let spawned = support::add_thread(&mut process, support::DEFAULT_PRIORITY);
    let memory = support::memory_object(MEMORY_FRAMES);

    // What the root task will install into a process it starts: a handle
    // to the process itself, one to its thread, and one to the memory it
    // was given. The fourth word is a handle that names nothing, which is
    // what the failure half of every case is made of.
    let own_process = support::install(
        process.id,
        AnyObjectId::of(process.id),
        Rights::MANAGE | Rights::MAP | Rights::INSTALL | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let own_thread = support::install(
        process.id,
        AnyObjectId::of(spawned.thread),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
    );
    let own_memory = support::install(
        process.id,
        AnyObjectId::of(memory),
        Rights::READ
            | Rights::WRITE
            | Rights::MAP
            | Rights::INFO
            | Rights::DUPLICATE
            | Rights::TRANSFER,
    );
    let Some(nothing) = Handle::new(BEYOND_THE_ARENA, 1) else {
        testing::fail(format_args!(
            "the handle that names nothing is not a handle"
        ));
    };
    support::set_buffer_word(spawned.buffer, PROCESS_WORD, own_process.raw());
    support::set_buffer_word(spawned.buffer, THREAD_WORD, own_thread.raw());
    support::set_buffer_word(spawned.buffer, MEMORY_WORD, own_memory.raw());
    support::set_buffer_word(spawned.buffer, BAD_WORD, nothing.raw());

    support::start(spawned.thread);
    say!("switching into a thread that makes every call of the table");
    support::run_threads(None);
    say!("back in the kernel");

    if support::state_of(spawned.thread).is_some() {
        testing::fail(format_args!(
            "the thread did not end; it made {} of its calls",
            read(spawned.buffer).len
        ));
    }
    if LOG.init(read(spawned.buffer)).is_err() {
        testing::fail(format_args!("the log is already in place"));
    }
}

/// Reads the pairs out of the buffer the thread wrote them into.
fn read(buffer: kernel_types::PhysFrame) -> Log {
    let mut log = Log {
        answers: [None; Log::CAPACITY],
        len: 0,
    };
    for index in 0..Log::CAPACITY {
        let at = FIRST_RESULT.saturating_add(index.saturating_mul(2));
        let number = support::buffer_word(buffer, at);
        if number == 0 {
            break;
        }
        let raw = support::buffer_word(buffer, at.saturating_add(1));
        let Ok(call) = Syscall::from_word(number) else {
            testing::fail(format_args!("the thread wrote down call number {number}"));
        };
        let Ok(status) = Status::from_raw(raw) else {
            testing::fail(format_args!(
                "the thread wrote down status {raw:#x} for {}",
                call.name()
            ));
        };
        if let Some(slot) = log.answers.get_mut(index) {
            *slot = Some(Answer { call, status });
        }
        log.len = index.saturating_add(1);
    }
    log
}

/// Runs `body` with the log of the run.
fn with_log(body: impl FnOnce(&Log)) {
    run_once();
    let Ok(log) = LOG.borrow(&audhsos_sync::UncontendedToken) else {
        testing::fail(format_args!("the log is not reachable"));
    };
    body(&log);
}

/// `true` for a call this phase leaves to Phase 6.
fn is_of_a_later_phase(call: Syscall) -> bool {
    UNIMPLEMENTED.contains(&call)
}

/// The refusal a call of a later phase has to answer with, given what the
/// program passes it: the handle of its own process where the call takes
/// one, and nothing where it takes none.
///
/// A call whose first argument names a type this phase holds is reachable,
/// and the dispatcher gets as far as the call itself, which refuses it. A
/// call whose first argument names a type of a later phase is refused by
/// the type check before that, because a process is not an endpoint.
fn refusal_of(call: Syscall) -> Error {
    match call.first_argument() {
        FirstArgument::Nothing | FirstArgument::Any => Error::Unsupported,
        FirstArgument::Object(ObjectType::Process) => Error::Unsupported,
        FirstArgument::Object(_) => Error::WrongObjectType,
    }
}

/// Every call of the table was made from user mode.
#[test_case]
fn every_call_of_the_table_was_made_from_ring_three() {
    with_log(|log| {
        for call in Syscall::ALL {
            if log.count(*call) == 0 {
                testing::fail(format_args!(
                    "{} (number {}) was never called",
                    call.name(),
                    call.number()
                ));
            }
        }
        say!(
            "{} calls of the table answered {} times",
            Syscall::ALL.len(),
            log.len
        );
    });
}

/// Every call this phase implements answered a success at least once.
#[test_case]
fn every_call_this_phase_implements_succeeds_from_user_mode() {
    with_log(|log| {
        for call in Syscall::ALL {
            if is_of_a_later_phase(*call) {
                continue;
            }
            if *call == Syscall::ThreadExit {
                // Its success is that the thread is gone, which `run_once`
                // has already held against the pool: a thread that ended
                // has nothing left to write a status with.
                continue;
            }
            if !log.succeeded(*call) {
                let seen = log.errors(*call).next();
                testing::fail(format_args!(
                    "{} never succeeded from user mode; it answered {seen:?}",
                    call.name()
                ));
            }
        }
        say!("every call of this phase succeeded from user mode");
    });
}

/// Every call this phase implements answered an error at least once, for
/// arguments that do not check out.
#[test_case]
fn every_call_this_phase_implements_fails_from_user_mode() {
    with_log(|log| {
        for call in Syscall::ALL {
            if is_of_a_later_phase(*call) {
                continue;
            }
            let Some(error) = log.errors(*call).next() else {
                testing::fail(format_args!(
                    "{} never failed from user mode; nothing of it was refused",
                    call.name()
                ));
            };
            if error == Error::Unsupported {
                testing::fail(format_args!(
                    "{} answered Unsupported, which is what a call of a later phase answers",
                    call.name()
                ));
            }
        }
        say!("every call of this phase refused arguments that do not check out");
    });
}

/// Every call of a later phase is refused, and refused with the error the
/// table of the interface asks for.
#[test_case]
fn every_call_of_a_later_phase_is_refused() {
    with_log(|log| {
        for call in UNIMPLEMENTED {
            let wanted = refusal_of(*call);
            let mut answers = log.statuses(*call);
            let Some(status) = answers.next() else {
                testing::fail(format_args!("{} was never called", call.name()));
            };
            match status.error() {
                Some(error) if error == wanted => {}
                Some(error) => testing::fail(format_args!(
                    "{} answered {error:?}, not {wanted:?}",
                    call.name()
                )),
                None => testing::fail(format_args!(
                    "{} succeeded; no call of a later phase may",
                    call.name()
                )),
            }
        }
        say!(
            "all {} calls of a later phase were refused, each with its own error",
            UNIMPLEMENTED.len()
        );
    });
}

/// The two halves of the table are the whole of it and nothing twice: what
/// the kernel calls unimplemented is what the tests above skip.
#[test_case]
fn the_table_is_split_in_two_and_nothing_falls_between() {
    let implemented = Syscall::ALL
        .iter()
        .filter(|call| !is_of_a_later_phase(**call))
        .count();
    if implemented != 20 {
        testing::fail(format_args!(
            "this phase implements {implemented} calls, not the twenty of the plan"
        ));
    }
    if UNIMPLEMENTED.len() != 21 {
        testing::fail(format_args!(
            "{} calls are left to Phase 6, not the twenty-one of the plan",
            UNIMPLEMENTED.len()
        ));
    }
    if Syscall::ALL.len() != implemented + UNIMPLEMENTED.len() {
        testing::fail(format_args!("the two halves are not the whole table"));
    }
    say!(
        "{implemented} calls of this phase, {} of the next",
        UNIMPLEMENTED.len()
    );
}
