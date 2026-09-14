// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the orchestrator and its workers say to each other.
//!
//! One pipe per direction per worker, one byte of kind per message, and
//! every variable part behind a length. Nothing here allocates on the
//! strength of a length alone: a length past [`MAX_BYTES`] is refused, so
//! a pipe that carries garbage ends the reader rather than the machine.
//!
//! Invariant: the common message is small. A worker sends a whole input
//! and its features only when the input reached something, which is rare
//! once a corpus is loaded; everything else it sends is one kind byte and
//! a count.

use std::io::{self, Read, Write};

/// The longest byte string a message may carry. A whole coverage table is
/// the largest thing sent and is far below this.
pub const MAX_BYTES: usize = 1 << 28;

/// The place of an input the worker holds already, in place of its bytes.
pub const CACHED: u32 = u32::MAX;

/// The origin of an input that came from no corpus file.
pub const NO_FILE: u32 = u32::MAX;

/// An input the orchestrator drew for a worker: where it sits in the pool,
/// and its bytes unless the worker was sent them before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draw {
    /// The place of the input in the pool.
    pub index: u32,
    /// The bytes, for a worker that has not been sent them yet.
    pub bytes: Option<Vec<u8>>,
}

/// One round of work: the input to change, and the one to paste from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Round {
    /// The input the round starts from.
    pub seed: Draw,
    /// The input the mutator may paste out of.
    pub cross: Draw,
}

/// What the orchestrator tells a worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Down {
    /// Run these corpus files, named by their place in the run's file list.
    Load(Vec<u32>),
    /// Replace the coverage table with this one.
    Cover(Vec<u8>),
    /// Mutate and run these rounds, under this length limit.
    Batch {
        /// The longest input the mutator may produce.
        limit: u32,
        /// The inputs to work from.
        rounds: Vec<Round>,
    },
    /// Stop.
    Stop,
}

/// What a worker tells the orchestrator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Up {
    /// An input that reached something, with the features the worker had
    /// not seen reached as cheaply.
    Found {
        /// The corpus file it was read from, or [`NO_FILE`].
        origin: u32,
        /// The bytes.
        bytes: Vec<u8>,
        /// The features it claimed.
        features: Vec<u32>,
    },
    /// The work the worker was given is done, after this many runs.
    Idle(u64),
    /// This input made the target panic.
    Crash(Vec<u8>),
}

impl Down {
    /// Writes the message and flushes it.
    ///
    /// # Errors
    ///
    /// Whatever the pipe reports.
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        match self {
            Self::Load(files) => {
                out.write_all(&[1])?;
                write_u32s(out, files)?;
            }
            Self::Cover(bytes) => {
                out.write_all(&[2])?;
                write_bytes(out, bytes)?;
            }
            Self::Batch { limit, rounds } => {
                out.write_all(&[3])?;
                write_u32(out, *limit)?;
                write_u32(out, count(rounds.len()))?;
                for round in rounds {
                    round.seed.write(out)?;
                    round.cross.write(out)?;
                }
            }
            Self::Stop => out.write_all(&[4])?,
        }
        out.flush()
    }

    /// Reads one message.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::UnexpectedEof`] when the pipe ended, and
    /// [`io::ErrorKind::InvalidData`] for a kind or a length this does not
    /// know.
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        match kind(input)? {
            1 => Ok(Self::Load(read_u32s(input)?)),
            2 => Ok(Self::Cover(read_bytes(input)?)),
            3 => {
                let limit = read_u32(input)?;
                let mut rounds = Vec::new();
                for _ in 0..read_u32(input)? {
                    rounds.push(Round {
                        seed: Draw::read(input)?,
                        cross: Draw::read(input)?,
                    });
                }
                Ok(Self::Batch { limit, rounds })
            }
            4 => Ok(Self::Stop),
            other => Err(unknown(other)),
        }
    }
}

impl Draw {
    /// Writes the draw: its place, then either [`CACHED`] or its bytes.
    fn write(&self, out: &mut impl Write) -> io::Result<()> {
        write_u32(out, self.index)?;
        match &self.bytes {
            Some(bytes) => write_bytes(out, bytes),
            None => write_u32(out, CACHED),
        }
    }

