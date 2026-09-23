// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A thread that reports the registers it started with, so that a test can
//! tell that the kernel handed it no kernel address and no kernel data.
//!
//! The entry point is written as `naked_asm!` instead of `sys::entry!`,
//! because a compiled function is free to overwrite a register before its
//! first statement runs.

#![no_std]
#![no_main]
#![allow(unsafe_code)]

use core::arch::naked_asm;

use audhsos_abi::Syscall;
use user_rt as _;
use user_sys_x86_64 as sys;

/// What the thread leaves in the second payload word of its buffer.
pub const MARK: u64 = 0x5EED_0002;

/// Number of registers [`_start`] saves: every general register but `rdi`,
/// which carries the buffer address, and `rsp`, which carries the stack.
pub const SAVED: usize = 14;

/// Saves the registers the thread started with and calls [`report`] with
/// the buffer address, the address of the saved words, and the stack
/// pointer at entry.
///
/// The kernel starts the thread with `rsp + 8` a multiple of 16 (D-193).
/// The saves are fourteen words and `sub rsp, 8` one more, so the `call`
/// leaves the alignment the ABI asks of a callee.
///
/// # Safety
///
/// The kernel starts a thread here once, in user mode, with the address of
/// the thread's IPC buffer in `rdi`.
#[unsafe(naked)]
#[unsafe(no_mangle)]
#[unsafe(link_section = ".text.entry")]
pub unsafe extern "sysv64" fn _start(ipc_buffer: u64) -> ! {
    naked_asm!(
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push r11",
        "push r10",
        "push r9",
        "push r8",
        "push rbp",
        "push rsi",
        "push rdx",
        "push rcx",
        "push rbx",
        "push rax",
        "mov rsi, rsp",
        "lea rdx, [rsp + 112]",
        "sub rsp, 8",
        "call {report}",
        "ud2",
        report = sym report,
    )
}

/// Writes the bitwise or of the saved registers, the mark, and the entry
/// stack pointer modulo 16 into the buffer, then ends the thread.
///
/// The or is one word instead of fourteen, because a test asks one
/// question of it: did the kernel leave anything in a register.
extern "sysv64" fn report(ipc_buffer: u64, saved: *const u64, entry_rsp: u64) -> ! {
    // SAFETY: `_start` pushed `SAVED` words and passed their address, and
    // nothing wrote below them since.
    let words = unsafe { core::slice::from_raw_parts(saved, SAVED) };
    let mut combined = 0_u64;
    for word in words {
        combined |= *word;
    }
    {
        // SAFETY: the address is the one the kernel started this thread
        // with, and the borrow ends before the call.
        let mut buffer = unsafe { sys::buffer(ipc_buffer) };
        buffer.set_word(0, combined);
        buffer.set_word(1, MARK);
        buffer.set_word(2, entry_rsp % 16);
    }
    loop {
        // SAFETY: as above. Nothing after the call runs, but a program of
        // this system never falls off its own end.
        unsafe {
            let _ = sys::call(ipc_buffer, Syscall::ThreadExit, &[]);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // D-193: the thread stops in `Faulted`.
    sys::stop()
}
