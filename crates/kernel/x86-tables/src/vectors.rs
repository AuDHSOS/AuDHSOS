// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which vector carries what.
//!
//! Invariants: no two ranges of the plan overlap; every device vector is
//! above the last exception the processor defines, so a device interrupt
//! is never mistaken for a fault.

#![expect(
    clippy::as_conversions,
    reason = "the one conversion here widens a byte, and the function is `const`"
)]

/// Number of vectors the processor reserves for its own exceptions.
pub const EXCEPTIONS: u8 = 32;

/// The first vector the two legacy interrupt controllers are moved to.
/// The kernel masks them and expects nothing here but the spurious
/// interrupt one of them can raise on its own.
pub const PIC_BASE: u8 = 0x20;

/// The first vector of the second legacy controller.
pub const PIC_SLAVE_BASE: u8 = 0x28;

/// The last vector of the two legacy controllers.
pub const PIC_LAST: u8 = 0x2F;

/// The vector of the local APIC timer.
pub const TIMER: u8 = 0x30;

/// The first vector an I/O APIC line is routed to; line `gsi` goes to
/// `IOAPIC_BASE + gsi`.
pub const IOAPIC_BASE: u8 = 0x40;

/// Number of global system interrupts the plan reserves vectors for.
pub const IOAPIC_LINES: u32 = 24;

/// The vector the system call interface uses.
pub const SYSCALL: u8 = 0x80;

/// The vector a spurious local APIC interrupt arrives on.
pub const SPURIOUS: u8 = 0xFF;

// No two ranges of the plan overlap.
const _: () = assert!(PIC_BASE >= EXCEPTIONS);
const _: () = assert!(PIC_SLAVE_BASE > PIC_BASE && PIC_SLAVE_BASE <= PIC_LAST);
const _: () = assert!(TIMER > PIC_LAST);
const _: () = assert!(IOAPIC_BASE > TIMER);
const _: () = assert!(IOAPIC_LINES <= SYSCALL.wrapping_sub(IOAPIC_BASE) as u32);
const _: () = assert!(SYSCALL < SPURIOUS);

/// The vector global system interrupt `gsi` is routed to, or `None` for a
/// line beyond what the plan reserves.
#[must_use]
pub fn for_gsi(gsi: u32) -> Option<u8> {
    if gsi >= IOAPIC_LINES {
        return None;
    }
    let line = u8::try_from(gsi).ok()?;
    Some(IOAPIC_BASE.wrapping_add(line))
}

/// The global system interrupt a vector carries, or `None` for a vector
/// that carries none.
#[must_use]
pub const fn gsi_of(vector: u8) -> Option<u32> {
    if vector < IOAPIC_BASE {
        return None;
    }
    let line = vector.wrapping_sub(IOAPIC_BASE) as u32;
    if line >= IOAPIC_LINES {
        None
    } else {
        Some(line)
    }
}

/// `true` if the processor, and not a device, raises this vector.
#[must_use]
pub const fn is_exception(vector: u8) -> bool {
    vector < EXCEPTIONS
}
