// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The register block of an I/O APIC: two memory-mapped registers that
//! reach an indexed register file, and the encoding of a redirection entry.
//!
//! Invariant: a redirection entry this module builds names its vector, its
//! polarity, its trigger mode, its mask bit, and its destination and sets
//! nothing else; delivery mode `fixed` and destination mode `physical` are
//! what the kernel uses, so both stay zero.

#![expect(
    clippy::as_conversions,
    reason = "this module encodes and decodes bit fields; every conversion widens or narrows a value that was masked to the width first, and the functions are const"
)]

/// Number of bytes of the register window.
pub const REGISTER_WINDOW_LEN: usize = 4096;

/// Offset of the register that selects which indexed register the window
/// reaches.
pub const IOREGSEL: usize = 0x00;

/// Offset of the window onto the selected register.
pub const IOWIN: usize = 0x10;

/// Index of the identifier register.
pub const ID: u32 = 0;

/// Index of the version register, whose bits 16 to 23 hold one less than
/// the number of lines.
pub const VERSION: u32 = 1;

/// Index of the low word of the redirection entry of the first line.
pub const REDIRECTION_BASE: u32 = 0x10;

/// The bit of a redirection entry that stops its delivery.
pub const MASKED: u64 = 1 << 16;

/// The bit of a redirection entry that says the line is asserted low.
pub const ACTIVE_LOW: u64 = 1 << 13;

/// The bit of a redirection entry that says the line is held rather than
/// pulsed.
pub const LEVEL_TRIGGERED: u64 = 1 << 15;

/// The indices of the two words of the redirection entry of `line`.
#[must_use]
pub const fn redirection_index(line: u8) -> (u32, u32) {
    let low = REDIRECTION_BASE.wrapping_add((line as u32).wrapping_mul(2));
    (low, low.wrapping_add(1))
}

/// One less than the number of lines the version register reports.
#[must_use]
pub const fn version_line_count(version: u32) -> u32 {
    ((version >> 16) & 0xFF).wrapping_add(1)
}

/// The identifier the identifier register reports.
#[must_use]
pub const fn id_of(register: u32) -> u8 {
    ((register >> 24) & 0x0F) as u8
}

/// A redirection entry that sends `line` to `vector` on the processor with
/// local APIC identifier `destination`, with delivery mode `fixed` and
/// destination mode `physical`.
#[must_use]
pub const fn redirection_entry(
    vector: u8,
    active_low: bool,
    level: bool,
    masked: bool,
    destination: u8,
) -> u64 {
    let mut entry = vector as u64;
    if active_low {
        entry |= ACTIVE_LOW;
    }
    if level {
        entry |= LEVEL_TRIGGERED;
    }
    if masked {
        entry |= MASKED;
    }
    entry | ((destination as u64) << 56)
}

/// The vector a redirection entry names.
#[must_use]
pub const fn entry_vector(entry: u64) -> u8 {
    (entry & 0xFF) as u8
}

/// `true` if a redirection entry is masked.
#[must_use]
pub const fn entry_masked(entry: u64) -> bool {
    entry & MASKED != 0
}

/// `true` if a redirection entry says the line is asserted low.
#[must_use]
pub const fn entry_active_low(entry: u64) -> bool {
    entry & ACTIVE_LOW != 0
}

/// `true` if a redirection entry says the line is held rather than pulsed.
#[must_use]
pub const fn entry_level(entry: u64) -> bool {
    entry & LEVEL_TRIGGERED != 0
}

/// The local APIC identifier a redirection entry names.
#[must_use]
pub const fn entry_destination(entry: u64) -> u8 {
    ((entry >> 56) & 0xFF) as u8
}

/// The entry with its mask bit set or cleared.
#[must_use]
pub const fn with_mask(entry: u64, masked: bool) -> u64 {
    if masked {
        entry | MASKED
    } else {
        entry & !MASKED
    }
}

/// The two words a redirection entry is written as, low first.
#[must_use]
pub const fn entry_words(entry: u64) -> (u32, u32) {
    ((entry & 0xFFFF_FFFF) as u32, (entry >> 32) as u32)
}

/// The entry the two words describe.
#[must_use]
pub const fn entry_from_words(low: u32, high: u32) -> u64 {
    (low as u64) | ((high as u64) << 32)
}
