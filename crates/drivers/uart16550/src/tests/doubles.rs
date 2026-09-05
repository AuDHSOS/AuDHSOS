// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::doubles`.

use crate::doubles::{Access, RecordingRegisters};
use crate::uart::{Register, Registers};

#[test]
fn a_fresh_block_reads_zero_and_records_every_access() {
    let mut registers = RecordingRegisters::new();
    assert_eq!(registers.read(Register::Scratch), 0);
    registers.write(Register::Scratch, 0x5A);
    assert_eq!(registers.read(Register::Scratch), 0x5A);
    assert_eq!(
        registers.log(),
        &[
            Access::Read(Register::Scratch, 0),
            Access::Write(Register::Scratch, 0x5A),
            Access::Read(Register::Scratch, 0x5A),
        ]
    );
    assert_eq!(registers.reads_of(Register::Scratch), 2);
    assert_eq!(registers.writes(), vec![(Register::Scratch, 0x5A)]);
}

#[test]
fn a_script_is_consumed_before_the_stored_value() {
    let mut registers = RecordingRegisters::new();
    registers.set(Register::LineStatus, 0x20);
    registers.script(Register::LineStatus, &[0x01, 0x02]);
    assert_eq!(registers.read(Register::LineStatus), 0x01);
    assert_eq!(registers.read(Register::LineStatus), 0x02);
    assert_eq!(registers.read(Register::LineStatus), 0x20);
}

#[test]
fn clearing_forgets_the_log_but_not_the_values() {
    let mut registers = RecordingRegisters::default();
    registers.write(Register::ModemControl, 0x0B);
    registers.clear();
    assert!(registers.log().is_empty());
    assert_eq!(registers.read(Register::ModemControl), 0x0B);
}
