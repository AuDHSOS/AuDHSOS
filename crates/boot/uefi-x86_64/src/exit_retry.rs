// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Retry `ExitBootServices` without returning to firmware diagnostics.

use audhsos_uefi::status::Status;

/// Whether any call to `ExitBootServices` has been made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitError {
    /// The first map read failed; console output is still permitted.
    BeforeExit(Status),
    /// An exit was attempted; only the exit device is permitted.
    AfterExit,
}

/// Read a fresh key for each exit attempt.
///
/// # Errors
///
/// [`ExitError::BeforeExit`] for the first map read, or
/// [`ExitError::AfterExit`] after an exit attempt.
pub fn leave<T>(
    mut read_map: impl FnMut() -> Result<(T, usize), Status>,
    mut exit: impl FnMut(usize) -> Status,
) -> Result<T, ExitError> {
    for attempt in 0..3 {
        let (map, key) = read_map().map_err(|status| {
            if attempt == 0 {
                ExitError::BeforeExit(status)
            } else {
                ExitError::AfterExit
            }
        })?;
        if exit(key).is_success() {
            return Ok(map);
        }
    }
    Err(ExitError::AfterExit)
}
