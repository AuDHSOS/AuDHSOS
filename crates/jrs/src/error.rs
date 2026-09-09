// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Errors cross the embedding boundary as typed values, never panics.

use alloc::string::String;
use core::fmt;

/// A compilation, execution, or embedding error.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// Invalid or currently unsupported syntax at a UTF-8 byte offset.
    Syntax {
        /// Zero-based byte offset in the source.
        offset: usize,
        /// The grammar condition that failed.
        message: &'static str,
    },
    /// An unresolved or uninitialized binding.
    Reference {
        /// The binding name.
        name: String,
    },
    /// An operation is invalid for its operand.
    Type {
        /// Description of the invalid operation.
        message: &'static str,
    },
    /// A numeric argument is outside a language-defined range.
    Range {
        /// Description of the invalid range.
        message: &'static str,
    },
    /// A caller-supplied resource budget was exhausted.
    Limit {
        /// The resource that ran out.
        resource: &'static str,
    },
    /// Valid language functionality is unavailable, not a language exception.
    /// This fatal diagnostic cannot satisfy negative conformance tests.
    Unsupported {
        /// Unimplemented or deliberately forbidden feature.
        feature: &'static str,
    },
    /// A bytecode invariant failed; programs cannot be constructed externally.
    InvalidBytecode,
    /// A host capability failed or returned an invalid/foreign heap value.
    Host,
    /// An uncaught JavaScript throw completion. Realm APIs retain heap identities
    /// until release/drop; identities from isolated Runtime runs are diagnostic.
    Thrown {
        /// The exact value thrown by the script.
        value: crate::Value,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { offset, message } => {
                write!(f, "SyntaxError at byte {offset}: {message}")
            }
            Self::Reference { name } => {
                write!(f, "ReferenceError: {name} is not initialized or defined")
            }
            Self::Type { message } => write!(f, "TypeError: {message}"),
            Self::Range { message } => write!(f, "RangeError: {message}"),
            Self::Limit { resource } => write!(f, "resource limit: {resource}"),
            Self::Unsupported { feature } => write!(f, "unsupported feature: {feature}"),
            Self::InvalidBytecode => f.write_str("invalid bytecode"),
            Self::Host => f.write_str("host capability failed"),
            Self::Thrown { value } => write!(f, "uncaught JavaScript exception: {value}"),
        }
    }
}

impl core::error::Error for Error {}
