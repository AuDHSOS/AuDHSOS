// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The message of RFC 1035, section 4: a header of six fields and four
//! sections behind it.
//!
//! A parsed message is a borrowed view over the bytes it arrived in,
//! because a name anywhere in it may point at a name anywhere before it
//! and the whole is therefore the unit a section is read against. The
//! header and the question section are read at once, so that a response
//! can be matched against the question that went out before anything else
//! is looked at; the answer section is walked by an iterator, which is
//! what lets a record this crate has no use for cost nothing but the walk
//! past it.
//!
//! Nothing is compressed on write. A query carries one name, so there is
//! nothing to point at; and a message this crate writes is read by a
//! server that must accept the uncompressed form anyway.

use core::fmt;

use net_wire::{WireError, Writer};

use crate::error::DnsError;
use crate::name::{MAX_NAME_LEN, Name};
use crate::record::{Class, Record, RecordType};

/// The six fields of the header: the id, the flags, and the four counts.
pub const HEADER_LEN: usize = 12;

/// The longest message this crate reads or writes over UDP without the
/// extension of RFC 6891, which it does not have (RFC 1035, section 2.3.4).
pub const MAX_MESSAGE_LEN: usize = 512;

/// The longest query: a header, the longest name, and the two fields
/// behind it.
pub const MAX_QUERY_LEN: usize = HEADER_LEN + MAX_NAME_LEN + 4;

/// The response is a response and not a query.
const FLAG_RESPONSE: u16 = 0x8000;

/// The opcode sits in bits 3 to 6 of the first byte of the field.
const OPCODE_SHIFT: u32 = 3;

/// The opcode, once shifted down.
const OPCODE_MASK: u8 = 0x0F;

/// The answer comes from a server with authority for the name.
const FLAG_AUTHORITATIVE: u16 = 0x0400;

/// The answer did not fit and was cut short.
const FLAG_TRUNCATED: u16 = 0x0200;

/// The asker would like the server to recurse.
const FLAG_RECURSION_DESIRED: u16 = 0x0100;

/// The server does recurse.
const FLAG_RECURSION_AVAILABLE: u16 = 0x0080;

/// The response code, in the low four bits of the second byte.
const RCODE_MASK: u8 = 0x0F;

/// What the query was for. Only a standard query is ever written here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Opcode(u8);

impl Opcode {
    /// A standard query (RFC 1035, section 4.1.1).
    pub const QUERY: Opcode = Opcode(0);

