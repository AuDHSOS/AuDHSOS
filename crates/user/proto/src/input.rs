// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The input protocol: subscribing to what the keyboard and the mouse do,
//! the record one of those is, and the ring the server writes them into.
//!
//! A client subscribes once. It hands over a capability to a notification
//! of its own, reduced to `SIGNAL`, and receives a memory object of one
//! page: the ring. From then on the server appends every event to that ring
//! and signals the notification, and the client reads the ring whenever it
//! wakes. Nothing after the subscription is a message, which is why a
//! keystroke costs no system call on the server side beyond the signal.
//!
//! The ring is allocated by the server and not by the client, as the surface
//! of the display protocol is, and for the same reason: a server that keeps
//! something per client cannot trust an object a client hands it to be a
//! page long and to stay one.
//!
//! The event types are those of `driver-i8042` and are re-exported here, so
//! that the driver, the server, and every client speak of one `KeyCode`.
//!
//! Invariants: a record is sixteen bytes and the kind byte names a kind the
//! reader knows, or the record is refused; the writer never passes the
//! reader and reports what it dropped instead of overwriting it.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, Handle};
use user_rt::message::{Reader, Writer};

use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

pub use driver_i8042::keyboard::{KeyCode, KeyEvent};
pub use driver_i8042::mouse::PointerEvent;

/// `subscribe`: send me what the keyboard and the mouse do.
pub const SUBSCRIBE: u16 = 1;

/// `unsubscribe`: stop.
pub const UNSUBSCRIBE: u16 = 2;

/// How many bytes one event record has.
pub const EVENT_LEN: usize = 16;

/// The kind byte of a key event.
pub const KIND_KEY: u8 = 1;

/// The kind byte of a pointer event.
pub const KIND_POINTER: u8 = 2;

/// One thing that happened, as it stands in the ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A key went down or came up.
    Key(KeyEvent),
    /// The pointer moved, its wheel turned, or its buttons changed.
    Pointer(PointerEvent),
}

impl Event {
    /// The kind byte of the record.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Event::Key(_) => KIND_KEY,
            Event::Pointer(_) => KIND_POINTER,
        }
    }

    /// The record, little-endian, with every byte the kind does not use set
    /// to zero.
    #[must_use]
    pub fn to_bytes(self) -> [u8; EVENT_LEN] {
        let mut record = [0u8; EVENT_LEN];
        let mut put = |at: usize, from: &[u8]| {
            if let Some(slot) = record.get_mut(at..at.saturating_add(from.len())) {
                slot.copy_from_slice(from);
            }
        };
        put(0, &[self.kind()]);
        match self {
            Event::Key(key) => {
                put(2, &key.code.code().to_le_bytes());
                put(4, &[u8::from(key.pressed)]);
            }
            Event::Pointer(pointer) => {
                put(2, &pointer.dx.to_le_bytes());
                put(4, &pointer.dy.to_le_bytes());
                put(6, &pointer.wheel.to_le_bytes());
                put(7, &[pointer.buttons]);
            }
        }
        record
    }

    /// Reads a record, or `None` for one whose kind byte names no kind, whose
    /// second byte is not zero, or whose key code names no key.
    #[must_use]
    pub fn from_bytes(record: &[u8; EVENT_LEN]) -> Option<Self> {
        let at = |index: usize| record.get(index).copied().unwrap_or(0);
        if at(1) != 0 {
            return None;
        }
        let pair = |index: usize| [at(index), at(index.saturating_add(1))];
        match at(0) {
            KIND_KEY => Some(Event::Key(KeyEvent {
                code: KeyCode::from_code(u16::from_le_bytes(pair(2)))?,
                pressed: at(4) != 0,
            })),
            KIND_POINTER => Some(Event::Pointer(PointerEvent {
                dx: i16::from_le_bytes(pair(2)),
                dy: i16::from_le_bytes(pair(4)),
                wheel: i8::from_le_bytes([at(6)]),
                buttons: at(7),
            })),
            _other => None,
        }
    }
}

/// How many bytes the header of a ring takes.
pub const RING_HEADER_LEN: usize = 24;

/// How long the memory object of a ring is.
pub const RING_PAGE_LEN: usize = 4096;

