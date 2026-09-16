// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The window protocol: a window on the desktop, the commands that draw
//! into it, and the events that happen in it.
//!
//! A client of this protocol never sees pixels (D-152). It asks the compositor for
//! a window, sends lists of [`gfx::draw::Command`] to draw into it, and
//! takes the events that reached it one at a time. The surface is the
//! compositor's: it holds the pixels of every window, carries the commands
//! out into them, and composes what is on the screen out of them, so a
//! window that is covered and uncovered again needs nothing of its client.
//!
//! Asking for a window means handing over two capabilities: a notification
//! of one's own reduced to `SIGNAL`, which the compositor signals when an
//! event waits, and a capability to one's own process reduced to `INFO`,
//! whose end the compositor watches (D-106). A client that exits, or that
//! is killed for faulting, therefore gives its window back without saying
//! anything.
//!
//! Invariants: a reply carries the window only when its status word says
//! one was opened; a `Draw` carries at most [`gfx::draw::LIST_CAPACITY`]
//! commands, because that is what a list holds; the kind word of an event
//! names a kind this protocol has, or the message is refused.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, Handle};
use gfx::draw::{Command, LIST_CAPACITY, List, TEXT_CAPACITY, Text};
use gfx::{Color, Rect};
use user_rt::message::{Reader, Writer};

use crate::bytes::Bytes;
use crate::input::KeyCode;
use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// `open`: a window of this size, please.
pub const OPEN: u16 = 1;

/// `draw`: carry these commands out in my window.
pub const DRAW: u16 = 2;

/// `close`: this window is no longer needed.
pub const CLOSE: u16 = 3;

/// `poll`: what happened in my window?
pub const POLL: u16 = 4;

/// How many bytes the title of a window has at most.
pub const MAX_TITLE: usize = 24;

/// The title of a window.
pub type Title = Bytes<MAX_TITLE>;

/// How far up the first half of a packed word sits.
const FIELD_SHIFT: u32 = 32;

/// The low half of a packed word.
const FIELD_MASK: u64 = 0xFFFF_FFFF;

/// How far up the green channel sits in a packed color.
const GREEN_SHIFT: u32 = 8;

/// How far up the red channel sits in one.
const RED_SHIFT: u32 = 16;

/// The kind word of a command that clears a window.
const CLEAR: u64 = 1;
/// The kind word of a fill.
const FILL: u64 = 2;
/// The kind word of an outline.
const FRAME: u64 = 3;
/// The kind word of a segment.
const LINE: u64 = 4;
/// The kind word of a run of text.
const TEXT: u64 = 5;

/// The kind word of a key event.
const KEY_EVENT: u64 = 1;
/// The kind word of a pointer event.
const POINTER_EVENT: u64 = 2;
/// The kind word of a change of focus.
const FOCUS_EVENT: u64 = 3;
/// The kind word of the event that says the window is going away.
const CLOSED_EVENT: u64 = 4;

/// A window the compositor opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    /// What the client names the window in later messages.
    pub id: u32,
    /// Columns of its content, which is what the compositor granted and
    /// may be less than what was asked for.
    pub width: u32,
    /// Rows of it.
    pub height: u32,
}

/// What happened in a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A key went down or came up while the window had the focus.
    Key {
        /// Which key.
        code: KeyCode,
        /// `true` while it is down.
        pressed: bool,
    },
    /// The pointer is over the content of the window, at this pixel of it.
    Pointer {
        /// Column inside the content.
        x: u32,
        /// Row inside it.
        y: u32,
        /// Which buttons are held.
        buttons: u8,
    },
    /// The window took the focus or lost it.
    Focus {
        /// `true` when it now has the focus.
        has: bool,
    },
    /// The window is going away: the desktop is ending, or its close box
    /// was clicked.
    Closed,
}

