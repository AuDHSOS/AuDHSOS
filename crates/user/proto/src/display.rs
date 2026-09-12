// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The display protocol: what the screen is, a surface to draw into, and
//! putting what was drawn on the screen.
//!
//! A client never sees the framebuffer. It asks for a surface, receives a
//! memory object of its own to draw into, and says which rectangles of it
//! changed; the display server copies those rectangles and nothing else.
//! What a client may do to a surface is decided by the badge its messages
//! carry, so a surface belongs to whoever created it and to nobody else.
//!
//! Asking for a surface means handing over a capability to oneself: the
//! client sends a handle to its own process that carries `INFO` and no
//! other right, and the server watches the end of it (D-106). A client that
//! exits, or that is killed for faulting, therefore gives its surface back
//! without having to say anything.
//!
//! Invariants: a reply carries the memory object in the handle area only
//! when its status word says the surface was created; a `Present` carries
//! at most [`gfx::DAMAGE_CAPACITY`] rectangles, because that is what the
//! damage set of a surface holds; the shape word of a `SetCursor` names a
//! sprite this protocol has, or the message is refused.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, FramebufferFormat, Handle};
use gfx::{Damage, Rect};
use user_rt::message::{Reader, Writer};

use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// `info`: what the screen is.
pub const INFO: u16 = 1;

/// `create_surface`: a surface to draw into.
pub const CREATE_SURFACE: u16 = 2;

/// `present`: put what was drawn on the screen.
pub const PRESENT: u16 = 3;

/// `destroy_surface`: give a surface up.
pub const DESTROY_SURFACE: u16 = 4;

/// `set_cursor`: where the pointer is and whether it is shown.
pub const SET_CURSOR: u16 = 5;

/// How far up the first half of a packed word sits.
const FIELD_SHIFT: u32 = 32;

/// The low half of a packed word.
const FIELD_MASK: u64 = 0xFFFF_FFFF;

/// Which sprite the pointer shows.
///
/// The shape is the client's to choose, because the server knows where the
/// pointer is and nothing about what it stands over: a program that drags
/// the corner of a window is the one that knows the drag is a resize.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The arrow, which is what the pointer is when nothing is going on.
    #[default]
    Arrow,
    /// The double arrow of a resize, along the diagonal a corner moves on.
    Resize,
}

impl CursorShape {
    /// The number the shape travels as.
    #[must_use]
    pub const fn code(self) -> u32 {
        match self {
            CursorShape::Arrow => 0,
            CursorShape::Resize => 1,
        }
    }

    /// The shape a number names, or nothing for a number that names none.
    #[must_use]
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(CursorShape::Arrow),
            1 => Some(CursorShape::Resize),
            _other => None,
        }
    }
}

/// What the screen is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mode {
    /// Visible columns.
    pub width: u32,
    /// Visible rows.
    pub height: u32,
    /// The order of the channels a surface is drawn in.
    pub format: FramebufferFormat,
}

/// A surface the server made, and the memory it is drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Surface {
    /// What the client names the surface in later messages.
    pub id: u32,
    /// The memory object holding its pixels.
    pub memory: Handle,
}

/// What a client asks the display server.
#[expect(
    clippy::large_enum_variant,
    reason = "a present carries its sixteen rectangles by value, because this system has no allocator and the message is decoded onto the stack of the server loop"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// What is the screen?
    Info,
    /// A surface of this size, please. The handle travels in the handle
    /// area: it is a capability to the process of the client, carrying
    /// `INFO` and nothing else, and the server watches it so that a client
    /// that is gone does not leave its surface behind for ever.
    CreateSurface {
        /// Visible columns.
        width: u32,
        /// Visible rows.
        height: u32,
        /// The client, as something the server may watch the end of.
        process: Handle,
    },
    /// These rectangles of this surface changed.
    Present {
        /// The surface, as [`Surface::id`] named it.
        id: u32,
        /// What changed.
        damage: Damage,
    },
    /// This surface is no longer needed.
    DestroySurface {
        /// The surface, as [`Surface::id`] named it.
        id: u32,
    },
    /// The pointer is here, shows this sprite, and is shown or is not.
    SetCursor {
        /// Column of the hot spot.
        x: u32,
        /// Row of the hot spot.
        y: u32,
        /// Whether the sprite is drawn.
        visible: bool,
        /// Which sprite it is.
        shape: CursorShape,
    },
}

/// What the display server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// The screen, or why there is none.
    Screen(Result<Mode, Error>),
    /// The surface, or why there is none.
    Created(Result<Surface, Error>),
    /// Whether what was drawn is on the screen.
    Presented(Result<(), Error>),
    /// Whether the surface is gone.
    Destroyed(Result<(), Error>),
    /// Whether the cursor moved.
    CursorSet(Result<(), Error>),
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

/// Writes the rectangles of `damage`, the count first.
fn write_damage(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    damage: &Damage,
) -> Result<(), ProtoError> {
    writer.word(buffer, u64::try_from(damage.len()).unwrap_or(0))?;
    for rect in damage.iter() {
        writer.word(buffer, pack(rect.x, rect.y))?;
        writer.word(buffer, pack(rect.w, rect.h))?;
    }
    Ok(())
}

