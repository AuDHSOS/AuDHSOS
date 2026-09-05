// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The exit device of the machine the tests run on.
//!
//! Invariant: a write to the device port ends the machine, so nothing
//! after the write runs.

use kernel_hal_api::exit::{ExitStatus, TestExit};

/// Port of the `isa-debug-exit` device.
pub const EXIT_PORT: u16 = 0xF4;

/// The byte that reports a success; the machine exits with `33`.
pub const SUCCESS_BYTE: u32 = 0x10;

/// The byte that reports a failure; the machine exits with `35`.
pub const FAILURE_BYTE: u32 = 0x11;

/// The exit device.
#[derive(Clone, Copy, Debug, Default)]
pub struct QemuExit;

impl QemuExit {
    /// The device at [`EXIT_PORT`].
    #[must_use]
    pub const fn new() -> Self {
        QemuExit
    }
}

impl TestExit for QemuExit {
    fn exit(&mut self, status: ExitStatus) {
        let value = match status {
            ExitStatus::Success => SUCCESS_BYTE,
            ExitStatus::Failure => FAILURE_BYTE,
        };
        // SAFETY: the port belongs to the exit device of the machine the
        // tests run on, which the kernel owns; the machine ends here.
        unsafe {
            crate::instructions::write_port_u32(EXIT_PORT, value);
        }
        crate::instructions::halt_forever();
    }
}
