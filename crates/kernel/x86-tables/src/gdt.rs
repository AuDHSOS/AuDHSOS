// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The global descriptor table: four flat segment descriptors, the task
//! state segment descriptor, and the selectors that name them.
//!
//! Invariants: the table starts with the null descriptor; the code and data
//! descriptors are flat and differ only in the type and privilege fields;
//! the task state segment descriptor occupies two entries.

#![expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "this module encodes and decodes bit fields; every conversion widens or narrows a value that was masked to the width first, and the functions are const"
)]

/// The null descriptor.
pub const NULL: u64 = 0;

/// Ring 0 code: present, code, executable, read, long mode.
pub const KERNEL_CODE: u64 = 0x00AF_9A00_0000_FFFF;

/// Ring 0 data: present, data, writable.
pub const KERNEL_DATA: u64 = 0x00CF_9200_0000_FFFF;

/// Ring 3 data: present, data, writable, privilege level 3.
pub const USER_DATA: u64 = 0x00CF_F200_0000_FFFF;

/// Ring 3 code: present, code, executable, read, long mode, privilege
/// level 3.
pub const USER_CODE: u64 = 0x00AF_FA00_0000_FFFF;

/// Number of entries: the null descriptor, four segments, and the two
/// quadwords of the task state segment descriptor.
pub const GDT_ENTRIES: usize = 7;

/// Index of the task state segment descriptor.
pub const TSS_INDEX: u16 = 5;

/// The type field of an available 64-bit task state segment.
pub const TSS_TYPE: u64 = 0x9;

/// A segment selector: an index into the table and a requested privilege
/// level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Selector(u16);

impl Selector {
    /// The selector for `index` with the requested privilege level `rpl`.
    #[must_use]
    pub const fn new(index: u16, rpl: u16) -> Self {
        Selector((index << 3) | (rpl & 0x3))
    }

    /// The raw selector value.
    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// The index into the table.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.0 >> 3
    }

    /// The requested privilege level.
    #[must_use]
    pub const fn rpl(self) -> u16 {
        self.0 & 0x3
    }
}

/// Selector of the ring 0 code segment.
pub const KERNEL_CODE_SELECTOR: Selector = Selector::new(1, 0);

/// Selector of the ring 0 data segment.
pub const KERNEL_DATA_SELECTOR: Selector = Selector::new(2, 0);

/// Selector of the ring 3 data segment.
pub const USER_DATA_SELECTOR: Selector = Selector::new(3, 3);

/// Selector of the ring 3 code segment.
pub const USER_CODE_SELECTOR: Selector = Selector::new(4, 3);

/// Selector of the task state segment.
pub const TSS_SELECTOR: Selector = Selector::new(TSS_INDEX, 0);

/// The two quadwords of the task state segment descriptor for a segment of
/// `limit` bytes minus one at `base`.
#[must_use]
pub const fn tss_descriptor(base: u64, limit: u32) -> [u64; 2] {
    let limit = limit as u64;
    // The fields the processor reads are scattered across the quadword.
    let low = (limit & 0xFFFF)
        | ((base & 0x00FF_FFFF) << 16)
        | (TSS_TYPE << 40)
        | (1 << 47)
        | (((limit >> 16) & 0xF) << 48)
        | (((base >> 24) & 0xFF) << 56);
    [low, base >> 32]
}

/// The base address a task state segment descriptor names.
#[must_use]
pub const fn tss_descriptor_base(descriptor: [u64; 2]) -> u64 {
    let [low, high] = descriptor;
    ((low >> 16) & 0x00FF_FFFF) | (((low >> 56) & 0xFF) << 24) | ((high & 0xFFFF_FFFF) << 32)
}

/// The limit a task state segment descriptor names.
#[must_use]
pub const fn tss_descriptor_limit(descriptor: [u64; 2]) -> u32 {
    let [low, _high] = descriptor;
    ((low & 0xFFFF) | (((low >> 48) & 0xF) << 16)) as u32
}

/// The whole table for a task state segment of [`crate::tss::TSS_LEN`]
/// bytes at `tss_base`.
#[must_use]
pub const fn build_gdt(tss_base: u64) -> [u64; GDT_ENTRIES] {
    let [low, high] = tss_descriptor(tss_base, TSS_LIMIT);
    [
        NULL,
        KERNEL_CODE,
        KERNEL_DATA,
        USER_DATA,
        USER_CODE,
        low,
        high,
    ]
}

/// The limit of the task state segment descriptor: one less than the
/// length of the segment.
pub const TSS_LIMIT: u32 = (crate::tss::TSS_LEN as u32).wrapping_sub(1);
