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
//! - every call answered exactly one error, and it is the one its failure
//!   case asks for;
//! - every call whose success a single thread can observe answered a success
//!   as well. The six endpoint calls whose success is a rendezvous are the
//!   exception: a thread that sends with nobody receiving waits for ever, and
//!   the `ipc` image covers those with two threads that meet.
//!
//! No call answers `Unsupported`: after Phase 6 the table is complete.

#![no_std]
#![no_main]
#![allow(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_hal_x86_64::testing::run_tests)]
#![reexport_test_harness_main = "test_main"]

use audhsos_abi::ipc_buffer::Status;
use audhsos_abi::{Error, Handle, Rights, Syscall};
use kernel_hal_x86_64::testing;
use kernel_objects::object::{AnyObjectId, SystemControl};
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
const CONTROL_WORD: usize = 4;

/// The payload word the first pair is in. Above the twenty-six words
/// `system_info` writes into the message area of the caller's own buffer.
const FIRST_RESULT: usize = 28;

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
    // The interrupt controller and the ports, without the timer: the image
    // wants the calls that touch them and no preemption.
    support::bring_up_devices();
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
        Rights::MANAGE
            | Rights::MAP
            | Rights::INSTALL
            | Rights::INFO
            | Rights::DUPLICATE
            | Rights::TRANSFER,
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
    // The root authority, which the root task will hold and hand out.
    let control = support::install(
        process.id,
        AnyObjectId::of(SystemControl::ID),
        Rights::MANAGE | Rights::DUPLICATE | Rights::TRANSFER,
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
    support::set_buffer_word(spawned.buffer, CONTROL_WORD, control.raw());

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

/// The six calls whose success is a rendezvous: a thread on its own cannot
/// observe one, because a send with nobody receiving waits for ever. The
/// `ipc` image covers them with two threads that meet.
const SUCCEEDS_IN_THE_IPC_IMAGE: &[Syscall] = &[
    Syscall::IpcSend,
    Syscall::IpcCall,
    Syscall::IpcRecv,
    Syscall::IpcTryRecv,
    Syscall::IpcReply,
    Syscall::IpcReplyRecv,
];

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
fn every_call_whose_success_one_thread_can_observe_succeeds_from_user_mode() {
    with_log(|log| {
        for call in Syscall::ALL {
            if SUCCEEDS_IN_THE_IPC_IMAGE.contains(call) {
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
        say!("every call whose success one thread can observe succeeded");
    });
}

/// The one failure case of every call of the table, and the error it has to
/// answer with. `every_syscall.rs` makes exactly one of these per call; what
/// each of them is wrong about is in the second column.
const REFUSALS: &[(Syscall, Error, &str)] = &[
    (
        Syscall::DebugLog,
        Error::ArgumentCount,
        "an argument word above the count of a call that takes none",
    ),
    (
        Syscall::ThreadYield,
        Error::ArgumentCount,
        "an argument word above the count of a call that takes none",
    ),
    (
        Syscall::ThreadExit,
        Error::ArgumentCount,
        "an argument word above the count of a call that takes none",
    ),
    (
        Syscall::ThreadInfo,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ProcessCreate,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ProcessInstallHandle,
        Error::InvalidHandle,
        "a handle that names nothing to install",
    ),
    (
        Syscall::ProcessKill,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ThreadCreate,
        Error::InvalidArgument,
        "a priority above the maximum the creator granted",
    ),
    (
        Syscall::ThreadStart,
        Error::InvalidState,
        "a thread that has already started",
    ),
    (
        Syscall::ThreadSuspend,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ThreadResume,
        Error::InvalidState,
        "a thread that is ready and was never suspended",
    ),
    (
        Syscall::ThreadSetPriority,
        Error::InvalidArgument,
        "a priority outside the priorities of this system",
    ),
    (
        Syscall::ThreadKill,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::MemoryInfo,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::MemoryMap,
        Error::AddressInUse,
        "an address something is already mapped at",
    ),
    (
        Syscall::MemoryProtect,
        Error::NotMapped,
        "an address nothing is mapped at",
    ),
    (
        Syscall::MemoryUnmap,
        Error::NotMapped,
        "an address nothing is mapped at",
    ),
    (
        Syscall::MemorySplit,
        Error::Unaligned,
        "an offset that is no page",
    ),
    (
        Syscall::MemoryMerge,
        Error::InvalidArgument,
        "an object joined to itself",
    ),
    (
        Syscall::HandleDuplicate,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::HandleClose,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ProcessSetFaultHandler,
        Error::InvalidHandle,
        "a handle that names nothing as the handler endpoint",
    ),
    (
        Syscall::EndpointCreate,
        Error::ArgumentCount,
        "an argument word above the count of a call that takes none",
    ),
    (
        Syscall::EndpointBadge,
        Error::InvalidArgument,
        "a badge of zero, which is what an unbadged capability carries",
    ),
    (
        Syscall::IpcSend,
        Error::InvalidArgument,
        "a label of the range the kernel keeps for its own messages",
    ),
    (
        Syscall::IpcCall,
        Error::InvalidArgument,
        "a label of the range the kernel keeps for its own messages",
    ),
    (
        Syscall::IpcRecv,
        Error::AccessDenied,
        "a badged capability, which carries `SEND` alone",
    ),
    (
        Syscall::IpcTryRecv,
        Error::WouldBlock,
        "an endpoint nobody is sending on",
    ),
    (
        Syscall::IpcReply,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::IpcReplyRecv,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::NotificationCreate,
        Error::ArgumentCount,
        "an argument word above the count of a call that takes none",
    ),
    (
        Syscall::NotificationSignal,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::NotificationWait,
        Error::AccessDenied,
        "a capability that may signal and not wait",
    ),
    (
        Syscall::NotificationPoll,
        Error::AccessDenied,
        "a capability that may signal and not wait",
    ),
    (
        Syscall::InterruptCreate,
        Error::InvalidArgument,
        "a line the vector plan of the machine reserves no vector for",
    ),
    (
        Syscall::InterruptBind,
        Error::InvalidArgument,
        "a bit index above the sixty-four a notification has",
    ),
    (
        Syscall::InterruptAck,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::IoPortCreate,
        Error::InvalidArgument,
        "a range of no ports at all",
    ),
    (
        Syscall::IoPortRead,
        Error::InvalidArgument,
        "a port one past the end of the range",
    ),
    (
        Syscall::IoPortWrite,
        Error::InvalidArgument,
        "a width that is not one, two, or four bytes",
    ),
    (
        Syscall::MemoryCreateDevice,
        Error::InvalidArgument,
        "an aperture of no frames at all",
    ),
    (
        Syscall::SystemInfo,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ProcessWatch,
        Error::InvalidArgument,
        "a bit index above the sixty-four a notification has",
    ),
    (
        Syscall::ProcessUnwatch,
        Error::InvalidArgument,
        "an invalid watch bit",
    ),
    (
        Syscall::MemoryReferences,
        Error::InvalidHandle,
        "an invalid memory handle",
    ),
    (
        Syscall::IoPortWriteString,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
    (
        Syscall::ClockNow,
        Error::ArgumentCount,
        "an argument word above the none it reads",
    ),
    (
        Syscall::NotificationWaitUntil,
        Error::AccessDenied,
        "a capability that may signal and not wait",
    ),
    (
        Syscall::RandomBytes,
        Error::ArgumentCount,
        "an argument word above the none it reads",
    ),
    (
        Syscall::InterruptCreateMsi,
        Error::InvalidHandle,
        "a handle that names nothing",
    ),
];

/// Every call this phase implements answered exactly one error, and it is
/// the one its failure case asks for. A table as long as the table of
/// calls: nothing is refused for the wrong reason, and nothing is missing.
#[test_case]
fn every_call_of_the_table_fails_from_user_mode_for_a_reason_of_its_own() {
    with_log(|log| {
        for call in Syscall::ALL {
            let Some((_, wanted, what)) = REFUSALS.iter().find(|(named, _, _)| named == call)
            else {
                testing::fail(format_args!(
                    "{} has no failure case in the table of this image",
                    call.name()
                ));
            };
            let mut errors = log.errors(*call);
            let Some(error) = errors.next() else {
                testing::fail(format_args!(
                    "{} never failed from user mode; nothing of it was refused",
                    call.name()
                ));
            };
            if error != *wanted {
                testing::fail(format_args!(
                    "{} answered {error:?} to {what}, not {wanted:?}",
                    call.name()
                ));
            }
            if let Some(second) = errors.next() {
                testing::fail(format_args!(
                    "{} failed twice, the second time with {second:?}; \
                     one failure case per call is what the table holds",
                    call.name()
                ));
            }
        }
        say!(
            "all {} calls of the table refused their one case, each with its own error",
            REFUSALS.len()
        );
    });
}

/// No call of the table answers that the kernel does not have it: Phase 6
/// completes the table, so `Unsupported` is an answer nothing gives.
#[test_case]
fn no_call_of_the_table_answers_that_it_does_not_exist() {
    if !UNIMPLEMENTED.is_empty() {
        testing::fail(format_args!(
            "{} calls are still missing",
            UNIMPLEMENTED.len()
        ));
    }
    with_log(|log| {
        for call in Syscall::ALL {
            for status in log.statuses(*call) {
                if status.error() == Some(Error::Unsupported) {
                    testing::fail(format_args!(
                        "{} answered that the kernel does not have it",
                        call.name()
                    ));
                }
            }
        }
        say!("no call of the table is missing");
    });
}

/// The table and the two tables of this image are the same length: every
/// call has a failure case, and nothing is named twice.
#[test_case]
fn the_table_of_refusals_covers_the_table_of_calls_exactly() {
    if REFUSALS.len() != Syscall::ALL.len() {
        testing::fail(format_args!(
            "{} calls and {} failure cases",
            Syscall::ALL.len(),
            REFUSALS.len()
        ));
    }
    for (index, (call, _, _)) in REFUSALS.iter().enumerate() {
        if REFUSALS
            .iter()
            .skip(index.saturating_add(1))
            .any(|(other, _, _)| other == call)
        {
            testing::fail(format_args!("{} has two failure cases", call.name()));
        }
    }
    say!(
        "{} calls, {} failure cases",
        Syscall::ALL.len(),
        REFUSALS.len()
    );
}
