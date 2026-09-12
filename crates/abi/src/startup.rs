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
//! A pair carries either a handle or a plain number, and the role says
//! which: the display server is given the memory object over the
//! framebuffer, which is a capability, and the width, the height, the
//! stride, and the format of the mode the firmware set, which are numbers
//! and describe no object (D-103). A reader that meets a value role it
//! does not know passes over it like any other.
//!
//! ```text
//! label       STARTUP_LABEL
//! word 0      role of the first pair
//! word 1      its handle, or its value
//! word 2      role of the second pair
//! word 3      its handle, or its value
//! ...
//! ```
//!
//! Invariants: the word count is even; every role word names a role; the
//! second word of a pair whose role names a handle is one; the message
//! carries no handles in the handle area, because the handles are already
//! in the table of the process and the message only says what they are.

use crate::ipc_buffer::{Buffer, BufferMut, MessageError};
use crate::{Error, Handle};

/// The label of the startup message.
///
/// The bytes spell `STARTUP`, which makes it recognizable in a dump of the
/// buffer, and it lies far below [`crate::ipc_buffer::KERNEL_LABEL_BASE`],
/// so it is a label a user thread may also send.
pub const STARTUP_LABEL: u64 = u64::from_be_bytes(*b"STARTUP\0");

const _: () = assert!(STARTUP_LABEL < crate::ipc_buffer::KERNEL_LABEL_BASE);

/// Whether a role names a handle or a value, as the table spells it.
macro_rules! carries_value {
    (handle) => {
        false
    };
    (value) => {
        true
    };
}

/// Declares the role table once and derives the enum, the code lookup, the
/// name, and what each role carries from it.
macro_rules! roles {
    ($($variant:ident = $code:literal, $kind:ident => $doc:literal),+ $(,)?) => {
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

            /// `true` when the second word of the pair is a number the
            /// parent tells the process, and not a handle of its table.
            #[must_use]
            pub const fn carries_value(self) -> bool {
                match self {
                    $(Role::$variant => carries_value!($kind),)+
                }
            }
        }
    };
}

roles! {
    OwnProcess = 1, handle => "A handle to the process itself, which it needs to map memory into its own address space.",
    OwnEndpoint = 2, handle => "The endpoint the process receives requests on. A server gets one; a client that serves nobody does not.",
    SystemControl = 3, handle => "The capability to create interrupts, port ranges, and device memory. Only the root task receives it.",
    BootImage = 4, handle => "A memory object over the boot image, which carries the archive. Only the root task receives it.",
    Ram = 5, handle => "A memory object over a free region of memory. The root task receives one per region, so this role appears more than once.",
    NameServer = 6, handle => "The endpoint of the name server.",
    MemoryServer = 7, handle => "The endpoint of the memory server.",
    Log = 8, handle => "The endpoint diagnostics are sent to. It is the console once there is one.",
    IoPorts = 9, handle => "A range of I/O ports the process is allowed to reach. A driver receives one.",
    Interrupt = 10, handle => "An interrupt object for the line the process serves. A driver receives one.",
    Parent = 11, handle => "The endpoint of the process that started this one, badged with what that process knows it by. It is the same endpoint the kernel sends this process's faults to, so a report and a fault arrive at one place, told apart by the label.",
    Framebuffer = 12, handle => "A device memory object over the framebuffer of the machine. Only the display server receives it.",
    FramebufferGeometry = 13, value => "The width of the framebuffer in the high half of the word and its height in the low half. It comes with `Framebuffer`.",
    FramebufferLine = 14, value => "The pixels from the start of one row of the framebuffer to the start of the next in the high half of the word, and the code of the pixel format in the low half. It comes with `Framebuffer`.",
    DisplayServer = 15, handle => "The endpoint of the display server, badged with what that server is to know this process by. A program that draws receives one; a program that finds the server by name instead receives an endpoint that names nobody, and the server refuses it.",
    AuxInterrupt = 16, handle => "An interrupt object for the second line of a controller the process serves. The driver of the PS/2 controller receives one for the mouse beside the `Interrupt` of the keyboard; a driver that serves one line receives none (D-109).",
    InputServer = 17, handle => "The endpoint of the input server, badged with what that server is to know this process by. A program that listens receives one, exactly as a program that draws receives `DisplayServer`, and for the same reason (D-109).",
    Ecam = 18, handle => "A device memory object over the configuration window of the PCI bus. Only the program that enumerates the bus receives it.",
    EcamBuses = 19, value => "The segment group of that window in the high half of the word, its first bus in bits 15 to 8, and its last bus in bits 7 to 0. It comes with `Ecam`.",
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
    /// A handle was written under a role that carries a value, or the
    /// other way round.
    WrongKind(Role),
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
            | StartupError::WrongKind(_)
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
            StartupError::WrongKind(role) => {
                let carries = if role.carries_value() {
                    "a value"
                } else {
                    "a handle"
                };
                write!(f, "the role {} carries {carries}", role.name())
            }
        }
    }
}

/// What the second word of a pair carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Payload {
    /// A handle, as it stands in the table of the process.
    Handle(Handle),
    /// A number the parent tells the process.
    Value(u64),
}

/// One pair of the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Given {
    /// What it was given for.
    pub role: Role,
    /// What was given.
    pub payload: Payload,
}

impl Given {
    /// The handle, or `None` for a pair that carries a value.
    #[must_use]
    pub const fn handle(self) -> Option<Handle> {
        match self.payload {
            Payload::Handle(handle) => Some(handle),
            Payload::Value(_) => None,
        }
    }

    /// The value, or `None` for a pair that carries a handle.
    #[must_use]
    pub const fn value(self) -> Option<u64> {
        match self.payload {
            Payload::Value(word) => Some(word),
            Payload::Handle(_) => None,
        }
    }
}

