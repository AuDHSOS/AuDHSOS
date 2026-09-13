// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `format(F,...)`, which is also spelled `printf(F,...)`, and the
//! format language it reads.
//!
//! `sqlite3_str_vappendf` in `src/printf.c` is the document. The routine
//! serves both the C library's own formatting and this function, and the
//! two differ where the arguments are values rather than a `va_list`:
//! `%n` writes nowhere, `%z` is `%s`, `%c` takes the first character of
//! the text of its argument, and `%T` and `%S`, which name a token and a
//! term of a FROM clause the parser alone holds, end the conversion.
//!
//! A conversion character the table does not name ends the format as
//! well, and the answer is what was written before that character.
//!
//! Every conversion writes a bounded number of bytes per byte of the
//! format and per argument, so one call is O(n) in what it writes.

use alloc::vec::Vec;

use crate::eval::Error;
use crate::fp::{self, Shape, Style};
use crate::func::MAX_LENGTH;
use crate::value::Value;

/// The longest buffer a conversion takes for itself, which is
/// `mxAlloc` as `printfTempBuf` reads it.
fn limit() -> i64 {
    i64::try_from(MAX_LENGTH).unwrap_or(0)
}

/// The digits every radix conversion writes, in both cases.
const LOWER: &[u8] = b"0123456789abcdef";

/// The digits `%X` writes.
const UPPER: &[u8] = b"0123456789ABCDEF";

/// What `format(F,...)` answers: the text the format writes, or nothing
/// where the format itself is `NULL`.
///
/// # Errors
///
/// [`Error::TooBig`] where the text would pass what a value holds.
pub fn format(args: &[Value]) -> Result<Value, Error> {
    let Some(text) = args.first().and_then(Value::text) else {
        return Ok(Value::Null);
    };
    let mut arguments = Arguments {
        values: args.get(1..).unwrap_or_default(),
        used: 0,
    };
    let mut out = Out {
        bytes: Vec::new(),
        touched: false,
    };
    write(&mut out, cropped(&text), &mut arguments)?;
    if out.touched {
        Ok(Value::Text(out.bytes))
    } else {
        Ok(Value::Null)
    }
}

/// The bytes up to the first nought, which is what `strlen` counts of
/// what `sqlite3_value_text` answers.
fn cropped(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    bytes.get(..end).unwrap_or_default()
}

/// The low thirty-two bits of a value, read as signed, which is the
/// `(int)` cast a width and a precision come through.
const fn narrowed(value: i64) -> i32 {
    let [a, b, c, d, ..] = value.to_le_bytes();
    i32::from_le_bytes([a, b, c, d])
}

/// The values after the format, and how many conversions have taken.
/// Past the last one every conversion reads a nothing, which is what
/// `getIntArg` and the two beside it answer.
struct Arguments<'a> {
    /// The values.
    values: &'a [Value],
    /// How many are taken.
    used: usize,
}

impl Arguments<'_> {
    /// The next value, or nothing past the last one.
    fn next(&mut self) -> Option<&Value> {
        let value = self.values.get(self.used);
        if value.is_some() {
            self.used = self.used.saturating_add(1);
        }
        value
    }

    /// `sqlite3_value_int64` of the next value.
    fn integer(&mut self) -> i64 {
        self.next().map_or(0, Value::to_integer)
    }

    /// `sqlite3_value_double` of the next value.
    fn real(&mut self) -> f64 {
        self.next().map_or(0.0, Value::to_real)
    }

    /// `sqlite3_value_text` of the next value, cropped at the first
    /// nought, and nothing for a `NULL`.
    fn text(&mut self) -> Option<Vec<u8>> {
        self.next()
            .and_then(Value::text)
            .map(|bytes| cropped(&bytes).to_vec())
    }
}

/// The text being written.
struct Out {
    /// What is written so far.
    bytes: Vec<u8>,
    /// Whether any conversion has written, which a conversion of no
    /// bytes counts as. `sqlite3StrAccumFinish` answers a pointer to
    /// nothing where none has, and `sqlite3_result_text` reads that
    /// pointer as `NULL`.
    touched: bool,
}

impl Out {
    /// Room for `count` more bytes, or the refusal
    /// `sqlite3StrAccumEnlarge` answers a buffer past the limit with.
    /// The C library refuses a little sooner where its buffer doubles.
    const fn room(&self, count: usize) -> Result<(), Error> {
        if self.bytes.len().saturating_add(count) >= MAX_LENGTH {
            return Err(Error::TooBig);
        }
        Ok(())
    }