/// What a client asks the compositor.
#[expect(
    clippy::large_enum_variant,
    reason = "a draw carries its sixteen commands by value, because this system has no allocator and the message is decoded onto the stack of the server loop"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// A window of this size under this title, please. Both handles travel
    /// in the handle area: a notification the compositor signals, and the
    /// client as something whose end it may watch.
    Open {
        /// Columns of content asked for.
        width: u32,
        /// Rows of it.
        height: u32,
        /// What stands in the title bar.
        title: Title,
        /// What is signalled when an event waits.
        notification: Handle,
        /// The client, as something the compositor may watch the end of.
        process: Handle,
    },
    /// Carry these commands out in this window.
    Draw {
        /// The window, as [`Window::id`] named it.
        id: u32,
        /// What to draw.
        commands: List,
    },
    /// This window is no longer needed.
    Close {
        /// The window, as [`Window::id`] named it.
        id: u32,
    },
    /// The next event of this window, if one waits.
    Poll {
        /// The window, as [`Window::id`] named it.
        id: u32,
    },
}

/// What the compositor answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The window, or why there is none.
    Opened(Result<Window, Error>),
    /// Whether the commands were carried out.
    Drawn(Result<(), Error>),
    /// Whether the window is gone.
    Closed(Result<(), Error>),
    /// The next event, nothing when none waits, or why there is no answer.
    Polled(Result<Option<Event>, Error>),
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
    (
        ((word >> FIELD_SHIFT) & FIELD_MASK) as u32,
        (word & FIELD_MASK) as u32,
    )
}

/// The three channels of a color in one word.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "widening three 8-bit channels into the word they are packed in, in a const fn"
)]
pub const fn color_word(color: Color) -> u64 {
    ((color.r as u64) << RED_SHIFT) | ((color.g as u64) << GREEN_SHIFT) | color.b as u64
}

/// The color a word stands for, whose bytes above the three channels are
/// ignored.
#[must_use]
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "taking the three channels out of the word, in a const fn; each truncation is the channel"
)]
pub const fn color_of(word: u64) -> Color {
    Color::new(
        (word >> RED_SHIFT) as u8,
        (word >> GREEN_SHIFT) as u8,
        word as u8,
    )
}

/// Writes one command.
fn write_command(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    command: &Command,
) -> Result<(), ProtoError> {
    match *command {
        Command::Clear { color } => {
            writer.word(buffer, CLEAR)?;
            writer.word(buffer, color_word(color))?;
        }
        Command::Fill { rect, color } | Command::Frame { rect, color } => {
            let kind = if matches!(command, Command::Fill { .. }) {
                FILL
            } else {
                FRAME
            };
            writer.word(buffer, kind)?;
            writer.word(buffer, pack(rect.x, rect.y))?;
            writer.word(buffer, pack(rect.w, rect.h))?;
            writer.word(buffer, color_word(color))?;
        }
        Command::Line { from, to, color } => {
            writer.word(buffer, LINE)?;
            writer.word(buffer, pack(from.0, from.1))?;
            writer.word(buffer, pack(to.0, to.1))?;
            writer.word(buffer, color_word(color))?;
        }
        Command::Text {
            x,
            y,
            color,
            background,
            text,
        } => {
            writer.word(buffer, TEXT)?;
            writer.word(buffer, pack(x, y))?;
            writer.word(buffer, color_word(color))?;
            match background {
                Some(behind) => writer.word(buffer, pack(1, 0) | color_word(behind))?,
                None => writer.word(buffer, 0)?,
            }
            writer.bytes(buffer, text.as_bytes())?;
        }
    }
    Ok(())
}

