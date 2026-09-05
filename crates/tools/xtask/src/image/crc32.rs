// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The CRC-32 the GUID partition table uses: the reflected IEEE
//! polynomial, initial and final value `0xFFFF_FFFF`.

#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "the table is built in a const fn, where the index and the shift are bounded by the loop"
)]

/// The reflected IEEE polynomial.
pub(crate) const POLYNOMIAL: u32 = 0xEDB8_8320;

/// The lookup table, one entry per byte value.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut value = index as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 == 0 {
                value >> 1
            } else {
                (value >> 1) ^ POLYNOMIAL
            };
            bit += 1;
        }
        table[index] = value;
        index += 1;
    }
    table
}

/// A running checksum, so that a caller can feed a header in pieces.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Crc32 {
    value: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    /// A checksum over nothing.
    pub(crate) const fn new() -> Self {
        Crc32 { value: u32::MAX }
    }

    /// Adds `bytes` to the checksum.
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            let index = usize::from(u8::try_from(self.value & 0xFF).unwrap_or(0) ^ *byte);
            let entry = TABLE.get(index).copied().unwrap_or(0);
            self.value = (self.value >> 8) ^ entry;
        }
    }

    /// The checksum of everything added so far.
    pub(crate) const fn finish(self) -> u32 {
        self.value ^ u32::MAX
    }
}

/// The checksum of `bytes`.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(bytes);
    crc.finish()
}
