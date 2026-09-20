// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The record format: what one row is once it is bytes.
//!
//! `docs/sqlite/fileformat2.html`, section 2.1, *Record Format*. A record
//! is a header of serial types and a body of the values they describe, and
//! the serial type is both the type and the length.

use alloc::vec::Vec;

use crate::bytes::{put_varint, signed, size, varint, varint_len};
use crate::error::Error;
use crate::value::{self, Affinity};

/// One value of a row. Text and blob borrow the bytes of the page they
/// were read from; text is in the database's encoding, which the header
/// names, because the record itself does not say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    /// `NULL`.
    Null,
    /// An integer, whatever width it was stored in.
    Int(i64),
    /// A double.
    Real(f64),
    /// Text, in the database's encoding.
    Text(&'a [u8]),
    /// A blob.
    Blob(&'a [u8]),
}

/// The type and the length of one value, as the header writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serial {
    /// Type 0.
    Null,
    /// Types 1, 2, 3, 4, 5 and 6: an integer of 1, 2, 3, 4, 6 or 8 bytes.
    Int(u8),
    /// Type 7.
    Real,
    /// Type 8: the value zero, stored in no bytes at all.
    Zero,
    /// Type 9: the value one, stored in no bytes at all.
    One,
    /// An even type of 12 or more: a blob of `(type - 12) / 2` bytes.
    Blob(usize),
    /// An odd type of 13 or more: text of `(type - 13) / 2` bytes.
    Text(usize),
}

impl Serial {
    /// The serial type a header code stands for.
    ///
    /// # Errors
    ///
    /// [`Error::SerialType`] for 10 and 11, which the format reserves.
    pub fn from_code(code: u64) -> Result<Self, Error> {
        match code {
            0 => Ok(Serial::Null),
            1 => Ok(Serial::Int(1)),
            2 => Ok(Serial::Int(2)),
            3 => Ok(Serial::Int(3)),
            4 => Ok(Serial::Int(4)),
            5 => Ok(Serial::Int(6)),
            6 => Ok(Serial::Int(8)),
            7 => Ok(Serial::Real),
            8 => Ok(Serial::Zero),
            9 => Ok(Serial::One),
            10 | 11 => Err(Error::SerialType(code)),
            other if other % 2 == 0 => Ok(Serial::Blob(size(other.saturating_sub(12) / 2))),
            other => Ok(Serial::Text(size(other.saturating_sub(13) / 2))),
        }
    }

    /// The header code this serial type is written as, which is the
    /// inverse of [`Serial::from_code`].
    #[must_use]
    pub fn code(self) -> u64 {
        match self {
            Serial::Null => 0,
            Serial::Int(bytes) => match bytes {
                1..=4 => u64::from(bytes),
                6 => 5,
                _ => 6,
            },
            Serial::Real => 7,
            Serial::Zero => 8,
            Serial::One => 9,
            Serial::Blob(len) => wide(len).saturating_mul(2).saturating_add(12),
            Serial::Text(len) => wide(len).saturating_mul(2).saturating_add(13),
        }
    }

    /// How many bytes of the body this value takes.
    #[must_use]
    pub fn len(self) -> usize {
        match self {
            Serial::Null | Serial::Zero | Serial::One => 0,
            Serial::Int(bytes) => usize::from(bytes),
            Serial::Real => 8,
            Serial::Blob(len) | Serial::Text(len) => len,
        }
    }

    /// Whether the value takes no bytes of the body.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The value this serial type describes, read from the start of
    /// `body`.
    ///
    /// # Errors
    ///
    /// [`Error::Overrun`] where the body ends before the value does.
    pub fn read(self, body: &[u8]) -> Result<Value<'_>, Error> {
        let bytes = body.get(..self.len()).ok_or(Error::Overrun)?;
        Ok(match self {
            Serial::Null => Value::Null,
            Serial::Zero => Value::Int(0),
            Serial::One => Value::Int(1),
            Serial::Int(_) => Value::Int(integer(bytes)),
            Serial::Real => Value::Real(f64::from_bits(double(bytes))),
            Serial::Blob(_) => Value::Blob(bytes),
            Serial::Text(_) => Value::Text(bytes),
        })
    }
}

