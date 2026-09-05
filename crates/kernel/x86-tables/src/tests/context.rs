// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::context`, covering the frame item of the catalog
//! 6.6.16.

use crate::context::{
    CS_WORD, FRAME_WORDS, INITIAL_RFLAGS, RDI_WORD, RFLAGS_WORD, RIP_WORD, RSP_WORD,
    SAVED_REGISTERS, SS_WORD, TRAMPOLINE_WORD, frame_at, prepare_user,
};
use crate::gdt::{USER_CODE_SELECTOR, USER_DATA_SELECTOR};

/// Where a test thread starts.
const ENTRY: u64 = 0x40_0000;

/// Where it stands.
const USER_STACK: u64 = 0x50_0000;

/// The address of the code that runs the interrupt return.
const TRAMPOLINE: u64 = 0xFFFF_FFFF_8000_1234;

/// Where the IPC buffer of the thread is mapped.
const BUFFER: u64 = 0x0000_7FFF_FFFF_F000;

#[test]
fn the_frame_lies_at_the_top_of_the_stack() {
    let mut stack = [0xAA_u64; 64];
    let base = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    assert_eq!(base, 64 - FRAME_WORDS);
    for (index, word) in stack.iter().enumerate().take(base) {
        assert_eq!(*word, 0xAA, "word {index} below the frame was touched");
    }
}

#[test]
fn the_words_are_the_ones_the_switch_and_the_interrupt_return_read() {
    let mut stack = [0_u64; 32];
    let base = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    let frame = &stack[base..];
    for (index, word) in frame.iter().enumerate().take(SAVED_REGISTERS) {
        assert_eq!(*word, 0, "callee-saved register {index} starts at zero");
    }
    assert_eq!(frame[TRAMPOLINE_WORD], TRAMPOLINE);
    assert_eq!(
        frame[RDI_WORD], BUFFER,
        "the thread is told where its buffer is"
    );
    assert_eq!(frame[RIP_WORD], ENTRY);
    assert_eq!(frame[CS_WORD], u64::from(USER_CODE_SELECTOR.as_u16()));
    assert_eq!(frame[RFLAGS_WORD], INITIAL_RFLAGS);
    assert_eq!(frame[RSP_WORD], USER_STACK);
    assert_eq!(frame[SS_WORD], u64::from(USER_DATA_SELECTOR.as_u16()));
}

#[test]
fn the_selectors_are_the_ring_three_ones_of_the_descriptor_table() {
    let mut stack = [0_u64; 16];
    let base = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    let frame = frame_at(&stack, base).unwrap();
    assert_eq!(frame.code_selector, 0x23, "index four, privilege three");
    assert_eq!(frame.stack_selector, 0x1B, "index three, privilege three");
    assert_eq!(frame.code_selector & 0x3, 3);
    assert_eq!(frame.stack_selector & 0x3, 3);
}

#[test]
fn a_thread_starts_with_interrupts_on_and_nothing_else_set() {
    let mut stack = [0_u64; 16];
    let base = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    let frame = frame_at(&stack, base).unwrap();
    // Bit nine is the interrupt flag; bit one is the one the processor
    // always reads as set.
    assert_eq!(frame.rflags & (1 << 9), 1 << 9);
    assert_eq!(frame.rflags & 0x2, 0x2);
    assert_eq!(frame.rflags & !0x202, 0, "nothing else");
}

#[test]
fn the_frame_reads_back_as_it_was_written() {
    let mut stack = [0_u64; 20];
    let base = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    let frame = frame_at(&stack, base).unwrap();
    assert_eq!(frame.entry, ENTRY);
    assert_eq!(frame.user_stack, USER_STACK);
    assert_eq!(frame.trampoline, TRAMPOLINE);
    assert_eq!(frame.ipc_buffer, BUFFER);
}

#[test]
fn a_stack_of_exactly_the_frame_works_and_a_smaller_one_is_refused() {
    let mut exact = [0_u64; FRAME_WORDS];
    assert_eq!(
        prepare_user(&mut exact, TRAMPOLINE, ENTRY, USER_STACK, BUFFER),
        Some(0)
    );

    let mut small = [0xCC_u64; FRAME_WORDS - 1];
    assert_eq!(
        prepare_user(&mut small, TRAMPOLINE, ENTRY, USER_STACK, BUFFER),
        None
    );
    assert!(
        small.iter().all(|word| *word == 0xCC),
        "a stack too small is left alone"
    );

    let mut empty: [u64; 0] = [];
    assert_eq!(
        prepare_user(&mut empty, TRAMPOLINE, ENTRY, USER_STACK, BUFFER),
        None
    );
}

#[test]
fn reading_a_frame_that_does_not_fit_is_refused() {
    let stack = [0_u64; FRAME_WORDS];
    assert!(frame_at(&stack, 0).is_some());
    assert!(frame_at(&stack, 1).is_none());
    assert!(frame_at(&stack, usize::MAX).is_none());
}

#[test]
fn a_second_thread_on_the_same_stack_overwrites_the_first_frame() {
    let mut stack = [0_u64; 24];
    let first = prepare_user(&mut stack, TRAMPOLINE, ENTRY, USER_STACK, BUFFER).unwrap();
    let second = prepare_user(&mut stack, TRAMPOLINE, 0x41_0000, 0x51_0000, BUFFER).unwrap();
    assert_eq!(first, second);
    let frame = frame_at(&stack, second).unwrap();
    assert_eq!(frame.entry, 0x41_0000);
    assert_eq!(frame.user_stack, 0x51_0000);
}
