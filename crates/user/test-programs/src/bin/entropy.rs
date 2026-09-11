// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two seeds out of `random_bytes`, and whether they differ.
//!
//! That two draws differ is a smoke test and no statement about the
//! distribution: what it shows is that the call reaches the hardware and
//! that the kernel does not answer one word twice.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// Where the kernel maps the page the program and the test share.
const SHARED: u64 = 0x90_0000;

/// How many words a seed is.
const SEED_WORDS: usize = 4;

/// The words of the shared page.
const FIRST_STATUS: usize = 0;
const SECOND_STATUS: usize = 1;
const COUNT: usize = 2;
const DIFFER: usize = 3;
const FIRST_WORD: usize = 4;
const DONE: usize = 5;

/// The value the program writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0E_71D0;

/// Draws two seeds and reports whether they differ.
fn main(ipc_buffer: u64) -> ! {
    let mut first = [0_u64; SEED_WORDS];
    let mut second = [0_u64; SEED_WORDS];

    // SAFETY: the address is the one the kernel started this thread with,
    // and no other reference to the buffer is alive. Every call below has
    // the same argument.
    let (status, count) = unsafe { sys::call(ipc_buffer, Syscall::RandomBytes, &[]) };
    write(FIRST_STATUS, status);
    write(COUNT, count);
    read_seed(ipc_buffer, &mut first);

    // SAFETY: as above.
    let (status, _) = unsafe { sys::call(ipc_buffer, Syscall::RandomBytes, &[]) };
    write(SECOND_STATUS, status);
    read_seed(ipc_buffer, &mut second);

    write(FIRST_WORD, first[0]);
    write(DIFFER, u64::from(first != second));
    write(DONE, FINISHED);
    end(ipc_buffer)
}

/// Copies the words of the message area into `seed`.
fn read_seed(ipc_buffer: u64, seed: &mut [u64; SEED_WORDS]) {
    // SAFETY: as `main`; the borrow ends with this function.
    let buffer = unsafe { sys::buffer(ipc_buffer) };
    let reader = buffer.reader();
    for (index, word) in seed.iter_mut().enumerate() {
        *word = reader.word(index).unwrap_or(0);
    }
}

/// Writes `value` at word `index` of the shared page.
fn write(index: usize, value: u64) {
    let offset = u64::try_from(index).unwrap_or(0).saturating_mul(8);
    let address = SHARED.saturating_add(offset);
    let pointer = core::ptr::without_provenance_mut::<u64>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the kernel mapped the page read and write for this process
    // alone, and the index stays inside it.
    unsafe {
        pointer.write_volatile(value);
    }
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
