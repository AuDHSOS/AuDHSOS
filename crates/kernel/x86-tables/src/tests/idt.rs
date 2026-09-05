// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::idt`, covering the gate items of the catalog 6.6.16.

#![allow(clippy::arithmetic_side_effects)]

use crate::gdt::KERNEL_CODE_SELECTOR;
use crate::idt::{
    DOUBLE_FAULT_IST, GATE_INTERRUPT_DPL0, GATE_INTERRUPT_DPL3, IDT_ENTRIES, MISSING,
    SYSCALL_VECTOR, gate, gate_attributes, gate_handler, gate_ist, gate_present, gate_privilege,
    gate_selector,
};
use test_support::generators::range;
use test_support::property::check;

#[test]
fn a_gate_splits_the_handler_address_across_three_fields() {
    let handler = 0xFFFF_FFFF_8012_3456;
    let entry = gate(
        handler,
        KERNEL_CODE_SELECTOR.as_u16(),
        0,
        GATE_INTERRUPT_DPL0,
    );
    let [low, high] = entry;
    assert_eq!(low & 0xFFFF, 0x3456, "offset bits 0 to 15");
    assert_eq!((low >> 48) & 0xFFFF, 0x8012, "offset bits 16 to 31");
    assert_eq!(high, 0xFFFF_FFFF, "offset bits 32 to 63");
    assert_eq!(gate_handler(entry), handler);
    assert_eq!(gate_selector(entry), 0x08);
}

#[test]
fn the_interrupt_stack_index_is_carried_and_bounded() {
    for index in [0u8, 1, 7] {
        let entry = gate(0x1000, 0x08, index, GATE_INTERRUPT_DPL0);
        assert_eq!(gate_ist(entry), index);
    }
    let masked = gate(0x1000, 0x08, 0xFF, GATE_INTERRUPT_DPL0);
    assert_eq!(gate_ist(masked), 7, "only three bits carry the index");
    assert_eq!(DOUBLE_FAULT_IST, 1);
}

#[test]
fn the_attribute_byte_names_an_interrupt_gate_and_its_privilege_level() {
    let kernel = gate(0x1000, 0x08, 0, GATE_INTERRUPT_DPL0);
    assert_eq!(gate_attributes(kernel), 0x8E);
    assert!(gate_present(kernel));
    assert_eq!(gate_privilege(kernel), 0);

    let user = gate(0x2000, 0x08, 0, GATE_INTERRUPT_DPL3);
    assert_eq!(gate_attributes(user), 0xEE);
    assert!(gate_present(user));
    assert_eq!(
        gate_privilege(user),
        3,
        "vector 0x80 is entered from ring 3"
    );
    assert_eq!(SYSCALL_VECTOR, 0x80);
    assert_eq!(GATE_INTERRUPT_DPL0 & 0xF, 0xE, "an interrupt gate");
    assert_eq!(GATE_INTERRUPT_DPL3 & 0xF, 0xE);
}

#[test]
fn a_missing_entry_is_absent_and_names_nothing() {
    assert!(!gate_present(MISSING));
    assert_eq!(gate_handler(MISSING), 0);
    assert_eq!(gate_selector(MISSING), 0);
    assert_eq!(gate_ist(MISSING), 0);
    assert_eq!(IDT_ENTRIES, 256);
}

#[test]
fn property_every_handler_address_round_trips() {
    check("gate_round_trip", &range(0u64..=u64::MAX), |handler| {
        let entry = gate(*handler, 0x08, 1, GATE_INTERRUPT_DPL0);
        if gate_handler(entry) != *handler {
            return Err(format!("{handler:#x} did not round-trip"));
        }
        if gate_ist(entry) != 1 || gate_selector(entry) != 0x08 {
            return Err("the other fields changed".to_owned());
        }
        Ok(())
    });
}
