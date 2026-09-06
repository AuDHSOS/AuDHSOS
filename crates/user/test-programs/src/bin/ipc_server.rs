// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The server half of the endpoint test: four messages received, each
//! answered, and what each of them carried written into the page the test
//! reads.
//!
//! The first message carries four words and a handle to a memory object.
//! The server maps that object into its own address space, writes a word the
//! client will read, and answers. The three after it are the widths the
//! catalog asks for: no words at all, and the widest payload the message
//! area holds. What comes after them is an `ipc_try_recv` on an endpoint
//! nobody is sending on, which answers `WouldBlock` and leaves both queues
//! as they were.
//!
//! The observations go into the page the kernel shares with the test and not
//! into the IPC buffer: a message of four hundred and eighty words fills the
//! message area, and a log kept there would be overwritten by the message it
//! is about.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use audhsos_abi::layout::MAX_MESSAGE_WORDS;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the endpoint is in.
pub const ENDPOINT_WORD: usize = 0;

/// The payload word the handle of the server's own process is in, which is
/// what it maps the memory object it receives through.
pub const PROCESS_WORD: usize = 1;

/// Where the kernel maps the page the server and the test share.
const SHARED: u64 = 0x90_0000;

/// Where the server maps the memory object the client sends it.
const MAPPING: u64 = 0x0100_0000;

/// The word the server writes into that object, which the client reads back
/// through its own mapping.
const WRITTEN: u64 = 0x1234_5678_9ABC_DEF0;

/// One page, as the length argument of `memory_map` takes it.
const PAGE: u64 = 0x1000;

/// Read and write, as `memory_map` takes the permissions.
const READ_WRITE: u64 = 0b01;

/// How many messages the server answers.
const MESSAGES: usize = 3;

/// The words of the shared page the server writes.
const HANDLED: usize = 0;
const BADGE: usize = 1;
const FIRST_COUNT: usize = 2;
const FIRST_WORD: usize = 3;
const LAST_WORD: usize = 4;
const HANDLES: usize = 5;
const MAP_STATUS: usize = 6;
const SECOND_COUNT: usize = 7;
const THIRD_COUNT: usize = 8;
const THIRD_LAST: usize = 9;
const TRY_STATUS: usize = 10;
const DONE: usize = 11;

/// The value the server writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0D0E_5E00;

/// Receives, maps, answers, and ends.
fn main(ipc_buffer: u64) -> ! {
    let endpoint = given(ipc_buffer, ENDPOINT_WORD);
    let process = given(ipc_buffer, PROCESS_WORD);
    let mut reply = 0;
    for index in 0..MESSAGES {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, values) = unsafe { sys::returning(ipc_buffer, Syscall::IpcRecv, &[endpoint]) };
        if status != 0 {
            break;
        }
        reply = values[1];
        let (label, words, handles) = header(ipc_buffer);
        let _ = label;
        match index {
            0 => {
                write(SHARED, BADGE, values[0]);
                write(SHARED, FIRST_COUNT, words);
                write(SHARED, FIRST_WORD, word(ipc_buffer, 0));
                write(SHARED, LAST_WORD, word(ipc_buffer, 3));
                write(SHARED, HANDLES, handles);
                let carried = handle(ipc_buffer, 0);
                // SAFETY: as above.
                let (status, _) = unsafe {
                    sys::call(
                        ipc_buffer,
                        Syscall::MemoryMap,
                        &[process, carried, MAPPING, 0, PAGE, READ_WRITE],
                    )
                };
                write(SHARED, MAP_STATUS, status);
                if status == 0 {
                    write(MAPPING, 0, WRITTEN);
                }
            }
            1 => write(SHARED, SECOND_COUNT, words),
            _ => {
                write(SHARED, THIRD_COUNT, words);
                write(SHARED, THIRD_LAST, word(ipc_buffer, MAX_MESSAGE_WORDS - 1));
            }
        }
        answer(ipc_buffer, reply, index);
        write(
            SHARED,
            HANDLED,
            u64::try_from(index).unwrap_or(0).saturating_add(1),
        );
    }
    let _ = reply;
    // Nobody is sending any more, and a receive that refuses to wait says so.
    // SAFETY: as above.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcTryRecv, &[endpoint]) };
    write(SHARED, TRY_STATUS, status);
    write(SHARED, DONE, FINISHED);
    end(ipc_buffer)
}

/// Answers the message `index` with one word: the number of the message.
fn answer(ipc_buffer: u64, reply: u64, index: usize) {
    // SAFETY: the address is the one the kernel started this thread with,
    // and the borrow ends at the end of the block.
    unsafe {
        let mut buffer = sys::buffer(ipc_buffer);
        buffer.set_label(0x5E_2000);
        let _ = buffer.set_counts(1, 0);
        buffer.set_word(0, u64::try_from(index).unwrap_or(0));
    }
    // SAFETY: as above.
    let _ = unsafe { sys::call(ipc_buffer, Syscall::IpcReply, &[reply]) };
}

/// The label, the word count, and the handle count of the message the
/// buffer holds.
fn header(ipc_buffer: u64) -> (u64, u64, u64) {
    // SAFETY: as `main`; the borrow ends with the expression.
    let view = unsafe { sys::buffer(ipc_buffer) };
    let reader = view.reader();
    reader.message().map_or((0, 0, 0), |message| {
        (
            message.label,
            u64::try_from(message.word_count).unwrap_or(0),
            u64::try_from(message.handle_count).unwrap_or(0),
        )
    })
}

/// The payload word `index` of the message the buffer holds.
fn word(ipc_buffer: u64, index: usize) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(index)
        .unwrap_or(0)
}

/// The raw handle word `index` of the message the buffer holds.
fn handle(ipc_buffer: u64, index: usize) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .handle_word(index)
        .unwrap_or(0)
}

/// The word the kernel left at `index` of the buffer.
fn given(ipc_buffer: u64, index: usize) -> u64 {
    word(ipc_buffer, index)
}

/// Writes `value` at word `index` of the page at `base`.
fn write(base: u64, index: usize, value: u64) {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = base.saturating_add(offset);
    let pointer = core::ptr::without_provenance_mut::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the kernel mapped the page read and write for this process
    // alone, and the index stays inside it.
    unsafe {
        pointer.write_volatile(value);
    }
}

/// Reads word `index` of the page at `base`.
#[expect(
    dead_code,
    reason = "the client half of the pair reads, this one writes"
)]
fn read(base: u64, index: usize) -> u64 {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = base.saturating_add(offset);
    let pointer = core::ptr::without_provenance::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: as `write`.
    unsafe { pointer.read_volatile() }
}

/// Ends the thread. Nothing after this runs.
fn end(ipc_buffer: u64) -> ! {
    loop {
        // SAFETY: as `main`.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
