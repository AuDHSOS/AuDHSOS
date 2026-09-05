// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The interrupt descriptor table: gate descriptors of sixteen bytes with
//! the handler address split across three fields.
//!
//! Invariant: a gate this module builds is present, names an interrupt
//! gate, and carries the handler address unchanged.

#![expect(
    clippy::as_conversions,
    reason = "this module encodes and decodes bit fields; every conversion widens or narrows a value that was masked to the width first, and the functions are const"
)]

/// Number of vectors.
pub const IDT_ENTRIES: usize = 256;

/// Attribute byte of an interrupt gate the processor may enter from ring 0
/// only.
pub const GATE_INTERRUPT_DPL0: u8 = 0x8E;

/// Attribute byte of an interrupt gate the processor may enter from ring 3,
/// which vector `0x80` needs.
pub const GATE_INTERRUPT_DPL3: u8 = 0xEE;

/// The vector the system call interface uses.
pub const SYSCALL_VECTOR: u8 = 0x80;

/// Index of the interrupt stack the double fault handler runs on.
pub const DOUBLE_FAULT_IST: u8 = 1;

/// An entry that maps no handler.
pub const MISSING: [u64; 2] = [0, 0];

/// The two quadwords of a gate for `handler`, entered through `selector`,
/// running on interrupt stack `ist` (zero for the current stack) with the
/// given attribute byte.
#[must_use]
pub const fn gate(handler: u64, selector: u16, ist: u8, attributes: u8) -> [u64; 2] {
    let low = (handler & 0xFFFF)
        | ((selector as u64) << 16)
        | (((ist & 0x7) as u64) << 32)
        | ((attributes as u64) << 40)
        | (((handler >> 16) & 0xFFFF) << 48);
    [low, handler >> 32]
}

/// The handler address a gate names.
#[must_use]
pub const fn gate_handler(entry: [u64; 2]) -> u64 {
    let [low, high] = entry;
    (low & 0xFFFF) | (((low >> 48) & 0xFFFF) << 16) | ((high & 0xFFFF_FFFF) << 32)
}

/// The selector a gate names.
#[must_use]
pub const fn gate_selector(entry: [u64; 2]) -> u16 {
    let [low, _high] = entry;
    ((low >> 16) & 0xFFFF) as u16
}

/// The interrupt stack index a gate names.
#[must_use]
pub const fn gate_ist(entry: [u64; 2]) -> u8 {
    let [low, _high] = entry;
    ((low >> 32) & 0x7) as u8
}

/// The attribute byte of a gate.
#[must_use]
pub const fn gate_attributes(entry: [u64; 2]) -> u8 {
    let [low, _high] = entry;
    ((low >> 40) & 0xFF) as u8
}

/// `true` if the gate is present.
#[must_use]
pub const fn gate_present(entry: [u64; 2]) -> bool {
    gate_attributes(entry) & 0x80 != 0
}

/// The privilege level from which the gate may be entered.
#[must_use]
pub const fn gate_privilege(entry: [u64; 2]) -> u8 {
    (gate_attributes(entry) >> 5) & 0x3
}