    /// One byte.
    fn push(&mut self, byte: u8) -> Result<(), Error> {
        self.room(1)?;
        self.touched = true;
        self.bytes.push(byte);
        Ok(())
    }

    /// A run of bytes.
    fn extend(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.room(bytes.len())?;
        self.touched = true;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// `count` copies of one byte.
    fn repeat(&mut self, byte: u8, count: usize) -> Result<(), Error> {
        self.room(count)?;
        self.touched = true;
        self.bytes.extend(core::iter::repeat_n(byte, count));
        Ok(())
    }
}

/// What one conversion carries: the flags before it, the field width and
/// the precision.
#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per flag of `sqlite3_str_vappendf`, which is what the reader of a format sets"
)]
struct Field {
    /// `-`: the field width goes after the bytes.
    left: bool,
    /// `+` or a space: what goes before a number at or above zero.
    sign: Option<u8>,
    /// `#`.
    alternate: bool,
    /// `!`: the width and the precision count characters.
    wide: bool,
    /// `0`: the field width is filled with zeros.
    zeros: bool,
    /// `,`: a comma between each three digits.
    thousands: bool,
    /// The field width, never below zero.
    width: i32,
    /// The precision, below zero where the format names none.
    precision: i32,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            left: false,
            sign: None,
            alternate: false,
            wide: false,
            zeros: false,
            thousands: false,
            width: 0,
            precision: -1,
        }
    }
}

/// What one conversion of an integer writes.
#[expect(
    clippy::struct_excessive_bools,
    reason = "one field per column of `fmtinfo`, which is what the table holds"
)]
struct Radix {
    /// The base the digits are written in.
    base: u64,
    /// `A` to `F` rather than `a` to `f`.
    upper: bool,
    /// The value is read as signed and may carry a minus.
    signed: bool,
    /// An English ending after the digits: `st`, `nd`, `rd` or `th`.
    ordinal: bool,
    /// What `#` writes before the digits.
    alternate: &'static [u8],
    /// Whether `,` counts here, which every base but ten refuses.
    thousands: bool,
}

/// `%d` and `%i`.
const DECIMAL: Radix = Radix {
    base: 10,
    upper: false,
    signed: true,
    ordinal: false,
    alternate: b"",
    thousands: true,
};

/// `%u`.
const UNSIGNED: Radix = Radix {
    signed: false,
    ..DECIMAL
};

/// `%r`.
const ORDINAL: Radix = Radix {
    ordinal: true,
    thousands: false,
    ..DECIMAL
};

/// `%x`.
const HEX: Radix = Radix {
    base: 16,
    upper: false,
    signed: false,
    ordinal: false,
    alternate: b"0x",
    thousands: false,
};

/// `%X`.
const HEX_UPPER: Radix = Radix {
    upper: true,
    alternate: b"0X",
    ..HEX
};

/// `%p`, which writes a value where the C library writes an address, in
/// upper case digits behind a lower case `0x`.
const POINTER: Radix = Radix { upper: true, ..HEX };

/// `%o`.
const OCTAL: Radix = Radix {
    base: 8,
    alternate: b"0",
    ..HEX
};

/// The format, one conversion at a time.
fn write(out: &mut Out, format: &[u8], args: &mut Arguments) -> Result<(), Error> {
    let mut at = 0;
    while at < format.len() {
        if format.get(at) != Some(&b'%') {
            let start = at;
            while at < format.len() && format.get(at) != Some(&b'%') {
                at = at.saturating_add(1);
            }
            out.extend(format.get(start..at).unwrap_or_default())?;
            continue;
        }
        at = at.saturating_add(1);
        if at >= format.len() {
            // A format that ends in `%` writes it.
            out.push(b'%')?;
            break;
        }
        let (field, conversion) = field(format, &mut at, args);
        match conversion {
            b'd' | b'i' => integer(out, &field, &DECIMAL, args)?,
            b'u' => integer(out, &field, &UNSIGNED, args)?,
            b'r' => integer(out, &field, &ORDINAL, args)?,
            b'x' => integer(out, &field, &HEX, args)?,
            b'p' => integer(out, &field, &POINTER, args)?,
            b'X' => integer(out, &field, &HEX_UPPER, args)?,
            b'o' => integer(out, &field, &OCTAL, args)?,
            b'f' => real(out, &field, Shape::Fixed, false, args)?,
            b'e' | b'E' => real(out, &field, Shape::Exponential, conversion == b'E', args)?,
            b'g' | b'G' => real(out, &field, Shape::Shortest, conversion == b'G', args)?,
            b'c' => character(out, &field, args)?,
            b's' | b'z' => string(out, &field, args)?,
            b'q' | b'Q' | b'w' => escape(out, &field, conversion, args)?,
            b'%' => widen(out, b"%", &field, None)?,
            // `%n` answers how much is written to a pointer the caller
            // hands in, which a value is not. It writes no byte and
            // still counts as a conversion that wrote.
            b'n' => out.extend(b"")?,
            // `%T` and `%S` name a token and a term of a FROM clause,
            // and every other character names no conversion at all.
            // Each ends the format where it stands.
            _ => return Ok(()),
        }
        at = at.saturating_add(1);
    }
    Ok(())
}

