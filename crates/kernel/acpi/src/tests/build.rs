// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Builders that write the tables the parsers read, so that every test
//! starts from bytes a machine could produce.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::madt::MADT_SIGNATURE;
use crate::rsdp::{RSDP_LEN, RSDP_SIGNATURE, RSDP_V1_LEN};
use crate::sdt::SDT_HEADER_LEN;

/// The byte that makes the sum of `bytes` zero.
pub(crate) fn completing(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_sub(*byte))
}

/// A root pointer of the given revision, with both checksums correct.
pub(crate) fn rsdp(revision: u8, rsdt: u32, xsdt: u64) -> [u8; RSDP_LEN] {
    let mut bytes = [0u8; RSDP_LEN];
    bytes[..8].copy_from_slice(&RSDP_SIGNATURE);
    bytes[9..15].copy_from_slice(b"AUDHSO");
    bytes[15] = revision;
    bytes[16..20].copy_from_slice(&rsdt.to_le_bytes());
    bytes[20..24].copy_from_slice(&(RSDP_LEN as u32).to_le_bytes());
    bytes[24..32].copy_from_slice(&xsdt.to_le_bytes());
    fix_rsdp(&mut bytes);
    bytes
}

/// Recomputes both checksums of a root pointer.
pub(crate) fn fix_rsdp(bytes: &mut [u8; RSDP_LEN]) {
    bytes[8] = 0;
    bytes[8] = completing(&bytes[..RSDP_V1_LEN]);
    bytes[32] = 0;
    let length = u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) as usize;
    let covered = length.min(RSDP_LEN);
    bytes[32] = completing(&bytes[..covered]);
}

/// A table with the given signature, revision, and body, with the length
/// and the checksum filled in.
pub(crate) fn table(signature: [u8; 4], revision: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; SDT_HEADER_LEN];
    bytes[..4].copy_from_slice(&signature);
    bytes[8] = revision;
    bytes[10..16].copy_from_slice(b"AUDHSO");
    bytes[16..24].copy_from_slice(b"AUDHSOS0");
    bytes.extend_from_slice(body);
    fix_table(&mut bytes);
    bytes
}

/// Writes the length of `bytes` into its header and recomputes the
/// checksum.
pub(crate) fn fix_table(bytes: &mut [u8]) {
    let length = bytes.len() as u32;
    bytes[4..8].copy_from_slice(&length.to_le_bytes());
    bytes[9] = 0;
    bytes[9] = completing(bytes);
}

/// A root table with four-byte entries.
pub(crate) fn rsdt(entries: &[u32]) -> Vec<u8> {
    let mut body = Vec::new();
    for entry in entries {
        body.extend_from_slice(&entry.to_le_bytes());
    }
    table(*b"RSDT", 1, &body)
}

/// A root table with eight-byte entries.
pub(crate) fn xsdt(entries: &[u64]) -> Vec<u8> {
    let mut body = Vec::new();
    for entry in entries {
        body.extend_from_slice(&entry.to_le_bytes());
    }
    table(*b"XSDT", 1, &body)
}

/// A multiple APIC description table over the given entries.
pub(crate) fn madt(lapic: u32, flags: u32, entries: &[Vec<u8>]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&lapic.to_le_bytes());
    body.extend_from_slice(&flags.to_le_bytes());
    for entry in entries {
        body.extend_from_slice(entry);
    }
    table(MADT_SIGNATURE, 5, &body)
}

/// A processor local APIC entry.
pub(crate) fn local_apic(processor: u8, apic: u8) -> Vec<u8> {
    let mut entry = vec![0u8, 8, processor, apic];
    entry.extend_from_slice(&1u32.to_le_bytes());
    entry
}

/// An I/O APIC entry.
pub(crate) fn io_apic(id: u8, address: u32, gsi_base: u32) -> Vec<u8> {
    let mut entry = vec![1u8, 12, id, 0];
    entry.extend_from_slice(&address.to_le_bytes());
    entry.extend_from_slice(&gsi_base.to_le_bytes());
    entry
}

/// An interrupt source override entry.
pub(crate) fn source_override(bus: u8, source: u8, gsi: u32, flags: u16) -> Vec<u8> {
    let mut entry = vec![2u8, 10, bus, source];
    entry.extend_from_slice(&gsi.to_le_bytes());
    entry.extend_from_slice(&flags.to_le_bytes());
    entry
}

/// A local APIC address override entry.
pub(crate) fn lapic_override(address: u64) -> Vec<u8> {
    let mut entry = vec![5u8, 12, 0, 0];
    entry.extend_from_slice(&address.to_le_bytes());
    entry
}

/// An entry of a type this parser does not read.
pub(crate) fn unknown(kind: u8, length: u8) -> Vec<u8> {
    let mut entry = vec![kind, length];
    entry.resize(usize::from(length), 0);
    entry
}
