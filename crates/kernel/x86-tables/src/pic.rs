// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two legacy interrupt controllers, which this kernel programs once
//! and then never uses: the machine may still have them, and a controller
//! left as the firmware set it up delivers its lines on the vectors the
//! processor uses for exceptions.
//!
//! Invariant: the sequence this module names ends with both controllers
//! masked, so nothing arrives from them afterwards.

/// The command port of the first controller.
pub const MASTER_COMMAND: u16 = 0x20;

/// The data port of the first controller.
pub const MASTER_DATA: u16 = 0x21;

/// The command port of the second controller.
pub const SLAVE_COMMAND: u16 = 0xA0;

/// The data port of the second controller.
pub const SLAVE_DATA: u16 = 0xA1;

/// The first initialization word: the sequence starts and a fourth word
/// follows.
const ICW1_INIT: u8 = 0x11;

/// The third initialization word of the first controller: the second one
/// hangs off line two.
const ICW3_MASTER: u8 = 0x04;

/// The third initialization word of the second controller: it is the one
/// on line two.
const ICW3_SLAVE: u8 = 0x02;

/// The fourth initialization word: 8086 mode.
const ICW4_8086: u8 = 0x01;

/// The mask that stops every line.
pub const MASK_ALL: u8 = 0xFF;

/// Number of writes the remapping takes.
pub const REMAP_WRITES: usize = 10;

/// Number of lines one controller has.
pub const LINES_PER_CONTROLLER: u8 = 8;

/// The writes that move the two controllers to `master_base` and
/// `slave_base` and then mask every line of both, in order.
#[must_use]
pub const fn remap(master_base: u8, slave_base: u8) -> [(u16, u8); REMAP_WRITES] {
    [
        (MASTER_COMMAND, ICW1_INIT),
        (SLAVE_COMMAND, ICW1_INIT),
        (MASTER_DATA, master_base),
        (SLAVE_DATA, slave_base),
        (MASTER_DATA, ICW3_MASTER),
        (SLAVE_DATA, ICW3_SLAVE),
        (MASTER_DATA, ICW4_8086),
        (SLAVE_DATA, ICW4_8086),
        (MASTER_DATA, MASK_ALL),
        (SLAVE_DATA, MASK_ALL),
    ]
}
