// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The startup message: what a process finds in the IPC buffer of its
//! first thread, saying which handle of its table holds which role.
//!
//! A handle is a number, and a process that has just been created has no
//! way of knowing what the numbers in its table name. Whoever created it —
//! the kernel for the root task, the root task for every other process —
//! installs the handles and then writes this message, which pairs each
//! handle with the role it was installed for.
//!
//! The message is a list of pairs and not a list of fixed positions,
//! because the processes of this system are given different things: the
//! root task receives one memory object per free region of memory, of
//! which there are as many as the machine has, and an application receives
//! two endpoints and nothing else. A reader takes the roles it knows and
//! passes over the rest, so a role added later reaches an older program as
//! nothing at all rather than as a shifted field.
//!
//! ```text
//! label       STARTUP_LABEL
//! word 0      role of the first handle
//! word 1      the first handle
//! word 2      role of the second handle
//! word 3      the second handle
//! ...
//! ```
//!
//! Invariants: the word count is even; every role word names a role; the
//! message carries no handles in the handle area, because the handles are
//! already in the table of the process and the message only says what they
//! are.

use crate::ipc_buffer::{Buffer, BufferMut, MessageError};
use crate::{Error, Handle};

/// The label of the startup message.
///
/// The bytes spell `STARTUP`, which makes it recognizable in a dump of the
/// buffer, and it lies far below [`crate::ipc_buffer::KERNEL_LABEL_BASE`],
/// so it is a label a user thread may also send.
pub const STARTUP_LABEL: u64 = u64::from_be_bytes(*b"STARTUP\0");

const _: () = assert!(STARTUP_LABEL < crate::ipc_buffer::KERNEL_LABEL_BASE);

/// Declares the role table once and derives the enum, the code lookup, and
/// the name from it.
macro_rules! roles {
    ($($variant:ident = $code:literal => $doc:literal),+ $(,)?) => {
        /// What a handle of the startup message was installed for.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u32)]
        pub enum Role {
            $(
                #[doc = $doc]
                $variant = $code,
            )+
        }

        impl Role {
            /// Every role, in table order.
            pub const ALL: &[Role] = &[$(Role::$variant),+];

            /// The stable numeric code of this role.
            #[must_use]
            #[expect(
                clippy::as_conversions,
                reason = "discriminant of a repr(u32) enum in a const fn"
            )]
            pub const fn code(self) -> u32 {
                self as u32
            }

            /// Decodes a numeric code.
            #[must_use]
            pub const fn from_code(code: u32) -> Option<Self> {
                match code {
                    $($code => Some(Role::$variant),)+
                    _ => None,
                }
            }

            /// The name of the role.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Role::$variant => stringify!($variant),)+
                }
            }
        }
    };
}

roles! {
    OwnProcess = 1 => "A handle to the process itself, which it needs to map memory into its own address space.",
    OwnEndpoint = 2 => "The endpoint the process receives requests on. A server gets one; a client that serves nobody does not.",
    SystemControl = 3 => "The capability to create interrupts, port ranges, and device memory. Only the root task receives it.",
    BootImage = 4 => "A memory object over the boot image, which carries the archive. Only the root task receives it.",
    Ram = 5 => "A memory object over a free region of memory. The root task receives one per region, so this role appears more than once.",
    NameServer = 6 => "The endpoint of the name server.",
    MemoryServer = 7 => "The endpoint of the memory server.",
    Log = 8 => "The endpoint diagnostics are sent to. It is the console once there is one.",
    IoPorts = 9 => "A range of I/O ports the process is allowed to reach. A driver receives one.",
    Interrupt = 10 => "An interrupt object for the line the process serves. A driver receives one.",
}

/// Why a startup message could not be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StartupError {
    /// The label is not [`STARTUP_LABEL`].
    NotStartup,
    /// The word count is odd, so the last pair is incomplete.
    OddWordCount(usize),
    /// A role word names no role.
    UnknownRole(u64),
    /// A handle word is no handle.
    BadHandle(u64),
    /// The message header could not be read at all.
    Message(MessageError),
    /// The message area has no room for another pair.
    Full,
}

impl From<MessageError> for StartupError {
    fn from(error: MessageError) -> Self {
        StartupError::Message(error)
    }
}

impl From<StartupError> for Error {
    fn from(error: StartupError) -> Self {
        match error {
            StartupError::NotStartup
            | StartupError::OddWordCount(_)
            | StartupError::UnknownRole(_)
            | StartupError::BadHandle(_)
            | StartupError::Message(_) => Error::InvalidArgument,
            StartupError::Full => Error::BufferTooSmall,
        }
    }
}

