// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Writing and reading the message area of the IPC buffer.
//!
//! The buffer accessors of `audhsos-abi` take a word index and hand back a
//! word; a protocol thinks in fields that follow one another and in byte
//! strings that do not fit a word. This module is the step between: a
//! writer that appends and a reader that walks, both counting for
//! themselves so that a protocol never names an index.
//!
//! A byte string is a word saying how many bytes there are and then the
//! bytes, eight to a word, little-endian, the last word padded with zeros.
//! The length is written rather than derived from the word count, because a
//! message carries more than one field and the padding of one field cannot
//! be told from the beginning of the next.
//!
//! Invariants: the writer never writes beyond the message area and reports
//! the first field that did not fit; the reader never reads beyond the word
//! count of the message it was built from, so a message whose fields say
//! anything at all is walked without a panic.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, MessageError, WORD};
use audhsos_abi::layout::{MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS};
use audhsos_abi::{Error, Handle, Message};

/// The widest byte string a message holds: everything the message area
/// has, less the word that says how long it is.
pub const MAX_BYTES: usize = (MAX_MESSAGE_WORDS - 1) * WORD;

/// Why a field could not be written or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodecError {
    /// The message area has no room for the field.
    Full,
    /// The handle area has no room for the handle.
    NoHandleSlot,
    /// The message ended before the field did.
    Truncated,
    /// A byte string longer than the destination.
    TooLong {
        /// How many bytes the string has.
        len: usize,
        /// How many the destination holds.
        capacity: usize,
    },
    /// A byte length that no message can carry.
    BadLength(u64),
    /// A word that is no handle.
    BadHandle(u64),
    /// The counts of the message are not readable.
    Header(MessageError),
}

impl From<MessageError> for CodecError {
    fn from(error: MessageError) -> Self {
        CodecError::Header(error)
    }
}

impl From<CodecError> for Error {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::Full | CodecError::NoHandleSlot | CodecError::TooLong { .. } => {
                Error::BufferTooSmall
            }
            CodecError::Truncated
            | CodecError::BadLength(_)
            | CodecError::BadHandle(_)
            | CodecError::Header(_) => Error::InvalidArgument,
        }
    }
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CodecError::Full => f.write_str("the message area is full"),
            CodecError::NoHandleSlot => f.write_str("the message carries no further handle"),
            CodecError::Truncated => f.write_str("the message ended inside a field"),
            CodecError::TooLong { len, capacity } => {
                write!(f, "{len} bytes do not fit into {capacity}")
            }
            CodecError::BadLength(len) => write!(f, "{len} is no byte length a message carries"),
            CodecError::BadHandle(word) => write!(f, "{word:#x} is no handle"),
            CodecError::Header(MessageError::TooManyWords) => {
                f.write_str("the message claims more words than the area holds")
            }
            CodecError::Header(MessageError::TooManyHandles) => {
                f.write_str("the message claims more handles than the area holds")
            }
        }
    }
}

/// How many words `len` bytes occupy.
const fn words_for(len: usize) -> usize {
    len.div_ceil(WORD)
}

/// Appends fields to the message area of a buffer.
///
/// The writer holds no buffer: it is handed one on every call, so the same
/// writer serves a caller that builds a message in its own buffer and one
/// that builds it in a frame it has mapped for somebody else.
#[derive(Clone, Copy, Debug, Default)]
pub struct Writer {
    words: usize,
    handles: usize,
}

impl Writer {
    /// A writer that has written nothing.
    #[must_use]
    pub const fn new() -> Self {
        Writer {
            words: 0,
            handles: 0,
        }
    }

    /// How many words have been appended.
    #[must_use]
    pub const fn words(&self) -> usize {
        self.words
    }

    /// How many handles have been appended.
    #[must_use]
    pub const fn handles(&self) -> usize {
        self.handles
    }

    /// Appends one word.
    ///
    /// # Errors
    ///
    /// [`CodecError::Full`] when the message area is full; nothing is
    /// written then.
    pub fn word(&mut self, buffer: &mut BufferMut<'_>, value: u64) -> Result<(), CodecError> {
        if !buffer.set_word(self.words, value) {
            return Err(CodecError::Full);
        }
        self.words = self.words.wrapping_add(1);
        Ok(())
    }

    /// Appends a byte string: one word with the length, then the bytes.
    ///
    /// # Errors
    ///
    /// [`CodecError::Full`] when the string does not fit; the length word
    /// is written only when the bytes fit behind it, so a message that was
    /// refused carries no half a field.
    pub fn bytes(&mut self, buffer: &mut BufferMut<'_>, bytes: &[u8]) -> Result<(), CodecError> {
        let needed = words_for(bytes.len())
            .checked_add(1)
            .ok_or(CodecError::Full)?;
        let end = self.words.checked_add(needed).ok_or(CodecError::Full)?;
        if end > MAX_MESSAGE_WORDS {
            return Err(CodecError::Full);
        }
        let len = u64::try_from(bytes.len()).map_err(|_| CodecError::Full)?;
        self.word(buffer, len)?;
        for chunk in bytes.chunks(WORD) {
            let mut word = [0u8; WORD];
            if let Some(slot) = word.get_mut(..chunk.len()) {
                slot.copy_from_slice(chunk);
            }
            self.word(buffer, u64::from_le_bytes(word))?;
        }
        Ok(())
    }

