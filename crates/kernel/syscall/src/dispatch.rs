// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one entry point: what a thread asked for, what the kernel checks,
//! and what it writes back.
//!
//! Invariants: the checks run in the order of
//! [2.8](../../../docs/02-architecture.md) — number, argument count,
//! handle, object type, rights, arguments, quota — and the first failing
//! check decides the error, so an error never depends on what a later
//! check would have found; a call that fails changes nothing; the status
//! word and the return words are written exactly once per call.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE, Status};
use audhsos_abi::layout::MAX_RESULT_WORDS;
use audhsos_abi::{Error, FirstArgument, Handle, Rights, Syscall};
use kernel_objects::object::{ProcessId, ThreadId};
use kernel_objects::store::Objects;
use kernel_sched::{Outcome, Scheduler};

use crate::calls;
use crate::environment::Environment;

/// What the kernel hands the call functions: the objects, the scheduler,
/// and everything that is not an object.
#[derive(Debug)]
pub struct Machine<
    'a,
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
> {
    /// The pools and the handle arena.
    pub objects: &'a mut Objects<NP, NT, NM, NH>,
    /// The run queues.
    pub scheduler: &'a mut Scheduler,
    /// Address spaces, stacks, frames, and the console.
    pub environment: &'a mut E,
}

/// What a call leaves for its caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The return words, written whether the call fills them or not.
    pub values: [u64; 2],
    /// The words of a result that does not fit into the return words. They
    /// go into the message area of the caller's own buffer, with label zero
    /// and handle count zero.
    pub words: [u64; MAX_RESULT_WORDS],
    /// How many of them the call wrote, or `None` for a call that leaves the
    /// message area of its caller alone. `Some(0)` is a call that uses the
    /// convention and had nothing to say, which is what `thread_info`
    /// reports for a thread that did not fault.
    pub word_count: Option<u8>,
    /// The call did part of its work; the first return word says how much.
    pub partial: bool,
    /// The call blocked its caller, which therefore has no result yet: the
    /// dispatcher writes neither the status word nor the return words, and
    /// the thread that completes the rendezvous writes them later.
    pub blocked: bool,
    /// What the caller should do before it returns to user mode.
    pub outcome: Outcome,
}

impl Default for Reply {
    fn default() -> Self {
        Reply::DONE
    }
}

impl Reply {
    /// A call that returns nothing and changes nobody's turn.
    pub const DONE: Reply = Reply {
        values: [0, 0],
        words: [0; MAX_RESULT_WORDS],
        word_count: None,
        partial: false,
        blocked: false,
        outcome: Outcome::NOTHING,
    };

    /// A call whose caller blocks and asks for a switch. Nothing of the
    /// result area is written: there is no result yet.
    pub const BLOCKED: Reply = Reply {
        blocked: true,
        outcome: Outcome::RESCHEDULE,
        ..Reply::DONE
    };

    /// A call that returns one value.
    #[must_use]
    pub const fn value(value: u64) -> Self {
        Reply {
            values: [value, 0],
            ..Reply::DONE
        }
    }

    /// A call that returns two values.
    #[must_use]
    pub const fn values(first: u64, second: u64) -> Self {
        Reply {
            values: [first, second],
            ..Reply::DONE
        }
    }

    /// The same reply, asking the caller to switch threads.
    #[must_use]
    pub const fn reschedule(self) -> Self {
        Reply {
            outcome: Outcome::RESCHEDULE,
            ..self
        }
    }

    /// The same reply with the outcome a scheduler operation returned.
    #[must_use]
    pub const fn after(self, outcome: Outcome) -> Self {
        Reply { outcome, ..self }
    }

    /// A call that did part of its work; `progress` says how much.
    #[must_use]
    pub const fn partial(progress: u64) -> Self {
        Reply {
            values: [progress, 0],
            partial: true,
            ..Reply::DONE
        }
    }

