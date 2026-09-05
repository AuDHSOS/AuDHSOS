// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::pic`.

use crate::pic::{MASK_ALL, MASTER_COMMAND, MASTER_DATA, SLAVE_COMMAND, SLAVE_DATA, remap};

#[test]
fn the_remapping_writes_the_sequence_the_controllers_expect() {
    assert_eq!(
        remap(0x20, 0x28),
        [
            (MASTER_COMMAND, 0x11),
            (SLAVE_COMMAND, 0x11),
            (MASTER_DATA, 0x20),
            (SLAVE_DATA, 0x28),
            (MASTER_DATA, 0x04),
            (SLAVE_DATA, 0x02),
            (MASTER_DATA, 0x01),
            (SLAVE_DATA, 0x01),
            (MASTER_DATA, MASK_ALL),
            (SLAVE_DATA, MASK_ALL),
        ]
    );
}

#[test]
fn the_sequence_ends_with_every_line_of_both_controllers_masked() {
    let writes = remap(0x20, 0x28);
    let tail = writes.split_at(8).1;
    assert_eq!(tail, [(MASTER_DATA, MASK_ALL), (SLAVE_DATA, MASK_ALL)]);
}

#[test]
fn the_ports_are_the_ones_the_machine_has_had_since_the_first_one() {
    assert_eq!(
        [MASTER_COMMAND, MASTER_DATA, SLAVE_COMMAND, SLAVE_DATA],
        [0x20, 0x21, 0xA0, 0xA1]
    );
}
