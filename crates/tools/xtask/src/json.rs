// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The JSON subset used by the QEMU machine protocol and Cargo artifacts.
//!
//! Objects, arrays, strings, integers, `true`, `false`, and `null`: that is
//! what a QMP greeting, a command, and an answer are made of. Numbers with
//! a fraction or an exponent are refused rather than rounded, because
//! nothing this tool sends or reads has one and a number that arrives as
//! something else is a surprise worth failing on.
//!
//! Invariants: parsing is bounded — nesting deeper than [`MAX_DEPTH`] is an
//! error and not a stack overflow; what the writer produces parses back to
//! the value it was given.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// How deep an object or array may nest.
pub(crate) const MAX_DEPTH: usize = 32;

/// A value of the subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A whole number.
    Int(i64),
    /// A string.
    Text(String),
    /// An array of values.
    Array(Vec<Value>),
    /// An object, its members in the order of their names.
    Object(BTreeMap<String, Value>),
}

/// Why a text is no value of this subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum JsonError {
    /// The text ended in the middle of a value.
    End,
    /// What stands at this byte belongs to no value.
    Unexpected(usize),
    /// A number that is not a whole number, or one that does not fit.
    Number(String),
    /// An escape sequence this subset does not read.
    Escape(usize),
    /// Nesting deeper than [`MAX_DEPTH`].
    Depth,
    /// Something stands after the value.
    Trailing(usize),
}

impl std::fmt::Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonError::End => f.write_str("the text ends in the middle of a value"),
            JsonError::Unexpected(at) => write!(f, "byte {at} begins no value"),
            JsonError::Number(text) => write!(f, "`{text}` is no whole number"),
            JsonError::Escape(at) => write!(f, "the escape at byte {at} is not one of this subset"),
            JsonError::Depth => write!(f, "nesting deeper than {MAX_DEPTH}"),
            JsonError::Trailing(at) => write!(f, "byte {at} stands after the value"),
        }
    }
}

impl Value {
    /// The member `name`, for an object that has one.
    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.get(name),
            _other => None,
        }
    }

    /// The string, for a value that is one.
    pub(crate) fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _other => None,
        }
    }

    /// An object out of these members.
    pub(crate) fn object(members: impl IntoIterator<Item = (String, Value)>) -> Value {
        Value::Object(members.into_iter().collect())
    }

    /// The value as one line of JSON.
    pub(crate) fn write(&self) -> String {
        let mut text = String::new();
        self.write_into(&mut text);
        text
    }

    /// Appends the value to `text`.
    fn write_into(&self, text: &mut String) {
        match self {
            Value::Null => text.push_str("null"),
            Value::Bool(true) => text.push_str("true"),
            Value::Bool(false) => text.push_str("false"),
            Value::Int(number) => {
                let _written = write!(text, "{number}");
            }
            Value::Text(value) => write_string(value, text),
            Value::Array(items) => {
                text.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    item.write_into(text);
                }
                text.push(']');
            }
            Value::Object(members) => {
                text.push('{');
                for (index, (name, value)) in members.iter().enumerate() {
                    if index > 0 {
                        text.push(',');
                    }
                    write_string(name, text);
                    text.push(':');
                    value.write_into(text);
                }
                text.push('}');
            }
        }
    }
}

/// Writes one string with the escapes this subset reads back.
fn write_string(value: &str, text: &mut String) {
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            '\n' => text.push_str("\\n"),
            '\r' => text.push_str("\\r"),
            '\t' => text.push_str("\\t"),
            other if u32::from(other) < 0x20 => {
                let _written = write!(text, "\\u{:04x}", u32::from(other));
            }
            other => text.push(other),
        }
    }
    text.push('"');
}

/// Reads one value out of `text`, which must hold nothing else.
///
/// # Errors
///
/// [`JsonError`] for a text that is no value of this subset.
pub(crate) fn parse(text: &str) -> Result<Value, JsonError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    let value = parser.value(0)?;
    parser.spaces();
    if parser.at < parser.bytes.len() {
        return Err(JsonError::Trailing(parser.at));
    }
    Ok(value)
}

/// Where the reader stands in the text.
struct Parser<'a> {
    /// The bytes of it.
    bytes: &'a [u8],
    /// The byte it reads next.
    at: usize,
}

