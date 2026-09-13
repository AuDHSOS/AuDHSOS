// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The file protocol: opening, reading, writing and removing a file on the
//! volume the file system server mounted.
//!
//! Three rules shape it.
//!
//! A file handle is a number the server chose, not a kernel object,
//! because a file is not a kernel object. [`ROOT`] is the one number every
//! client holds without asking: the root directory of the volume.
//!
//! The server keeps one table of open files per client and finds that
//! table by the badge on the endpoint the message came through, exactly as
//! the display server finds a surface.
//!
//! Bulk data travels in the message area, at most [`MAX_DATA`] bytes of
//! it, and the limit is stated here so that a client loops over a large
//! file instead of discovering the limit by failing.
//!
//! Invariant: a reply carries bytes only when its status word says the
//! request succeeded, so a client that reads the status first never takes
//! data out of a message that has none.

use audhsos_abi::Error;
use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use user_rt::message::{Reader, Writer};

use crate::bytes::Bytes;
use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// How many bytes a name has: the eleven of an 8.3 entry, plus the dot a
/// client writes between the stem and the extension.
pub const MAX_NAME: usize = 12;

/// A name of one entry, as a client writes it.
pub type Name = Bytes<MAX_NAME>;

/// How many bytes of a file one message carries.
///
/// One sector, which is the unit the server moves anyway, and small
/// enough to stand on the stack of everyone who handles it, as the
/// console protocol bounds its chunk for the same reason. A client with
/// more to move sends more messages.
pub const MAX_DATA: usize = 512;

/// The file handle of the root directory, which every client holds.
pub const ROOT: u32 = 0;

/// `open`: the handle of an entry that is there.
pub const OPEN: u16 = 1;

/// `create`: the handle of an entry made now.
pub const CREATE: u16 = 2;

/// `read`: bytes out of a file.
pub const READ: u16 = 3;

/// `write`: bytes into a file.
pub const WRITE: u16 = 4;

/// `read_dir`: one entry of a directory and where to carry on.
pub const READ_DIR: u16 = 5;

/// `stat`: what a file handle stands for.
pub const STAT: u16 = 6;

/// `remove`: take an entry off the volume.
pub const REMOVE: u16 = 7;

/// `close`: give a file handle back.
pub const CLOSE: u16 = 8;

/// `flush`: put what was written onto the disk.
pub const FLUSH: u16 = 9;

/// What a client asks the file system server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the sector is what the protocol carries, and boxing it would need a heap this system does not have"
)]
pub enum Request {
    /// The handle of `name` in `parent`.
    Open {
        /// The directory to look in.
        parent: u32,
        /// The entry to open.
        name: Name,
    },
    /// Makes `name` in `parent` and answers its handle.
    Create {
        /// The directory to make it in.
        parent: u32,
        /// The entry to make.
        name: Name,
        /// Whether it is a directory.
        directory: bool,
    },
    /// At most `len` bytes of `file`, from `offset`.
    Read {
        /// The open file.
        file: u32,
        /// Where to read from.
        offset: u32,
        /// How many bytes at most, capped at [`MAX_DATA`].
        len: u32,
    },
    /// The bytes of the message into `file`, at `offset`.
    Write {
        /// The open file.
        file: u32,
        /// Where to write to.
        offset: u32,
        /// The bytes, at most [`MAX_DATA`] of them.
        data: Data,
    },
    /// The entry of `dir` at `cursor`, and where the next one is.
    ReadDir {
        /// The open directory.
        dir: u32,
        /// Where to carry on, [`START`] for the first entry.
        cursor: u64,
    },
    /// What `file` stands for.
    Stat {
        /// The open file.
        file: u32,
    },
    /// Takes `name` in `parent` off the volume.
    Remove {
        /// The directory to take it out of.
        parent: u32,
        /// The entry to take off.
        name: Name,
    },
    /// Gives `file` back. [`ROOT`] is not closed.
    Close {
        /// The open file.
        file: u32,
    },
    /// Puts everything that was written onto the disk.
    Flush,
}

/// The bytes of one write, which the message area carries.
pub type Data = Bytes<MAX_DATA>;

/// The cursor of the first entry of a directory.
pub const START: u64 = 0;

/// What the server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "as `Request`: the sector is the message"
)]
pub enum Reply {
    /// The answer to an open: the handle and what it stands for.
    Opened(Result<Opened, Error>),
    /// The answer to a create: the handle of what was made.
    Created(Result<u32, Error>),
    /// The answer to a read: the bytes, however few.
    Read(Result<Data, Error>),
    /// The answer to a write: how many bytes the server took.
    Written(Result<u32, Error>),
    /// The answer to a directory read: one entry, or nothing for the end.
    Entry(Result<Option<DirEntry>, Error>),
    /// The answer to a stat.
    Stat(Result<Stat, Error>),
    /// The answer to a remove.
    Removed(Result<(), Error>),
    /// The answer to a close.
    Closed(Result<(), Error>),
    /// The answer to a flush.
    Flushed(Result<(), Error>),
}