/// A length as the number a header code is computed from.
fn wide(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// A big-endian two's complement integer of one to eight bytes.
fn integer(bytes: &[u8]) -> i64 {
    let negative = bytes.first().is_some_and(|byte| byte & 0x80 != 0);
    let mut wide = [if negative { 0xff } else { 0x00 }; 8];
    // The value is right-aligned in eight bytes, so a shorter one keeps
    // the sign byte it was padded with.
    let start = 8usize.saturating_sub(bytes.len());
    for (slot, byte) in wide.iter_mut().skip(start).zip(bytes) {
        *slot = *byte;
    }
    signed(u64::from_be_bytes(wide))
}

/// The bits of a double, which the format stores big-endian.
fn double(bytes: &[u8]) -> u64 {
    let mut wide = [0u8; 8];
    for (slot, byte) in wide.iter_mut().zip(bytes) {
        *slot = *byte;
    }
    u64::from_be_bytes(wide)
}

/// One row as the bytes of a record, which is `OP_MakeRecord`.
///
/// The affinity of each column is applied first, as the opcode applies
/// it, so a caller hands in the values an expression answered. `format`
/// is the schema format number: below four there are no serial types 8
/// and 9, so a zero and a one take a byte of the body each.
#[must_use]
pub fn write(values: &[value::Value], affinities: &[Affinity], format: u32) -> Vec<u8> {
    write_in(values, affinities, format, crate::header::Encoding::Utf8)
}

/// The same, written into the encoding the file names.
///
/// The affinity is applied to the text an expression answered, which is
/// UTF-8, before the text is written into that encoding, because
/// `sqlite3ApplyAffinity` reads the text as a number and the bytes of
/// UTF-16 are not the digits of one. Costs O(n) over the bytes of the
/// row.
#[must_use]
pub fn write_in(
    values: &[value::Value],
    affinities: &[Affinity],
    format: u32,
    encoding: crate::header::Encoding,
) -> Vec<u8> {
    let mut fields = Vec::with_capacity(values.len());
    let mut codes: usize = 0;
    let mut body: usize = 0;
    for (at, value) in values.iter().enumerate() {
        let mut value = value.clone();
        let affinity = affinities.get(at).copied().unwrap_or(Affinity::None);
        value::apply(&mut value, affinity);
        if let value::Value::Text(bytes) = &value {
            value = value::Value::Text(value::stored(bytes, encoding));
        }
        let (serial, value) = stored_as(value, affinity, format);
        codes = codes.saturating_add(varint_len(serial.code()));
        body = body.saturating_add(serial.len());
        fields.push((serial, value));
    }
    // The size varint counts itself, so a header of 126 code bytes or
    // fewer takes one more byte and a longer one may take one further
    // byte than the count without it did.
    let mut header = codes;
    if codes <= 126 {
        header = header.saturating_add(1);
    } else {
        let first = varint_len(wide(codes));
        header = header.saturating_add(first);
        if first < varint_len(wide(header)) {
            header = header.saturating_add(1);
        }
    }
    let mut out = Vec::with_capacity(header.saturating_add(body));
    put_varint(&mut out, wide(header));
    for (serial, _) in &fields {
        put_varint(&mut out, serial.code());
    }
    for (serial, value) in &fields {
        put_body(&mut out, *serial, value);
    }
    out
}

/// The serial type a value is stored under, and the value as that type
/// holds it: an integer in a column of real affinity that needs eight
/// bytes is stored as the double it stands for, which is `MEM_IntReal`
/// becoming `MEM_Real`.
fn stored_as(value: value::Value, affinity: Affinity, format: u32) -> (Serial, value::Value) {
    let serial = match &value {
        value::Value::Null => Serial::Null,
        value::Value::Real(_) => Serial::Real,
        value::Value::Text(bytes) => Serial::Text(bytes.len()),
        value::Value::Blob(bytes) => Serial::Blob(bytes.len()),
        value::Value::Int(number) => {
            let magnitude = if *number < 0 { !*number } else { *number };
            match magnitude {
                0..=127 if *number & 1 == *number && format >= 4 => {
                    return (
                        if *number == 0 {
                            Serial::Zero
                        } else {
                            Serial::One
                        },
                        value,
                    );
                }
                0..=127 => Serial::Int(1),
                128..=32_767 => Serial::Int(2),
                32_768..=8_388_607 => Serial::Int(3),
                8_388_608..=2_147_483_647 => Serial::Int(4),
                2_147_483_648..=140_737_488_355_327 => Serial::Int(6),
                _ if affinity == Affinity::Real => {
                    return (Serial::Real, value::Value::Real(value.to_real()));
                }
                _ => Serial::Int(8),
            }
        }
    };
    (serial, value)
}

/// The bytes one value takes in the body.
fn put_body(out: &mut Vec<u8>, serial: Serial, value: &value::Value) {
    match serial {
        Serial::Null | Serial::Zero | Serial::One => {}
        Serial::Int(bytes) => {
            let wide = value.to_integer().cast_unsigned().to_be_bytes();
            let start = 8usize.saturating_sub(usize::from(bytes));
            out.extend_from_slice(wide.get(start..).unwrap_or_default());
        }
        Serial::Real => out.extend_from_slice(&value.to_real().to_bits().to_be_bytes()),
        Serial::Blob(_) | Serial::Text(_) => {
            out.extend_from_slice(value.bytes().unwrap_or_default());
        }
    }
}

/// One record: the serial types of its header, and the body they describe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record<'a> {
    /// The serial type codes, still as bytes.
    header: &'a [u8],
    /// Everything after the header.
    body: &'a [u8],
}

