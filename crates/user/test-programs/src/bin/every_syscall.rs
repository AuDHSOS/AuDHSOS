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
//! Every call of the table gets a failure, and every call whose success one
//! thread can observe gets a success as well, in an order that leaves the
//! destructive ones for the end: what the process kills, it created. The six
//! endpoint calls whose success is a rendezvous are the exception — a single
//! thread that sends with nobody receiving waits for ever — and the `ipc`
//! image covers those with two threads that meet.
//!
//! The last call is `thread_exit`, which writes nothing down: a thread
//! that has ended has nothing to report with.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::ipc_buffer;
use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Rights, Syscall};
use user_rt as _;
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

/// The payload word the handle to the system control capability is in.
pub const CONTROL_WORD: usize = 4;

/// The payload word the first pair goes into. The kernel reads the pairs
/// from here to [`RESULTS_END`].
///
/// Above the thirty words `system_info` writes into the message area of the
/// caller's own buffer, which is what the convention for a result that does
/// not fit into two return words does with it.
pub const FIRST_RESULT: usize = 32;

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

/// The badge the program attaches to a capability of its own endpoint.
const BADGE: u64 = 0x5EED;

/// The ISA line the program makes an interrupt object for. Line zero
/// reaches the I/O APIC through the interrupt source override the tables
/// carry, and nothing of this image ever unmasks it.
const ISA_LINE: u64 = 0;

/// A line the vector plan of the machine reserves no vector for.
const NO_SUCH_LINE: u64 = 200;

/// The first port of the range the program takes: the diagnostic port and
/// the three above it, which no device of this machine drives.
const FIRST_PORT: u64 = 0x80;

/// How many ports that range covers.
const PORT_COUNT: u64 = 4;

/// The word of the `system_info` result the physical address of the
/// framebuffer stands in.
const FRAMEBUFFER_START_WORD: usize = 20;

/// The bit of a notification the interrupt is bound to.
const BOUND_BIT: u64 = 3;

/// A bit index above the sixty-four a notification has.
const NO_SUCH_BIT: u64 = 64;

