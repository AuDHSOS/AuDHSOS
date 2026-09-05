// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::madt`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use kernel_types::PhysAddr;
use kernel_types::phys::MAX_PHYS_ADDR;

use crate::error::AcpiError;
use crate::madt::{
    IoApic, MADT_HEADER_LEN, MAX_IO_APICS, MAX_OVERRIDES, PCAT_COMPAT, Polarity, Trigger, parse,
};
use crate::sdt::SDT_HEADER_LEN;
use crate::tests::build::{
    fix_table, io_apic, lapic_override, local_apic, madt, source_override, table, unknown,
};

/// The address QEMU puts the local APIC at.
const LAPIC: u32 = 0xFEE0_0000;

/// The address QEMU puts the first I/O APIC at.
const IOAPIC: u32 = 0xFEC0_0000;

#[test]
fn a_table_of_one_processor_and_one_io_apic_is_read_in_full() {
    let bytes = madt(
        LAPIC,
        PCAT_COMPAT,
        &[local_apic(0, 0), io_apic(0, IOAPIC, 0)],
    );
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(
        parsed.lapic_address,
        PhysAddr::new(u64::from(LAPIC)).unwrap()
    );
    assert!(parsed.has_legacy_pic());
    assert_eq!(parsed.processors, 1);
    assert_eq!(parsed.io_apic_count(), 1);
    assert_eq!(
        parsed.io_apics[0],
        Some(IoApic {
            id: 0,
            address: PhysAddr::new(u64::from(IOAPIC)).unwrap(),
            gsi_base: 0,
        })
    );
}