impl<'a> Record<'a> {
    /// Splits a payload into its header and its body.
    ///
    /// # Errors
    ///
    /// [`Error::Varint`] for a header size that does not end inside the
    /// payload, and [`Error::Overrun`] where the header it names is longer
    /// than the payload.
    pub fn parse(payload: &'a [u8]) -> Result<Self, Error> {
        let (declared, read) = varint(payload)?;
        let header_len = size(declared);
        if header_len < read {
            return Err(Error::Overrun);
        }
        let header = payload.get(read..header_len).ok_or(Error::Overrun)?;
        // The header lies inside the payload, so what follows it does too.
        let body = payload.get(header_len..).unwrap_or_default();
        Ok(Record { header, body })
    }

    /// The values of the record, in order.
    #[must_use]
    pub const fn values(&self) -> Values<'a> {
        Values {
            header: self.header,
            body: self.body,
        }
    }

    /// The value at `index`, or nothing where the record is shorter.
    ///
    /// # Errors
    ///
    /// The errors of reading the values before it.
    pub fn value(&self, index: usize) -> Result<Option<Value<'a>>, Error> {
        for (at, value) in self.values().enumerate() {
            let value = value?;
            if at == index {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }
}

/// Where in `payload` the value of the column at `index` begins, how
/// many bytes it holds, and the serial type it carries.
///
/// `sqlite3BtreePayloadSize` and the walk of `sqlite3VdbeMemFromBtree`
/// read a value out of a record this way, which is what
/// `sqlite3_blob_open` of `research/sqlite/src/vdbeblob.c:74` reads to
/// know where the bytes of one column lie. Nothing where the record
/// holds fewer columns. The walk is O(n) in the columns before `index`.
///
/// # Errors
///
/// [`Error::Varint`] for a header that does not end inside the payload,
/// [`Error::Overrun`] where the header it names is longer than the
/// payload, and [`Error::SerialType`] for a code no type answers.
pub fn placed(payload: &[u8], index: usize) -> Result<Option<(usize, usize, Serial)>, Error> {
    let (declared, read) = varint(payload)?;
    let header = size(declared);
    if header < read || header > payload.len() {
        return Err(Error::Overrun);
    }
    let mut at = read;
    let mut body = header;
    let mut column = 0;
    while at < header {
        let (code, used) = varint(payload.get(at..).ok_or(Error::Overrun)?)?;
        let serial = Serial::from_code(code)?;
        if column == index {
            return Ok(Some((body, serial.len(), serial)));
        }
        body = body.saturating_add(serial.len());
        at = at.saturating_add(used);
        column = column.saturating_add(1);
    }
    Ok(None)
}

/// The values of a record, read as the iterator walks its header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Values<'a> {
    /// What is left of the header.
    header: &'a [u8],
    /// What is left of the body.
    body: &'a [u8],
}

impl<'a> Iterator for Values<'a> {
    type Item = Result<Value<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.header.is_empty() {
            return None;
        }
        let (code, read) = match varint(self.header) {
            Ok(pair) => pair,
            Err(error) => return Some(Err(self.stop(error))),
        };
        let serial = match Serial::from_code(code) {
            Ok(serial) => serial,
            Err(error) => return Some(Err(self.stop(error))),
        };
        let value = match serial.read(self.body) {
            Ok(value) => value,
            Err(error) => return Some(Err(self.stop(error))),
        };
        self.header = self.header.get(read..).unwrap_or_default();
        self.body = self.body.get(serial.len()..).unwrap_or_default();
        Some(Ok(value))
    }
}

impl Values<'_> {
    /// Ends the walk, so that an error is answered once and not forever.
    const fn stop(&mut self, error: Error) -> Error {
        self.header = &[];
        self.body = &[];
        error
    }
}
