// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a label says: which version, which protocol, which message.
//!
//! ```text
//! bits 63..48   version of the protocol
//! bits 47..32   reserved, zero
//! bits 31..16   the protocol
//! bits 15..0    the message inside it
//! ```
//!
//! The version is at the top so that a server can refuse a message of a
//! version it does not speak before it reads a word of it, which is what
//! makes an older server safe against a newer client rather than merely
//! wrong.
//!
//! Every reply begins with a status word: zero, or the code of the error
//! the server answers with. A client therefore reads the outcome from the
//! same place whatever it asked, and a server that fails has one shape of
//! answer and not one per request.
//!
//! Invariants: a label this module builds lies below
//! [`KERNEL_LABEL_BASE`](audhsos_abi::ipc_buffer::KERNEL_LABEL_BASE), so
//! the kernel takes it from a user thread; a label that does not decode is
//! refused before any word of the message is read.

use audhsos_abi::Error;
use user_rt::message::CodecError;

/// The version of every protocol in this crate.
pub const VERSION: u16 = 1;

/// How far up the version sits in a label.
const VERSION_SHIFT: u32 = 48;

/// How far up the protocol sits in a label.
const PROTOCOL_SHIFT: u32 = 16;

/// The bits between the version and the protocol, which are zero.
const RESERVED: u64 = 0x0000_FFFF_0000_0000;

/// A protocol of this system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Protocol {
    /// The name server.
    Name = 1,
    /// The console driver.
    Console = 2,
    /// The memory server.
    Memory = 3,
}

impl Protocol {
    /// Every protocol, in table order.
    pub const ALL: &[Protocol] = &[Protocol::Name, Protocol::Console, Protocol::Memory];

    /// The stable code of the protocol.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "discriminant of a repr(u16) enum in a const fn"
    )]
    pub const fn code(self) -> u16 {
        self as u16
    }

    /// Decodes a protocol code.
    #[must_use]
    pub const fn from_code(code: u16) -> Option<Self> {
        match code {
            1 => Some(Protocol::Name),
            2 => Some(Protocol::Console),
            3 => Some(Protocol::Memory),
            _ => None,
        }
    }

    /// The name of the protocol.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Protocol::Name => "name",
            Protocol::Console => "console",
            Protocol::Memory => "memory",
        }
    }
}

/// A label taken apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    /// The version the sender speaks.
    pub version: u16,
    /// The protocol.
    pub protocol: Protocol,
    /// Which message of that protocol.
    pub message: u16,
}

impl Label {
    /// The label of `message` in `protocol`, at the current version.
    #[must_use]
    pub const fn new(protocol: Protocol, message: u16) -> Self {
        Label {
            version: VERSION,
            protocol,
            message,
        }
    }

    /// The label as it stands in the message header.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening two 16-bit fields into the word they are packed in, in a const fn"
    )]
    pub const fn raw(self) -> u64 {
        let version = (self.version as u64) << VERSION_SHIFT;
        let protocol = (self.protocol.code() as u64) << PROTOCOL_SHIFT;
        version | protocol | self.message as u64
    }

    /// Takes a label apart.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Reserved`] when a bit between the version and the
    /// protocol is set; [`ProtoError::Version`] for a version this crate
    /// does not speak; [`ProtoError::Protocol`] for a protocol it does not
    /// know. The version is read before the protocol, so a message of a
    /// later version is reported as such and not as an unknown protocol.
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "taking the three 16-bit fields out of the word, in a const fn; each truncation is the field"
    )]
    pub const fn parse(raw: u64) -> Result<Self, ProtoError> {
        if raw & RESERVED != 0 {
            return Err(ProtoError::Reserved(raw));
        }
        let version = (raw >> VERSION_SHIFT) as u16;
        if version != VERSION {
            return Err(ProtoError::Version(version));
        }
        let code = (raw >> PROTOCOL_SHIFT) as u16;
        let Some(protocol) = Protocol::from_code(code) else {
            return Err(ProtoError::Protocol(code));
        };
        Ok(Label {
            version,
            protocol,
            message: raw as u16,
        })
    }
}

/// Why a message could not be read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProtoError {
    /// A bit of the label that has no meaning yet was set.
    Reserved(u64),
    /// A version this crate does not speak.
    Version(u16),
    /// A protocol this crate does not know.
    Protocol(u16),
    /// A message number the protocol does not have.
    Message(Protocol, u16),
    /// The label named another protocol than the one being read.
    WrongProtocol {
        /// What was expected.
        expected: Protocol,
        /// What arrived.
        found: Protocol,
    },
    /// A byte string longer than the field that carries it.
    TooLong {
        /// How many bytes the string has.
        len: usize,
        /// How many the field holds.
        capacity: usize,
    },
    /// A status word that names no error.
    Status(u64),
    /// The fields of the message could not be written or read.
    Codec(CodecError),
}

impl From<CodecError> for ProtoError {
    fn from(error: CodecError) -> Self {
        ProtoError::Codec(error)
    }
}

impl From<ProtoError> for Error {
    fn from(error: ProtoError) -> Self {
        match error {
            ProtoError::Codec(codec) => Error::from(codec),
            ProtoError::TooLong { .. } => Error::BufferTooSmall,
            ProtoError::Reserved(_)
            | ProtoError::Version(_)
            | ProtoError::Protocol(_)
            | ProtoError::Message(_, _)
            | ProtoError::WrongProtocol { .. }
            | ProtoError::Status(_) => Error::InvalidArgument,
        }
    }
}

impl core::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProtoError::Reserved(raw) => write!(f, "the label {raw:#x} sets a reserved bit"),
            ProtoError::Version(version) => write!(f, "version {version} is not spoken here"),
            ProtoError::Protocol(code) => write!(f, "{code} names no protocol"),
            ProtoError::Message(protocol, message) => {
                write!(
                    f,
                    "{message} is no message of the {} protocol",
                    protocol.name()
                )
            }
            ProtoError::WrongProtocol { expected, found } => write!(
                f,
                "a message of the {} protocol where the {} protocol was read",
                found.name(),
                expected.name()
            ),
            ProtoError::TooLong { len, capacity } => {
                write!(f, "{len} bytes where the field holds {capacity}")
            }
            ProtoError::Status(word) => write!(f, "the status word {word} names no error"),
            ProtoError::Codec(error) => write!(f, "{error}"),
        }
    }
}

/// Turns a status word into what the server answered.
///
/// # Errors
///
/// [`ProtoError::Status`] for a word that is neither zero nor the code of
/// an error of this interface.
pub fn status_of(word: u64) -> Result<Result<(), Error>, ProtoError> {
    if word == 0 {
        return Ok(Ok(()));
    }
    let code = u32::try_from(word).map_err(|_| ProtoError::Status(word))?;
    let error = Error::from_code(code).ok_or(ProtoError::Status(word))?;
    Ok(Err(error))
}

/// The status word for `outcome`.
#[must_use]
pub fn status_word(outcome: Result<(), Error>) -> u64 {
    match outcome {
        Ok(()) => 0,
        Err(error) => u64::from(error.code()),
    }
}