#[test]
fn a_table_without_the_legacy_flag_reports_no_legacy_controllers() {
    let bytes = madt(LAPIC, 0, &[]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert!(!parsed.has_legacy_pic());
}

#[test]
fn a_table_with_no_io_apic_names_none() {
    let bytes = madt(LAPIC, 0, &[local_apic(0, 0), local_apic(1, 1)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.processors, 2);
    assert_eq!(parsed.io_apic_count(), 0);
    assert_eq!(parsed.io_apic_for(0), None);
}

#[test]
fn a_local_apic_address_override_replaces_the_address_of_the_header() {
    let raw = 0x0000_0001_FEE0_0000;
    let bytes = madt(LAPIC, 0, &[lapic_override(raw)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.lapic_address, PhysAddr::new(raw).unwrap());
}

#[test]
fn a_local_apic_address_override_beyond_the_physical_width_is_refused() {
    let raw = MAX_PHYS_ADDR + 1;
    let bytes = madt(LAPIC, 0, &[lapic_override(raw)]);
    assert_eq!(parse(&bytes), Err(AcpiError::Address(raw)));
}

#[test]
fn an_entry_of_an_unknown_type_is_skipped_by_its_length() {
    let bytes = madt(
        LAPIC,
        0,
        &[unknown(9, 16), io_apic(3, IOAPIC, 0), unknown(0x10, 4)],
    );
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.io_apic_count(), 1);
    assert_eq!(parsed.io_apics[0].map(|apic| apic.id), Some(3));
}

#[test]
fn an_entry_of_length_zero_is_refused_instead_of_walked_forever() {
    let mut bytes = madt(LAPIC, 0, &[io_apic(0, IOAPIC, 0)]);
    bytes[MADT_HEADER_LEN + 1] = 0;
    fix_table(&mut bytes);
    assert_eq!(
        parse(&bytes),
        Err(AcpiError::EntryLengthZero(MADT_HEADER_LEN))
    );
}

#[test]
fn an_entry_whose_length_leaves_the_table_is_refused() {
    let mut bytes = madt(LAPIC, 0, &[io_apic(0, IOAPIC, 0)]);
    bytes[MADT_HEADER_LEN + 1] = 40;
    fix_table(&mut bytes);
    assert_eq!(
        parse(&bytes),
        Err(AcpiError::EntryTruncated {
            kind: 1,
            length: 40
        })
    );
}

#[test]
fn an_entry_of_a_known_type_with_the_wrong_length_is_refused() {
    for (kind, entry) in [
        (0u8, local_apic(0, 0)),
        (1, io_apic(0, IOAPIC, 0)),
        (2, source_override(0, 0, 2, 0)),
        (5, lapic_override(0xFEE0_0000)),
    ] {
        let wrong = entry.len() as u8 + 2;
        let mut bytes = madt(LAPIC, 0, &[entry]);
        bytes[MADT_HEADER_LEN + 1] = wrong;
        bytes.resize(MADT_HEADER_LEN + usize::from(wrong), 0);
        fix_table(&mut bytes);
        assert_eq!(
            parse(&bytes),
            Err(AcpiError::EntryLength {
                kind,
                length: wrong
            }),
            "an entry of type {kind}"
        );
    }
}

#[test]
fn a_table_that_ends_inside_an_entry_header_is_refused() {
    let mut bytes = madt(LAPIC, 0, &[]);
    bytes.push(1);
    fix_table(&mut bytes);
    assert_eq!(parse(&bytes), Err(AcpiError::TooShort(MADT_HEADER_LEN)));
}

#[test]
fn a_table_that_ends_before_the_flags_is_refused() {
    for length in [SDT_HEADER_LEN, SDT_HEADER_LEN + 4] {
        let mut bytes = madt(LAPIC, 0, &[]);
        bytes.truncate(length);
        fix_table(&mut bytes);
        assert_eq!(
            parse(&bytes),
            Err(AcpiError::TooShort(length)),
            "a table of {length} bytes"
        );
    }
}

#[test]
fn more_io_apics_than_the_kernel_holds_are_refused_and_not_truncated() {
    let entries: Vec<Vec<u8>> = (0..=MAX_IO_APICS)
        .map(|index| io_apic(index as u8, IOAPIC, (index as u32) * 24))
        .collect();
    let bytes = madt(LAPIC, 0, &entries);
    assert_eq!(parse(&bytes), Err(AcpiError::TooManyIoApics));
}

#[test]
fn more_overrides_than_the_kernel_holds_are_refused_and_not_truncated() {
    let entries: Vec<Vec<u8>> = (0..=MAX_OVERRIDES)
        .map(|index| source_override(0, index as u8, index as u32, 0))
        .collect();
    let bytes = madt(LAPIC, 0, &entries);
    assert_eq!(parse(&bytes), Err(AcpiError::TooManyOverrides));
}

#[test]
fn a_table_of_another_signature_is_refused() {
    let bytes = table(*b"FACP", 5, &[0u8; 8]);
    assert_eq!(parse(&bytes), Err(AcpiError::Signature(*b"FACP")));
}

#[test]
fn the_timer_line_is_routed_through_the_override_the_firmware_names() {
    let bytes = madt(
        LAPIC,
        PCAT_COMPAT,
        &[
            io_apic(0, IOAPIC, 0),
            source_override(0, 0, 2, 0),
            source_override(0, 4, 4, 0b1111),
        ],
    );
    let parsed = parse(&bytes).expect("the table is well formed");
    let timer = parsed.route_isa(0);
    assert_eq!(timer.gsi, 2);
    assert_eq!(timer.polarity, Polarity::ActiveHigh);
    assert_eq!(timer.trigger, Trigger::Edge);
    let serial = parsed.route_isa(4);
    assert_eq!(serial.gsi, 4);
    assert_eq!(serial.polarity, Polarity::ActiveLow);
    assert_eq!(serial.trigger, Trigger::Level);
    assert_eq!(parsed.override_count(), 2);
}

#[test]
fn a_line_without_an_override_is_the_interrupt_of_the_same_number() {
    let bytes = madt(LAPIC, 0, &[io_apic(0, IOAPIC, 0)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    let routing = parsed.route_isa(7);
    assert_eq!(routing.gsi, 7);
    assert_eq!(routing.polarity, Polarity::ActiveHigh);
    assert_eq!(routing.trigger, Trigger::Edge);
    assert_eq!(parsed.override_for(7), None);
}

#[test]
fn an_override_of_another_bus_does_not_route_an_isa_line() {
    let bytes = madt(LAPIC, 0, &[source_override(1, 4, 20, 0b1111)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.route_isa(4).gsi, 4);
}

#[test]
fn flags_that_say_conforms_or_that_the_specification_reserves_give_the_isa_default() {
    for flags in [0b0000u16, 0b1010] {
        let bytes = madt(LAPIC, 0, &[source_override(0, 4, 4, flags)]);
        let parsed = parse(&bytes).expect("the table is well formed");
        let routing = parsed.route_isa(4);
        assert_eq!(routing.polarity, Polarity::ActiveHigh, "flags {flags:#b}");
        assert_eq!(routing.trigger, Trigger::Edge, "flags {flags:#b}");
    }
}

#[test]
fn a_global_interrupt_belongs_to_the_io_apic_with_the_highest_base_below_it() {
    let bytes = madt(
        LAPIC,
        0,
        &[io_apic(0, IOAPIC, 0), io_apic(1, IOAPIC + 0x1000, 24)],
    );
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.io_apic_for(0).map(|apic| apic.id), Some(0));
    assert_eq!(parsed.io_apic_for(23).map(|apic| apic.id), Some(0));
    assert_eq!(parsed.io_apic_for(24).map(|apic| apic.id), Some(1));
    assert_eq!(parsed.io_apic_for(100).map(|apic| apic.id), Some(1));
}

#[test]
fn an_interrupt_below_every_base_belongs_to_no_io_apic() {
    let bytes = madt(LAPIC, 0, &[io_apic(0, IOAPIC, 8)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.io_apic_for(0), None);
}
