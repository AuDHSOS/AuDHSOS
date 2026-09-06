// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![no_std]
#![allow(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![doc = include_str!("../README.md")]

pub mod gate;
pub mod log;

use core::arch::asm;

use audhsos_abi::Syscall;
use audhsos_abi::ipc_buffer::{BufferMut, SIZE};

pub use gate::{Gate, MemoryInfo, Received, SYSTEM_INFO_WORDS, SystemInfo, ThreadInfo};
pub use log::write_line;
pub use user_rt::Startup;

/// Makes the system call the IPC buffer describes and returns when the
/// kernel has written its answer into the same buffer.
///
/// # Safety
///
/// The buffer of the calling thread must hold a call number and the
/// arguments that call reads. Everything the kernel does with them it
/// checks; a buffer holding anything at all is answered with an error and
/// never with a fault.
pub unsafe fn syscall() {
    // SAFETY: the vector is the one gate of the interrupt table a user
    // thread may enter, and the kernel reads and writes only the buffer of
    // the calling thread. Nothing of this thread's own state is touched:
    // the interrupt uses the kernel stack the task state segment names.
    unsafe {
        asm!("int 0x80", options(nostack));
    }
}

/// The IPC buffer of the calling thread, as the kernel handed it over.
///
/// # Safety
///
/// `address` must be what the kernel put in the first argument register at
/// the start of this thread, and no other reference to the buffer may be
/// alive.
#[must_use]
pub unsafe fn buffer<'a>(address: u64) -> BufferMut<'a> {
    let pointer =
        core::ptr::without_provenance_mut::<[u8; SIZE]>(usize::try_from(address).unwrap_or(0));
    // SAFETY: the kernel mapped one page of memory there, read and write,
    // for this thread alone, and the caller promises this is the only
    // reference to it.
    BufferMut::new(unsafe { &mut *pointer })
}

/// Makes the call `number` with `arguments` and returns the status word
/// and the first return word.
///
/// # Safety
///
/// As [`syscall`] and [`buffer`].
#[must_use]
pub unsafe fn call(address: u64, number: Syscall, arguments: &[u64]) -> (u64, u64) {
    // SAFETY: the caller promises what `returning` requires.
    let (status, values) = unsafe { returning(address, number, arguments) };
    (status, values[0])
}

/// Makes the call `number` with `arguments` and returns the status word and
/// both return words. `ipc_recv` needs the second one, which carries the
/// reply handle.
///
/// # Safety
///
/// As [`syscall`] and [`buffer`].
#[must_use]
pub unsafe fn returning(address: u64, number: Syscall, arguments: &[u64]) -> (u64, [u64; 2]) {
    {
        // SAFETY: the caller promises the address is the one the kernel
        // gave this thread; the borrow ends before the call.
        let mut writer = unsafe { buffer(address) };
        writer.set_syscall_number(u64::from(number.number()));
        for index in 0..audhsos_abi::layout::MAX_SYSCALL_ARGUMENTS {
            writer.set_argument(index, arguments.get(index).copied().unwrap_or(0));
        }
    }
    // SAFETY: the buffer now holds a call and its arguments.
    unsafe {
        syscall();
    }
    // SAFETY: as above; the kernel has written its answer.
    let reader = unsafe { buffer(address) };
    let view = reader.reader();
    (
        view.status().map_or(u64::MAX, audhsos_abi::Status::raw),
        [
            view.return_word(0).unwrap_or(0),
            view.return_word(1).unwrap_or(0),
        ],
    )
}

/// The startup message that stands in the buffer of `gate`.
///
/// A message that is none, or one whose pairs do not read, gives a startup
/// that was given nothing — which is what a program started without one was
/// in fact given. The distinction between an empty message and a broken one
/// has no consequence a program could act on.
///
/// It must be read before the first system call, because a call writes its
/// own arguments over the message.
#[must_use]
pub fn startup(gate: &Gate) -> Startup {
    Startup::read(gate.reader()).unwrap_or_default()
}

/// Declares the entry point of a user program that works through a gate.
///
/// The function it names is called once and never returns; it is given the
/// gate of the thread and what the process was given at its start, both by
/// value, because a program owns its gate and there is exactly one. A
/// program that has nothing left to do ends its thread with
/// [`Gate::thread_exit`].
///
/// The macro writes the panic handler too. It halts: the workspace lint set
/// denies `panic!`, `unwrap`, `expect`, indexing, and unchecked arithmetic
/// in product code, so nothing of a program of this system reaches it, and
/// a handler that made a system call would need the buffer of the thread
/// that panicked, which the language hands it no way of naming.
#[macro_export]
macro_rules! program {
    ($main:path) => {
        /// The address the kernel starts the thread at.
        ///
        /// The section is what puts it at the front of the program. An
        /// ELF names its entry in its header, so this is not what finds
        /// it — but the kernel insists that the entry of the root task is
        /// the first byte of the image it maps at `ROOT_TASK_BASE`, and
        /// this is what makes that true. For the programs of the archive
        /// it is readability: the one function the kernel jumps to stands
        /// where a reader of a disassembly looks for it first.
        ///
        /// # Safety
        ///
        /// The kernel calls this once per thread, in user mode, with the
        /// address of the thread's IPC buffer in the first argument.
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".text.entry")]
        pub unsafe extern "sysv64" fn _start(ipc_buffer: u64) -> ! {
            // SAFETY: the kernel starts this thread once and puts the
            // address of its buffer in the first argument register; this
            // is the only gate over it, because it is the only one built.
            let gate = unsafe { $crate::Gate::adopt(ipc_buffer) };
            let startup = $crate::startup(&gate);
            let main: fn($crate::Gate, $crate::Startup) -> ! = $main;
            main(gate, startup)
        }

        #[panic_handler]
        const fn panic(_info: &core::panic::PanicInfo) -> ! {
            loop {}
        }
    };
}

/// Declares the entry point of a user program that works the buffer
/// directly.
///
/// The function it names receives the address of the IPC buffer of the
/// thread and never returns. This is what the programs of the kernel test
/// images use: they make calls the gate has no method for on purpose —
/// numbers that name nothing, arguments the kernel has to refuse — and a
/// typed wrapper is the wrong tool for that.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        /// The address the kernel starts the thread at.
        ///
        /// The section is what puts it at the front of the program: the
        /// kernel maps a flat binary and jumps at its first byte, so the
        /// entry point has to be that byte and not wherever the linker
        /// would otherwise have put it.
        ///
        /// # Safety
        ///
        /// The kernel calls this once per thread, in user mode, with the
        /// address of the thread's IPC buffer in the first argument.
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".text.entry")]
        pub unsafe extern "sysv64" fn _start(ipc_buffer: u64) -> ! {
            let main: fn(u64) -> ! = $main;
            main(ipc_buffer)
        }
    };
}
