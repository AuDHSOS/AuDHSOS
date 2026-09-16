// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Errors cross the embedding boundary as typed values, never panics.

use alloc::string::String;
use core::fmt;

/// A compilation, execution, or embedding error.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// A parser or static-semantics rejection at a UTF-8 byte offset.
    /// Recognized valid-but-unavailable syntax uses [`Self::Unsupported`].
    Syntax {
        /// Zero-based byte offset in the source.
        offset: usize,
        /// The grammar condition that failed.
        message: &'static str,
    },
    /// The current parser rejected source whose validity has not been proven.
    /// Conformance runners must not treat this as a verified `SyntaxError`.
    UnverifiedSyntax {
        /// Zero-based UTF-8 byte offset in the source.
        offset: usize,
        /// The parser expectation that failed.
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
    /// An uncaught throw of a value the embedding has no way to hold.
    ///
    /// The Script threw, which is a completion of the language and not a
    /// feature the engine is missing. An Object of the register engine has no
    /// identity outside it, so what was thrown cannot be handed over; that the
    /// Script threw at all is what crosses, with the text 20.5.3.4 would
    /// answer where the object holds its two names as data properties.
    ThrownUnrepresentable {
        /// `name: message` of the thrown object, empty where it holds neither
        /// as a String of its own.
        description: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { offset, message } => {
                write!(f, "SyntaxError at byte {offset}: {message}")
            }
            Self::UnverifiedSyntax { offset, message } => {
                write!(f, "unverified syntax rejection at byte {offset}: {message}")
            }
            Self::Reference { name } => {
                write!(f, "ReferenceError: {name} is not initialized or defined")
            }
            Self::Type { message } => write!(f, "TypeError: {message}"),
            Self::Range { message } => write!(f, "RangeError: {message}"),
            Self::Limit { resource } => write!(f, "resource limit: {resource}"),
            Self::ThrownUnrepresentable { description } if description.is_empty() => {
                write!(f, "threw a value the embedding cannot hold")
            }
            Self::ThrownUnrepresentable { description } => {
                write!(f, "threw a value the embedding cannot hold: {description}")
            }
            Self::Unsupported { feature } => write!(f, "unsupported feature: {feature}"),
            Self::InvalidBytecode => f.write_str("invalid bytecode"),
            Self::Host => f.write_str("host capability failed"),
            Self::Thrown { value } => write!(f, "uncaught JavaScript exception: {value}"),
        }
    }
}

impl core::error::Error for Error {}