impl Parser<'_> {
    /// The byte at the position, if there is one.
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    /// Steps over one byte.
    const fn step(&mut self) {
        self.at = self.at.saturating_add(1);
    }

    /// Steps over spaces, tabs, and line ends.
    fn spaces(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.step();
        }
    }

    /// Reads one value.
    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::Depth);
        }
        self.spaces();
        match self.peek().ok_or(JsonError::End)? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Value::Text),
            b't' => self.word("true", Value::Bool(true)),
            b'f' => self.word("false", Value::Bool(false)),
            b'n' => self.word("null", Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _other => Err(JsonError::Unexpected(self.at)),
        }
    }

    /// Reads a word that stands for one value.
    fn word(&mut self, wanted: &str, value: Value) -> Result<Value, JsonError> {
        let end = self.at.saturating_add(wanted.len());
        let found = self.bytes.get(self.at..end).ok_or(JsonError::End)?;
        if found != wanted.as_bytes() {
            return Err(JsonError::Unexpected(self.at));
        }
        self.at = end;
        Ok(value)
    }

    /// Reads a whole number.
    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.step();
        }
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.step();
        }
        let digits = self.bytes.get(start..self.at).unwrap_or_default();
        let text = String::from_utf8_lossy(digits).into_owned();
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(JsonError::Number(text));
        }
        text.parse::<i64>()
            .map(Value::Int)
            .map_err(|_| JsonError::Number(text))
    }

    /// Reads a string.
    fn string(&mut self) -> Result<String, JsonError> {
        self.step();
        let mut text = Vec::new();
        loop {
            let byte = self.peek().ok_or(JsonError::End)?;
            self.step();
            match byte {
                b'"' => return String::from_utf8(text).map_err(|_| JsonError::Unexpected(self.at)),
                b'\\' => {
                    let mut encoded = [0; 4];
                    text.extend_from_slice(self.escape()?.encode_utf8(&mut encoded).as_bytes());
                }
                other => text.push(other),
            }
        }
    }

    /// Reads what stands after a backslash.
    fn escape(&mut self) -> Result<char, JsonError> {
        let at = self.at;
        let byte = self.peek().ok_or(JsonError::End)?;
        self.step();
        match byte {
            b'"' => Ok('"'),
            b'\\' => Ok('\\'),
            b'/' => Ok('/'),
            b'b' => Ok('\u{8}'),
            b'f' => Ok('\u{c}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => self.unicode(at),
            _other => Err(JsonError::Escape(at)),
        }
    }

    /// Reads the four hexadecimal digits of a `\u` escape.
    fn unicode(&mut self, at: usize) -> Result<char, JsonError> {
        let end = self.at.saturating_add(4);
        let digits = self.bytes.get(self.at..end).ok_or(JsonError::End)?;
        let text = std::str::from_utf8(digits).map_err(|_| JsonError::Escape(at))?;
        let code = u32::from_str_radix(text, 16).map_err(|_| JsonError::Escape(at))?;
        self.at = end;
        char::from_u32(code).ok_or(JsonError::Escape(at))
    }

    /// Reads an array.
    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.step();
        let mut items = Vec::new();
        self.spaces();
        if self.peek() == Some(b']') {
            self.step();
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth.saturating_add(1))?);
            self.spaces();
            match self.peek().ok_or(JsonError::End)? {
                b',' => self.step(),
                b']' => {
                    self.step();
                    return Ok(Value::Array(items));
                }
                _other => return Err(JsonError::Unexpected(self.at)),
            }
        }
    }

    /// Reads an object.
    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.step();
        let mut members = BTreeMap::new();
        self.spaces();
        if self.peek() == Some(b'}') {
            self.step();
            return Ok(Value::Object(members));
        }
        loop {
            self.spaces();
            if self.peek() != Some(b'"') {
                return Err(JsonError::Unexpected(self.at));
            }
            let name = self.string()?;
            self.spaces();
            if self.peek() != Some(b':') {
                return Err(JsonError::Unexpected(self.at));
            }
            self.step();
            let value = self.value(depth.saturating_add(1))?;
            members.insert(name, value);
            self.spaces();
            match self.peek().ok_or(JsonError::End)? {
                b',' => self.step(),
                b'}' => {
                    self.step();
                    return Ok(Value::Object(members));
                }
                _other => return Err(JsonError::Unexpected(self.at)),
            }
        }
    }
}
