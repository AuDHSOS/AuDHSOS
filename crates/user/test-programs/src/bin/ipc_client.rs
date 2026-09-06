// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The client half of the endpoint test: a call through a badged capability
//! with four words and a handle, then the two widths the catalog asks for,
//! then one word too many.
//!
//! What the server wrote into the memory object the first message carried is
//! read back through this process's own mapping of it, so the test sees the
//! handle arrive by what it makes possible and not only by a count.
//!
//! The observations go into the page the kernel shares with the test, for the
//! reason the server has: a message of four hundred and eighty words fills
//! the message area, and a log kept there would be overwritten.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use audhsos_abi::layout::{MAX_MESSAGE_WORDS, PAGE_SIZE};
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the badged handle of the endpoint is in.
pub const ENDPOINT_WORD: usize = 0;

/// The payload word the handle of the memory object is in.
pub const MEMORY_WORD: usize = 1;

/// Where the kernel maps the page the client and the test share.
const SHARED: u64 = 0x90_0000;

/// Where the kernel mapped the memory object for this process, and where the
/// client reads what the server wrote into it.
const MAPPING: u64 = 0x0100_0000;

/// The label the client sends with.
const LABEL: u64 = 0xC1_1E00;

/// The words of the shared page the client writes.
const FIRST_STATUS: usize = 0;
const FIRST_REPLY: usize = 1;
const SEEN: usize = 2;
const EMPTY_STATUS: usize = 3;
const WIDEST_STATUS: usize = 4;
const TOO_WIDE_STATUS: usize = 5;
const DONE: usize = 6;

/// The value the client writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0C_1DEA;

/// Calls, reads, calls again, and ends.
fn main(ipc_buffer: u64) -> ! {
    let endpoint = given(ipc_buffer, ENDPOINT_WORD);
    let memory = given(ipc_buffer, MEMORY_WORD);

    // Four words and a handle to the memory object.
    set_message(ipc_buffer, 4, &[memory]);
    for index in 0..4 {
        set_word(
            ipc_buffer,
            index,
            u64::try_from(index).unwrap_or(0).saturating_add(1),
        );
    }
    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcCall, &[endpoint]) };
    write(SHARED, FIRST_STATUS, status);
    write(SHARED, FIRST_REPLY, reply_word(ipc_buffer));
    write(SHARED, SEEN, read(MAPPING, 0));

    // A message of no words at all.
    set_message(ipc_buffer, 0, &[]);
    // SAFETY: as above.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcCall, &[endpoint]) };
    write(SHARED, EMPTY_STATUS, status);

    // The widest payload the message area holds.
    set_message(ipc_buffer, MAX_MESSAGE_WORDS, &[]);
    set_word(
        ipc_buffer,
        MAX_MESSAGE_WORDS.saturating_sub(1),
        u64::try_from(MAX_MESSAGE_WORDS).unwrap_or(0),
    );
    // SAFETY: as above.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcCall, &[endpoint]) };
    write(SHARED, WIDEST_STATUS, status);

    // One word more than the area holds, written by hand: the setter refuses
    // the count, and a thread that wrote its own header is what the kernel
    // has to answer.
    set_message(ipc_buffer, 0, &[]);
    set_raw_count(
        ipc_buffer,
        u64::try_from(MAX_MESSAGE_WORDS)
            .unwrap_or(0)
            .saturating_add(1),
    );
    // SAFETY: as above.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::IpcCall, &[endpoint]) };
    write(SHARED, TOO_WIDE_STATUS, status);

    write(SHARED, DONE, FINISHED);
    end(ipc_buffer)
}

/// Writes the header of the message the client is about to send.
fn set_message(ipc_buffer: u64, words: usize, handles: &[u64]) {
    // SAFETY: as `main`; the borrow ends at the end of the block.
    unsafe {
        let mut buffer = sys::buffer(ipc_buffer);
        buffer.set_label(LABEL);
        let _ = buffer.set_counts(words, handles.len());
        for (index, handle) in handles.iter().enumerate() {
            buffer.set_handle_word(index, *handle);
        }
    }
}

/// Writes the payload word `index`.
fn set_word(ipc_buffer: u64, index: usize, value: u64) {
    // SAFETY: as `main`.
    unsafe {
        sys::buffer(ipc_buffer).set_word(index, value);
    }
}

/// Writes the word count of the header by hand, past what the setter allows.
fn set_raw_count(ipc_buffer: u64, count: u64) {
    let offset = u64::try_from(audhsos_abi::ipc_buffer::WORD_COUNT).unwrap_or(0);
    let address = ipc_buffer.saturating_add(offset);
    let pointer = core::ptr::without_provenance_mut::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the offset is inside the page the kernel mapped for this
    // thread, read and write.
    unsafe {
        pointer.write_volatile(count);
    }
}

/// The first payload word of the answer the buffer holds.
fn reply_word(ipc_buffer: u64) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(0)
        .unwrap_or(0)
}

/// The word the kernel left at `index` of the buffer.
fn given(ipc_buffer: u64, index: usize) -> u64 {
    // SAFETY: as `main`.
    unsafe { sys::buffer(ipc_buffer) }
        .reader()
        .word(index)
        .unwrap_or(0)
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

const _: () = assert!(PAGE_SIZE == 4096);

#[panic_handler]
const fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Nothing of this program panics; the handler is what the language
    // asks for, and a thread that reached it has nothing left to do.
    loop {}
}
