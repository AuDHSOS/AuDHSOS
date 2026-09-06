// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPC buffer: the page a thread and the kernel exchange everything
//! through. Registers carry nothing.
//!
//! Invariants: every field lies at a fixed offset that is part of the ABI;
//! every accessor stays inside the page, so a buffer whose fields hold
//! arbitrary bytes is readable without a panic and without reaching past
//! the page; a message is only handed out after its counts have been
//! checked against [`MAX_MESSAGE_WORDS`] and [`MAX_MESSAGE_HANDLES`].
//!
//! ```text
//! 0     syscall number        (1 word)
//! 8     arguments             (6 words)
//! 56    status                (1 word)
//! 64    return values         (2 words)
//! 80    reserved              (6 words)
//! 128   message label         (1 word)
//! 136   message word count    (1 word)
//! 144   message handle count  (1 word)
//! 152   message handles       (4 words)
//! 184   message words         (480 words, ending at 4024)
//! 4024  reserved              (9 words)
//! ```

use crate::layout::{
    IPC_BUFFER_SIZE, MAX_MESSAGE_HANDLES, MAX_MESSAGE_WORDS, MAX_SYSCALL_ARGUMENTS,
    MAX_SYSCALL_RETURN_WORDS,
};
use crate::{Error, FaultKind, Handle};

/// Size of the buffer in bytes, as a `usize` for indexing.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "the page size is 4096 and fits every pointer width this system builds for"
)]
pub const SIZE: usize = IPC_BUFFER_SIZE as usize;

/// Size of one word in bytes.
pub const WORD: usize = 8;

/// Offset of the system call number.
pub const SYSCALL_NUMBER: usize = 0;

/// Offset of the first argument word.
pub const ARGS: usize = 8;

/// Offset of the status word the kernel writes.
pub const STATUS: usize = 56;

/// Offset of the first return word the kernel writes.
pub const RETURN: usize = 64;

/// Offset of the message area.
pub const MESSAGE: usize = 128;

/// Offset of the message label.
pub const LABEL: usize = MESSAGE;

/// Offset of the number of payload words.
pub const WORD_COUNT: usize = 136;

/// Offset of the number of handles.
pub const HANDLE_COUNT: usize = 144;

/// Offset of the first handle word.
pub const HANDLES: usize = 152;

/// Offset of the first payload word.
pub const WORDS: usize = 184;

const _: () = assert!(ARGS + MAX_SYSCALL_ARGUMENTS * WORD <= STATUS);
const _: () = assert!(RETURN + MAX_SYSCALL_RETURN_WORDS * WORD <= MESSAGE);
const _: () = assert!(HANDLES + MAX_MESSAGE_HANDLES * WORD <= WORDS);
const _: () = assert!(WORDS + MAX_MESSAGE_WORDS * WORD <= SIZE);
const _: () = assert!(WORDS + MAX_MESSAGE_WORDS * WORD == 4024);

/// The first label the kernel keeps for messages of its own.
///
/// A `send` or a `call` whose label is at or above this is refused with
/// [`Error::InvalidArgument`] before anything is copied, which is what
/// makes the range reserved rather than merely documented. The check is at
/// the entry of the two calls and not in the transfer itself, because the
/// fault message the kernel builds carries such a label by construction.
pub const KERNEL_LABEL_BASE: u64 = 0xFFFF_FFFF_FFFF_FF00;

/// The base of the fault labels. The label of a fault message is this plus
/// the code of its [`FaultKind`], so the six kinds occupy the first six
/// labels of the reserved range and 250 are left for the kernel messages
/// of later phases.
pub const FAULT_LABEL_BASE: u64 = KERNEL_LABEL_BASE;

/// The label of the fault message for `kind`.
#[must_use]
pub const fn fault_label(kind: FaultKind) -> u64 {
    #[expect(
        clippy::as_conversions,
        reason = "widening a fault code to the width of a label in a const fn"
    )]
    let code = kind.code() as u64;
    FAULT_LABEL_BASE.wrapping_add(code)
}

/// The fault kind `label` names, or `None` for a label that is no fault
/// label.
#[must_use]
pub const fn fault_kind_of(label: u64) -> Option<FaultKind> {
    let Some(code) = label.checked_sub(FAULT_LABEL_BASE) else {
        return None;
    };
    if code > 0xFFFF_FFFF {
        return None;
    }
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "the bound above keeps the value inside u32, and the function is const"
    )]
    let code = code as u32;
    FaultKind::from_code(code)
}