/// The mode of a framebuffer, as the two value roles carry it.
///
/// It is here and not in the display server because both sides need it: the
/// root task packs the words out of what `system_info` told it, and the
/// server unpacks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Screen {
    /// Visible columns.
    pub width: u32,
    /// Visible rows.
    pub height: u32,
    /// Pixels from the start of one row to the start of the next.
    pub stride: u32,
    /// The order of the channels.
    pub format: crate::FramebufferFormat,
}

/// How far up the first field of a value word sits.
const FIELD_SHIFT: u32 = 32;

/// The low half of a value word.
const FIELD_MASK: u64 = 0xFFFF_FFFF;

impl Screen {
    /// The word of [`Role::FramebufferGeometry`].
    #[must_use]
    pub const fn geometry(self) -> u64 {
        pack(self.width, self.height)
    }

    /// The word of [`Role::FramebufferLine`].
    #[must_use]
    pub const fn line(self) -> u64 {
        pack(self.stride, self.format.code())
    }

    /// The mode the two words describe, or `None` when the format code
    /// names no format.
    #[must_use]
    pub const fn from_words(geometry: u64, line: u64) -> Option<Self> {
        let (width, height) = unpack(geometry);
        let (stride, code) = unpack(line);
        match crate::FramebufferFormat::from_code(code) {
            Some(format) => Some(Screen {
                width,
                height,
                stride,
                format,
            }),
            None => None,
        }
    }
}

/// The buses of a configuration window, as its value role carries them.
///
/// It is here for the reason [`Screen`] is: the root task packs the word out
/// of what `system_info` told it, and the program that enumerates unpacks
/// it. The base address is no part of it — the program reaches the window
/// through the mapping of its memory object and never names an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BusRange {
    /// The segment group the window covers.
    pub segment: u16,
    /// The first bus of the group the window holds.
    pub first_bus: u8,
    /// The last bus of the group the window holds.
    pub last_bus: u8,
}

impl BusRange {
    /// The word of [`Role::EcamBuses`].
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "widening three fields into the word they are packed in, in a const fn"
    )]
    pub const fn word(self) -> u64 {
        pack(
            self.segment as u32,
            ((self.first_bus as u32) << 8) | (self.last_bus as u32),
        )
    }

    /// The buses the word describes.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "each mask keeps its field inside the type it goes into, and the function is const"
    )]
    pub const fn from_word(word: u64) -> Self {
        let (segment, buses) = unpack(word);
        BusRange {
            segment: (segment & 0xFFFF) as u16,
            first_bus: ((buses >> 8) & 0xFF) as u8,
            last_bus: (buses & 0xFF) as u8,
        }
    }
}

/// Puts two halves into one word.
#[expect(
    clippy::as_conversions,
    reason = "widening two 32-bit fields into the word they are packed in, in a const fn"
)]
const fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << FIELD_SHIFT) | (low as u64)
}

/// Takes one word apart into its two halves.
#[expect(
    clippy::as_conversions,
    reason = "the mask keeps each half inside u32, and the function is const"
)]
const fn unpack(word: u64) -> (u32, u32) {
    let high = (word >> FIELD_SHIFT) & FIELD_MASK;
    let low = word & FIELD_MASK;
    (high as u32, low as u32)
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
    /// [`StartupError::WrongKind`] for a role that carries a value;
    /// [`StartupError::Full`] when the message area holds no further pair.
    /// Nothing is written in either case.
    pub fn give(
        &mut self,
        buffer: &mut BufferMut<'_>,
        role: Role,
        handle: Handle,
    ) -> Result<(), StartupError> {
        if role.carries_value() {
            return Err(StartupError::WrongKind(role));
        }
        self.append(buffer, role, handle.raw())
    }

    /// Appends `value` under `role`.
    ///
    /// # Errors
    ///
    /// [`StartupError::WrongKind`] for a role that names a handle;
    /// [`StartupError::Full`] when the message area holds no further pair.
    /// Nothing is written in either case.
    pub fn tell(
        &mut self,
        buffer: &mut BufferMut<'_>,
        role: Role,
        value: u64,
    ) -> Result<(), StartupError> {
        if !role.carries_value() {
            return Err(StartupError::WrongKind(role));
        }
        self.append(buffer, role, value)
    }

    /// Writes one pair, whatever its second word means.
    fn append(
        &mut self,
        buffer: &mut BufferMut<'_>,
        role: Role,
        payload: u64,
    ) -> Result<(), StartupError> {
        let role_index = self.pairs.checked_mul(2).ok_or(StartupError::Full)?;
        let payload_index = role_index.checked_add(1).ok_or(StartupError::Full)?;
        if !buffer.set_word(role_index, u64::from(role.code())) {
            return Err(StartupError::Full);
        }
        if !buffer.set_word(payload_index, payload) {
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
    let payload_index = role_index.checked_add(1).ok_or(StartupError::Full)?;
    let role_word = buffer.word(role_index).ok_or(StartupError::Full)?;
    let payload_word = buffer.word(payload_index).ok_or(StartupError::Full)?;
    let code = u32::try_from(role_word).map_err(|_| StartupError::UnknownRole(role_word))?;
    let role = Role::from_code(code).ok_or(StartupError::UnknownRole(role_word))?;
    if role.carries_value() {
        return Ok(Given {
            role,
            payload: Payload::Value(payload_word),
        });
    }
    let handle = Handle::from_raw(payload_word).ok_or(StartupError::BadHandle(payload_word))?;
    Ok(Given {
        role,
        payload: Payload::Handle(handle),
    })
}
