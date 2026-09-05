// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The status codes every UEFI service returns.
//!
//! Invariant: a code with the high bit set is an error, a code without it
//! is a success or a warning.

use core::fmt;

/// The bit that separates errors from successes and warnings.
pub const ERROR_BIT: usize = 1 << (usize::BITS - 1);

/// A status code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Status(pub usize);

impl Status {
    /// The operation succeeded.
    pub const SUCCESS: Status = Status(0);
    /// The image failed to load.
    pub const LOAD_ERROR: Status = Status::error(1);
    /// A parameter was outside its range.
    pub const INVALID_PARAMETER: Status = Status::error(2);
    /// The operation is not supported.
    pub const UNSUPPORTED: Status = Status::error(3);
    /// The buffer was too small; the required size came back.
    pub const BUFFER_TOO_SMALL: Status = Status::error(5);
    /// The device is not ready.
    pub const NOT_READY: Status = Status::error(6);
    /// A resource ran out.
    pub const OUT_OF_RESOURCES: Status = Status::error(9);
    /// The item does not exist.
    pub const NOT_FOUND: Status = Status::error(14);

    /// The error code with the given number.
    #[must_use]
    pub const fn error(code: usize) -> Status {
        Status(code | ERROR_BIT)
    }

    /// `true` if the code reports success.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 == 0
    }

    /// `true` if the code reports an error.
    #[must_use]
    pub const fn is_error(self) -> bool {
        self.0 & ERROR_BIT != 0
    }

    /// `Ok(value)` for a success, `Err(self)` for anything else.
    ///
    /// # Errors
    ///
    /// The status itself when it does not report success.
    pub fn ok<T>(self, value: T) -> Result<T, Status> {
        if self.is_success() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_error() {
            write!(f, "UEFI error {}", self.0 & !ERROR_BIT)
        } else {
            write!(f, "UEFI status {}", self.0)
        }
    }
}
