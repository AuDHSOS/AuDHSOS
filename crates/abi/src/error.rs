// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Error codes of the system call interface.
//!
//! Invariants: every variant has a unique, non-zero code that never changes;
//! code `0` is reserved for success and is not an error.

use core::fmt;

/// Declares the error table once and derives the enum, the code lookup, the
/// decoding, the message, and the list of all variants from it.
macro_rules! error_codes {
    ($($variant:ident = $code:literal => $message:literal),+ $(,)?) => {
        /// An error returned by a system call.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum Error {
            $(
                #[doc = $message]
                $variant = $code,
            )+
        }

        impl Error {
            /// Every variant, in table order.
            pub const ALL: &[Error] = &[$(Error::$variant),+];

            /// The stable numeric code of this error.
            #[must_use]
            #[expect(clippy::as_conversions, reason = "discriminant of a repr(u32) enum in a const fn")]
            pub const fn code(self) -> u32 {
                self as u32
            }

            /// Decodes a numeric code. Returns `None` for `0` and for codes
            /// that are not in the table.
            #[must_use]
            pub const fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(Error::$variant),)+
                    _ => None,
                }
            }

            /// A short description of the error.
            #[must_use]
            pub const fn message(self) -> &'static str {
                match self {
                    $(Error::$variant => $message,)+
                }
            }
        }
    };
}

error_codes! {
    InvalidHandle = 1 => "the handle does not name a live capability of the caller",
    WrongObjectType = 2 => "the object behind the handle has a different type than the operation requires",
    AccessDenied = 3 => "the capability lacks a right the operation requires",
    InvalidArgument = 4 => "an argument is outside its valid range",
    OutOfKernelMemory = 5 => "the kernel reserve has no frame left for the operation",
    PoolExhausted = 6 => "the pool for the requested object type is full",
    OutOfHandles = 7 => "the handle table of the target process is full",
    QuotaExceeded = 8 => "the operation would exceed a quota of the process",
    AddressInUse = 9 => "the requested virtual range overlaps an existing region",
    NotMapped = 10 => "the requested virtual range has no mapping",
    AlreadyMapped = 11 => "the page is already mapped",
    Unaligned = 12 => "an address, offset, or length is not aligned as required",
    BufferTooSmall = 13 => "the data does not fit into the buffer",
    WouldBlock = 14 => "the operation would block and non-blocking behavior was requested",
    Busy = 15 => "another thread already waits on the object",
    ObjectDestroyed = 16 => "the object was destroyed while the caller waited on it",
    ReplyDropped = 17 => "the reply object was dropped without a reply",
    NotFound = 18 => "the requested item does not exist",
    AlreadyExists = 19 => "the item exists already",
    Unsupported = 20 => "the operation is not supported on this platform",
    UnknownSyscall = 21 => "the system call number is not in the table",
    ArgumentCount = 22 => "the number of arguments does not match the system call",
    InvalidState = 23 => "the object is not in a state the operation allows",
    NotRunnable = 24 => "the thread cannot run: it has no entry point, no stack, or it has exited",
    Cancelled = 25 => "the operation was cancelled before it completed",
}

impl TryFrom<u32> for Error {
    type Error = Error;

    /// Decodes a numeric code; an unknown code yields `InvalidArgument`.
    fn try_from(code: u32) -> Result<Self, Self::Error> {
        Self::from_code(code).ok_or(Error::InvalidArgument)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}
