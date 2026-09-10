// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The console protocol: writing bytes to the serial line and reading the
//! bytes that arrived on it.
//!
//! A write carries at most [`MAX_CHUNK`] bytes. The message area would hold
//! far more, but a chunk is a value on the stack of everyone who handles
//! it, and a client with more to say sends more messages — which is what it
//! would have to do for a line of any length anyway.
//!
//! A read says how many bytes it will take and gets what has arrived. A
//! read that finds nothing waits: the driver keeps the call and answers it
//! when the next byte comes in. Asking again instead is a loop, and a
//! program in a loop takes the whole processor of this machine, because
//! there is no timer a program can ask to be woken by. The driver holds one
//! read at a time; a second one is answered at once with what has arrived,
//! which is nothing.
//!
//! Invariant: a reply carries bytes only when its status word says the read
//! succeeded.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use user_rt::message::{Reader, Writer};

use crate::bytes::Bytes;
use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// How many bytes one write or one read carries at most.
pub const MAX_CHUNK: usize = 256;

/// The bytes of one write or one read.
pub type Chunk = Bytes<MAX_CHUNK>;

/// `write`: put bytes on the line.
pub const WRITE: u16 = 1;

/// `read`: take the bytes that have arrived.
pub const READ: u16 = 2;

/// What a client asks the console driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "a chunk is what the protocol carries, and boxing it would need a heap this system does not have"
)]
pub enum Request {
    /// Put these bytes on the line.
    Write {
        /// The bytes.
        bytes: Chunk,
    },
    /// Take at most this many of the bytes that have arrived.
    Read {
        /// The upper bound the client can hold.
        max: u64,
    },
}

/// What the console driver answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "as `Request`: the bytes are the message"
)]
pub enum Reply {
    /// How many bytes went out, or why none did.
    Written(Result<u64, Error>),
    /// The bytes that had arrived, or why they could not be had.
    Read(Result<Chunk, Error>),
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Request::Write { .. } => Label::new(Protocol::Console, WRITE),
            Request::Read { .. } => Label::new(Protocol::Console, READ),
        }
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Write { bytes } => writer.bytes(buffer, bytes.as_bytes())?,
            Request::Read { max } => writer.word(buffer, *max)?,
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a request out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] for a label of another protocol or another version,
    /// for a message number this protocol does not have, for a chunk longer
    /// than [`MAX_CHUNK`], and for a message whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            WRITE => {
                let mut into = [0u8; MAX_CHUNK];
                let len = reader.bytes(&mut into)?;
                Ok(Request::Write {
                    bytes: Chunk::new(into.get(..len).unwrap_or(&[]))?,
                })
            }
            READ => Ok(Request::Read {
                max: reader.word()?,
            }),
            other => Err(ProtoError::Message(Protocol::Console, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Reply::Written(_) => Label::new(Protocol::Console, WRITE),
            Reply::Read(_) => Label::new(Protocol::Console, READ),
        }
    }

    /// Writes the reply into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Reply::Written(Ok(written)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, *written)?;
            }
            Reply::Read(Ok(bytes)) => {
                writer.word(buffer, 0)?;
                writer.bytes(buffer, bytes.as_bytes())?;
            }
            Reply::Written(Err(error)) | Reply::Read(Err(error)) => {
                writer.word(buffer, u64::from(error.code()))?;
            }
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a reply out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] as [`Request::decode`], and [`ProtoError::Status`]
    /// for a status word that names no error.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let outcome = status_of(reader.word()?)?;
        match (label.message, outcome) {
            (WRITE, Ok(())) => Ok(Reply::Written(Ok(reader.word()?))),
            (WRITE, Err(error)) => Ok(Reply::Written(Err(error))),
            (READ, Ok(())) => {
                let mut into = [0u8; MAX_CHUNK];
                let len = reader.bytes(&mut into)?;
                Ok(Reply::Read(Ok(Chunk::new(into.get(..len).unwrap_or(&[]))?)))
            }
            (READ, Err(error)) => Ok(Reply::Read(Err(error))),
            (other, _) => Err(ProtoError::Message(Protocol::Console, other)),
        }
    }
}

/// The status word of a write, which is what a `Reply::Written` carries
/// when it failed.
#[must_use]
pub fn write_status(outcome: Result<(), Error>) -> u64 {
    status_word(outcome)
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Console {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Console,
            found: label.protocol,
        });
    }
    Ok(label)
}
