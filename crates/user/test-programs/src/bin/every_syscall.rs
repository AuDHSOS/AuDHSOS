// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every system call of this kernel, made from ring three.
//!
//! The kernel installs four handles in the table of this process and
//! leaves them in the first words of the buffer; the program then walks
//! the whole table of calls and writes down what came back. Every entry it
//! writes is a pair — the number of the call and the status word the
//! kernel answered with — so that the test which reads them does not have
//! to know the order the program went in.
//!
//! Two parts. The calls this phase implements get a success and a failure
//! each, in an order that leaves the destructive ones for the end: what
//! the process kills, it created. The calls of later phases are refused,
//! and the program derives what to pass each of them from the table in
//! `audhsos-abi` — the handle of its own process where the call takes one,
//! nothing where it takes none — so that a call added to the table is
//! covered without a line being written here.
//!
//! The last call is `thread_exit`, which writes nothing down: a thread
//! that has ended has nothing to report with.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::layout::MAX_SYSCALL_ARGUMENTS;
use audhsos_abi::{Rights, Syscall};
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the kernel leaves the handle of this process in.
pub const PROCESS_WORD: usize = 0;

/// The payload word the handle of this thread is in.
pub const THREAD_WORD: usize = 1;

/// The payload word the handle of a memory object is in.
pub const MEMORY_WORD: usize = 2;

/// The payload word a handle that names nothing is in.
pub const BAD_WORD: usize = 3;

/// The payload word the first pair goes into. The kernel reads the pairs
/// from here to [`RESULTS_END`].
pub const FIRST_RESULT: usize = 8;

/// The payload word past the last pair the program may write.
pub const RESULTS_END: usize = 400;

/// Where the child process maps its program and stands its stack. Nothing
/// runs there: the thread this program creates is killed before it is ever
/// switched to.
const CHILD_ENTRY: u64 = 0x40_0000;

/// The stack of that thread.
const CHILD_STACK: u64 = 0x80_0000;

/// Where this process maps a memory object of its own, above everything
/// the kernel mapped for it.
const MAPPING: u64 = 0x0100_0000;

/// An address of the user half that nothing is mapped at.
const NOTHING_MAPPED: u64 = 0x0200_0000;

/// One page, as the length arguments of the memory calls take it.
const PAGE: u64 = 0x1000;

/// Read and write, as `memory_map` and `memory_protect` take them: bit
/// zero is writable and bit one executable. Every mapping is readable.
const READ_WRITE: u64 = 0b01;

/// The rights of a handle this program installs or duplicates: what a
/// memory object is worth passing on with. Never more than the handle the
/// kernel gave the program carries, because rights only ever shrink.
const MEMORY_RIGHTS: Rights = Rights::READ
    .union(Rights::WRITE)
    .union(Rights::MAP)
    .union(Rights::INFO)
    .union(Rights::DUPLICATE)
    .union(Rights::TRANSFER);

/// What the program writes down, and where it is.
struct Log {
    /// The buffer of the thread.
    buffer: u64,
    /// The next word a pair goes into.
    at: usize,
}

impl Log {
    /// Makes `call` with `arguments` and writes the pair down. Returns the
    /// first return word, which is a handle for the calls that make one.
    fn run(&mut self, call: Syscall, arguments: &[u64]) -> u64 {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, value) = unsafe { sys::call(self.buffer, call, arguments) };
        if self.at.saturating_add(1) < RESULTS_END {
            // SAFETY: as above; the borrow ends at the end of the block.
            unsafe {
                let mut buffer = sys::buffer(self.buffer);
                buffer.set_word(self.at, u64::from(call.number()));
                buffer.set_word(self.at.saturating_add(1), status);
            }
            self.at = self.at.saturating_add(2);
        }
        value
    }
}

/// Reads the word the kernel left at `index`.
fn given(buffer: u64, index: usize) -> u64 {
    // SAFETY: the address is the one the kernel started this thread with,
    // and the borrow ends with the expression.
    unsafe { sys::buffer(buffer) }
        .reader()
        .word(index)
        .unwrap_or(0)
}