/// What an open answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The handle the client uses from now on.
    pub file: u32,
    /// How many bytes the file holds.
    pub size: u32,
    /// Whether it is a directory.
    pub directory: bool,
}

/// One entry of a directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The name in its 8.3 form, with the dot written in.
    pub name: Name,
    /// How many bytes it holds; zero for a directory.
    pub size: u32,
    /// Whether it is a directory.
    pub directory: bool,
    /// Where to carry on to read the next one.
    pub cursor: u64,
}

/// What a stat answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    /// How many bytes the file holds.
    pub size: u32,
    /// The attribute byte of the entry (`fs_fat::ATTR_DIRECTORY` and the
    /// rest).
    pub attributes: u32,
    /// When the entry was last written, in seconds since the epoch.
    pub modified: u64,
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Request::Open { .. } => OPEN,
            Request::Create { .. } => CREATE,
            Request::Read { .. } => READ,
            Request::Write { .. } => WRITE,
            Request::ReadDir { .. } => READ_DIR,
            Request::Stat { .. } => STAT,
            Request::Remove { .. } => REMOVE,
            Request::Close { .. } => CLOSE,
            Request::Flush => FLUSH,
        };
        Label::new(Protocol::File, message)
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room, which only
    /// a write of more than [`MAX_DATA`] bytes runs into.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Open { parent, name } | Request::Remove { parent, name } => {
                writer.word(buffer, u64::from(*parent))?;
                writer.bytes(buffer, name.as_bytes())?;
            }
            Request::Create {
                parent,
                name,
                directory,
            } => {
                writer.word(buffer, u64::from(*parent))?;
                writer.word(buffer, u64::from(*directory))?;
                writer.bytes(buffer, name.as_bytes())?;
            }
            Request::Read { file, offset, len } => {
                writer.word(buffer, u64::from(*file))?;
                writer.word(buffer, u64::from(*offset))?;
                writer.word(buffer, u64::from(*len))?;
            }
            Request::Write { file, offset, data } => {
                writer.word(buffer, u64::from(*file))?;
                writer.word(buffer, u64::from(*offset))?;
                writer.bytes(buffer, data.as_bytes())?;
            }
            Request::ReadDir { dir, cursor } => {
                writer.word(buffer, u64::from(*dir))?;
                writer.word(buffer, *cursor)?;
            }
            Request::Stat { file } | Request::Close { file } => {
                writer.word(buffer, u64::from(*file))?;
            }
            Request::Flush => {}
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
    /// than [`MAX_NAME`] or data longer than [`MAX_DATA`], and for a
    /// message whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            OPEN => Ok(Request::Open {
                parent: word32(reader.word()?),
                name: name_of(&mut reader)?,
            }),
            REMOVE => Ok(Request::Remove {
                parent: word32(reader.word()?),
                name: name_of(&mut reader)?,
            }),
            CREATE => Ok(Request::Create {
                parent: word32(reader.word()?),
                directory: reader.word()? != 0,
                name: name_of(&mut reader)?,
            }),
            READ => Ok(Request::Read {
                file: word32(reader.word()?),
                offset: word32(reader.word()?),
                len: word32(reader.word()?),
            }),
            WRITE => Ok(Request::Write {
                file: word32(reader.word()?),
                offset: word32(reader.word()?),
                data: data_of(&mut reader)?,
            }),
            READ_DIR => Ok(Request::ReadDir {
                dir: word32(reader.word()?),
                cursor: reader.word()?,
            }),
            STAT => Ok(Request::Stat {
                file: word32(reader.word()?),
            }),
            CLOSE => Ok(Request::Close {
                file: word32(reader.word()?),
            }),
            FLUSH => Ok(Request::Flush),
            other => Err(ProtoError::Message(Protocol::File, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under, which is the label of the
    /// request it answers.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Reply::Opened(_) => OPEN,
            Reply::Created(_) => CREATE,
            Reply::Read(_) => READ,
            Reply::Written(_) => WRITE,
            Reply::Entry(_) => READ_DIR,
            Reply::Stat(_) => STAT,
            Reply::Removed(_) => REMOVE,
            Reply::Closed(_) => CLOSE,
            Reply::Flushed(_) => FLUSH,
        };
        Label::new(Protocol::File, message)
    }

    /// Writes the reply into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Reply::Opened(Ok(opened)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(opened.file))?;
                writer.word(buffer, u64::from(opened.size))?;
                writer.word(buffer, u64::from(opened.directory))?;
            }
            Reply::Created(Ok(file)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(*file))?;
            }
            Reply::Read(Ok(data)) => {
                writer.word(buffer, 0)?;
                writer.bytes(buffer, data.as_bytes())?;
            }
            Reply::Written(Ok(taken)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(*taken))?;
            }
            Reply::Entry(Ok(entry)) => {
                writer.word(buffer, 0)?;
                match entry {
                    Some(entry) => {
                        writer.word(buffer, 1)?;
                        writer.word(buffer, u64::from(entry.size))?;
                        writer.word(buffer, u64::from(entry.directory))?;
                        writer.word(buffer, entry.cursor)?;
                        writer.bytes(buffer, entry.name.as_bytes())?;
                    }
                    None => writer.word(buffer, 0)?,
                }
            }
            Reply::Stat(Ok(stat)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(stat.size))?;
                writer.word(buffer, u64::from(stat.attributes))?;
                writer.word(buffer, stat.modified)?;
            }
            Reply::Removed(outcome) | Reply::Closed(outcome) | Reply::Flushed(outcome) => {
                writer.word(buffer, status_word(*outcome))?;
            }
            Reply::Opened(Err(error))
            | Reply::Created(Err(error))
            | Reply::Read(Err(error))
            | Reply::Written(Err(error))
            | Reply::Entry(Err(error))
            | Reply::Stat(Err(error)) => {
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
            (REMOVE, outcome) => Ok(Reply::Removed(outcome)),
            (CLOSE, outcome) => Ok(Reply::Closed(outcome)),
            (FLUSH, outcome) => Ok(Reply::Flushed(outcome)),
            (OPEN, Err(error)) => Ok(Reply::Opened(Err(error))),
            (CREATE, Err(error)) => Ok(Reply::Created(Err(error))),
            (READ, Err(error)) => Ok(Reply::Read(Err(error))),
            (WRITE, Err(error)) => Ok(Reply::Written(Err(error))),
            (READ_DIR, Err(error)) => Ok(Reply::Entry(Err(error))),
            (STAT, Err(error)) => Ok(Reply::Stat(Err(error))),
            (OPEN, Ok(())) => Ok(Reply::Opened(Ok(Opened {
                file: word32(reader.word()?),
                size: word32(reader.word()?),
                directory: reader.word()? != 0,
            }))),
            (CREATE, Ok(())) => Ok(Reply::Created(Ok(word32(reader.word()?)))),
            (READ, Ok(())) => Ok(Reply::Read(Ok(data_of(&mut reader)?))),
            (WRITE, Ok(())) => Ok(Reply::Written(Ok(word32(reader.word()?)))),
            (READ_DIR, Ok(())) => Ok(Reply::Entry(Ok(entry_of(&mut reader)?))),
            (STAT, Ok(())) => Ok(Reply::Stat(Ok(Stat {
                size: word32(reader.word()?),
                attributes: word32(reader.word()?),
                modified: reader.word()?,
            }))),
            (other, _) => Err(ProtoError::Message(Protocol::File, other)),
        }
    }
}

