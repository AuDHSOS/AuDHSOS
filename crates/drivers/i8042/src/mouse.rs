// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The packet a PS/2 mouse sends, and the state machine that reads it.
//!
//! A packet is three bytes, or four when the device answered [`WHEEL_ID`]
//! to `GET_ID`. The first byte carries the buttons, the signs of the two
//! deltas, the two overflow bits, and one bit that is always set; that bit
//! is the only thing that says where a packet begins, so a stream that has
//! lost a byte is found again by dropping bytes until one has it.
//!
//! The vertical delta of the wire runs upwards and the vertical delta of a
//! screen runs downwards. It is turned around here, once, so that everything
//! above this module counts rows the way a surface counts them.
//!
//! Invariants: the buffer holds at most four bytes and is emptied by every
//! packet, so no stream can make the decoder grow; a delta whose overflow
//! bit is set is clamped and never wraps.

use crate::device::WHEEL_ID;

/// Bit of the first byte: the left button is down.
pub const BUTTON_LEFT: u8 = 0x01;

/// Bit of the first byte: the right button is down.
pub const BUTTON_RIGHT: u8 = 0x02;

/// Bit of the first byte: the middle button is down.
pub const BUTTON_MIDDLE: u8 = 0x04;

/// The buttons of the first byte.
pub const BUTTON_MASK: u8 = BUTTON_LEFT | BUTTON_RIGHT | BUTTON_MIDDLE;

/// Bit of the first byte that is set in every packet, and in no other byte
/// the mouse sends by itself.
pub const SYNC: u8 = 0x08;

/// Bit of the first byte: the horizontal delta is negative.
pub const X_SIGN: u8 = 0x10;

/// Bit of the first byte: the vertical delta is negative.
pub const Y_SIGN: u8 = 0x20;

/// Bit of the first byte: the horizontal movement did not fit.
pub const X_OVERFLOW: u8 = 0x40;

/// Bit of the first byte: the vertical movement did not fit.
pub const Y_OVERFLOW: u8 = 0x80;

/// The largest delta a packet can carry.
pub const DELTA_MAX: i16 = 255;

/// The smallest delta a packet can carry.
pub const DELTA_MIN: i16 = -256;

/// How many bytes a packet has without a wheel.
pub const PACKET_LEN: usize = 3;

/// How many bytes a packet has with one.
pub const WHEEL_PACKET_LEN: usize = 4;

/// The wheel field of the fourth byte.
pub const WHEEL_MASK: u8 = 0x0F;

/// Above this the wheel field of the fourth byte is negative.
pub const WHEEL_SIGN: u8 = 0x08;

/// A movement of the pointer, its wheel, or its buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PointerEvent {
    /// Columns moved, positive to the right.
    pub dx: i16,
    /// Rows moved, positive downwards, as a surface counts them.
    pub dy: i16,
    /// Wheel notches, positive away from the hand.
    pub wheel: i8,
    /// Which buttons are down, as [`BUTTON_LEFT`] and its neighbours name
    /// them.
    pub buttons: u8,
}

/// How many bytes a mouse that answered `id` sends per packet.
#[must_use]
pub const fn packet_len(id: u8) -> usize {
    if id == WHEEL_ID {
        WHEEL_PACKET_LEN
    } else {
        PACKET_LEN
    }
}

/// The state machine of the mouse packet.
#[derive(Clone, Copy, Debug)]
pub struct Decoder {
    /// How many bytes one packet has, which the identifier of the device
    /// decided.
    len: usize,
    /// The bytes of the packet being read.
    bytes: [u8; WHEEL_PACKET_LEN],
    /// How many of them have arrived.
    have: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder::new(PACKET_LEN)
    }
}

impl Decoder {
    /// A decoder for packets of `len` bytes, clamped to what a packet can
    /// be: a device that answered something this driver does not know
    /// sends three bytes, which every mouse does.
    #[must_use]
    pub const fn new(len: usize) -> Self {
        Decoder {
            len: if len == WHEEL_PACKET_LEN {
                WHEEL_PACKET_LEN
            } else {
                PACKET_LEN
            },
            bytes: [0; WHEEL_PACKET_LEN],
            have: 0,
        }
    }

    /// A decoder for the device that answered `id` to `GET_ID`.
    #[must_use]
    pub const fn for_id(id: u8) -> Self {
        Decoder::new(packet_len(id))
    }

    /// How many bytes one packet has.
    #[must_use]
    pub const fn packet_len(&self) -> usize {
        self.len
    }

    /// How many bytes of the packet being read have arrived.
    #[must_use]
    pub const fn pending(&self) -> usize {
        self.have
    }

    /// Throws away the half packet that stands, which is what a driver does
    /// when it has lost bytes and cannot say how many.
    pub const fn reset(&mut self) {
        self.have = 0;
    }

    /// Takes one byte and answers with the packet it completed, if it
    /// completed one.
    pub fn feed(&mut self, byte: u8) -> Option<PointerEvent> {
        // The first byte of a packet is the only one whose value is
        // checked: it is what says that this is a first byte at all.
        if self.have == 0 && byte & SYNC == 0 {
            return None;
        }
        let Some(slot) = self.bytes.get_mut(self.have) else {
            self.have = 0;
            return None;
        };
        *slot = byte;
        self.have = self.have.saturating_add(1);
        if self.have < self.len {
            return None;
        }
        self.have = 0;
        Some(self.decode())
    }

    /// Turns the bytes that have arrived into an event.
    fn decode(&self) -> PointerEvent {
        let flags = self.bytes.first().copied().unwrap_or(0);
        let x = self.bytes.get(1).copied().unwrap_or(0);
        let y = self.bytes.get(2).copied().unwrap_or(0);
        let wheel = if self.len == WHEEL_PACKET_LEN {
            notches(self.bytes.get(3).copied().unwrap_or(0))
        } else {
            0
        };
        PointerEvent {
            dx: delta(x, flags & X_SIGN != 0, flags & X_OVERFLOW != 0),
            // The wire counts rows upwards and a surface counts them
            // downwards, so what the mouse calls up is a negative row here.
            dy: delta(y, flags & Y_SIGN != 0, flags & Y_OVERFLOW != 0).saturating_neg(),
            wheel,
            buttons: flags & BUTTON_MASK,
        }
    }
}

/// One delta of a packet: nine bits, sign and magnitude apart, clamped to
/// what those nine bits hold when the device says the movement did not fit.
fn delta(magnitude: u8, negative: bool, overflow: bool) -> i16 {
    if overflow {
        return if negative { DELTA_MIN } else { DELTA_MAX };
    }
    let value = i16::from(magnitude);
    if negative {
        return value.saturating_sub(256);
    }
    value
}

/// The wheel field of the fourth byte, which is four bits in two's
/// complement.
fn notches(byte: u8) -> i8 {
    let field = byte & WHEEL_MASK;
    if field >= WHEEL_SIGN {
        return i8::try_from(i16::from(field).saturating_sub(16)).unwrap_or(0);
    }
    i8::try_from(field).unwrap_or(0)
}