/// The flags, the field width and the precision before a conversion.
/// Answers the conversion character, and leaves `at` on it.
fn field(format: &[u8], at: &mut usize, args: &mut Arguments) -> (Field, u8) {
    let byte = |at: usize| format.get(at).copied().unwrap_or(0);
    let mut field = Field::default();
    let mut c = byte(*at);
    loop {
        match c {
            b'-' => field.left = true,
            b'+' => field.sign = Some(b'+'),
            b' ' => field.sign = Some(b' '),
            b'#' => field.alternate = true,
            b'!' => field.wide = true,
            b'0' => field.zeros = true,
            b',' => field.thousands = true,
            // `l` and `ll` say how wide the C argument is, which a value
            // does not answer to; either ends the flags.
            b'l' => {
                *at = at.saturating_add(1);
                c = byte(*at);
                if c == b'l' {
                    *at = at.saturating_add(1);
                    c = byte(*at);
                }
                break;
            }
            b'1'..=b'9' => {
                let mut width = u32::from(c.wrapping_sub(b'0'));
                loop {
                    *at = at.saturating_add(1);
                    c = byte(*at);
                    if !c.is_ascii_digit() {
                        break;
                    }
                    width = width
                        .wrapping_mul(10)
                        .wrapping_add(u32::from(c.wrapping_sub(b'0')));
                }
                field.width = i32::try_from(width & 0x7fff_ffff).unwrap_or(0);
                if c != b'.' && c != b'l' {
                    break;
                }
                continue;
            }
            b'*' => {
                let width = narrowed(args.integer());
                if width < 0 {
                    field.left = true;
                    field.width = width.checked_neg().unwrap_or(0);
                } else {
                    field.width = width;
                }
                *at = at.saturating_add(1);
                c = byte(*at);
                if c != b'.' && c != b'l' {
                    break;
                }
                continue;
            }
            b'.' => {
                *at = at.saturating_add(1);
                c = byte(*at);
                if c == b'*' {
                    let precision = narrowed(args.integer());
                    field.precision = if precision < 0 {
                        precision.checked_neg().unwrap_or(-1)
                    } else {
                        precision
                    };
                    *at = at.saturating_add(1);
                    c = byte(*at);
                } else {
                    let mut precision: u32 = 0;
                    while c.is_ascii_digit() {
                        precision = precision
                            .wrapping_mul(10)
                            .wrapping_add(u32::from(c.wrapping_sub(b'0')));
                        *at = at.saturating_add(1);
                        c = byte(*at);
                    }
                    field.precision = i32::try_from(precision & 0x7fff_ffff).unwrap_or(0);
                }
                if c == b'l' {
                    continue;
                }
                break;
            }
            _ => break,
        }
        *at = at.saturating_add(1);
        c = byte(*at);
        if c == 0 {
            break;
        }
    }
    (field, c)
}

/// `%d`, `%i`, `%u`, `%r`, `%x`, `%X`, `%o` and `%p`.
fn integer(out: &mut Out, field: &Field, radix: &Radix, args: &mut Arguments) -> Result<(), Error> {
    let taken = args.integer();
    let (mut value, prefix) = if radix.signed {
        if taken < 0 {
            (unsigned(taken).wrapping_neg(), Some(b'-'))
        } else {
            (unsigned(taken), field.sign)
        }
    } else {
        (unsigned(taken), None)
    };
    let alternate = field.alternate && value != 0;
    let mut precision = field.precision;
    if field.zeros {
        precision = precision.max(field.width.saturating_sub(i32::from(prefix.is_some())));
    }
    // `printfTempBuf` refuses a buffer past the limit before it takes
    // one, which is what refuses a precision wider than any answer.
    let mut wanted = i64::from(precision.max(0)).saturating_add(10);
    if field.thousands && radix.thousands {
        wanted = wanted.saturating_add(i64::from(precision.max(0)) / 3);
    }
    if wanted > limit() {
        return Err(Error::TooBig);
    }
    // The digits are written from the right, as `sqlite3_str_vappendf`
    // writes them, and turned around at the end.
    let mut digits = Vec::new();
    if radix.ordinal {
        digits.extend(ending(value).iter().rev());
    }
    let charset = if radix.upper { UPPER } else { LOWER };
    loop {
        let digit = usize::try_from(value.checked_rem(radix.base).unwrap_or(0)).unwrap_or(0);
        digits.push(charset.get(digit).copied().unwrap_or(b'0'));
        value = value.checked_div(radix.base).unwrap_or(0);
        if value == 0 {
            break;
        }
    }
    let count = i32::try_from(digits.len()).unwrap_or(0);
    if precision > count {
        digits.extend(core::iter::repeat_n(
            b'0',
            usize::try_from(precision.saturating_sub(count)).unwrap_or(0),
        ));
    }
    if field.thousands && radix.thousands {
        digits = separated(&digits);
    }
    if let Some(byte) = prefix {
        digits.push(byte);
    }
    if alternate {
        digits.extend(radix.alternate.iter().rev());
    }
    digits.reverse();
    widen(out, &digits, field, None)
}

