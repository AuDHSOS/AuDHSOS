// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The two legacy interrupt controllers, moved out of the way and masked.
//!
//! Invariant: after [`disable`] both controllers deliver on vectors the
//! plan reserves for them and every line of both is masked, so the only
//! thing that can still arrive from them is the spurious interrupt one of
//! them raises on its own.

use kernel_x86_tables::pic;
use kernel_x86_tables::vectors::{PIC_BASE, PIC_SLAVE_BASE};

use crate::instructions::write_port_u8;

/// Moves both controllers to the vectors of the plan and masks every line
/// of both.
///
/// The remapping comes first and the masking second, so that a line that
/// was already asserted when the kernel started arrives on a vector that
/// has a handler rather than on an exception vector that means something
/// else.
///
/// # Safety
///
/// The two controllers must belong to the kernel, which they do until a
/// driver in userland asks for them, and the interrupt descriptor table
/// must already carry handlers for the vectors of the plan.
pub unsafe fn disable() {
    for (port, value) in pic::remap(PIC_BASE, PIC_SLAVE_BASE) {
        // SAFETY: the caller promises that the two controllers belong to
        // the kernel; the sequence is the one the specification gives and
        // it ends with every line masked.
        unsafe {
            write_port_u8(port, value);
        }
    }
}
