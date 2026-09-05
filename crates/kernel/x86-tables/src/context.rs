// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The frame a new thread starts through: what the kernel writes onto a
//! fresh kernel stack so that the first switch into the thread leaves the
//! processor in user mode at the entry point.
//!
//! Invariants: the frame lies at the top of the stack and nothing else is
//! written; the word the switch loads as the stack pointer is the lowest
//! word of the frame; the order of the words is the order `switch` pops
//! them in and `iretq` reads them in, which is what the two agree on and
//! nothing else enforces.
//!
//! ```text
//! high    ss      = the ring three data selector
//!         rsp     = where the thread stands in its own address space
//!         rflags  = interrupts on, nothing else
//!         cs      = the ring three code selector
//!         rip     = where the thread starts        <- `iretq` reads from here
//!         rdi     = the address of its IPC buffer  <- the trampoline pops it
//!         return address of `switch`: the trampoline
//!         rbx, rbp, r12, r13, r14, r15 (all zero)
//! low     r15                                      <- the saved context
//! ```
//!
//! The word below the interrupt frame is what a thread is told: the
//! address of its own IPC buffer, which it cannot ask for, because asking
//! would need the buffer. The trampoline pops it into the first argument
//! register and runs the interrupt return.
//!
//! The whole of it is written as `u64` values in safe Rust; nothing here
//! touches a register (D-67).

use crate::gdt::{USER_CODE_SELECTOR, USER_DATA_SELECTOR};

/// Number of words the frame occupies: six callee-saved registers, the
/// return address of the switch, the argument the trampoline pops, and the
/// five words `iretq` reads.
pub const FRAME_WORDS: usize = 13;

/// Number of callee-saved registers the switch pushes and pops.
pub const SAVED_REGISTERS: usize = 6;

/// The flags a thread starts with: interrupts on, every other bit off but
/// the one the processor always reads as set.
pub const INITIAL_RFLAGS: u64 = 0x202;

/// Offset of the return address of the switch, in words from the saved
/// context.
pub const TRAMPOLINE_WORD: usize = SAVED_REGISTERS;

/// Offset of the word the trampoline pops into the first argument
/// register: the address of the thread's IPC buffer.
pub const RDI_WORD: usize = TRAMPOLINE_WORD + 1;

/// Offset of the instruction pointer the interrupt return reads.
pub const RIP_WORD: usize = RDI_WORD + 1;

/// Offset of the code selector.
pub const CS_WORD: usize = RIP_WORD + 1;

/// Offset of the flags.
pub const RFLAGS_WORD: usize = CS_WORD + 1;

/// Offset of the user stack pointer.
pub const RSP_WORD: usize = RFLAGS_WORD + 1;

/// Offset of the data selector.
pub const SS_WORD: usize = RSP_WORD + 1;

const _: () = assert!(SS_WORD + 1 == FRAME_WORDS);

/// Writes the frame into the top of `stack` and returns the index of the
/// word the first switch loads as its stack pointer.
///
/// `stack` is the kernel stack of the thread as words, lowest address
/// first; `trampoline` is the address of the code that runs the interrupt
/// return. Returns `None` for a stack too small to hold the frame, and
/// writes nothing then.
#[must_use]
pub fn prepare_user(
    stack: &mut [u64],
    trampoline: u64,
    entry: u64,
    user_stack: u64,
    ipc_buffer: u64,
) -> Option<usize> {
    let base = stack.len().checked_sub(FRAME_WORDS)?;
    let frame = stack.get_mut(base..)?;
    for word in frame.iter_mut().take(SAVED_REGISTERS) {
        *word = 0;
    }
    *frame.get_mut(TRAMPOLINE_WORD)? = trampoline;
    *frame.get_mut(RDI_WORD)? = ipc_buffer;
    *frame.get_mut(RIP_WORD)? = entry;
    *frame.get_mut(CS_WORD)? = u64::from(USER_CODE_SELECTOR.as_u16());
    *frame.get_mut(RFLAGS_WORD)? = INITIAL_RFLAGS;
    *frame.get_mut(RSP_WORD)? = user_stack;
    *frame.get_mut(SS_WORD)? = u64::from(USER_DATA_SELECTOR.as_u16());
    Some(base)
}

/// The frame as it stands on a stack, for reading one back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    /// Where the thread starts.
    pub entry: u64,
    /// The code selector it runs under.
    pub code_selector: u64,
    /// The flags it starts with.
    pub rflags: u64,
    /// Where it stands in its own address space.
    pub user_stack: u64,
    /// The data selector of that stack.
    pub stack_selector: u64,
    /// The address the switch returns to.
    pub trampoline: u64,
    /// The address of the thread's IPC buffer.
    pub ipc_buffer: u64,
}

/// Reads the frame that starts at `base` of `stack`, or `None` when the
/// stack does not hold a whole one there.
#[must_use]
pub fn frame_at(stack: &[u64], base: usize) -> Option<Frame> {
    let end = base.checked_add(FRAME_WORDS)?;
    let frame = stack.get(base..end)?;
    Some(Frame {
        trampoline: *frame.get(TRAMPOLINE_WORD)?,
        ipc_buffer: *frame.get(RDI_WORD)?,
        entry: *frame.get(RIP_WORD)?,
        code_selector: *frame.get(CS_WORD)?,
        rflags: *frame.get(RFLAGS_WORD)?,
        user_stack: *frame.get(RSP_WORD)?,
        stack_selector: *frame.get(SS_WORD)?,
    })
}
