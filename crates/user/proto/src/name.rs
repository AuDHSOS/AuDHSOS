// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The name protocol: putting an endpoint under a name, and asking for the
//! endpoint a name stands for.
//!
//! It is the only way two processes that the root task started separately
//! can reach each other. A capability cannot be guessed, and a process
//! holds only what it was given at its start or was sent in a message; the
//! name server is where a server leaves a capability to itself for whoever
//! asks.
//!
//! Invariant: a reply carries the endpoint in the handle area only when its
//! status word says the lookup succeeded, so a client that reads the status
//! first never takes a handle out of a message that has none.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, Handle};
use user_rt::message::{Reader, Writer};

use crate::bytes::Bytes;
use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// How many bytes a name has at most.
pub const MAX_NAME: usize = 32;

/// A name in the registry.
pub type Name = Bytes<MAX_NAME>;

/// `register`: put an endpoint under a name.
pub const REGISTER: u16 = 1;

/// `lookup`: ask for the endpoint a name stands for.
pub const LOOKUP: u16 = 2;

/// What a client asks the name server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Put `endpoint` under `name`. The handle travels in the handle area
    /// of the message, so the server receives a capability and not a number
    /// out of somebody else's table.
    Register {
        /// The name to register under.
        name: Name,
        /// The endpoint being registered.
        endpoint: Handle,
    },
    /// Ask for the endpoint `name` stands for.
    Lookup {
        /// The name to look up.
        name: Name,
    },
}

/// What the name server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The answer to a registration.
    Registered(Result<(), Error>),
    /// The answer to a lookup: the endpoint, or why there is none.
    Found(Result<Handle, Error>),
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Request::Register { .. } => Label::new(Protocol::Name, REGISTER),
            Request::Lookup { .. } => Label::new(Protocol::Name, LOOKUP),
        }
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area or the handle area has
    /// no room, which a name of at most thirty-two bytes and one handle
    /// never runs into.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Register { name, endpoint } => {
                writer.bytes(buffer, name.as_bytes())?;
                writer.handle(buffer, *endpoint)?;
            }
            Request::Lookup { name } => writer.bytes(buffer, name.as_bytes())?,
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a request out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] for a label of another protocol or another version,
    /// for a message number this protocol does not have, for a name longer
    /// than [`MAX_NAME`], and for a message whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let mut into = [0u8; MAX_NAME];
        let len = reader.bytes(&mut into)?;
        let name = Name::new(into.get(..len).unwrap_or(&[]))?;
        match label.message {
            REGISTER => Ok(Request::Register {
                name,
                endpoint: reader.handle()?,
            }),
            LOOKUP => Ok(Request::Lookup { name }),
            other => Err(ProtoError::Message(Protocol::Name, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under, which is the label of the
    /// request it answers.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Reply::Registered(_) => Label::new(Protocol::Name, REGISTER),
            Reply::Found(_) => Label::new(Protocol::Name, LOOKUP),
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
            Reply::Registered(outcome) => writer.word(buffer, status_word(*outcome))?,
            Reply::Found(Ok(endpoint)) => {
                writer.word(buffer, 0)?;
                writer.handle(buffer, *endpoint)?;
            }
            Reply::Found(Err(error)) => writer.word(buffer, u64::from(error.code()))?,
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a reply out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] as [`Request::decode`], and
    /// [`ProtoError::Status`] for a status word that names no error.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let outcome = status_of(reader.word()?)?;
        match label.message {
            REGISTER => Ok(Reply::Registered(outcome)),
            LOOKUP => match outcome {
                Ok(()) => Ok(Reply::Found(Ok(reader.handle()?))),
                Err(error) => Ok(Reply::Found(Err(error))),
            },
            other => Err(ProtoError::Message(Protocol::Name, other)),
        }
    }
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Name {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Name,
            found: label.protocol,
        });
    }
    Ok(label)
}