impl core::fmt::Display for StartupError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StartupError::NotStartup => f.write_str("the message is no startup message"),
            StartupError::OddWordCount(count) => {
                write!(f, "the startup message has {count} words, which is odd")
            }
            StartupError::UnknownRole(word) => write!(f, "{word} names no role"),
            StartupError::BadHandle(word) => write!(f, "{word:#x} is no handle"),
            StartupError::Message(MessageError::TooManyWords) => {
                f.write_str("the startup message claims more words than the area holds")
            }
            StartupError::Message(MessageError::TooManyHandles) => {
                f.write_str("the startup message claims more handles than the area holds")
            }
            StartupError::Full => f.write_str("the startup message holds no further pair"),
        }
    }
}

/// One pair of the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Given {
    /// What the handle was installed for.
    pub role: Role,
    /// The handle, as it stands in the table of the process.
    pub handle: Handle,
}

/// Writes a startup message into the buffer of a process that is about to
/// start.
///
/// The writer holds no buffer of its own: it appends into the one it is
/// given, so the kernel can build the message straight in the frame it
/// mapped for the thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct Writer {
    pairs: usize,
}

impl Writer {
    /// A writer that has written nothing.
    #[must_use]
    pub const fn new() -> Self {
        Writer { pairs: 0 }
    }

    /// How many pairs have been appended.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.pairs
    }

    /// `true` before the first pair.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pairs == 0
    }

    /// Appends `handle` under `role`.
    ///
    /// # Errors
    ///
    /// [`StartupError::Full`] when the message area holds no further pair;
    /// nothing is written then.
    pub fn give(
        &mut self,
        buffer: &mut BufferMut<'_>,
        role: Role,
        handle: Handle,
    ) -> Result<(), StartupError> {
        let role_index = self.pairs.checked_mul(2).ok_or(StartupError::Full)?;
        let handle_index = role_index.checked_add(1).ok_or(StartupError::Full)?;
        if !buffer.set_word(role_index, u64::from(role.code())) {
            return Err(StartupError::Full);
        }
        if !buffer.set_word(handle_index, handle.raw()) {
            return Err(StartupError::Full);
        }
        self.pairs = self.pairs.wrapping_add(1);
        Ok(())
    }

    /// Writes the label and the counts, which makes the message readable.
    /// Call it once, after the last [`give`](Self::give).
    ///
    /// # Errors
    ///
    /// [`StartupError::Full`] when the pairs do not fit the message area,
    /// which [`give`](Self::give) has already refused.
    pub fn finish(&self, buffer: &mut BufferMut<'_>) -> Result<(), StartupError> {
        let words = self.pairs.checked_mul(2).ok_or(StartupError::Full)?;
        buffer.set_label(STARTUP_LABEL);
        buffer.set_counts(words, 0)?;
        Ok(())
    }
}

/// Reads the pairs of a startup message.
///
/// # Errors
///
/// [`StartupError`] for a message that is none, whose word count is odd,
/// or that carries a word naming no role or no handle. The pairs are
/// checked in the order they stand in, so the first fault is the one
/// reported.
pub fn read(buffer: Buffer<'_>) -> Result<impl Iterator<Item = Given>, StartupError> {
    let message = buffer.message()?;
    if message.label != STARTUP_LABEL {
        return Err(StartupError::NotStartup);
    }
    if !message.word_count.is_multiple_of(2) {
        return Err(StartupError::OddWordCount(message.word_count));
    }
    let pairs = message.word_count.wrapping_div(2);
    // Every pair is checked before the first is handed out, so a caller
    // that walks the iterator never meets an error halfway through.
    for pair in 0..pairs {
        let _ = given(buffer, pair)?;
    }
    Ok((0..pairs).filter_map(move |pair| given(buffer, pair).ok()))
}

/// The pair at `index`, checked.
fn given(buffer: Buffer<'_>, index: usize) -> Result<Given, StartupError> {
    let role_index = index.checked_mul(2).ok_or(StartupError::Full)?;
    let handle_index = role_index.checked_add(1).ok_or(StartupError::Full)?;
    let role_word = buffer.word(role_index).ok_or(StartupError::Full)?;
    let handle_word = buffer.word(handle_index).ok_or(StartupError::Full)?;
    let code = u32::try_from(role_word).map_err(|_| StartupError::UnknownRole(role_word))?;
    let role = Role::from_code(code).ok_or(StartupError::UnknownRole(role_word))?;
    let handle = Handle::from_raw(handle_word).ok_or(StartupError::BadHandle(handle_word))?;
    Ok(Given { role, handle })
}
