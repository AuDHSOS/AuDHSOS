// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The register block of the local APIC: the offsets the specification
//! gives and the encoding of the local vector table entries.
//!
//! Invariant: an entry this module builds names its vector, its delivery
//! mode, and its mask bit and sets nothing else; the reserved bits stay
//! zero.

#![expect(
    clippy::as_conversions,
    reason = "this module encodes and decodes bit fields; every conversion widens or narrows a value that was masked to the width first, and the functions are const"
)]

/// Number of bytes of the register window. The window is one page even
/// though the last register sits well below its end.
pub const REGISTER_WINDOW_LEN: usize = 4096;

/// The identifier of this processor's local APIC.
pub const ID: usize = 0x20;

/// The version of the local APIC and the number of its local vector table
/// entries.
pub const VERSION: usize = 0x30;

/// The task priority register, which decides which vectors get through.
pub const TPR: usize = 0x80;

/// The end-of-interrupt register; a write of zero acknowledges.
pub const EOI: usize = 0xB0;

/// The spurious interrupt vector register, which also carries the enable
/// bit of the whole unit.
pub const SVR: usize = 0xF0;

/// The local vector table entry of the timer.
pub const LVT_TIMER: usize = 0x320;

/// The local vector table entry of the first local interrupt pin.
pub const LVT_LINT0: usize = 0x350;

/// The local vector table entry of the second local interrupt pin.
pub const LVT_LINT1: usize = 0x360;

/// The local vector table entry of the error interrupt.
pub const LVT_ERROR: usize = 0x370;

/// The count the timer is loaded with.
pub const TIMER_INITIAL: usize = 0x380;

/// The count the timer has left.
pub const TIMER_CURRENT: usize = 0x390;

/// The divisor the timer counts the bus clock down by.
pub const TIMER_DIVIDE: usize = 0x3E0;

/// The bit of [`SVR`] that turns the unit on.
pub const SVR_ENABLE: u32 = 1 << 8;

/// The bit of a local vector table entry that stops its delivery.
pub const LVT_MASKED: u32 = 1 << 16;

/// The bit of the timer's entry that makes it reload itself.
pub const LVT_PERIODIC: u32 = 1 << 17;

/// The value of [`TIMER_DIVIDE`] that divides the bus clock by sixteen.
/// The field is split around bit 2, which is why sixteen is `0b0011` and
/// not `0b0100`.
pub const DIVIDE_BY_16: u32 = 0b0011;

/// The value of [`TIMER_DIVIDE`] that does not divide at all.
pub const DIVIDE_BY_1: u32 = 0b1011;

/// The spurious interrupt vector register with the unit enabled and
/// `vector` as the vector a spurious interrupt arrives on.
#[must_use]
pub const fn spurious(vector: u8) -> u32 {
    SVR_ENABLE | vector as u32
}

/// A local vector table entry for `vector`, masked or not, periodic or
/// one-shot. Delivery mode `fixed` and the reserved bits stay zero.
#[must_use]
pub const fn lvt(vector: u8, masked: bool, periodic: bool) -> u32 {
    let mut entry = vector as u32;
    if masked {
        entry |= LVT_MASKED;
    }
    if periodic {
        entry |= LVT_PERIODIC;
    }
    entry
}

/// The vector a local vector table entry names.
#[must_use]
pub const fn lvt_vector(entry: u32) -> u8 {
    (entry & 0xFF) as u8
}

/// `true` if a local vector table entry is masked.
#[must_use]
pub const fn lvt_masked(entry: u32) -> bool {
    entry & LVT_MASKED != 0
}

/// `true` if the timer entry reloads itself.
#[must_use]
pub const fn lvt_periodic(entry: u32) -> bool {
    entry & LVT_PERIODIC != 0
}

/// The count the timer is loaded with to deliver `ticks_per_second` ticks
/// per second, given that it counts down `ticks_per_ms` times in a
/// millisecond. `None` for a rate of zero, for a rate the timer counts too
/// slowly for, and for a count that does not fit the register.
#[must_use]
pub fn count_for_rate(ticks_per_ms: u32, ticks_per_second: u32) -> Option<u32> {
    if ticks_per_second == 0 {
        return None;
    }
    let per_second = u64::from(ticks_per_ms).saturating_mul(1000);
    let count = per_second.checked_div(u64::from(ticks_per_second))?;
    if count == 0 {
        return None;
    }
    u32::try_from(count).ok()
}