/// Walks the whole table of calls and ends.
fn main(ipc_buffer: u64) -> ! {
    let process = given(ipc_buffer, PROCESS_WORD);
    let thread = given(ipc_buffer, THREAD_WORD);
    let memory = given(ipc_buffer, MEMORY_WORD);
    let bad = given(ipc_buffer, BAD_WORD);
    let mut log = Log {
        buffer: ipc_buffer,
        at: FIRST_RESULT,
    };

    refused(&mut log, process);
    implemented(&mut log, process, thread, memory, bad);

    // `thread_exit` last, and its failure before its success: a call whose
    // arguments do not check never reaches the call itself, so the thread
    // is still there to make the one that does.
    log.run(Syscall::ThreadExit, &[1]);
    loop {
        // SAFETY: the address is the one the kernel started this thread
        // with. Nothing after the call runs, but a program of this system
        // never falls off its own end.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

/// Every call of a later phase, with what the table of the interface says
/// to pass it: the handle of this process where the call takes one, and
/// nothing where it takes none.
fn refused(log: &mut Log, process: u64) {
    for call in Syscall::ALL {
        if !is_of_a_later_phase(*call) {
            continue;
        }
        let mut arguments = [0_u64; MAX_SYSCALL_ARGUMENTS];
        if call.takes_handle()
            && let Some(first) = arguments.first_mut()
        {
            *first = process;
        }
        let count = usize::from(call.argument_count()).min(MAX_SYSCALL_ARGUMENTS);
        log.run(*call, arguments.get(..count).unwrap_or(&[]));
    }
}

/// `true` for the twenty-one calls Phase 6 owns. The list is the one in
/// `kernel_syscall::calls::UNIMPLEMENTED`, which the kernel side of the
/// test holds against it.
const fn is_of_a_later_phase(call: Syscall) -> bool {
    matches!(
        call,
        Syscall::ProcessSetFaultHandler
            | Syscall::EndpointCreate
            | Syscall::EndpointBadge
            | Syscall::IpcCall
            | Syscall::IpcSend
            | Syscall::IpcRecv
            | Syscall::IpcTryRecv
            | Syscall::IpcReply
            | Syscall::IpcReplyRecv
            | Syscall::NotificationCreate
            | Syscall::NotificationSignal
            | Syscall::NotificationWait
            | Syscall::NotificationPoll
            | Syscall::InterruptCreate
            | Syscall::InterruptBind
            | Syscall::InterruptAck
            | Syscall::IoPortCreate
            | Syscall::IoPortRead
            | Syscall::IoPortWrite
            | Syscall::MemoryCreateDevice
            | Syscall::SystemInfo
    )
}

/// The nineteen calls this phase implements that a thread survives, each
/// once with arguments that work and once with arguments that do not.
/// `thread_exit` is the twentieth and comes after this.
fn implemented(log: &mut Log, process: u64, thread: u64, memory: u64, bad: u64) {
    // The rights word as a call takes it: a plain number in the buffer.
    let rights = u64::from(MEMORY_RIGHTS.bits());

    // What needs nothing but itself.
    log.run(Syscall::DebugLog, &[]);
    log.run(Syscall::DebugLog, &[1]);
    log.run(Syscall::ThreadYield, &[]);
    log.run(Syscall::ThreadYield, &[1]);
    log.run(Syscall::ThreadInfo, &[thread]);
    log.run(Syscall::ThreadInfo, &[bad]);

    // A process of its own to do the destructive things to.
    let child = log.run(Syscall::ProcessCreate, &[process, 8, 8, 8, 0]);
    log.run(Syscall::ProcessCreate, &[bad, 8, 8, 8, 0]);
    log.run(Syscall::ProcessInstallHandle, &[child, memory, rights]);
    log.run(Syscall::ProcessInstallHandle, &[child, bad, rights]);

    // A thread of that process, which never runs: it is created, moved
    // through the states its calls name, and killed.
    let born = log.run(
        Syscall::ThreadCreate,
        &[child, CHILD_ENTRY, CHILD_STACK, 2, 4, 0],
    );
    log.run(
        Syscall::ThreadCreate,
        &[child, CHILD_ENTRY, CHILD_STACK, 9, 4, 0],
    );
    log.run(Syscall::ThreadStart, &[born]);
    log.run(Syscall::ThreadStart, &[born]);
    log.run(Syscall::ThreadSuspend, &[born]);
    log.run(Syscall::ThreadSuspend, &[bad]);
    log.run(Syscall::ThreadResume, &[born]);
    log.run(Syscall::ThreadResume, &[born]);
    log.run(Syscall::ThreadSetPriority, &[born, 3]);
    log.run(Syscall::ThreadSetPriority, &[born, 200]);
    log.run(Syscall::ThreadKill, &[born]);
    log.run(Syscall::ThreadKill, &[bad]);

    // Memory: one object asked about, mapped, protected, unmapped, and
    // split in two. The split comes last, because what it makes is what
    // the handle calls work on.
    log.run(Syscall::MemoryInfo, &[memory]);
    log.run(Syscall::MemoryInfo, &[bad]);
    log.run(
        Syscall::MemoryMap,
        &[process, memory, MAPPING, 0, PAGE, READ_WRITE],
    );
    log.run(
        Syscall::MemoryMap,
        &[process, memory, MAPPING, 0, PAGE, READ_WRITE],
    );
    log.run(Syscall::MemoryProtect, &[process, MAPPING, PAGE, 0]);
    log.run(Syscall::MemoryProtect, &[process, NOTHING_MAPPED, PAGE, 0]);
    log.run(Syscall::MemoryUnmap, &[process, MAPPING, PAGE]);
    log.run(Syscall::MemoryUnmap, &[process, NOTHING_MAPPED, PAGE]);
    let half = log.run(Syscall::MemorySplit, &[memory, PAGE.saturating_mul(2)]);
    log.run(Syscall::MemorySplit, &[memory, 1]);

    // Handles, and the process that held them.
    let copy = log.run(Syscall::HandleDuplicate, &[half, rights]);
    log.run(Syscall::HandleDuplicate, &[bad, rights]);
    log.run(Syscall::HandleClose, &[copy]);
    log.run(Syscall::HandleClose, &[bad]);
    log.run(Syscall::ProcessKill, &[child]);
    log.run(Syscall::ProcessKill, &[bad]);
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