    /// The opcode `value` names.
    #[must_use]
    pub const fn new(value: u8) -> Opcode {
        Opcode(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// How the server answered (RFC 1035, section 4.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResponseCode(u8);

impl ResponseCode {
    /// The question was answered.
    pub const NO_ERROR: ResponseCode = ResponseCode(0);
    /// The server could not read the query.
    pub const FORMAT_ERROR: ResponseCode = ResponseCode(1);
    /// The server broke down.
    pub const SERVER_FAILURE: ResponseCode = ResponseCode(2);
    /// The name does not exist.
    pub const NAME_ERROR: ResponseCode = ResponseCode(3);
    /// The server does not do this kind of query.
    pub const NOT_IMPLEMENTED: ResponseCode = ResponseCode(4);
    /// The server will not answer this asker.
    pub const REFUSED: ResponseCode = ResponseCode(5);

    /// The code `value` names.
    #[must_use]
    pub const fn new(value: u8) -> ResponseCode {
        ResponseCode(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl fmt::Display for ResponseCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ResponseCode::NO_ERROR => f.write_str("no error"),
            ResponseCode::FORMAT_ERROR => f.write_str("format error"),
            ResponseCode::SERVER_FAILURE => f.write_str("server failure"),
            ResponseCode::NAME_ERROR => f.write_str("no such name"),
            ResponseCode::NOT_IMPLEMENTED => f.write_str("not implemented"),
            ResponseCode::REFUSED => f.write_str("refused"),
            ResponseCode(other) => write!(f, "response code {other}"),
        }
    }
}

/// The second field of the header, which holds seven things.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Flags(u16);

impl Flags {
    /// The flags of a query that would like the server to recurse.
    pub const QUERY: Flags = Flags(FLAG_RECURSION_DESIRED);

    /// The flags `value` holds.
    #[must_use]
    pub const fn new(value: u16) -> Flags {
        Flags(value)
    }

    /// The field as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether this is a response.
    #[must_use]
    pub const fn is_response(self) -> bool {
        self.0 & FLAG_RESPONSE != 0
    }

    /// What the query was for.
    #[must_use]
    pub const fn opcode(self) -> Opcode {
        let [high, _] = self.0.to_be_bytes();
        Opcode(high >> OPCODE_SHIFT & OPCODE_MASK)
    }

    /// Whether the server has authority for the name.
    #[must_use]
    pub const fn is_authoritative(self) -> bool {
        self.0 & FLAG_AUTHORITATIVE != 0
    }

    /// Whether the answer was cut short.
    #[must_use]
    pub const fn is_truncated(self) -> bool {
        self.0 & FLAG_TRUNCATED != 0
    }

    /// Whether the asker would like the server to recurse.
    #[must_use]
    pub const fn recursion_desired(self) -> bool {
        self.0 & FLAG_RECURSION_DESIRED != 0
    }

    /// Whether the server does recurse.
    #[must_use]
    pub const fn recursion_available(self) -> bool {
        self.0 & FLAG_RECURSION_AVAILABLE != 0
    }

    /// How the server answered.
    #[must_use]
    pub const fn response_code(self) -> ResponseCode {
        let [_, low] = self.0.to_be_bytes();
        ResponseCode(low & RCODE_MASK)
    }

    /// The same flags, marked as a response.
    #[must_use]
    pub const fn as_response(self) -> Flags {
        Flags(self.0 | FLAG_RESPONSE)
    }

    /// The same flags, marked as cut short.
    #[must_use]
    pub const fn truncated(self) -> Flags {
        Flags(self.0 | FLAG_TRUNCATED)
    }

    /// The same flags, with `code` as the answer.
    #[must_use]
    pub const fn with_response_code(self, code: ResponseCode) -> Flags {
        let [high, low] = self.0.to_be_bytes();
        Flags(u16::from_be_bytes([high, low & !RCODE_MASK | code.get()]))
    }
}

/// The header of a message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Header {
    /// What ties a response to the query it answers.
    pub id: u16,
    /// The seven things of the second field.
    pub flags: Flags,
    /// How many questions the message carries.
    pub questions: u16,
    /// How many answers.
    pub answers: u16,
    /// How many authority records.
    pub authorities: u16,
    /// How many additional records.
    pub additionals: u16,
}

impl Header {
    /// The header in the first twelve bytes of `bytes`.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when there are not twelve.
    pub const fn parse(bytes: &[u8]) -> Result<Header, DnsError> {
        let Some(fixed) = bytes.first_chunk::<HEADER_LEN>() else {
            return Err(DnsError::Wire(WireError::OutOfBounds {
                needed: HEADER_LEN,
                available: bytes.len(),
            }));
        };
        let [
            id_high,
            id_low,
            flags_high,
            flags_low,
            questions_high,
            questions_low,
            answers_high,
            answers_low,
            authorities_high,
            authorities_low,
            additionals_high,
            additionals_low,
        ] = *fixed;
        Ok(Header {
            id: u16::from_be_bytes([id_high, id_low]),
            flags: Flags(u16::from_be_bytes([flags_high, flags_low])),
            questions: u16::from_be_bytes([questions_high, questions_low]),
            answers: u16::from_be_bytes([answers_high, answers_low]),
            authorities: u16::from_be_bytes([authorities_high, authorities_low]),
            additionals: u16::from_be_bytes([additionals_high, additionals_low]),
        })
    }

    /// Writes the header.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when the buffer has no room.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), DnsError> {
        writer.write_u16(self.id)?;
        writer.write_u16(self.flags.0)?;
        writer.write_u16(self.questions)?;
        writer.write_u16(self.answers)?;
        writer.write_u16(self.authorities)?;
        writer.write_u16(self.additionals)?;
        Ok(())
    }
}

/// One question: a name, a type, and a class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Question {
    /// What is asked about.
    pub name: Name,
    /// Which kind of record is wanted.
    pub record_type: RecordType,
    /// Which class.
    pub class: Class,
}

/// The two fields behind the name of a question.
const QUESTION_FIXED_LEN: usize = 4;

impl Question {
    /// The question about `name` of type `record_type`, in class `IN`.
    #[must_use]
    pub const fn new(name: Name, record_type: RecordType) -> Question {
        Question {
            name,
            record_type,
            class: Class::IN,
        }
    }