/// The name that stands next in the message.
fn name_of(reader: &mut Reader<'_>) -> Result<Name, ProtoError> {
    let mut into = [0u8; MAX_NAME];
    let len = reader.bytes(&mut into)?;
    Name::new(into.get(..len).unwrap_or(&[]))
}

/// The bytes that stand next in the message.
fn data_of(reader: &mut Reader<'_>) -> Result<Data, ProtoError> {
    let mut into = [0u8; MAX_DATA];
    let len = reader.bytes(&mut into)?;
    Data::new(into.get(..len).unwrap_or(&[]))
}

/// The directory entry that stands next in the message, or nothing when
/// the directory ended.
fn entry_of(reader: &mut Reader<'_>) -> Result<Option<DirEntry>, ProtoError> {
    if reader.word()? == 0 {
        return Ok(None);
    }
    let size = word32(reader.word()?);
    let directory = reader.word()? != 0;
    let cursor = reader.word()?;
    Ok(Some(DirEntry {
        name: name_of(reader)?,
        size,
        directory,
        cursor,
    }))
}

/// The low half of a word, which is what every number of this protocol but
/// a cursor and a moment is.
fn word32(word: u64) -> u32 {
    u32::try_from(word & 0xFFFF_FFFF).unwrap_or(0)
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::File {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::File,
            found: label.protocol,
        });
    }
    Ok(label)
}