    /// A result of `words` in the message area of the caller's own buffer,
    /// with the count in the first return word. That is the convention for
    /// every result that does not fit into two words; `system_info` uses it.
    #[must_use]
    pub fn message(words: &[u64]) -> Self {
        let reply = Reply::DONE.with_words(words);
        let count = reply.word_count.unwrap_or(0);
        Reply {
            values: [u64::from(count), 0],
            ..reply
        }
    }

    /// The same reply with `words` in the message area of the caller's own
    /// buffer and the return words left as they are. `thread_info` uses it:
    /// its two return words are the state and the fault kind, so the count
    /// of the message travels in the header of the message itself.
    #[must_use]
    pub fn with_words(self, words: &[u64]) -> Self {
        let mut reply = self;
        let count = words.len().min(MAX_RESULT_WORDS);
        for (slot, word) in reply.words.iter_mut().zip(words.iter()).take(count) {
            *slot = *word;
        }
        reply.word_count = Some(u8::try_from(count).unwrap_or(0));
        reply
    }
}

/// The rights the first handle of `call` must carry. A call whose first
/// argument is no handle, and one that checks its rights itself, needs
/// none here.
#[must_use]
pub const fn required_rights(call: Syscall) -> Rights {
    match call {
        // The creator manages the process it creates from.
        Syscall::ProcessCreate
        | Syscall::ProcessSetFaultHandler
        | Syscall::ProcessKill
        | Syscall::ThreadCreate
        | Syscall::ThreadStart
        | Syscall::ThreadSuspend
        | Syscall::ThreadResume
        | Syscall::ThreadKill
        | Syscall::ThreadSetPriority
        | Syscall::ThreadInfo
        | Syscall::InterruptCreate
        | Syscall::InterruptCreateMsi
        | Syscall::InterruptBind
        | Syscall::InterruptAck
        | Syscall::IoPortCreate
        | Syscall::MemoryCreateDevice
        | Syscall::SystemInfo => Rights::MANAGE,
        // Installing a handle into a process is its own right.
        Syscall::ProcessInstallHandle => Rights::INSTALL,
        // A mapping is a change to an address space, and splitting an
        // object is a change to what may be mapped.
        Syscall::MemoryMap
        | Syscall::MemoryUnmap
        | Syscall::MemoryProtect
        | Syscall::MemorySplit
        | Syscall::MemoryMerge => Rights::MAP,
        // Reading what an object is, and hearing that a process ended, are
        // both learning something about it and nothing more.
        Syscall::MemoryInfo
        | Syscall::ProcessWatch
        | Syscall::ProcessUnwatch
        | Syscall::MemoryReferences => Rights::INFO,
        Syscall::EndpointBadge => Rights::BADGE,
        Syscall::IpcSend | Syscall::IpcCall => Rights::SEND,
        Syscall::IpcRecv | Syscall::IpcTryRecv => Rights::RECV,
        Syscall::NotificationSignal => Rights::SIGNAL,
        Syscall::NotificationWait | Syscall::NotificationWaitUntil | Syscall::NotificationPoll => {
            Rights::WAIT
        }
        Syscall::IoPortRead => Rights::READ,
        Syscall::IoPortWrite | Syscall::IoPortWriteString => Rights::WRITE,
        // `handle_duplicate` checks `DUPLICATE` against the rights it is
        // asked for, `handle_close` needs nothing, a reply object carries
        // no rights, and the rest takes no handle.
        Syscall::HandleDuplicate
        | Syscall::HandleClose
        | Syscall::IpcReply
        | Syscall::IpcReplyRecv
        | Syscall::ThreadExit
        | Syscall::ThreadYield
        | Syscall::EndpointCreate
        | Syscall::NotificationCreate
        | Syscall::ClockNow
        | Syscall::RandomBytes
        | Syscall::DebugLog => Rights::EMPTY,
    }
}

/// What the checks before the call itself produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// The call the number named.
    pub call: Syscall,
    /// The argument words, all six of them; the words above the argument
    /// count of the call are zero, which is checked.
    pub arguments: [u64; 6],
    /// The handle of the first argument, for a call that takes one.
    pub handle: Option<Handle>,
}

