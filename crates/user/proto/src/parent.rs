// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The parent protocol: what a process says to the one that started it.
//!
//! It has one message and no reply. A child that is done sends it and
//! exits, so there is nobody left to receive an answer; a parent that
//! replied would wait for a thread that no longer exists.
//!
//! The endpoint it travels over is the one the kernel sends this process's
//! faults to, badged with what the parent knows the child by. So the parent
//! reads one endpoint and gets both kinds of news about its children, told
//! apart by the label: a fault carries a kernel label, a report carries
//! this one.
//!
//! Invariant: the status is what the child made of its work, not what the
//! machine is to do with it. What follows a report is the parent's
//! decision.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use user_rt::message::{Reader, Writer};

use crate::label::{Label, ProtoError, Protocol};

/// `finished`: the child has done what it was started for.
pub const FINISHED: u16 = 1;

/// The status of a child that finished its work.
pub const SUCCESS: u64 = 0;

/// What a child says to its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// The child is done and is about to exit.
    Finished {
        /// [`SUCCESS`], or whatever else the child made of its work.
        status: u64,
    },
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        match self {
            Request::Finished { .. } => Label::new(Protocol::Parent, FINISHED),
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
            Request::Finished { status } => writer.word(buffer, *status)?,
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
        let label = Label::parse(reader.label())?;
        if label.protocol != Protocol::Parent {
            return Err(ProtoError::WrongProtocol {
                expected: Protocol::Parent,
                found: label.protocol,
            });
        }
        match label.message {
            FINISHED => Ok(Request::Finished {
                status: reader.word()?,
            }),
            other => Err(ProtoError::Message(Protocol::Parent, other)),
        }
    }
}