/// Reads one command.
fn read_command(reader: &mut Reader<'_>) -> Result<Command, ProtoError> {
    let kind = reader.word()?;
    match kind {
        CLEAR => Ok(Command::Clear {
            color: color_of(reader.word()?),
        }),
        FILL | FRAME => {
            let (x, y) = unpack(reader.word()?);
            let (w, h) = unpack(reader.word()?);
            let rect = Rect::new(x, y, w, h);
            let color = color_of(reader.word()?);
            if kind == FILL {
                Ok(Command::Fill { rect, color })
            } else {
                Ok(Command::Frame { rect, color })
            }
        }
        LINE => Ok(Command::Line {
            from: unpack(reader.word()?),
            to: unpack(reader.word()?),
            color: color_of(reader.word()?),
        }),
        TEXT => {
            let (x, y) = unpack(reader.word()?);
            let color = color_of(reader.word()?);
            let behind = reader.word()?;
            let (flag, _low) = unpack(behind);
            let background = if flag == 0 {
                None
            } else {
                Some(color_of(behind))
            };
            let mut into = [0u8; TEXT_CAPACITY];
            let len = reader.bytes(&mut into)?;
            let text = into
                .get(..len)
                .and_then(Text::from_bytes)
                .ok_or(ProtoError::Message(Protocol::Window, DRAW))?;
            Ok(Command::Text {
                x,
                y,
                color,
                background,
                text,
            })
        }
        _other => Err(ProtoError::Message(Protocol::Window, DRAW)),
    }
}

/// Writes the commands of `list`, the count first.
fn write_list(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    list: &List,
) -> Result<(), ProtoError> {
    writer.word(buffer, u64::try_from(list.len()).unwrap_or(0))?;
    for command in list.iter() {
        write_command(writer, buffer, command)?;
    }
    Ok(())
}

/// Reads the commands a `Draw` carries.
fn read_list(reader: &mut Reader<'_>) -> Result<List, ProtoError> {
    let count = usize::try_from(reader.word()?).unwrap_or(usize::MAX);
    if count > LIST_CAPACITY {
        return Err(ProtoError::Message(Protocol::Window, DRAW));
    }
    let mut list = List::new();
    for _ in 0..count {
        let command = read_command(reader)?;
        if !list.push(command) {
            return Err(ProtoError::Message(Protocol::Window, DRAW));
        }
    }
    Ok(list)
}

/// Writes one event, or the word that says none waits.
fn write_event(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    event: Option<Event>,
) -> Result<(), ProtoError> {
    match event {
        None => writer.word(buffer, 0)?,
        Some(Event::Key { code, pressed }) => {
            writer.word(buffer, KEY_EVENT)?;
            writer.word(buffer, pack(u32::from(code.code()), u32::from(pressed)))?;
        }
        Some(Event::Pointer { x, y, buttons }) => {
            writer.word(buffer, POINTER_EVENT)?;
            writer.word(buffer, pack(x, y))?;
            writer.word(buffer, u64::from(buttons))?;
        }
        Some(Event::Focus { has }) => {
            writer.word(buffer, FOCUS_EVENT)?;
            writer.word(buffer, u64::from(has))?;
        }
        Some(Event::Closed) => writer.word(buffer, CLOSED_EVENT)?,
    }
    Ok(())
}

/// Reads the event a `Polled` carries.
fn read_event(reader: &mut Reader<'_>) -> Result<Option<Event>, ProtoError> {
    match reader.word()? {
        0 => Ok(None),
        KEY_EVENT => {
            let (code, pressed) = unpack(reader.word()?);
            let code = u16::try_from(code)
                .ok()
                .and_then(KeyCode::from_code)
                .ok_or(ProtoError::Message(Protocol::Window, POLL))?;
            Ok(Some(Event::Key {
                code,
                pressed: pressed != 0,
            }))
        }
        POINTER_EVENT => {
            let (x, y) = unpack(reader.word()?);
            let buttons = u8::try_from(reader.word()? & 0xFF).unwrap_or(0);
            Ok(Some(Event::Pointer { x, y, buttons }))
        }
        FOCUS_EVENT => Ok(Some(Event::Focus {
            has: reader.word()? != 0,
        })),
        CLOSED_EVENT => Ok(Some(Event::Closed)),
        _other => Err(ProtoError::Message(Protocol::Window, POLL)),
    }
}

