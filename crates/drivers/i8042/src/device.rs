// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What is said to the two devices behind the controller, and what they
//! say back.
//!
//! Every command is one byte and every device answers it with an
//! acknowledgement before anything else, so the answer of a command is
//! consumed here and never reaches a decoder. A device that asks for the
//! byte again gets it a bounded number of times; one that answers anything
//! else is a device this driver does not use.
//!
//! Invariants: no command is sent without its answer being read; a device
//! that does not answer makes the whole of its own initialization fail and
//! nothing else.

use crate::controller::{Controller, Error, Ports};

/// What a device answers a command it took.
pub const ACK: u8 = 0xFA;

/// What a device answers a command it wants again.
pub const RESEND: u8 = 0xFE;

/// What a device answers after a reset when it works.
pub const RESET_PASSED: u8 = 0xAA;

/// Command: reset yourself and run your self-test.
pub const RESET: u8 = 0xFF;

/// Command: the next byte says which scancode set to send.
pub const SET_SCANCODE_SET: u8 = 0xF0;

/// The scancode set this driver decodes.
pub const SCANCODE_SET_2: u8 = 0x02;

/// Command: send what happens from now on.
pub const ENABLE_REPORTING: u8 = 0xF4;

/// Command: the next byte says how often to report.
pub const SET_SAMPLE_RATE: u8 = 0xF3;

/// Command: say what you are.
pub const GET_ID: u8 = 0xF2;

/// The sample rates that make an `IntelliMouse` report its wheel. A mouse
/// that sees these three in this order answers [`GET_ID`] with
/// [`WHEEL_ID`] afterwards and sends four bytes per packet from then on.
pub const WHEEL_KNOCK: [u8; 3] = [200, 100, 80];

/// The identifier a mouse with a wheel answers with.
pub const WHEEL_ID: u8 = 3;

/// How often a device may ask for the same byte again.
pub const MAX_RESENDS: u32 = 3;

/// Resets the keyboard, puts it into scancode set 2, and lets it report.
///
/// # Errors
///
/// [`Error::Timeout`] when the controller does not answer,
/// [`Error::NotAcknowledged`] when the keyboard answers something else —
/// which is what a machine without a keyboard looks like.
pub fn start_keyboard<P: Ports>(controller: &mut Controller<P>) -> Result<(), Error> {
    keyboard_command(controller, RESET)?;
    expect(controller, RESET_PASSED)?;
    keyboard_command(controller, SET_SCANCODE_SET)?;
    keyboard_command(controller, SCANCODE_SET_2)?;
    keyboard_command(controller, ENABLE_REPORTING)
}

/// Resets the mouse, asks it for its wheel, and lets it report; answers
/// with the identifier it gave.
///
/// # Errors
///
/// As [`start_keyboard`].
pub fn start_mouse<P: Ports>(controller: &mut Controller<P>) -> Result<u8, Error> {
    mouse_command(controller, RESET)?;
    expect(controller, RESET_PASSED)?;
    // The identifier a reset answers with, which is zero for every mouse:
    // the wheel is asked for below and not here.
    controller.read()?;
    for rate in WHEEL_KNOCK {
        mouse_command(controller, SET_SAMPLE_RATE)?;
        mouse_command(controller, rate)?;
    }
    mouse_command(controller, GET_ID)?;
    let id = controller.read()?;
    mouse_command(controller, ENABLE_REPORTING)?;
    Ok(id)
}

/// Sends one byte to the keyboard and reads its acknowledgement.
///
/// # Errors
///
/// As [`start_keyboard`].
pub fn keyboard_command<P: Ports>(controller: &mut Controller<P>, byte: u8) -> Result<(), Error> {
    let mut tries = 0u32;
    loop {
        controller.write(byte)?;
        match controller.read()? {
            ACK => return Ok(()),
            RESEND if tries < MAX_RESENDS => tries = tries.saturating_add(1),
            other => return Err(Error::NotAcknowledged(other)),
        }
    }
}

/// Sends one byte to the mouse and reads its acknowledgement.
///
/// # Errors
///
/// As [`start_keyboard`].
pub fn mouse_command<P: Ports>(controller: &mut Controller<P>, byte: u8) -> Result<(), Error> {
    let mut tries = 0u32;
    loop {
        controller.write_aux(byte)?;
        match controller.read()? {
            ACK => return Ok(()),
            RESEND if tries < MAX_RESENDS => tries = tries.saturating_add(1),
            other => return Err(Error::NotAcknowledged(other)),
        }
    }
}

/// Reads one byte and insists it is `wanted`.
fn expect<P: Ports>(controller: &mut Controller<P>, wanted: u8) -> Result<(), Error> {
    let answer = controller.read()?;
    if answer == wanted {
        return Ok(());
    }
    Err(Error::NotAcknowledged(answer))
}
