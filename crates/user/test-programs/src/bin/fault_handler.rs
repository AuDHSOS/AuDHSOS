// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A fault handler: it receives the fault messages of another process,
//! writes down what each of them said, and answers them.
//!
//! What it does with a fault depends on what the test told it to do, in the
//! words the kernel leaves in its buffer: one for every fault but the last
//! and one for the last. It either answers the fault, which resumes the
//! thread at the instruction it faulted on, or maps a page at the address the
//! fault names and then answers, which is what makes the thread run on, or
//! kills the process the fault came from, which is what a handler does for a
//! thread it cannot repair.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use audhsos_abi::Syscall;
use audhsos_abi::ipc_buffer::fault_kind_of;
use user_sys_x86_64 as sys;

sys::entry!(main);

/// The payload word the handle of the endpoint is in.
pub const ENDPOINT_WORD: usize = 0;

/// The payload word the handle of the faulting process is in.
pub const TARGET_WORD: usize = 1;

/// The payload word the handle of a memory object to map is in.
pub const MEMORY_WORD: usize = 2;

/// The payload word what to do with every fault but the last is in.
pub const ACTION_WORD: usize = 3;

/// How many faults the handler takes.
pub const FAULTS_WORD: usize = 4;

/// The payload word what to do with the last fault is in.
pub const FINAL_WORD: usize = 5;

/// Answer the fault and let the thread try again.
pub const ANSWER: u64 = 0;

/// Kill the process the fault came from.
pub const KILL: u64 = 1;

/// Map a page at the address the fault names and then answer.
pub const REPAIR: u64 = 2;

/// Where the kernel maps the page the handler and the test share.
const SHARED: u64 = 0x90_0000;

/// One page, as the length argument of `memory_map` takes it.
const PAGE: u64 = 0x1000;

/// Read and write, as `memory_map` takes the permissions.
const READ_WRITE: u64 = 0b01;

/// The words of the shared page the handler writes: how many faults it took,
/// and the label, the kind, the address, and the instruction pointer of the
/// first one.
const TAKEN: usize = 0;
const LABEL: usize = 1;
const KIND: usize = 2;
const ADDRESS: usize = 3;
const POINTER: usize = 4;
const DONE: usize = 5;

/// The value the handler writes into [`DONE`] when it has finished.
const FINISHED: u64 = 0x0FA0_17ED;

/// Takes the faults the test asks for and ends.
fn main(ipc_buffer: u64) -> ! {
    let endpoint = given(ipc_buffer, ENDPOINT_WORD);
    let target = given(ipc_buffer, TARGET_WORD);
    let memory = given(ipc_buffer, MEMORY_WORD);
    let action = given(ipc_buffer, ACTION_WORD);
    let wanted = given(ipc_buffer, FAULTS_WORD);
    let last = given(ipc_buffer, FINAL_WORD);

    let mut taken = 0;
    while taken < wanted {
        // SAFETY: the address is the one the kernel started this thread
        // with, and no other reference to the buffer is alive.
        let (status, values) = unsafe { sys::returning(ipc_buffer, Syscall::IpcRecv, &[endpoint]) };
        if status != 0 {
            break;
        }
        let reply = values[1];
        let (label, address, pointer) = fault(ipc_buffer);
        if taken == 0 {
            write(SHARED, LABEL, label);
            write(
                SHARED,
                KIND,
                fault_kind_of(label).map_or(0, |kind| u64::from(kind.code())),
            );
            write(SHARED, ADDRESS, address);
            write(SHARED, POINTER, pointer);
        }
        taken = taken.saturating_add(1);
        write(SHARED, TAKEN, taken);

        // The last fault is the one the test decides differently about: it
        // is where a handler either repairs what the thread fell over or
        // gives up on the process.
        let action = if taken >= wanted { last } else { action };
        if action == KILL {
            // SAFETY: as above.
            unsafe {
                let _ = sys::call(ipc_buffer, Syscall::ProcessKill, &[target]);
            }
            break;
        }
        if action == REPAIR {
            // SAFETY: as above.
            unsafe {
                let _ = sys::call(
                    ipc_buffer,
                    Syscall::MemoryMap,
                    &[target, memory, page_of(address), 0, PAGE, READ_WRITE],
                );
            }
        }
        // An answer of no words at all: what the faulting thread needs is
        // the processor and not a message.
        // SAFETY: as above.
        unsafe {
            let mut buffer = sys::buffer(ipc_buffer);
            buffer.set_label(0);
            let _ = buffer.set_counts(0, 0);
        }
        // SAFETY: as above.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::IpcReply, &[reply]);
        }
    }
    write(SHARED, DONE, FINISHED);
    end(ipc_buffer)
}

/// The label of the message and the two addresses it carries.
fn fault(ipc_buffer: u64) -> (u64, u64, u64) {
    // SAFETY: as `main`; the borrow ends with the expression.
    let view = unsafe { sys::buffer(ipc_buffer) };
    let reader = view.reader();
    let label = reader.message().map_or(0, |message| message.label);
    (
        label,
        reader.word(0).unwrap_or(0),
        reader.word(1).unwrap_or(0),
    )
}

/// The page `address` lies in.
const fn page_of(address: u64) -> u64 {
    address & !(PAGE - 1)
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
