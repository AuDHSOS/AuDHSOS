// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tss`, covering the task state segment items of the
//! catalog 6.6.16.

#![allow(clippy::arithmetic_side_effects)]

use crate::tss::{
    INTERRUPT_STACKS, IOMAP_BASE, IOMAP_OFFSET, IST1_OFFSET, PRIVILEGE_STACKS, RSP0_OFFSET,
    TSS_LEN, TaskStateSegment, write_at,
};

fn u64_at(bytes: &[u8; TSS_LEN], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

#[test]
fn an_empty_segment_is_zero_except_for_the_io_map_base() {
    let bytes = TaskStateSegment::new().to_bytes();
    assert_eq!(bytes.len(), TSS_LEN);
    assert_eq!(
        u16::from_le_bytes([bytes[IOMAP_OFFSET], bytes[IOMAP_OFFSET + 1]]),
        IOMAP_BASE,
        "the I/O map base points past the end, so no bitmap exists"
    );
    assert_eq!(usize::from(IOMAP_BASE), TSS_LEN);
    assert!(
        bytes[..IOMAP_OFFSET].iter().all(|byte| *byte == 0),
        "every stack pointer is zero"
    );
    assert_eq!(TaskStateSegment::default(), TaskStateSegment::new());
}

#[test]
fn the_ring_zero_stack_pointer_lands_at_its_offset() {
    let segment = TaskStateSegment::new().with_kernel_stack(0x1234_5678_9ABC_DEF0);
    let bytes = segment.to_bytes();
    assert_eq!(u64_at(&bytes, RSP0_OFFSET), 0x1234_5678_9ABC_DEF0);
    assert_eq!(
        segment.privilege_stacks.first().copied(),
        Some(0x1234_5678_9ABC_DEF0)
    );
    assert_eq!(u64_at(&bytes, RSP0_OFFSET + 8), 0, "ring 1 is untouched");
    assert_eq!(u64_at(&bytes, RSP0_OFFSET + 16), 0, "ring 2 is untouched");
    assert_eq!(RSP0_OFFSET, 4);
    assert_eq!(PRIVILEGE_STACKS, 3);
}

#[test]
fn every_interrupt_stack_lands_at_its_offset() {
    let mut segment = TaskStateSegment::new();
    for index in 1..=INTERRUPT_STACKS {
        segment = segment.with_interrupt_stack(index, 0x1000 + u64::try_from(index).unwrap());
    }
    let bytes = segment.to_bytes();
    for index in 1..=INTERRUPT_STACKS {
        assert_eq!(
            u64_at(&bytes, IST1_OFFSET + (index - 1) * 8),
            0x1000 + u64::try_from(index).unwrap(),
            "interrupt stack {index}"
        );
    }
    assert_eq!(IST1_OFFSET, 36);
    assert_eq!(INTERRUPT_STACKS, 7);
}

#[test]
fn an_interrupt_stack_index_outside_the_table_changes_nothing() {
    let empty = TaskStateSegment::new();
    assert_eq!(empty.with_interrupt_stack(0, 0x9999), empty);
    assert_eq!(
        empty.with_interrupt_stack(INTERRUPT_STACKS + 1, 0x9999),
        empty
    );
    assert_ne!(empty.with_interrupt_stack(1, 0x9999), empty);
}

#[test]
fn the_double_fault_stack_is_the_first_interrupt_stack() {
    let segment = TaskStateSegment::new()
        .with_kernel_stack(0xAAAA)
        .with_interrupt_stack(usize::from(crate::idt::DOUBLE_FAULT_IST), 0xBBBB);
    let bytes = segment.to_bytes();
    assert_eq!(u64_at(&bytes, RSP0_OFFSET), 0xAAAA);
    assert_eq!(u64_at(&bytes, IST1_OFFSET), 0xBBBB);
}

#[test]
fn a_write_that_would_leave_the_image_changes_nothing() {
    let mut bytes = [0u8; 4];
    write_at(&mut bytes, 3, &[1, 2]);
    assert_eq!(bytes, [0, 0, 0, 0], "the write reaches past the end");
    write_at(&mut bytes, usize::MAX, &[1]);
    assert_eq!(bytes, [0, 0, 0, 0], "the offset overflows");
    write_at(&mut bytes, 2, &[7, 8]);
    assert_eq!(bytes, [0, 0, 7, 8]);
}