impl Request {
    /// Argument `index`, or zero beyond the six the buffer holds.
    #[must_use]
    pub fn argument(&self, index: usize) -> u64 {
        self.arguments.get(index).copied().unwrap_or(0)
    }
}

/// Reads the call and its arguments out of `buffer` and runs the checks
/// that do not need an object: the number and the argument count.
///
/// # Errors
///
/// [`Error::UnknownSyscall`] for a number that names no call;
/// [`Error::ArgumentCount`] when an argument word above what the call
/// reads is not zero, which is what a caller built against another version
/// of the table looks like.
pub fn decode(buffer: &Buffer<'_>) -> Result<Request, Error> {
    let call = Syscall::from_word(buffer.syscall_number())?;
    let arguments = buffer.arguments();
    let count = usize::from(call.argument_count());
    if arguments.iter().skip(count).any(|word| *word != 0) {
        return Err(Error::ArgumentCount);
    }
    let handle = if call.takes_handle() {
        Some(
            Handle::from_raw(arguments.first().copied().unwrap_or(0))
                .ok_or(Error::InvalidHandle)?,
        )
    } else {
        None
    };
    Ok(Request {
        call,
        arguments,
        handle,
    })
}

/// Runs the checks that need the objects: the handle, the type of what it
/// names, and the rights it carries.
fn check_capability<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &Machine<'_, E, NP, NT, NM, NH>,
    process: ProcessId,
    request: &Request,
) -> Result<(), Error> {
    let Some(handle) = request.handle else {
        return Ok(());
    };
    let entry = machine.objects.entry(process, handle)?;
    if let FirstArgument::Object(expected) = request.call.first_argument()
        && entry.object_type().code() != expected.code()
    {
        return Err(Error::WrongObjectType);
    }
    let required = required_rights(request.call);
    if !entry.rights.contains(required) {
        return Err(Error::AccessDenied);
    }
    Ok(())
}

/// Handles one system call of `caller` and writes the result into its IPC
/// buffer. The return value says whether the caller should switch threads
/// before it returns to user mode.
pub fn dispatch<
    E: Environment,
    const NP: usize,
    const NT: usize,
    const NM: usize,
    const NH: usize,
>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    buffer: &mut [u8; SIZE],
) -> Outcome {
    let result = handle(machine, caller, buffer);
    // A caller that blocked has no result yet, so nothing of its result area
    // is touched: the thread that completes the rendezvous writes it.
    if let Ok(reply) = result
        && reply.blocked
    {
        return reply.outcome;
    }
    let mut writer = BufferMut::new(buffer);
    writer.clear_result();
    match result {
        Ok(reply) => {
            writer.set_status(if reply.partial {
                Status::PARTIAL
            } else {
                Status::OK
            });
            for (index, value) in reply.values.iter().enumerate() {
                writer.set_return_word(index, *value);
            }
            write_message(&mut writer, &reply);
            reply.outcome
        }
        Err(error) => {
            writer.set_status(Status::failed(error));
            Outcome::NOTHING
        }
    }
}

/// Writes the words of a result that did not fit into the return words:
/// label zero, handle count zero, and the words themselves.
fn write_message(writer: &mut BufferMut<'_>, reply: &Reply) {
    let Some(count) = reply.word_count else {
        return;
    };
    let count = usize::from(count);
    writer.set_label(0);
    let _ = writer.set_counts(count, 0);
    for (index, word) in reply.words.iter().enumerate().take(count) {
        writer.set_word(index, *word);
    }
}

/// The call itself, before its result is written.
fn handle<E: Environment, const NP: usize, const NT: usize, const NM: usize, const NH: usize>(
    machine: &mut Machine<'_, E, NP, NT, NM, NH>,
    caller: ThreadId,
    buffer: &mut [u8; SIZE],
) -> Result<Reply, Error> {
    let process = machine
        .objects
        .threads
        .get(caller)
        .map_err(|_| Error::InvalidHandle)?
        .process;
    let request = decode(&Buffer::new(buffer))?;
    check_capability(machine, process, &request)?;
    calls::run(machine, caller, process, &request, buffer)
}