/// `true` for a label the kernel keeps for its own messages.
#[must_use]
pub const fn is_kernel_label(label: u64) -> bool {
    label >= KERNEL_LABEL_BASE
}

/// The byte offset of payload word `index`, or `None` beyond the area.
#[must_use]
pub const fn word_offset(index: usize) -> Option<usize> {
    if index >= MAX_MESSAGE_WORDS {
        return None;
    }
    match index.checked_mul(WORD) {
        Some(offset) => offset.checked_add(WORDS),
        None => None,
    }
}

/// The byte offset of handle `index`, or `None` beyond the area.
#[must_use]
pub const fn handle_offset(index: usize) -> Option<usize> {
    if index >= MAX_MESSAGE_HANDLES {
        return None;
    }
    match index.checked_mul(WORD) {
        Some(offset) => offset.checked_add(HANDLES),
        None => None,
    }
}

/// The byte offset of argument `index`, or `None` beyond the area.
#[must_use]
pub const fn argument_offset(index: usize) -> Option<usize> {
    if index >= MAX_SYSCALL_ARGUMENTS {
        return None;
    }
    match index.checked_mul(WORD) {
        Some(offset) => offset.checked_add(ARGS),
        None => None,
    }
}

/// The byte offset of return word `index`, or `None` beyond the area.
#[must_use]
pub const fn return_offset(index: usize) -> Option<usize> {
    if index >= MAX_SYSCALL_RETURN_WORDS {
        return None;
    }
    match index.checked_mul(WORD) {
        Some(offset) => offset.checked_add(RETURN),
        None => None,
    }
}

/// What a system call left in the status word.
///
/// The low half carries the error code, where `0` means success, and bit 32
/// says that the call did part of its work and left the rest: an operation
/// over a range processes at most [`crate::layout::MAX_PAGES_PER_CALL`]
/// pages per call, reports how far it came in the first return word, and
/// userland calls it again. Partial progress is a success, so it cannot be
/// an error code; the two live in one word because a caller reads one word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Status(u64);

/// The bit that marks partial progress.
const PARTIAL_BIT: u64 = 1 << 32;

/// The bits of the status word that carry the error code.
const ERROR_MASK: u64 = 0xFFFF_FFFF;

impl Status {
    /// The call succeeded and did all of its work.
    pub const OK: Status = Status(0);

    /// The call succeeded and did part of its work; the first return word
    /// says how much.
    pub const PARTIAL: Status = Status(PARTIAL_BIT);

    /// The call failed with `error`.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening an error code to the width of the status word in a const fn"
    )]
    pub const fn failed(error: Error) -> Self {
        Status(error.code() as u64)
    }

    /// The raw word as it appears in the buffer.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Decodes a raw status word.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a word with a bit outside the error
    /// code and the partial flag, for an error code that names no error,
    /// and for a failure that also claims partial progress.
    pub const fn from_raw(raw: u64) -> Result<Self, Error> {
        if raw & !(ERROR_MASK | PARTIAL_BIT) != 0 {
            return Err(Error::InvalidArgument);
        }
        let code = raw & ERROR_MASK;
        if code == 0 {
            return Ok(Status(raw));
        }
        if raw & PARTIAL_BIT != 0 {
            return Err(Error::InvalidArgument);
        }
        #[expect(
            clippy::as_conversions,
            reason = "the mask keeps the value inside u32, and the function is const"
        )]
        let code = code as u32;
        match Error::from_code(code) {
            Some(_) => Ok(Status(raw)),
            None => Err(Error::InvalidArgument),
        }
    }

    /// The error the call failed with, or `None` on success.
    #[must_use]
    pub const fn error(self) -> Option<Error> {
        let code = self.0 & ERROR_MASK;
        if code == 0 {
            return None;
        }
        #[expect(
            clippy::as_conversions,
            reason = "the mask keeps the value inside u32, and the function is const"
        )]
        let code = code as u32;
        Error::from_code(code)
    }

    /// `true` if the call did part of its work and left the rest.
    #[must_use]
    pub const fn is_partial(self) -> bool {
        self.0 & PARTIAL_BIT != 0
    }

    /// `true` if the call did not fail.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 & ERROR_MASK == 0
    }
}

impl From<Error> for Status {
    fn from(error: Error) -> Self {
        Status::failed(error)
    }
}

/// Why a message could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageError {
    /// The word count is above [`MAX_MESSAGE_WORDS`].
    TooManyWords,
    /// The handle count is above [`MAX_MESSAGE_HANDLES`].
    TooManyHandles,
}

