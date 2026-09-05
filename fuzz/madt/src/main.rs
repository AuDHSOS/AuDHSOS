// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The ACPI table parsers against arbitrary bytes: no input may panic, the
//! walk over the entries must always end, and a table the parser accepts
//! must stay inside the fixed capacities of the kernel and route every
//! line it named to the interrupt it named.
//!
//! The bytes are tried twice: as they are, so that the checks of signature,
//! length, and checksum are exercised, and once with those three repaired,
//! so that the fuzzer reaches the walk over the entries without having to
//! guess a checksum.


use kernel_acpi::madt::{ISA_BUS, MADT_HEADER_LEN, MADT_SIGNATURE, Madt, parse};
use kernel_acpi::rsdp::{RSDP_LEN, parse_rsdp};
use kernel_acpi::sdt::{RootTable, SdtHeader};
use kernel_acpi::{MAX_IO_APICS, MAX_OVERRIDES};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let _ = SdtHeader::parse(bytes);
    let _ = RootTable::parse(bytes);
    if let Ok(pointer) = <[u8; RSDP_LEN]>::try_from(bytes) {
        let _ = parse_rsdp(&pointer);
    }
    check(bytes);
    if let Some(repaired) = repair(bytes) {
        check(&repaired);
    }
});

/// Parses `bytes` and checks what an accepted table promises.
fn check(bytes: &[u8]) {
    let Ok(madt) = parse(bytes) else {
        return;
    };
    assert!(
        madt.io_apic_count() <= MAX_IO_APICS,
        "an accepted table names more i/o apics than the kernel holds"
    );
    assert!(
        madt.override_count() <= MAX_OVERRIDES,
        "an accepted table names more overrides than the kernel holds"
    );
    check_routing(&madt);
    check_io_apics(&madt);
}

/// Every line either has an override that decides where it goes, or goes
/// to the interrupt of its own number.
fn check_routing(madt: &Madt) {
    for line in 0..=u8::MAX {
        let routing = madt.route_isa(line);
        match madt.override_for(line) {
            Some(entry) => {
                assert_eq!(entry.bus, ISA_BUS, "an override of another bus routed a line");
                assert_eq!(
                    routing.gsi, entry.gsi,
                    "a line is not routed where its override says"
                );
                assert_eq!(routing.polarity, entry.polarity());
                assert_eq!(routing.trigger, entry.trigger());
            }
            None => assert_eq!(
                routing.gsi,
                u32::from(line),
                "a line without an override is not the interrupt of its own number"
            ),
        }
    }
}

/// Every I/O APIC serves the interrupt it starts at, and no interrupt below
/// every base belongs to one.
fn check_io_apics(madt: &Madt) {
    for apic in madt.io_apics.iter().flatten() {
        let found = madt
            .io_apic_for(apic.gsi_base)
            .expect("the interrupt an i/o apic starts at belongs to no i/o apic");
        assert!(
            found.gsi_base >= apic.gsi_base,
            "an interrupt was given to an i/o apic that starts above it"
        );
    }
}

/// The bytes with the signature, the length, and the checksum of a table
/// written over them, so that the fuzzer spends its time on the entries.
/// `None` for bytes too short to hold the fixed part of the table.
fn repair(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < MADT_HEADER_LEN || bytes.len() > u32::MAX as usize {
        return None;
    }
    let mut table = bytes.to_vec();
    table.get_mut(..4)?.copy_from_slice(&MADT_SIGNATURE);
    let length = u32::try_from(table.len()).ok()?;
    table.get_mut(4..8)?.copy_from_slice(&length.to_le_bytes());
    *table.get_mut(9)? = 0;
    let sum = table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    *table.get_mut(9)? = 0u8.wrapping_sub(sum);
    Some(table)
}
