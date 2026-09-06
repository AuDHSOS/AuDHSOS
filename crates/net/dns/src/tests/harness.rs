// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Building messages for the tests to read.
//!
//! A server is what this crate never is, so the writers here are the tests'
//! own: they put a header, a question and a list of records together, and
//! they can put them together wrongly, which is the greater part of what
//! there is to check.

use net_wire::{Ipv4Addr, Ipv6Addr, Writer};

use crate::message::{Flags, Header, Question, ResponseCode};
use crate::name::Name;
use crate::record::{Class, Record, RecordData, RecordType};

/// The name `text` stands for.
pub(crate) fn name(text: &str) -> Name {
    Name::from_ascii(text).expect("a name")
}

/// An `A` record for `owner`.
pub(crate) fn a(owner: &str, address: Ipv4Addr) -> Record<'static> {
    Record {
        name: name(owner),
        record_type: RecordType::A,
        class: Class::IN,
        ttl: 300,
        data: RecordData::A(address),
    }
}

/// An `AAAA` record for `owner`.
pub(crate) fn aaaa(owner: &str, address: Ipv6Addr) -> Record<'static> {
    Record {
        name: name(owner),
        record_type: RecordType::AAAA,
        class: Class::IN,
        ttl: 300,
        data: RecordData::Aaaa(address),
    }
}

/// A `CNAME` record that points `owner` at `target`.
pub(crate) fn cname(owner: &str, target: &str) -> Record<'static> {
    Record {
        name: name(owner),
        record_type: RecordType::CNAME,
        class: Class::IN,
        ttl: 300,
        data: RecordData::Cname(name(target)),
    }
}

/// A response to a query, with a question section and a list of answers.
pub(crate) struct Response {
    /// The transaction id it answers.
    pub(crate) id: u16,
    /// What was asked.
    pub(crate) question: Question,
    /// How the server answered.
    pub(crate) code: ResponseCode,
    /// Whether the answer was cut short.
    pub(crate) truncated: bool,
    /// What it carries.
    pub(crate) answers: Vec<Record<'static>>,
}

impl Response {
    /// A response to `id` about `owner` of type `record_type`.
    pub(crate) fn new(id: u16, owner: &str, record_type: RecordType) -> Response {
        Response {
            id,
            question: Question::new(name(owner), record_type),
            code: ResponseCode::NO_ERROR,
            truncated: false,
            answers: Vec::new(),
        }
    }

    /// The same response, carrying `record`.
    pub(crate) fn with(mut self, record: &Record<'static>) -> Response {
        self.answers.push(*record);
        self
    }

    /// The same response, with `code` as the answer.
    pub(crate) fn coded(mut self, code: ResponseCode) -> Response {
        self.code = code;
        self
    }

    /// The same response, marked as cut short.
    pub(crate) fn cut_short(mut self) -> Response {
        self.truncated = true;
        self
    }

    /// The bytes of it.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut buffer = [0u8; 1024];
        let mut writer = Writer::new(&mut buffer);
        let mut flags = Flags::QUERY.as_response().with_response_code(self.code);
        if self.truncated {
            flags = flags.truncated();
        }
        let header = Header {
            id: self.id,
            flags,
            questions: 1,
            answers: u16::try_from(self.answers.len()).expect("few answers"),
            authorities: 0,
            additionals: 0,
        };
        header.write(&mut writer).expect("room");
        self.question.write(&mut writer).expect("room");
        for record in &self.answers {
            record.write(&mut writer).expect("room");
        }
        writer.finish().to_vec()
    }
}