/// How many records fit into that page: the header leaves 4072 bytes, which
/// is 254 records of sixteen and eight bytes over.
pub const RING_CAPACITY: u32 = 254;

/// The header of a ring, as it stands in the first twenty-four bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RingHeader {
    /// How many events the server has written since the ring was made.
    pub write_seq: u64,
    /// How many the client has taken.
    pub read_seq: u64,
    /// How many events were dropped because the ring was full.
    pub overflow: u32,
    /// How many records the ring holds.
    pub capacity: u32,
}

/// Reads the header out of the first bytes of a ring.
fn header_of(bytes: &[u8]) -> RingHeader {
    let word = |at: usize| {
        bytes
            .get(at..at.saturating_add(8))
            .and_then(|slice| slice.first_chunk::<8>())
            .map_or(0, |eight| u64::from_le_bytes(*eight))
    };
    let half = |at: usize| {
        bytes
            .get(at..at.saturating_add(4))
            .and_then(|slice| slice.first_chunk::<4>())
            .map_or(0, |four| u32::from_le_bytes(*four))
    };
    RingHeader {
        write_seq: word(0),
        read_seq: word(8),
        overflow: half(16),
        capacity: half(20),
    }
}

/// Writes one field of the header.
fn put_bytes(bytes: &mut [u8], at: usize, from: &[u8]) {
    if let Some(slot) = bytes.get_mut(at..at.saturating_add(from.len())) {
        slot.copy_from_slice(from);
    }
}

/// Where the record with sequence number `seq` stands.
fn slot_at(capacity: u32, seq: u64) -> Option<usize> {
    if capacity == 0 {
        return None;
    }
    let index = usize::try_from(seq.checked_rem(u64::from(capacity))?).ok()?;
    RING_HEADER_LEN.checked_add(index.checked_mul(EVENT_LEN)?)
}

/// How many records fit into `len` bytes, which is what a ring over them
/// holds, never more than [`RING_CAPACITY`].
#[must_use]
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "a record count already bounded by RING_CAPACITY, in a const fn"
)]
pub const fn capacity_for(len: usize) -> u32 {
    let records = len.saturating_sub(RING_HEADER_LEN).wrapping_div(EVENT_LEN);
    if records > RING_CAPACITY as usize {
        RING_CAPACITY
    } else {
        records as u32
    }
}

/// The writing half of a ring, which is the server's.
#[derive(Debug)]
pub struct RingWriter<'a> {
    bytes: &'a mut [u8],
}

impl<'a> RingWriter<'a> {
    /// Makes a ring over `bytes`: the header is written and the records are
    /// whatever stood there, which nothing reads until they are written.
    ///
    /// `None` when the bytes hold no record at all.
    #[must_use]
    pub fn create(bytes: &'a mut [u8]) -> Option<Self> {
        let capacity = capacity_for(bytes.len());
        if capacity == 0 {
            return None;
        }
        put_bytes(bytes, 0, &0u64.to_le_bytes());
        put_bytes(bytes, 8, &0u64.to_le_bytes());
        put_bytes(bytes, 16, &0u32.to_le_bytes());
        put_bytes(bytes, 20, &capacity.to_le_bytes());
        Some(RingWriter { bytes })
    }

    /// Takes up a ring somebody has already made.
    ///
    /// `None` when the header says the ring holds no record.
    #[must_use]
    pub fn adopt(bytes: &'a mut [u8]) -> Option<Self> {
        if header_of(bytes).capacity == 0 {
            return None;
        }
        Some(RingWriter { bytes })
    }

    /// The header as it stands.
    #[must_use]
    pub fn header(&self) -> RingHeader {
        header_of(self.bytes)
    }

    /// Appends `event`, or counts it as lost when the ring is full.
    ///
    /// `true` when the event was written. A full ring drops the newest
    /// event and not the oldest: the oldest is the one the client is about
    /// to read, and a ring that overwrote it would hand out a stream with a
    /// hole nobody can see.
    pub fn push(&mut self, event: Event) -> bool {
        let header = self.header();
        let held = header.write_seq.wrapping_sub(header.read_seq);
        if held >= u64::from(header.capacity) {
            let lost = header.overflow.saturating_add(1);
            put_bytes(self.bytes, 16, &lost.to_le_bytes());
            return false;
        }
        let Some(at) = slot_at(header.capacity, header.write_seq) else {
            return false;
        };
        put_bytes(self.bytes, at, &event.to_bytes());
        put_bytes(
            self.bytes,
            0,
            &header.write_seq.wrapping_add(1).to_le_bytes(),
        );
        true
    }
}