/// The identifier of a window, which is a number that fits its field.
fn window_id(word: u64) -> Result<u32, ProtoError> {
    u32::try_from(word).map_err(|_| ProtoError::Message(Protocol::Window, DRAW))
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Request::Open { .. } => OPEN,
            Request::Draw { .. } => DRAW,
            Request::Close { .. } => CLOSE,
            Request::Poll { .. } => POLL,
        };
        Label::new(Protocol::Window, message)
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area or the handle area has
    /// no room, which sixteen commands of at most fifty-six bytes of text
    /// never run into.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Open {
                width,
                height,
                title,
                notification,
                process,
            } => {
                writer.word(buffer, pack(*width, *height))?;
                writer.bytes(buffer, title.as_bytes())?;
                writer.handle(buffer, *notification)?;
                writer.handle(buffer, *process)?;
            }
            Request::Draw { id, commands } => {
                writer.word(buffer, u64::from(*id))?;
                write_list(&mut writer, buffer, commands)?;
            }
            Request::Close { id } | Request::Poll { id } => {
                writer.word(buffer, u64::from(*id))?;
            }
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a request out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] for a label of another protocol or another version,
    /// for a message number this protocol does not have, for a title or a
    /// text longer than its field, for more commands than a list holds,
    /// for a command kind this protocol does not have, and for a message
    /// whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            OPEN => {
                let (width, height) = unpack(reader.word()?);
                let mut into = [0u8; MAX_TITLE];
                let len = reader.bytes(&mut into)?;
                Ok(Request::Open {
                    width,
                    height,
                    title: Title::new(into.get(..len).unwrap_or(&[]))?,
                    notification: reader.handle()?,
                    process: reader.handle()?,
                })
            }
            DRAW => Ok(Request::Draw {
                id: window_id(reader.word()?)?,
                commands: read_list(&mut reader)?,
            }),
            CLOSE => Ok(Request::Close {
                id: window_id(reader.word()?)?,
            }),
            POLL => Ok(Request::Poll {
                id: window_id(reader.word()?)?,
            }),
            other => Err(ProtoError::Message(Protocol::Window, other)),
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
            Reply::Drawn(_) => DRAW,
            Reply::Closed(_) => CLOSE,
            Reply::Polled(_) => POLL,
        };
        Label::new(Protocol::Window, message)
    }

    /// Writes the reply into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Reply::Opened(Ok(window)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(window.id))?;
                writer.word(buffer, pack(window.width, window.height))?;
            }
            Reply::Opened(Err(error)) | Reply::Polled(Err(error)) => {
                writer.word(buffer, u64::from(error.code()))?;
            }
            Reply::Drawn(outcome) | Reply::Closed(outcome) => {
                writer.word(buffer, status_word(*outcome))?;
            }
            Reply::Polled(Ok(event)) => {
                writer.word(buffer, 0)?;
                write_event(&mut writer, buffer, *event)?;
            }
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a reply out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] as [`Request::decode`], [`ProtoError::Status`] for a
    /// status word that names no error, and [`ProtoError::Message`] for an
    /// event kind or a key code this protocol does not have.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let outcome = status_of(reader.word()?)?;
        match (label.message, outcome) {
            (OPEN, Ok(())) => {
                let id = window_id(reader.word()?)?;
                let (width, height) = unpack(reader.word()?);
                Ok(Reply::Opened(Ok(Window { id, width, height })))
            }
            (OPEN, Err(error)) => Ok(Reply::Opened(Err(error))),
            (DRAW, outcome) => Ok(Reply::Drawn(outcome)),
            (CLOSE, outcome) => Ok(Reply::Closed(outcome)),
            (POLL, Ok(())) => Ok(Reply::Polled(Ok(read_event(&mut reader)?))),
            (POLL, Err(error)) => Ok(Reply::Polled(Err(error))),
            (other, _) => Err(ProtoError::Message(Protocol::Window, other)),
        }
    }
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Window {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Window,
            found: label.protocol,
        });
    }
    Ok(label)
}
