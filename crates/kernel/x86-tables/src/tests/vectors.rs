// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::vectors`.

use crate::idt::SYSCALL_VECTOR;
use crate::vectors::{
    EXCEPTIONS, IOAPIC_BASE, IOAPIC_LINES, PIC_BASE, PIC_LAST, PIC_SLAVE_BASE, SPURIOUS, SYSCALL,
    TIMER, for_gsi, gsi_of, is_exception,
};

#[test]
fn the_plan_puts_every_range_where_the_document_says() {
    assert_eq!(EXCEPTIONS, 32);
    assert_eq!([PIC_BASE, PIC_SLAVE_BASE, PIC_LAST], [0x20, 0x28, 0x2F]);
    assert_eq!(TIMER, 0x30);
    assert_eq!(IOAPIC_BASE, 0x40);
    assert_eq!(IOAPIC_LINES, 24);
    assert_eq!(SYSCALL, SYSCALL_VECTOR);
    assert_eq!(SPURIOUS, 0xFF);
}

#[test]
fn every_line_of_the_plan_has_a_vector_below_the_system_call() {
    // The ranges themselves are checked where they are declared, because
    // they are constants and an assertion over them belongs to the build.
    let last = for_gsi(IOAPIC_LINES - 1).expect("the last line has a vector");
    assert!(last < SYSCALL);
    assert!(last > IOAPIC_BASE);
}

#[test]
fn a_line_and_its_vector_name_each_other() {
    for gsi in 0..IOAPIC_LINES {
        let vector = for_gsi(gsi).unwrap();
        assert_eq!(gsi_of(vector), Some(gsi), "line {gsi}");
    }
}

#[test]
fn a_line_beyond_the_plan_has_no_vector() {
    assert_eq!(for_gsi(IOAPIC_LINES), None);
    assert_eq!(for_gsi(u32::MAX), None);
}

#[test]
fn a_vector_outside_the_range_of_the_lines_carries_none() {
    assert_eq!(gsi_of(TIMER), None);
    assert_eq!(gsi_of(IOAPIC_BASE - 1), None);
    assert_eq!(gsi_of(SPURIOUS), None);
    assert_eq!(gsi_of(SYSCALL), None);
    assert_eq!(gsi_of(0), None);
}

#[test]
fn only_the_first_thirty_two_vectors_are_exceptions() {
    assert!(is_exception(0));
    assert!(is_exception(EXCEPTIONS - 1));
    assert!(!is_exception(EXCEPTIONS));
    assert!(!is_exception(TIMER));
}