/// The reading half of a ring, which is the client's.
#[derive(Debug)]
pub struct RingReader<'a> {
    bytes: &'a mut [u8],
}

impl<'a> RingReader<'a> {
    /// Takes up the ring the server made.
    ///
    /// `None` when the header says the ring holds no record.
    #[must_use]
    pub fn new(bytes: &'a mut [u8]) -> Option<Self> {
        if header_of(bytes).capacity == 0 {
            return None;
        }
        Some(RingReader { bytes })
    }

    /// The header as it stands.
    #[must_use]
    pub fn header(&self) -> RingHeader {
        header_of(self.bytes)
    }

    /// `true` when nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let header = self.header();
        header.write_seq == header.read_seq
    }

    /// How many events were dropped since this was last asked, which also
    /// forgets them: a client hears about a gap once.
    pub fn take_overflow(&mut self) -> u32 {
        let lost = self.header().overflow;
        if lost != 0 {
            put_bytes(self.bytes, 16, &0u32.to_le_bytes());
        }
        lost
    }

    /// Takes the oldest event, or `None` when nothing is waiting.
    pub fn pop(&mut self) -> Option<Event> {
        let header = self.header();
        if header.write_seq == header.read_seq {
            return None;
        }
        let at = slot_at(header.capacity, header.read_seq)?;
        let record = self.bytes.get(at..at.checked_add(EVENT_LEN)?)?;
        let event = Event::from_bytes(record.first_chunk::<EVENT_LEN>()?);
        put_bytes(
            self.bytes,
            8,
            &header.read_seq.wrapping_add(1).to_le_bytes(),
        );
        event
    }
}

/// What a client asks the input server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Request {
    /// Send me what happens. The handle travels in the handle area: it is a
    /// capability to a notification of the client, carrying `SIGNAL` and
    /// nothing else, which the server signals whenever it has appended
    /// something to the ring.
    Subscribe {
        /// The notification the client waits on.
        notification: Handle,
    },
    /// Stop sending me anything. The badge says who asks.
    Unsubscribe,
}

/// What the input server answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reply {
    /// The memory object of the ring, or why there is none.
    Subscribed(Result<Handle, Error>),
    /// Whether the subscription is gone.
    Unsubscribed(Result<(), Error>),
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Request::Subscribe { .. } => SUBSCRIBE,
            Request::Unsubscribe => UNSUBSCRIBE,
        };
        Label::new(Protocol::Input, message)
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the handle area has no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Subscribe { notification } => writer.handle(buffer, *notification)?,
            Request::Unsubscribe => {}
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
    /// whose handle is not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            SUBSCRIBE => Ok(Request::Subscribe {
                notification: reader.handle()?,
            }),
            UNSUBSCRIBE => Ok(Request::Unsubscribe),
            other => Err(ProtoError::Message(Protocol::Input, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under, which is the label of the
    /// request it answers.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Reply::Subscribed(_) => SUBSCRIBE,
            Reply::Unsubscribed(_) => UNSUBSCRIBE,
        };
        Label::new(Protocol::Input, message)
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
            Reply::Subscribed(Ok(memory)) => {
                writer.word(buffer, 0)?;
                writer.handle(buffer, *memory)?;
            }
            Reply::Subscribed(Err(error)) => writer.word(buffer, u64::from(error.code()))?,
            Reply::Unsubscribed(outcome) => writer.word(buffer, status_word(*outcome))?,
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
            (SUBSCRIBE, Ok(())) => Ok(Reply::Subscribed(Ok(reader.handle()?))),
            (SUBSCRIBE, Err(error)) => Ok(Reply::Subscribed(Err(error))),
            (UNSUBSCRIBE, outcome) => Ok(Reply::Unsubscribed(outcome)),
            (other, _) => Err(ProtoError::Message(Protocol::Input, other)),
        }
    }
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Input {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Input,
            found: label.protocol,
        });
    }
    Ok(label)
}
