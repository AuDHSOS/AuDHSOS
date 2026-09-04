// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The test exit device.

/// The outcome reported to the machine host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitStatus {
    /// Every test passed.
    Success,
    /// At least one test failed.
    Failure,
}

/// Ends the machine with a status. Adapters do not return from `exit`; the
/// test double records the status and returns.
pub trait TestExit {
    /// Requests the exit.
    fn exit(&mut self, status: ExitStatus);
}