    /// Reads a draw.
    fn read(input: &mut impl Read) -> io::Result<Self> {
        let index = read_u32(input)?;
        let length = read_u32(input)?;
        if length == CACHED {
            return Ok(Self { index, bytes: None });
        }
        Ok(Self {
            index,
            bytes: Some(read_body(input, length)?),
        })
    }
}

impl Up {
    /// Writes the message and flushes it.
    ///
    /// # Errors
    ///
    /// Whatever the pipe reports.
    pub fn write(&self, out: &mut impl Write) -> io::Result<()> {
        match self {
            Self::Found {
                origin,
                bytes,
                features,
            } => {
                out.write_all(&[1])?;
                write_u32(out, *origin)?;
                write_bytes(out, bytes)?;
                write_u32s(out, features)?;
            }
            Self::Idle(runs) => {
                out.write_all(&[2])?;
                out.write_all(&runs.to_le_bytes())?;
            }
            Self::Crash(bytes) => {
                out.write_all(&[3])?;
                write_bytes(out, bytes)?;
            }
        }
        out.flush()
    }

    /// Reads one message.
    ///
    /// # Errors
    ///
    /// As [`Down::read`].
    pub fn read(input: &mut impl Read) -> io::Result<Self> {
        match kind(input)? {
            1 => Ok(Self::Found {
                origin: read_u32(input)?,
                bytes: read_bytes(input)?,
                features: read_u32s(input)?,
            }),
            2 => {
                let mut word = [0u8; 8];
                input.read_exact(&mut word)?;
                Ok(Self::Idle(u64::from_le_bytes(word)))
            }
            3 => Ok(Self::Crash(read_bytes(input)?)),
            other => Err(unknown(other)),
        }
    }
}

/// The kind byte of the next message.
fn kind(input: &mut impl Read) -> io::Result<u8> {
    let mut byte = [0u8; 1];
    input.read_exact(&mut byte)?;
    Ok(u8::from_le_bytes(byte))
}

/// The error a kind byte that means nothing here gets.
fn unknown(kind: u8) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("message kind {kind}"))
}

/// A length as it goes on the wire, saturating rather than wrapping.
fn count(length: usize) -> u32 {
    u32::try_from(length).unwrap_or(u32::MAX)
}

/// Writes one number.
fn write_u32(out: &mut impl Write, value: u32) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}

/// Reads one number.
fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut word = [0u8; 4];
    input.read_exact(&mut word)?;
    Ok(u32::from_le_bytes(word))
}

/// Writes a length and the bytes behind it.
fn write_bytes(out: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    write_u32(out, count(bytes.len()))?;
    out.write_all(bytes)
}

/// Reads a length and the bytes behind it.
fn read_bytes(input: &mut impl Read) -> io::Result<Vec<u8>> {
    let length = read_u32(input)?;
    read_body(input, length)
}

/// Reads `length` bytes, refusing a length no message has.
fn read_body(input: &mut impl Read, length: u32) -> io::Result<Vec<u8>> {
    let length = usize::try_from(length).unwrap_or(usize::MAX);
    if length > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a message of {length} bytes"),
        ));
    }
    let mut bytes = vec![0u8; length];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Writes a length and the numbers behind it.
fn write_u32s(out: &mut impl Write, values: &[u32]) -> io::Result<()> {
    write_u32(out, count(values.len()))?;
    let mut buffer = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        buffer.extend_from_slice(&value.to_le_bytes());
    }
    out.write_all(&buffer)
}

/// Reads a length and the numbers behind it.
fn read_u32s(input: &mut impl Read) -> io::Result<Vec<u32>> {
    let length = read_u32(input)?;
    let bytes = read_body(input, length.saturating_mul(4))?;
    let mut values = Vec::with_capacity(bytes.len().wrapping_div(4));
    for word in bytes.as_chunks::<4>().0 {
        values.push(u32::from_le_bytes(*word));
    }
    Ok(values)
}