/// A deadline that has passed on any machine, so that a wait with one never
/// blocks this single thread.
const PAST: u64 = 0;

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
        self.returning(call, arguments)[0]
    }

    /// The same, with both return words: `ipc_recv` answers the badge in the
    /// first and the reply handle in the second.
    fn returning(&mut self, call: Syscall, arguments: &[u64]) -> [u64; 2] {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, values) = unsafe { sys::returning(self.buffer, call, arguments) };
        if self.at.saturating_add(1) < RESULTS_END {
            // SAFETY: as above; the borrow ends at the end of the block.
            unsafe {
                let mut buffer = sys::buffer(self.buffer);
                buffer.set_word(self.at, u64::from(call.number()));
                buffer.set_word(self.at.saturating_add(1), status);
            }
            self.at = self.at.saturating_add(2);
        }
        values
    }

    /// Writes a label and a word count into the message area, which is what
    /// a thread that is about to send does.
    fn set_message(&mut self, label: u64, words: usize) {
        // SAFETY: as `run`; the borrow ends at the end of the block.
        unsafe {
            let mut buffer = sys::buffer(self.buffer);
            buffer.set_label(label);
            let _ = buffer.set_counts(words, 0);
        }
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
    let control = given(ipc_buffer, CONTROL_WORD);
    let mut log = Log {
        buffer: ipc_buffer,
        at: FIRST_RESULT,
    };

    rendezvous(&mut log, process, bad);
    devices(&mut log, control, bad);
    machine(&mut log);
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

/// The endpoint and notification calls. The successes a single thread can
/// observe are here; the six whose success is a rendezvous — a send, a call,
/// two receives, and the two replies — are refused here and succeed in the
/// `ipc` image, where two threads meet.
fn rendezvous(log: &mut Log, process: u64, bad: u64) {
    let endpoint = log.run(Syscall::EndpointCreate, &[]);
    log.run(Syscall::EndpointCreate, &[1]);
    let badged = log.run(Syscall::EndpointBadge, &[endpoint, BADGE]);
    log.run(Syscall::EndpointBadge, &[endpoint, 0]);

    // The endpoint is the fault handler of the process, and then it is not.
    log.run(Syscall::ProcessSetFaultHandler, &[process, endpoint]);
    log.run(Syscall::ProcessSetFaultHandler, &[process, bad]);
    log.run(Syscall::ProcessSetFaultHandler, &[process, 0]);

    // A label of the range the kernel keeps is refused before anything is
    // copied, which is the one way a send is refused without waiting.
    log.set_message(ipc_buffer::KERNEL_LABEL_BASE, 1);
    log.run(Syscall::IpcSend, &[endpoint]);
    log.run(Syscall::IpcCall, &[endpoint]);
    log.set_message(0, 0);

    // A capability with a badge carries `SEND` alone, so it may not receive.
    log.run(Syscall::IpcRecv, &[badged]);
    log.run(Syscall::IpcTryRecv, &[endpoint]);
    log.run(Syscall::IpcReply, &[bad]);
    log.run(Syscall::IpcReplyRecv, &[bad, endpoint]);

    let notification = log.run(Syscall::NotificationCreate, &[]);
    log.run(Syscall::NotificationCreate, &[1]);
    log.run(Syscall::NotificationSignal, &[notification, 0b1010]);
    log.run(Syscall::NotificationSignal, &[bad, 1]);
    // A capability that may signal and not wait, for the two failures below.
    let signal_only = u64::from(Rights::SIGNAL.union(Rights::DUPLICATE).bits());
    let weak = log.run(Syscall::HandleDuplicate, &[notification, signal_only]);
    // The word is not empty, so the wait takes it and does not block.
    log.run(Syscall::NotificationWait, &[notification]);
    log.run(Syscall::NotificationWait, &[weak]);
    log.run(Syscall::NotificationPoll, &[notification]);
    log.run(Syscall::NotificationPoll, &[weak]);
    // A wait with a deadline: the word is not empty, so it takes the bits
    // and the deadline is never consulted; the capability that may not wait
    // is refused for the same reason a plain wait is.
    log.run(Syscall::NotificationSignal, &[notification, 0b0110]);
    log.run(Syscall::NotificationWaitUntil, &[notification, PAST]);
    log.run(Syscall::NotificationWaitUntil, &[weak, PAST]);
    log.set_message(0, 0);
}

/// The three calls that ask the machine itself. None takes a handle, so
/// the one way any is refused is an argument word above what it reads,
/// which the dispatcher answers before the call is reached.
fn machine(log: &mut Log) {
    log.run(Syscall::ClockNow, &[]);
    log.run(Syscall::ClockNow, &[1]);
    log.run(Syscall::ClockWall, &[]);
    log.run(Syscall::ClockWall, &[1]);
    log.run(Syscall::RandomBytes, &[]);
    log.run(Syscall::RandomBytes, &[1]);
    // The four words a seed is sit below the log; the header they left says
    // nothing the calls after them read.
    log.set_message(0, 0);
}

/// The calls that need the root authority: interrupts, port ranges, device
/// memory, and the information about the machine.
fn devices(log: &mut Log, control: u64, bad: u64) {
    let interrupt = log.run(Syscall::InterruptCreate, &[control, ISA_LINE]);
    log.run(Syscall::InterruptCreate, &[control, NO_SUCH_LINE]);
    let notification = log.run(Syscall::NotificationCreate, &[]);
    log.run(
        Syscall::InterruptBind,
        &[interrupt, notification, BOUND_BIT],
    );
    log.run(
        Syscall::InterruptBind,
        &[interrupt, notification, NO_SUCH_BIT],
    );
    log.run(Syscall::InterruptAck, &[interrupt]);
    log.run(Syscall::InterruptAck, &[bad]);

    // A message interrupt, which needs the root authority and no line.
    log.run(Syscall::InterruptCreateMsi, &[control]);
    log.run(Syscall::InterruptCreateMsi, &[bad]);

    let ports = log.run(Syscall::IoPortCreate, &[control, FIRST_PORT, PORT_COUNT]);
    log.run(Syscall::IoPortCreate, &[control, FIRST_PORT, 0]);
    log.run(Syscall::IoPortRead, &[ports, FIRST_PORT, 1]);
    log.run(
        Syscall::IoPortRead,
        &[ports, FIRST_PORT.saturating_add(PORT_COUNT), 1],
    );
    log.run(Syscall::IoPortWrite, &[ports, FIRST_PORT, 1, 0]);
    log.run(Syscall::IoPortWrite, &[ports, FIRST_PORT, 3, 0]);
    // A run of no bytes: the message area of this thread holds the handles
    // the kernel left and the pairs written so far, and none of it is meant
    // for a port.
    log.run(Syscall::IoPortWriteString, &[ports, FIRST_PORT, 0]);
    log.run(Syscall::IoPortWriteString, &[bad, FIRST_PORT, 0]);

    // The framebuffer first: it is the one aperture this program knows the
    // address of, and `system_info` is what tells it.
    log.run(Syscall::SystemInfo, &[control]);
    let framebuffer = given(log.buffer, FRAMEBUFFER_START_WORD).wrapping_div(PAGE_SIZE);
    log.run(Syscall::MemoryCreateDevice, &[control, framebuffer, 1]);
    log.run(Syscall::MemoryCreateDevice, &[control, framebuffer, 0]);
    log.run(Syscall::SystemInfo, &[bad]);
    // The thirty words `system_info` wrote sit below the log; the header
    // it left says nothing the calls after it read.
    log.set_message(0, 0);
}

/// The calls of the memory, thread, process, and handle groups, each once
/// with arguments that work and once with arguments that do not.
/// `thread_exit` is the last of them and comes after this.
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

    // Somebody watching the end of that process: the child holds no thread
    // any more once the one above was killed, so this watch is answered by
    // the end that has already happened.
    let watcher = log.run(Syscall::NotificationCreate, &[]);
    log.run(Syscall::ProcessWatch, &[child, watcher, BOUND_BIT]);
    log.run(Syscall::ProcessWatch, &[child, watcher, NO_SUCH_BIT]);
    log.run(Syscall::ProcessUnwatch, &[child, watcher, BOUND_BIT]);
    log.run(Syscall::ProcessUnwatch, &[child, watcher, NO_SUCH_BIT]);
    log.run(Syscall::ThreadKill, &[bad]);

    // Memory: one object asked about, mapped, protected, unmapped, and
    // split in two. The split comes last, because what it makes is what
    // the handle calls work on.
    log.run(Syscall::MemoryInfo, &[memory]);
    log.run(Syscall::MemoryInfo, &[bad]);
    log.run(Syscall::MemoryReferences, &[memory]);
    log.run(Syscall::MemoryReferences, &[bad]);
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
    // Two objects this program alone holds, cut apart and joined again.
    // `memory` itself cannot be joined to anything: the image holds a
    // handle to it as well, and an object something else holds may not be
    // dissolved under it (D-90).
    let tail = log.run(Syscall::MemorySplit, &[half, PAGE]);
    log.run(Syscall::MemoryMerge, &[half, tail]);
    // And an object joined to itself, which no pair of neighbours is.
    log.run(Syscall::MemoryMerge, &[memory, memory]);

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
