// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The switch between two threads, and the way into user mode.
//!
//! Invariants: the two naked functions and
//! [`kernel_x86_tables::context`] agree on the order of the words on a
//! kernel stack, and nothing else depends on it; a switch leaves the
//! callee-saved registers of the thread it returns to exactly as that
//! thread left them; the trampoline runs on the stack the switch handed
//! it and returns to user mode through the frame lying there.

use core::arch::naked_asm;

use kernel_types::{PhysFrame, VirtAddr};
use kernel_x86_tables::context::{FRAME_WORDS, prepare_user as prepare_frame};

use crate::window::PhysicalWindow;

/// Saves the callee-saved registers of the running thread, writes its
/// stack pointer into `from`, loads the one in `to`, and returns onto the
/// stack of the other thread.
///
/// The two threads meet in the six words this pushes and pops: the thread
/// that returns here left them in the same order.
///
/// # Safety
///
/// `from` must be writable, and `to` must be a stack pointer this function
/// wrote, or a frame [`prepare_user`] built, of a stack that is mapped in
/// every address space the processor may be in when it returns.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn switch(from: *mut u64, to: u64) {
    naked_asm!(
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret"
    )
}

/// Switches from the thread whose saved context lives at `from` to the one
/// whose context is `to`.
///
/// This is [`switch`] with the two words named as what they are. The
/// pointer is a pointer into the thread pool of the kernel, which lives in
/// a `static` and therefore stays where it is across the switch.
///
/// # Safety
///
/// `from` must point at the context word of the thread that is running,
/// and `to` must be a context [`switch`] wrote or [`prepare_user`] built,
/// of a thread whose kernel stack is mapped in the address space the
/// processor is in.
pub unsafe fn switch_to(from: *mut VirtAddr, to: VirtAddr) {
    // SAFETY: `VirtAddr` is a `repr(transparent)` newtype over the word
    // the switch writes, so the pointer names exactly that word; the
    // caller promises the rest.
    unsafe {
        switch(from.cast::<u64>(), to.as_u64());
    }
}

/// Hands the thread the address of its IPC buffer and returns to user
/// mode through the interrupt frame the stack carries.
///
/// A new thread starts here: [`switch`] pops the six saved registers of
/// the frame [`prepare_user`] built and returns to this address, which
/// leaves the stack pointer at the word the thread is told and, below the
/// pop, at the five words `iretq` reads.
///
/// # Safety
///
/// It runs only as the return address of a frame [`prepare_user`] built,
/// and never as a call.
#[unsafe(naked)]
pub unsafe extern "sysv64" fn enter_user_trampoline() -> ! {
    naked_asm!("pop rdi", "iretq")
}

/// Writes the frame a new thread starts through into the top of its kernel
/// stack and returns the stack pointer the first [`switch`] to it must
/// load.
///
/// The stack is reached through the physical window, because a thread that
/// has not started has no address space the kernel could reach it in. The
/// frame is built as words first and copied in afterwards, so that nothing
/// here needs a second way of writing memory.
///
/// Returns `None` when the frame of the stack top is not reachable through
/// the window.
pub fn prepare_user(
    window: &mut PhysicalWindow,
    top_frame: PhysFrame,
    stack_top: VirtAddr,
    entry: VirtAddr,
    user_stack: VirtAddr,
    ipc_buffer: VirtAddr,
) -> Option<VirtAddr> {
    let mut frame = [0_u64; FRAME_WORDS];
    prepare_frame(
        &mut frame,
        trampoline_address(),
        entry.as_u64(),
        user_stack.as_u64(),
        ipc_buffer.as_u64(),
    )?;
    let bytes = window.frame_bytes_mut(top_frame)?;
    let base = bytes.len().checked_sub(FRAME_BYTES)?;
    for (index, word) in frame.iter().enumerate() {
        let offset = base.checked_add(index.checked_mul(WORD)?)?;
        let end = offset.checked_add(WORD)?;
        bytes
            .get_mut(offset..end)?
            .copy_from_slice(&word.to_le_bytes());
    }
    stack_top.checked_sub(u64::try_from(FRAME_BYTES).ok()?)
}

/// Size of one word of a kernel stack.
const WORD: usize = 8;

/// Size of the frame in bytes.
const FRAME_BYTES: usize = FRAME_WORDS * WORD;

/// The address of the trampoline, as the frame carries it.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "a frame carries the address of the code it returns to, and `as` is the only way to read one as a number"
)]
pub fn trampoline_address() -> u64 {
    (enter_user_trampoline as *const ()) as usize as u64
}