    /// The question that begins at `at` in `message`, and the offset just
    /// past it.
    ///
    /// # Errors
    ///
    /// Whatever [`Name::read`] returns, and [`DnsError::Wire`] when the
    /// two fields behind the name are not there.
    pub fn read(message: &[u8], at: usize) -> Result<(Question, usize), DnsError> {
        let (name, after_name) = Name::read(message, at)?;
        let end = after_name.saturating_add(QUESTION_FIXED_LEN);
        let fixed = message
            .get(after_name..)
            .and_then(<[u8]>::first_chunk::<QUESTION_FIXED_LEN>)
            .ok_or(DnsError::Wire(WireError::OutOfBounds {
                needed: end,
                available: message.len(),
            }))?;
        let [type_high, type_low, class_high, class_low] = *fixed;
        Ok((
            Question {
                name,
                record_type: RecordType::new(u16::from_be_bytes([type_high, type_low])),
                class: Class::new(u16::from_be_bytes([class_high, class_low])),
            },
            end,
        ))
    }

    /// Writes the question, with the name uncompressed.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when the buffer has no room.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), DnsError> {
        self.name.write(writer)?;
        writer.write_u16(self.record_type.get())?;
        writer.write_u16(self.class.get())?;
        Ok(())
    }
}

/// One message, borrowed from the bytes it arrived in.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    /// The six fields at the front.
    pub header: Header,
    /// The whole message, which a compression pointer is read against.
    bytes: &'a [u8],
    /// Where the answer section begins.
    answers_at: usize,
}

impl<'a> Message<'a> {
    /// The message in `bytes`.
    ///
    /// The header and the question section are read here, because a
    /// response is matched against the question that went out before
    /// anything else about it is believed. The three record sections are
    /// left to the iterators.
    ///
    /// # Errors
    ///
    /// [`DnsError::Wire`] when there is not even a header, and whatever
    /// [`Question::read`] returns for a question section that is not one.
    pub fn parse(bytes: &'a [u8]) -> Result<Message<'a>, DnsError> {
        let header = Header::parse(bytes)?;
        let mut at = HEADER_LEN;
        for _ in 0..header.questions {
            let (_, after) = Question::read(bytes, at)?;
            at = after;
        }
        Ok(Message {
            header,
            bytes,
            answers_at: at,
        })
    }

    /// The questions.
    #[must_use]
    pub const fn questions(&self) -> Questions<'a> {
        Questions {
            bytes: self.bytes,
            at: HEADER_LEN,
            left: self.header.questions,
        }
    }

    /// The first question, which is the only one a query of this crate
    /// carries.
    ///
    /// # Errors
    ///
    /// Whatever [`Question::read`] returns.
    pub fn question(&self) -> Result<Option<Question>, DnsError> {
        self.questions().next().transpose()
    }

    /// The answers.
    #[must_use]
    pub const fn answers(&self) -> Records<'a> {
        Records {
            bytes: self.bytes,
            at: self.answers_at,
            left: self.header.answers,
        }
    }
}

/// The questions of a message.
#[derive(Clone, Debug)]
pub struct Questions<'a> {
    /// The whole message.
    bytes: &'a [u8],
    /// Where the next one begins.
    at: usize,
    /// How many are left.
    left: u16,
}

impl Iterator for Questions<'_> {
    type Item = Result<Question, DnsError>;

    fn next(&mut self) -> Option<Result<Question, DnsError>> {
        self.left = self.left.checked_sub(1)?;
        match Question::read(self.bytes, self.at) {
            Ok((question, after)) => {
                self.at = after;
                Some(Ok(question))
            }
            Err(error) => {
                // A section that cannot be walked cannot be walked past
                // either, so this is the last thing the iterator says.
                self.left = 0;
                Some(Err(error))
            }
        }
    }
}

/// The records of one section.
#[derive(Clone, Debug)]
pub struct Records<'a> {
    /// The whole message.
    bytes: &'a [u8],
    /// Where the next one begins.
    at: usize,
    /// How many are left.
    left: u16,
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<Record<'a>, DnsError>;

    fn next(&mut self) -> Option<Result<Record<'a>, DnsError>> {
        self.left = self.left.checked_sub(1)?;
        match Record::read(self.bytes, self.at) {
            Ok((record, after)) => {
                self.at = after;
                Some(Ok(record))
            }
            Err(error) => {
                self.left = 0;
                Some(Err(error))
            }
        }
    }
}

/// Writes a query for `name` of type `record_type`, with recursion
/// desired.
///
/// # Errors
///
/// [`DnsError::Wire`] when the buffer has no room.
pub fn write_query(
    writer: &mut Writer<'_>,
    id: u16,
    name: &Name,
    record_type: RecordType,
) -> Result<(), DnsError> {
    let header = Header {
        id,
        flags: Flags::QUERY,
        questions: 1,
        answers: 0,
        authorities: 0,
        additionals: 0,
    };
    header.write(writer)?;
    Question::new(*name, record_type).write(writer)
}
