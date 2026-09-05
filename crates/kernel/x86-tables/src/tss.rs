// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The task state segment: the stack pointers the processor switches to on
//! a privilege change and on the interrupts that name an interrupt stack.
//!
//! Invariant: the byte image is exactly [`TSS_LEN`] bytes and its I/O map
//! base points past its own end, so that no I/O permission bitmap exists.

/// Length of the byte image in bytes.
pub const TSS_LEN: usize = 104;

/// Offset of the ring 0 stack pointer.
pub const RSP0_OFFSET: usize = 4;

/// Offset of the first interrupt stack pointer.
pub const IST1_OFFSET: usize = 36;

/// Offset of the I/O map base.
pub const IOMAP_OFFSET: usize = 102;

/// The value of the I/O map base: the length of the segment, so that the
/// permission bitmap starts past its end and therefore does not exist.
pub const IOMAP_BASE: u16 = 104;

const _: () = assert!(TSS_LEN == 104);

/// Number of privilege stack pointers.
pub const PRIVILEGE_STACKS: usize = 3;

/// Number of interrupt stack pointers.
pub const INTERRUPT_STACKS: usize = 7;

/// The stack pointers of one task state segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TaskStateSegment {
    /// The stack pointers for ring 0, 1, and 2.
    pub privilege_stacks: [u64; PRIVILEGE_STACKS],
    /// The stack pointers the interrupt stack table names.
    pub interrupt_stacks: [u64; INTERRUPT_STACKS],
}

impl TaskStateSegment {
    /// A segment without stack pointers.
    #[must_use]
    pub const fn new() -> Self {
        TaskStateSegment {
            privilege_stacks: [0; PRIVILEGE_STACKS],
            interrupt_stacks: [0; INTERRUPT_STACKS],
        }
    }

    /// The same segment with the ring 0 stack pointer set.
    #[must_use]
    pub const fn with_kernel_stack(self, top: u64) -> Self {
        let [_ring0, ring1, ring2] = self.privilege_stacks;
        TaskStateSegment {
            privilege_stacks: [top, ring1, ring2],
            interrupt_stacks: self.interrupt_stacks,
        }
    }

    /// The same segment with interrupt stack `index`, counted from one,
    /// set. An index outside the table changes nothing.
    #[must_use]
    pub fn with_interrupt_stack(mut self, index: usize, top: u64) -> Self {
        if let Some(slot) = index
            .checked_sub(1)
            .and_then(|slot| self.interrupt_stacks.get_mut(slot))
        {
            *slot = top;
        }
        self
    }

    /// The byte image the processor reads.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; TSS_LEN] {
        let mut bytes = [0u8; TSS_LEN];
        for (index, stack) in self.privilege_stacks.iter().enumerate() {
            let offset = RSP0_OFFSET.saturating_add(index.saturating_mul(8));
            write_at(&mut bytes, offset, &stack.to_le_bytes());
        }
        for (index, stack) in self.interrupt_stacks.iter().enumerate() {
            let offset = IST1_OFFSET.saturating_add(index.saturating_mul(8));
            write_at(&mut bytes, offset, &stack.to_le_bytes());
        }
        write_at(&mut bytes, IOMAP_OFFSET, &IOMAP_BASE.to_le_bytes());
        bytes
    }
}

/// Writes `value` at `offset`; a write that would leave `bytes` changes
/// nothing.
pub(crate) fn write_at(bytes: &mut [u8], offset: usize, value: &[u8]) {
    if let Some(slot) = offset
        .checked_add(value.len())
        .and_then(|end| bytes.get_mut(offset..end))
    {
        slot.copy_from_slice(value);
    }
}