/// The bits of a value read as unsigned, which is the `(u64)` cast a
/// radix conversion reads its argument through.
const fn unsigned(value: i64) -> u64 {
    u64::from_ne_bytes(value.to_ne_bytes())
}

/// The English ending `%r` writes, which is `zOrd`.
const fn ending(value: u64) -> &'static [u8] {
    if (value / 10) % 10 == 1 {
        return b"th";
    }
    match value % 10 {
        1 => b"st",
        2 => b"nd",
        3 => b"rd",
        _ => b"th",
    }
}

/// A comma between each three digits, over digits that are still turned
/// around, so the run counts from the right.
fn separated(digits: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (at, byte) in digits.iter().enumerate() {
        if at > 0 && at % 3 == 0 {
            out.push(b',');
        }
        out.push(*byte);
    }
    out
}

/// `%f`, `%e`, `%E`, `%g` and `%G`.
fn real(
    out: &mut Out,
    field: &Field,
    shape: Shape,
    upper: bool,
    args: &mut Arguments,
) -> Result<(), Error> {
    let value = args.real();
    let style = Style {
        shape,
        precision: field.precision,
        wide: field.wide,
        alternate: field.alternate,
        thousands: field.thousands,
        upper,
        sign: field.sign,
        zeros: field.zeros,
    };
    // `szBufNeeded` counts the field width as well, because the C
    // library renders the padded number into one buffer.
    let allowed = limit()
        .saturating_sub(1)
        .saturating_sub(i64::try_from(out.bytes.len()).unwrap_or(0))
        .saturating_sub(i64::from(field.width));
    let rendered = fp::render(value, &style, allowed).ok_or(Error::TooBig)?;
    let zeros = if field.zeros { rendered.zeros } else { None };
    widen(out, &rendered.bytes, field, zeros)
}

/// `%c`, which takes the first character of the text of its argument and
/// writes it `precision` times.
fn character(out: &mut Out, field: &Field, args: &mut Arguments) -> Result<(), Error> {
    let text = args.text().unwrap_or_default();
    let mut bytes = Vec::new();
    match text.first() {
        // A value with no text answers the nought `strlen` stops at.
        None => bytes.push(0),
        Some(first) => {
            bytes.push(*first);
            if first & 0xc0 == 0xc0 {
                for next in text.iter().skip(1) {
                    if bytes.len() >= 4 || next & 0xc0 != 0x80 {
                        break;
                    }
                    bytes.push(*next);
                }
            }
        }
    }
    let mut field = *field;
    if field.precision > 1 {
        field.width = field
            .width
            .saturating_sub(field.precision.saturating_sub(1));
        if field.width > 1 && !field.left {
            out.repeat(
                b' ',
                usize::try_from(field.width.saturating_sub(1)).unwrap_or(0),
            )?;
            field.width = 0;
        }
        // One copy is left for the field width to write.
        let copies = usize::try_from(field.precision).unwrap_or(0);
        out.room(copies.saturating_mul(bytes.len()))?;
        for _ in 1..copies {
            out.extend(&bytes)?;
        }
    }
    field.width = adjusted(field.width, &bytes);
    widen(out, &bytes, &field, None)
}

/// `%s` and `%z`.
fn string(out: &mut Out, field: &Field, args: &mut Arguments) -> Result<(), Error> {
    let text = args.text().unwrap_or_default();
    let bytes = if field.precision >= 0 {
        text.get(..bounded(&text, field)).unwrap_or_default()
    } else {
        text.as_slice()
    };
    let mut field = *field;
    if field.wide {
        field.width = adjusted(field.width, bytes);
    }
    widen(out, bytes, &field, None)
}