impl From<MessageError> for Error {
    fn from(error: MessageError) -> Self {
        match error {
            MessageError::TooManyWords | MessageError::TooManyHandles => Error::InvalidArgument,
        }
    }
}

/// The header of a message, after its counts have been checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Message {
    /// The label the sender chose.
    pub label: u64,
    /// How many payload words the message carries.
    pub word_count: usize,
    /// How many handles the message carries.
    pub handle_count: usize,
}

impl Message {
    /// `true` when the label lies in the range the kernel keeps for its own
    /// messages, which a user thread may not send.
    #[must_use]
    pub const fn is_kernel_label(&self) -> bool {
        is_kernel_label(self.label)
    }

    /// The fault kind the label names, for a message the kernel built.
    #[must_use]
    pub const fn fault_kind(&self) -> Option<FaultKind> {
        fault_kind_of(self.label)
    }
}

/// Reads the eight bytes at `offset` as a little-endian word.
fn read_word(bytes: &[u8; SIZE], offset: usize) -> Option<u64> {
    let end = offset.checked_add(WORD)?;
    let slice = bytes.get(offset..end)?;
    let word: [u8; WORD] = slice.try_into().ok()?;
    Some(u64::from_le_bytes(word))
}

/// Writes `value` at `offset` as a little-endian word. Returns `false` and
/// changes nothing when the word would leave the page.
fn write_word(bytes: &mut [u8; SIZE], offset: usize, value: u64) -> bool {
    let Some(end) = offset.checked_add(WORD) else {
        return false;
    };
    let Some(slice) = bytes.get_mut(offset..end) else {
        return false;
    };
    slice.copy_from_slice(&value.to_le_bytes());
    true
}

/// A read-only view of an IPC buffer.
#[derive(Clone, Copy, Debug)]
pub struct Buffer<'a> {
    bytes: &'a [u8; SIZE],
}

impl<'a> Buffer<'a> {
    /// A view of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8; SIZE]) -> Self {
        Buffer { bytes }
    }

    /// The bytes behind the view.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8; SIZE] {
        self.bytes
    }

    /// The system call number word.
    #[must_use]
    pub fn syscall_number(&self) -> u64 {
        read_word(self.bytes, SYSCALL_NUMBER).unwrap_or(0)
    }

    /// Argument `index`, or `None` beyond [`MAX_SYSCALL_ARGUMENTS`].
    #[must_use]
    pub fn argument(&self, index: usize) -> Option<u64> {
        read_word(self.bytes, argument_offset(index)?)
    }

    /// Every argument word, in order.
    #[must_use]
    pub fn arguments(&self) -> [u64; MAX_SYSCALL_ARGUMENTS] {
        let mut args = [0; MAX_SYSCALL_ARGUMENTS];
        for (index, slot) in args.iter_mut().enumerate() {
            *slot = self.argument(index).unwrap_or(0);
        }
        args
    }

    /// The status word.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a word that is not a status; see
    /// [`Status::from_raw`].
    pub fn status(&self) -> Result<Status, Error> {
        Status::from_raw(read_word(self.bytes, STATUS).unwrap_or(0))
    }

    /// Return word `index`, or `None` beyond [`MAX_SYSCALL_RETURN_WORDS`].
    #[must_use]
    pub fn return_word(&self, index: usize) -> Option<u64> {
        read_word(self.bytes, return_offset(index)?)
    }

    /// The message header.
    ///
    /// # Errors
    ///
    /// [`MessageError`] when a count is above what the area holds.
    pub fn message(&self) -> Result<Message, MessageError> {
        let label = read_word(self.bytes, LABEL).unwrap_or(0);
        let words = read_word(self.bytes, WORD_COUNT).unwrap_or(0);
        let handles = read_word(self.bytes, HANDLE_COUNT).unwrap_or(0);
        let word_count = usize::try_from(words).map_err(|_| MessageError::TooManyWords)?;
        if word_count > MAX_MESSAGE_WORDS {
            return Err(MessageError::TooManyWords);
        }
        let handle_count = usize::try_from(handles).map_err(|_| MessageError::TooManyHandles)?;
        if handle_count > MAX_MESSAGE_HANDLES {
            return Err(MessageError::TooManyHandles);
        }
        Ok(Message {
            label,
            word_count,
            handle_count,
        })
    }

    /// Payload word `index`, or `None` beyond [`MAX_MESSAGE_WORDS`]. The
    /// index is not checked against the word count of the message, which is
    /// the caller's business.
    #[must_use]
    pub fn word(&self, index: usize) -> Option<u64> {
        read_word(self.bytes, word_offset(index)?)
    }

    /// The raw handle word `index`, or `None` beyond
    /// [`MAX_MESSAGE_HANDLES`].
    #[must_use]
    pub fn handle_word(&self, index: usize) -> Option<u64> {
        read_word(self.bytes, handle_offset(index)?)
    }

    /// Handle `index` of the message, or `None` beyond the area and for a
    /// word that is no handle.
    #[must_use]
    pub fn handle(&self, index: usize) -> Option<Handle> {
        Handle::from_raw(self.handle_word(index)?)
    }
}

