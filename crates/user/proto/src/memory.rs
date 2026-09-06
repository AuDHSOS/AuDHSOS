// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory protocol: asking the memory server for a memory object and
//! giving one back.
//!
//! What travels is capabilities, not addresses. A client asks for a length
//! and an alignment and receives a memory object in the handle area of the
//! reply; where it maps it is its own business, and the server never learns
//! it. Giving one back is the same thing the other way round: the handle in
//! the request is the object, and the server takes it because it holds it.
//!
//! Invariant: a reply carries the object only when its status word says the
//! allocation succeeded.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, Handle};
use user_rt::message::{Reader, Writer};

use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// `allocate`: ask for a memory object.
pub const ALLOCATE: u16 = 1;

/// `release`: give a memory object back.
pub const RELEASE: u16 = 2;

/// What a client asks the memory server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Ask for `len` bytes, aligned to `align`.
    Allocate {
        /// How many bytes, rounded up to whole pages by the server.
        len: u64,
        /// What the first byte has to be a multiple of.
        align: u64,
    },
    /// Give a memory object back. The handle travels in the handle area.
    Release {
        /// The object being returned.
        memory: Handle,
    },
}

/// What the memory server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The object, or why there is none.
    Allocated(Result<Handle, Error>),
    /// Whether the object was taken back.
    Released(Result<(), Error>),
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Request::Allocate { .. } => Label::new(Protocol::Memory, ALLOCATE),
            Request::Release { .. } => Label::new(Protocol::Memory, RELEASE),
        }
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area or the handle area has
    /// no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Allocate { len, align } => {
                writer.word(buffer, *len)?;
                writer.word(buffer, *align)?;
            }
            Request::Release { memory } => writer.handle(buffer, *memory)?,
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a request out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] for a label of another protocol or another version,
    /// for a message number this protocol does not have, and for a message
    /// whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            ALLOCATE => Ok(Request::Allocate {
                len: reader.word()?,
                align: reader.word()?,
            }),
            RELEASE => Ok(Request::Release {
                memory: reader.handle()?,
            }),
            other => Err(ProtoError::Message(Protocol::Memory, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Reply::Allocated(_) => Label::new(Protocol::Memory, ALLOCATE),
            Reply::Released(_) => Label::new(Protocol::Memory, RELEASE),
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
            Reply::Allocated(Ok(memory)) => {
                writer.word(buffer, 0)?;
                writer.handle(buffer, *memory)?;
            }
            Reply::Allocated(Err(error)) => writer.word(buffer, u64::from(error.code()))?,
            Reply::Released(outcome) => writer.word(buffer, status_word(*outcome))?,
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
        match label.message {
            ALLOCATE => match outcome {
                Ok(()) => Ok(Reply::Allocated(Ok(reader.handle()?))),
                Err(error) => Ok(Reply::Allocated(Err(error))),
            },
            RELEASE => Ok(Reply::Released(outcome)),
            other => Err(ProtoError::Message(Protocol::Memory, other)),
        }
    }
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Memory {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Memory,
            found: label.protocol,
        });
    }
    Ok(label)
}