    /// Appends one handle to the handle area.
    ///
    /// # Errors
    ///
    /// [`CodecError::NoHandleSlot`] when the four slots are taken.
    pub fn handle(&mut self, buffer: &mut BufferMut<'_>, handle: Handle) -> Result<(), CodecError> {
        if self.handles >= MAX_MESSAGE_HANDLES {
            return Err(CodecError::NoHandleSlot);
        }
        if !buffer.set_handle_word(self.handles, handle.raw()) {
            return Err(CodecError::NoHandleSlot);
        }
        self.handles = self.handles.wrapping_add(1);
        Ok(())
    }

    /// Writes the label and the counts, which makes the message readable.
    ///
    /// # Errors
    ///
    /// [`CodecError::Header`] when a count is above what the area holds,
    /// which the appends have already refused.
    pub fn finish(&self, buffer: &mut BufferMut<'_>, label: u64) -> Result<(), CodecError> {
        buffer.set_label(label);
        buffer.set_counts(self.words, self.handles)?;
        Ok(())
    }
}

/// Walks the fields of a message.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    buffer: Buffer<'a>,
    message: Message,
    word: usize,
    handle: usize,
}

impl<'a> Reader<'a> {
    /// A reader over the message in `buffer`.
    ///
    /// # Errors
    ///
    /// [`CodecError::Header`] when the counts are above what the area
    /// holds, which is a message no sender of this system wrote.
    pub fn new(buffer: Buffer<'a>) -> Result<Self, CodecError> {
        let message = buffer.message()?;
        Ok(Reader {
            buffer,
            message,
            word: 0,
            handle: 0,
        })
    }

    /// The label of the message.
    #[must_use]
    pub const fn label(&self) -> u64 {
        self.message.label
    }

    /// The header of the message.
    #[must_use]
    pub const fn message(&self) -> Message {
        self.message
    }

    /// How many words are left.
    #[must_use]
    pub const fn remaining_words(&self) -> usize {
        self.message.word_count.saturating_sub(self.word)
    }

    /// How many handles are left.
    #[must_use]
    pub const fn remaining_handles(&self) -> usize {
        self.message.handle_count.saturating_sub(self.handle)
    }

    /// The next word.
    ///
    /// # Errors
    ///
    /// [`CodecError::Truncated`] at the end of the message.
    pub fn word(&mut self) -> Result<u64, CodecError> {
        if self.word >= self.message.word_count {
            return Err(CodecError::Truncated);
        }
        let value = self.buffer.word(self.word).ok_or(CodecError::Truncated)?;
        self.word = self.word.wrapping_add(1);
        Ok(value)
    }

    /// The next byte string, into `into`, and how many bytes it had.
    ///
    /// # Errors
    ///
    /// [`CodecError::Truncated`] when the message ends inside the string;
    /// [`CodecError::BadLength`] for a length no message can carry;
    /// [`CodecError::TooLong`] when `into` is smaller than the string. The
    /// reader stands before the string again in every one of those cases,
    /// so a caller that reports the error has not lost its place.
    pub fn bytes(&mut self, into: &mut [u8]) -> Result<usize, CodecError> {
        let start = self.word;
        let outcome = self.read_bytes(into);
        if outcome.is_err() {
            self.word = start;
        }
        outcome
    }

    /// The next handle.
    ///
    /// # Errors
    ///
    /// [`CodecError::Truncated`] when the message carries no further
    /// handle; [`CodecError::BadHandle`] for a word that is none.
    pub fn handle(&mut self) -> Result<Handle, CodecError> {
        if self.handle >= self.message.handle_count {
            return Err(CodecError::Truncated);
        }
        let word = self
            .buffer
            .handle_word(self.handle)
            .ok_or(CodecError::Truncated)?;
        let handle = Handle::from_raw(word).ok_or(CodecError::BadHandle(word))?;
        self.handle = self.handle.wrapping_add(1);
        Ok(handle)
    }

    /// The body of [`bytes`](Self::bytes), which may leave the cursor
    /// anywhere; its caller puts it back.
    fn read_bytes(&mut self, into: &mut [u8]) -> Result<usize, CodecError> {
        let word = self.word()?;
        let len = usize::try_from(word).map_err(|_| CodecError::BadLength(word))?;
        if len > MAX_BYTES {
            return Err(CodecError::BadLength(word));
        }
        if len > into.len() {
            return Err(CodecError::TooLong {
                len,
                capacity: into.len(),
            });
        }
        let slot = into.get_mut(..len).ok_or(CodecError::Truncated)?;
        for chunk in slot.chunks_mut(WORD) {
            let word = self.word()?.to_le_bytes();
            let taken = word.get(..chunk.len()).ok_or(CodecError::Truncated)?;
            chunk.copy_from_slice(taken);
        }
        Ok(len)
    }
}