/// A read-write view of an IPC buffer.
#[derive(Debug)]
pub struct BufferMut<'a> {
    bytes: &'a mut [u8; SIZE],
}

impl<'a> BufferMut<'a> {
    /// A view of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8; SIZE]) -> Self {
        BufferMut { bytes }
    }

    /// A read-only view of the same buffer.
    #[must_use]
    pub const fn reader(&self) -> Buffer<'_> {
        Buffer::new(self.bytes)
    }

    /// The bytes behind the view.
    #[must_use]
    pub const fn bytes_mut(&mut self) -> &mut [u8; SIZE] {
        self.bytes
    }

    /// Writes the system call number word.
    pub fn set_syscall_number(&mut self, number: u64) {
        write_word(self.bytes, SYSCALL_NUMBER, number);
    }

    /// Writes argument `index`. An index beyond
    /// [`MAX_SYSCALL_ARGUMENTS`] changes nothing and returns `false`.
    pub fn set_argument(&mut self, index: usize, value: u64) -> bool {
        argument_offset(index).is_some_and(|offset| write_word(self.bytes, offset, value))
    }

    /// Writes the status word.
    pub fn set_status(&mut self, status: Status) {
        write_word(self.bytes, STATUS, status.raw());
    }

    /// Writes return word `index`. An index beyond
    /// [`MAX_SYSCALL_RETURN_WORDS`] changes nothing and returns `false`.
    pub fn set_return_word(&mut self, index: usize, value: u64) -> bool {
        return_offset(index).is_some_and(|offset| write_word(self.bytes, offset, value))
    }

    /// Writes the message label.
    pub fn set_label(&mut self, label: u64) {
        write_word(self.bytes, LABEL, label);
    }

    /// Writes the counts of the message header.
    ///
    /// # Errors
    ///
    /// [`MessageError`] when a count is above what the area holds; nothing
    /// is written then.
    pub fn set_counts(
        &mut self,
        word_count: usize,
        handle_count: usize,
    ) -> Result<(), MessageError> {
        if word_count > MAX_MESSAGE_WORDS {
            return Err(MessageError::TooManyWords);
        }
        if handle_count > MAX_MESSAGE_HANDLES {
            return Err(MessageError::TooManyHandles);
        }
        let words = u64::try_from(word_count).map_err(|_| MessageError::TooManyWords)?;
        let handles = u64::try_from(handle_count).map_err(|_| MessageError::TooManyHandles)?;
        write_word(self.bytes, WORD_COUNT, words);
        write_word(self.bytes, HANDLE_COUNT, handles);
        Ok(())
    }

    /// Writes payload word `index`. An index beyond
    /// [`MAX_MESSAGE_WORDS`] changes nothing and returns `false`.
    pub fn set_word(&mut self, index: usize, value: u64) -> bool {
        word_offset(index).is_some_and(|offset| write_word(self.bytes, offset, value))
    }

    /// Writes the raw handle word `index`. An index beyond
    /// [`MAX_MESSAGE_HANDLES`] changes nothing and returns `false`.
    pub fn set_handle_word(&mut self, index: usize, value: u64) -> bool {
        handle_offset(index).is_some_and(|offset| write_word(self.bytes, offset, value))
    }

    /// Writes handle `index`. An index beyond [`MAX_MESSAGE_HANDLES`]
    /// changes nothing and returns `false`.
    pub fn set_handle(&mut self, index: usize, handle: Handle) -> bool {
        self.set_handle_word(index, handle.raw())
    }

    /// Clears the result area: the status word and every return word.
    pub fn clear_result(&mut self) {
        write_word(self.bytes, STATUS, 0);
        for index in 0..MAX_SYSCALL_RETURN_WORDS {
            self.set_return_word(index, 0);
        }
    }
}