/// Reads the rectangles a `Present` carries.
fn read_damage(reader: &mut Reader<'_>) -> Result<Damage, ProtoError> {
    let count = usize::try_from(reader.word()?).unwrap_or(usize::MAX);
    if count > gfx::DAMAGE_CAPACITY {
        return Err(ProtoError::Message(Protocol::Display, PRESENT));
    }
    let mut damage = Damage::new();
    for _ in 0..count {
        let (x, y) = unpack(reader.word()?);
        let (w, h) = unpack(reader.word()?);
        damage.push(Rect::new(x, y, w, h));
    }
    Ok(damage)
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Request::Info => INFO,
            Request::CreateSurface { .. } => CREATE_SURFACE,
            Request::Present { .. } => PRESENT,
            Request::DestroySurface { .. } => DESTROY_SURFACE,
            Request::SetCursor { .. } => SET_CURSOR,
        };
        Label::new(Protocol::Display, message)
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room, which
    /// sixteen rectangles never run into.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Info => {}
            Request::CreateSurface {
                width,
                height,
                process,
            } => {
                writer.word(buffer, pack(*width, *height))?;
                writer.handle(buffer, *process)?;
            }
            Request::Present { id, damage } => {
                writer.word(buffer, u64::from(*id))?;
                write_damage(&mut writer, buffer, damage)?;
            }
            Request::DestroySurface { id } => writer.word(buffer, u64::from(*id))?,
            Request::SetCursor {
                x,
                y,
                visible,
                shape,
            } => {
                writer.word(buffer, pack(*x, *y))?;
                writer.word(buffer, pack(shape.code(), u32::from(*visible)))?;
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
    /// for a message number this protocol does not have, for more
    /// rectangles than a damage set holds, and for a message whose fields
    /// are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            INFO => Ok(Request::Info),
            CREATE_SURFACE => {
                let (width, height) = unpack(reader.word()?);
                Ok(Request::CreateSurface {
                    width,
                    height,
                    process: reader.handle()?,
                })
            }
            PRESENT => {
                let id = surface_id(reader.word()?)?;
                Ok(Request::Present {
                    id,
                    damage: read_damage(&mut reader)?,
                })
            }
            DESTROY_SURFACE => Ok(Request::DestroySurface {
                id: surface_id(reader.word()?)?,
            }),
            SET_CURSOR => {
                let (x, y) = unpack(reader.word()?);
                let (shape, visible) = unpack(reader.word()?);
                Ok(Request::SetCursor {
                    x,
                    y,
                    visible: visible != 0,
                    shape: CursorShape::from_code(shape)
                        .ok_or(ProtoError::Message(Protocol::Display, SET_CURSOR))?,
                })
            }
            other => Err(ProtoError::Message(Protocol::Display, other)),
        }
    }
}

/// The identifier of a surface, which is a number that fits its field.
fn surface_id(word: u64) -> Result<u32, ProtoError> {
    u32::try_from(word).map_err(|_| ProtoError::Message(Protocol::Display, PRESENT))
}

impl Reply {
    /// The label this reply is sent under, which is the label of the
    /// request it answers.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Reply::Screen(_) => INFO,
            Reply::Created(_) => CREATE_SURFACE,
            Reply::Presented(_) => PRESENT,
            Reply::Destroyed(_) => DESTROY_SURFACE,
            Reply::CursorSet(_) => SET_CURSOR,
        };
        Label::new(Protocol::Display, message)
    }

    /// Writes the reply into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area or the handle area has
    /// no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Reply::Screen(Ok(mode)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, pack(mode.width, mode.height))?;
                writer.word(buffer, u64::from(mode.format.code()))?;
            }
            Reply::Created(Ok(surface)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(surface.id))?;
                writer.handle(buffer, surface.memory)?;
            }
            Reply::Screen(Err(error)) | Reply::Created(Err(error)) => {
                writer.word(buffer, u64::from(error.code()))?;
            }
            Reply::Presented(outcome) | Reply::Destroyed(outcome) | Reply::CursorSet(outcome) => {
                writer.word(buffer, status_word(*outcome))?;
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
    /// status word that names no error, and
    /// [`ProtoError::Message`] for a format code that names no format.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let outcome = status_of(reader.word()?)?;
        match (label.message, outcome) {
            (INFO, Ok(())) => {
                let (width, height) = unpack(reader.word()?);
                let code = u32::try_from(reader.word()?).unwrap_or(0);
                let format = FramebufferFormat::from_code(code)
                    .ok_or(ProtoError::Message(Protocol::Display, INFO))?;
                Ok(Reply::Screen(Ok(Mode {
                    width,
                    height,
                    format,
                })))
            }
            (INFO, Err(error)) => Ok(Reply::Screen(Err(error))),
            (CREATE_SURFACE, Ok(())) => Ok(Reply::Created(Ok(Surface {
                id: surface_id(reader.word()?)?,
                memory: reader.handle()?,
            }))),
            (CREATE_SURFACE, Err(error)) => Ok(Reply::Created(Err(error))),
            (PRESENT, outcome) => Ok(Reply::Presented(outcome)),
            (DESTROY_SURFACE, outcome) => Ok(Reply::Destroyed(outcome)),
            (SET_CURSOR, outcome) => Ok(Reply::CursorSet(outcome)),
            (other, _) => Err(ProtoError::Message(Protocol::Display, other)),
        }
    }
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Display {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Display,
            found: label.protocol,
        });
    }
    Ok(label)
}