/// `%q`, `%Q` and `%w`.
fn escape(out: &mut Out, field: &Field, kind: u8, args: &mut Arguments) -> Result<(), Error> {
    let quote = if kind == b'w' { b'"' } else { b'\'' };
    let (source, mut quoted) = match args.text() {
        None if kind == b'Q' => (b"NULL".to_vec(), 0u8),
        None => (b"(NULL)".to_vec(), 0),
        Some(text) => (text, u8::from(kind == b'Q')),
    };
    let taken = if field.precision >= 0 {
        bounded(&source, field)
    } else {
        source.len()
    };
    let source = source.get(..taken).unwrap_or_default();
    // `%#q` writes the escapes `unistr` reads, and `%#Q` does so only
    // where the text holds a character that has no other spelling.
    let mut alternate = field.alternate && kind != b'w';
    if alternate {
        let controls = source.iter().filter(|byte| **byte <= 0x1f).count();
        if controls == 0 && kind != b'q' {
            alternate = false;
        } else if kind == b'Q' {
            quoted = 2;
        }
    }
    // What the escapes and the quotes add, which is the `n` the C
    // library takes a buffer of before it writes any of them.
    #[expect(
        clippy::naive_bytecount,
        reason = "the crate has no dependencies, and the count is over one value"
    )]
    let doubled = source.iter().filter(|byte| **byte == quote).count();
    let mut wanted = source.len().saturating_add(3).saturating_add(doubled);
    if alternate {
        for byte in source {
            wanted = wanted.saturating_add(if *byte == b'\\' {
                1
            } else if *byte <= 0x1f {
                5
            } else {
                0
            });
        }
        if quoted == 2 {
            wanted = wanted.saturating_add(10);
        }
    }
    out.room(wanted)?;
    let mut bytes = Vec::new();
    if quoted == 2 {
        bytes.extend_from_slice(b"unistr('");
    } else if quoted == 1 {
        bytes.push(quote);
    }
    for byte in source {
        if *byte == quote {
            bytes.push(*byte);
            bytes.push(*byte);
        } else if alternate && *byte == b'\\' {
            bytes.extend_from_slice(b"\\\\");
        } else if alternate && *byte <= 0x1f {
            bytes.extend_from_slice(b"\\u00");
            bytes.push(if *byte >= 0x10 { b'1' } else { b'0' });
            bytes.push(LOWER.get(usize::from(byte & 0x0f)).copied().unwrap_or(b'0'));
        } else {
            bytes.push(*byte);
        }
    }
    if quoted > 0 {
        bytes.push(quote);
        if quoted == 2 {
            bytes.push(b')');
        }
    }
    let mut field = *field;
    if field.wide {
        field.width = adjusted(field.width, &bytes);
    }
    widen(out, &bytes, &field, None)
}

/// How many bytes of the text the precision takes: characters where `!`
/// is given, bytes otherwise.
fn bounded(text: &[u8], field: &Field) -> usize {
    if !field.wide {
        return usize::try_from(field.precision)
            .unwrap_or(0)
            .min(text.len());
    }
    let mut at = 0;
    for _ in 0..field.precision {
        let Some(byte) = text.get(at).copied() else {
            break;
        };
        at = at.saturating_add(1);
        if byte >= 0xc0 {
            while text.get(at).is_some_and(|next| next & 0xc0 == 0x80) {
                at = at.saturating_add(1);
            }
        }
    }
    at
}

/// The field width counted in characters rather than bytes, which is
/// what `!` asks for: every byte that continues a character widens it.
fn adjusted(width: i32, bytes: &[u8]) -> i32 {
    if width <= 0 {
        return width;
    }
    let more = bytes.iter().filter(|byte| **byte & 0xc0 == 0x80).count();
    width.saturating_add(i32::try_from(more).unwrap_or(0))
}

/// The bytes, and the field width around them: spaces after where `-`
/// was given, zeros after the sign where `0` was, spaces before
/// otherwise.
fn widen(out: &mut Out, bytes: &[u8], field: &Field, zeros: Option<usize>) -> Result<(), Error> {
    let pad = usize::try_from(field.width)
        .unwrap_or(0)
        .saturating_sub(bytes.len());
    if pad == 0 {
        return out.extend(bytes);
    }
    if field.left {
        out.extend(bytes)?;
        return out.repeat(b' ', pad);
    }
    if let Some(at) = zeros {
        out.extend(bytes.get(..at).unwrap_or_default())?;
        out.repeat(b'0', pad)?;
        return out.extend(bytes.get(at..).unwrap_or_default());
    }
    out.repeat(b' ', pad)?;
    out.extend(bytes)
}
