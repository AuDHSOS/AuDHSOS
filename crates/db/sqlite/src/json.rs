// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! JSON as `src/json.c` holds it: a value is read out of text into the
//! binary form of `doc/jsonb.md`, and written back out as text.
//!
//! One element is a header of one to nine bytes and a payload. The low
//! four bits of the first byte say what the element is, and the high
//! four say how long the payload is: 0 to 11 is the length itself, and
//! 12 to 15 say how many bytes after the first hold the length as a
//! big-endian number.

use alloc::vec::Vec;

use crate::value::Value;

/// `null`.
pub const NULL: u8 = 0;
/// `true`.
pub const TRUE: u8 = 1;
/// `false`.
pub const FALSE: u8 = 2;
/// A whole number both JSON and SQL read.
pub const INT: u8 = 3;
/// A whole number written the way JSON5 allows, which is `0x` and hex
/// digits or a leading `+`.
pub const INT5: u8 = 4;
/// A number with a fraction or an exponent that both JSON and SQL read.
pub const FLOAT: u8 = 5;
/// A number written the way JSON5 allows.
pub const FLOAT5: u8 = 6;
/// Text with no escape in it, which both JSON and SQL read.
pub const TEXT: u8 = 7;
/// Text with the escapes JSON allows.
pub const TEXTJ: u8 = 8;
/// Text with the escapes JSON5 allows.
pub const TEXT5: u8 = 9;
/// Text as SQL holds it, which is escaped on the way out.
pub const TEXTRAW: u8 = 10;
/// An array: the payload is the elements one after another.
pub const ARRAY: u8 = 11;
/// An object: the payload is label and value one pair after another.
pub const OBJECT: u8 = 12;

/// How deep one value may nest, which is `JSON_MAX_DEPTH`.
const DEPTH: usize = 1000;

/// The type of the element at `at`, where the payload begins and how
/// long it is. Answers nothing where the bytes hold no whole element.
#[must_use]
pub fn element(blob: &[u8], at: usize) -> Option<(u8, usize, usize)> {
    let first = *blob.get(at)?;
    let kind = first & 0x0f;
    if kind > OBJECT {
        return None;
    }
    let wide = usize::from(first >> 4);
    // The width is four bits, so it is 0 to 15: up to 11 it is the
    // length itself and past that it says how many bytes hold the
    // length.
    let (start, length) = if wide <= 11 {
        (at.saturating_add(1), wide)
    } else {
        {
            // 12 counts one byte, 13 two, 14 four and 15 eight.
            let bytes = 1_usize << wide.saturating_sub(12);
            let head = at.saturating_add(1);
            let end = head.saturating_add(bytes);
            let mut length = 0_usize;
            for byte in blob.get(head..end)? {
                length = length.checked_mul(256)?.checked_add(usize::from(*byte))?;
            }
            (end, length)
        }
    };
    if start.checked_add(length)? > blob.len() {
        return None;
    }
    Some((kind, start, length))
}

/// Where the element at `at` ends, which is the first byte after its
/// payload.
#[must_use]
pub fn after(blob: &[u8], at: usize) -> Option<usize> {
    let (_, start, length) = element(blob, at)?;
    Some(start.saturating_add(length))
}

/// Writes one element of `kind` holding `payload`.
pub fn append(blob: &mut Vec<u8>, kind: u8, payload: &[u8]) {
    let length = payload.len();
    if length <= 11 {
        let wide = u8::try_from(length).unwrap_or(0);
        blob.push(kind | (wide << 4));
    } else if length <= 0xff {
        blob.push(kind | 0xc0);
        blob.push(u8::try_from(length).unwrap_or(0xff));
    } else if length <= 0xffff {
        blob.push(kind | 0xd0);
        let wide = u16::try_from(length).unwrap_or(u16::MAX);
        blob.extend_from_slice(&wide.to_be_bytes());
    } else {
        blob.push(kind | 0xe0);
        let wide = u32::try_from(length).unwrap_or(u32::MAX);
        blob.extend_from_slice(&wide.to_be_bytes());
    }
    blob.extend_from_slice(payload);
}

/// What reading one element of text answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// The element was read and ends before this place.
    Ended(usize),
    /// The text holds no element from this place on.
    Done,
    /// The byte at this place begins nothing an element may hold.
    Wrong(usize),
    /// One of `}`, `]`, `,` and `:` stood at this place.
    Mark(u8, usize),
}

/// A value being read out of text.
struct Reading<'a> {
    /// The text itself.
    text: &'a [u8],
    /// Whether it held anything JSON5 allows and JSON does not.
    nonstandard: bool,
    /// How deep the reading stands.
    depth: usize,
}

/// The value the text holds, with whether the text held anything JSON5
/// allows and JSON does not, or where the text stopped being JSON.
///
/// Reading `n` bytes costs O(n).
///
/// # Errors
///
/// The place of the byte the text stopped at, counting from nought.
pub fn read(text: &[u8]) -> Result<(Vec<u8>, bool), usize> {
    let mut reading = Reading {
        text,
        nonstandard: false,
        depth: 0,
    };
    let mut blob = Vec::new();
    let at = skipped(text, 0);
    match reading.value(at, &mut blob) {
        Step::Ended(end) => {
            // `jsonConvertTextToBlob` reads the whitespace of JSON
            // after the value, and the whitespace JSON5 adds after
            // that, which marks the text as one JSON alone does not
            // hold.
            let rest = skipped(text, end);
            if rest < text.len() {
                let after = spaced(text, rest);
                if after < text.len() {
                    return Err(after);
                }
                reading.nonstandard = true;
            }
            Ok((blob, reading.nonstandard))
        }
        Step::Done => Err(text.len()),
        Step::Wrong(at) | Step::Mark(_, at) => Err(at),
    }
}

/// The whitespace of JSON alone, which is tab, newline, return and
/// space.
fn skipped(text: &[u8], at: usize) -> usize {
    let mut at = at;
    while matches!(text.get(at), Some(0x09 | 0x0a | 0x0d | 0x20)) {
        at = at.saturating_add(1);
    }
    at
}

/// The whitespace JSON5 allows, which is `json5Whitespace`: the four
/// JSON spaces, the vertical tab, the form feed, a comment of either
/// shape, and the seven spaces Unicode names.
fn spaced(text: &[u8], at: usize) -> usize {
    let mut at = at;
    loop {
        let byte = match text.get(at) {
            Some(byte) => *byte,
            None => return at,
        };
        let step = match byte {
            0x09 | 0x0a | 0x0b | 0x0c | 0x0d | 0x20 => 1,
            b'/' => match commented(text, at) {
                Some(step) => step,
                None => return at,
            },
            0xc2 if text.get(at.saturating_add(1)) == Some(&0xa0) => 2,
            0xe1 => match text.get(at.saturating_add(1)..at.saturating_add(3)) {
                Some([0x9a, 0x80]) => 3,
                _ => return at,
            },
            0xe2 => match text.get(at.saturating_add(1)..at.saturating_add(3)) {
                Some([0x80, third])
                    if (0x80..=0x8a).contains(third) || matches!(third, 0xa8 | 0xa9 | 0xaf) =>
                {
                    3
                }
                Some([0x81, 0x9f]) => 3,
                _ => return at,
            },
            0xe3 => match text.get(at.saturating_add(1)..at.saturating_add(3)) {
                Some([0x80, 0x80]) => 3,
                _ => return at,
            },
            0xef => match text.get(at.saturating_add(1)..at.saturating_add(3)) {
                Some([0xbb, 0xbf]) => 3,
                _ => return at,
            },
            _ => return at,
        };
        at = at.saturating_add(step);
    }
}

/// How many bytes the comment at `at` takes, and nothing where `/`
/// begins no comment. A comment `/*` that no `*/` ends runs to the end
/// of the text, which is what `json5Whitespace` reads it as.
fn commented(text: &[u8], at: usize) -> Option<usize> {
    let second = *text.get(at.saturating_add(1))?;
    if second == b'*' {
        text.get(at.saturating_add(2))?;
        let mut place = at.saturating_add(3);
        while text.get(place).is_some() {
            if text.get(place) == Some(&b'/') && text.get(place.saturating_sub(1)) == Some(&b'*') {
                return Some(place.saturating_add(1).saturating_sub(at));
            }
            place = place.saturating_add(1);
        }
        return Some(text.len().saturating_sub(at));
    }
    if second != b'/' {
        return None;
    }
    let mut place = at.saturating_add(2);
    while let Some(byte) = text.get(place) {
        if *byte == b'\n' || *byte == b'\r' {
            break;
        }
        if *byte == 0xe2
            && text.get(place.saturating_add(1)) == Some(&0x80)
            && matches!(text.get(place.saturating_add(2)), Some(0xa8 | 0xa9))
        {
            place = place.saturating_add(2);
            break;
        }
        place = place.saturating_add(1);
    }
    if text.get(place).is_some() {
        place = place.saturating_add(1);
    }
    Some(place.saturating_sub(at))
}

/// Whether a byte may begin the name JSON5 allows for a label, which
/// is `sqlite3JsonId1`: a letter, `$`, `_`, and every byte past ASCII.
const fn begins_name(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'$' || byte == b'_' || byte >= 0x80
}

/// Whether a byte may stand later in such a name, which is
/// `sqlite3JsonId2`: whatever begins one, and a digit.
const fn holds_name(byte: u8) -> bool {
    begins_name(byte) || byte.is_ascii_digit()
}

/// Whether the `count` bytes at `at` are all hex digits.
fn hex_at(text: &[u8], at: usize, count: usize) -> bool {
    text.get(at..at.saturating_add(count))
        .is_some_and(|bytes| bytes.iter().all(u8::is_ascii_hexdigit))
}

/// Whether the byte at `at` is a digit.
fn digit_at(text: &[u8], at: usize) -> bool {
    text.get(at).is_some_and(u8::is_ascii_digit)
}

/// The byte at `at`, and nought past the end, which is what a text the
/// C library reads holds there.
fn byte_at(text: &[u8], at: usize) -> u8 {
    text.get(at).copied().unwrap_or(0)
}

impl Reading<'_> {
    /// One element of the text at `at`, written into `out`.
    fn value(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        let mut at = at;
        loop {
            let Some(byte) = self.text.get(at).copied() else {
                return Step::Done;
            };
            return match byte {
                b'{' => self.object(at, out),
                b'[' => self.array(at, out),
                b'"' | b'\'' => self.string(at, out),
                b'}' | b']' | b',' | b':' => Step::Mark(byte, at),
                b'+' | b'-' | b'.' | b'0'..=b'9' => self.number(at, out),
                0x09 | 0x0a | 0x0d | 0x20 => {
                    at = skipped(self.text, at);
                    continue;
                }
                0x0b | 0x0c | b'/' | 0xc2 | 0xe1 | 0xe2 | 0xe3 | 0xef => {
                    let next = spaced(self.text, at);
                    if next == at {
                        return Step::Wrong(at);
                    }
                    self.nonstandard = true;
                    at = next;
                    continue;
                }
                0 => Step::Done,
                _ => self.word(at, out),
            };
        }
    }

    /// `true`, `false`, `null`, and the names JSON5 gives a number no
    /// number can hold, which is `aNanInfName`.
    fn word(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        let text = self.text;
        for (word, kind) in [
            (b"true".as_slice(), TRUE),
            (b"false", FALSE),
            (b"null", NULL),
        ] {
            let end = at.saturating_add(word.len());
            if text.get(at..end) == Some(word) && !byte_at(text, end).is_ascii_alphanumeric() {
                append(out, kind, &[]);
                return Step::Ended(end);
            }
        }
        // `inf` and `infinity` answer the largest number a float holds,
        // and `NaN`, `QNaN` and `SNaN` answer nothing, in any case.
        for (word, kind) in [
            (b"inf".as_slice(), FLOAT),
            (b"infinity", FLOAT),
            (b"nan", NULL),
            (b"qnan", NULL),
            (b"snan", NULL),
        ] {
            let end = at.saturating_add(word.len());
            let Some(held) = text.get(at..end) else {
                continue;
            };
            if !held.eq_ignore_ascii_case(word) || byte_at(text, end).is_ascii_alphanumeric() {
                continue;
            }
            self.nonstandard = true;
            if kind == FLOAT {
                append(out, FLOAT, b"9e999");
            } else {
                append(out, NULL, &[]);
            }
            return Step::Ended(end);
        }
        Step::Wrong(at)
    }
}

impl Reading<'_> {
    /// A string in either pair of quotes, written as the text between
    /// them under the type that says which escapes it holds.
    fn string(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        let text = self.text;
        let delimiter = byte_at(text, at);
        if delimiter == b'\'' {
            self.nonstandard = true;
        }
        let mut kind = TEXT;
        let mut place = at.saturating_add(1);
        loop {
            let Some(byte) = text.get(place).copied() else {
                return Step::Wrong(place);
            };
            if byte == delimiter {
                break;
            }
            if byte == b'\\' {
                place = place.saturating_add(1);
                match self.escaped(place, &mut kind) {
                    Some(step) => place = step,
                    None => return Step::Wrong(place),
                }
            } else if byte <= 0x1f {
                if byte == 0 {
                    return Step::Wrong(place);
                }
                // A control character is one JSON5 allows in a string
                // and JSON does not.
                kind = TEXT5;
                self.nonstandard = true;
            } else if byte == b'"' {
                kind = TEXT5;
            }
            place = place.saturating_add(1);
        }
        let payload = text.get(at.saturating_add(1)..place).unwrap_or_default();
        append(out, kind, payload);
        Step::Ended(place.saturating_add(1))
    }

    /// The escape at `at`, which is the byte after the backslash, with
    /// the type the string takes from it. Answers where the escape
    /// ends, or nothing for one neither JSON nor JSON5 holds.
    fn escaped(&mut self, at: usize, kind: &mut u8) -> Option<usize> {
        let text = self.text;
        let byte = *text.get(at)?;
        let after = at.saturating_add(1);
        if matches!(byte, b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't')
            || (byte == b'u' && hex_at(text, after, 4))
        {
            if *kind == TEXT {
                *kind = TEXTJ;
            }
            return Some(at);
        }
        let paragraph = byte == 0xe2
            && text.get(after) == Some(&0x80)
            && matches!(text.get(at.saturating_add(2)), Some(0xa8 | 0xa9));
        if byte == b'\''
            || byte == b'v'
            || byte == b'\n'
            || paragraph
            || (byte == b'0' && !digit_at(text, after))
            || (byte == b'x' && hex_at(text, after, 2))
        {
            *kind = TEXT5;
            self.nonstandard = true;
            return Some(at);
        }
        if byte == b'\r' {
            *kind = TEXT5;
            self.nonstandard = true;
            // A return and a newline together are one break.
            return Some(if text.get(after) == Some(&b'\n') {
                after
            } else {
                at
            });
        }
        None
    }
}

impl Reading<'_> {
    /// A number, which is `parse_number` of
    /// `jsonTranslateTextToBlob`: the digits of JSON, and what JSON5
    /// allows beside them, which is a leading `+`, a leading or
    /// trailing point, hexadecimal, and the two names of infinity.
    fn number(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        let text = self.text;
        let first = byte_at(text, at);
        // Bit 0x01 says JSON5 and bit 0x02 says the number has a
        // fraction or an exponent, which is what the four types of
        // number count up from `INT`.
        let mut flags = 0_u8;
        if first == b'+' {
            self.nonstandard = true;
        }
        if first == b'.' {
            if !digit_at(text, at.saturating_add(1)) {
                return Step::Wrong(at);
            }
            self.nonstandard = true;
            return self.digits(at, out, 0x03);
        }
        if first == b'0' {
            return match self.zero(at, out) {
                Some(step) => step,
                None => self.digits(at, out, flags),
            };
        }
        if (first == b'-' || first == b'+')
            && let Some(step) = self.signed(at, out, &mut flags)
        {
            return step;
        }
        self.digits(at, out, flags)
    }

    /// A number that begins with a nought: hexadecimal, which JSON5
    /// allows, and the refusal of a second digit after it. Answers
    /// nothing where the number is read on from the nought.
    fn zero(&mut self, at: usize, out: &mut Vec<u8>) -> Option<Step> {
        let text = self.text;
        let second = byte_at(text, at.saturating_add(1));
        if (second == b'x' || second == b'X') && hex_at(text, at.saturating_add(2), 1) {
            self.nonstandard = true;
            let mut end = at.saturating_add(3);
            while hex_at(text, end, 1) {
                end = end.saturating_add(1);
            }
            return Some(self.written(at, at, end, out, 0x01));
        }
        if digit_at(text, at.saturating_add(1)) {
            return Some(Step::Wrong(at.saturating_add(1)));
        }
        None
    }

    /// A number that begins with a sign: the two names of infinity, a
    /// point after the sign, and hexadecimal after a nought. Answers
    /// nothing where the number is read on from the sign.
    fn signed(&mut self, at: usize, out: &mut Vec<u8>, flags: &mut u8) -> Option<Step> {
        let text = self.text;
        let after = at.saturating_add(1);
        if !digit_at(text, after) {
            if text
                .get(after..at.saturating_add(4))
                .is_some_and(|held| held.eq_ignore_ascii_case(b"inf"))
            {
                self.nonstandard = true;
                let held: &[u8] = if byte_at(text, at) == b'-' {
                    b"-9e999"
                } else {
                    b"9e999"
                };
                append(out, FLOAT, held);
                let long = text
                    .get(at.saturating_add(4)..at.saturating_add(9))
                    .is_some_and(|held| held.eq_ignore_ascii_case(b"inity"));
                return Some(Step::Ended(at.saturating_add(if long { 9 } else { 4 })));
            }
            if byte_at(text, after) == b'.' {
                self.nonstandard = true;
                *flags |= 0x01;
                return None;
            }
            return Some(Step::Wrong(at));
        }
        if byte_at(text, after) == b'0' {
            let third = byte_at(text, at.saturating_add(2));
            if third.is_ascii_digit() {
                return Some(Step::Wrong(after));
            }
            if (third == b'x' || third == b'X') && hex_at(text, at.saturating_add(3), 1) {
                self.nonstandard = true;
                let mut end = at.saturating_add(4);
                while hex_at(text, end, 1) {
                    end = end.saturating_add(1);
                }
                return Some(self.written(at, at, end, out, *flags | 0x01));
            }
        }
        None
    }
}

impl Reading<'_> {
    /// The digits of a number from `at`, which is `parse_number_2`:
    /// digits, one point, and one exponent, with the JSON5 shapes
    /// marked in the flags.
    fn digits(&mut self, at: usize, out: &mut Vec<u8>, flags: u8) -> Step {
        let text = self.text;
        let mut flags = flags;
        let mut exponent = false;
        let mut place = at.saturating_add(1);
        loop {
            let byte = byte_at(text, place);
            if byte.is_ascii_digit() {
                place = place.saturating_add(1);
                continue;
            }
            if byte == b'.' {
                if flags & 0x02 != 0 {
                    return Step::Wrong(place);
                }
                flags |= 0x02;
                place = place.saturating_add(1);
                continue;
            }
            if byte != b'e' && byte != b'E' {
                break;
            }
            match self.exponent(place, &mut flags, exponent) {
                Some(step) => place = step,
                None => return Step::Wrong(place),
            }
            exponent = true;
            place = place.saturating_add(1);
        }
        // A number that ends on its point is one JSON5 allows, and a
        // number that ends on anything else the digits do not hold is
        // no number at all.
        if byte_at(text, place.saturating_sub(1)) < b'0' {
            if !ends_on_point(text, place) {
                return Step::Wrong(place);
            }
            self.nonstandard = true;
            flags |= 0x01;
        }
        let from = if byte_at(text, at) == b'+' {
            at.saturating_add(1)
        } else {
            at
        };
        self.written(at, from, place, out, flags)
    }

    /// The exponent at `place`, with the sign after it and the digits
    /// after that. Answers where the `e` of it stands to read on from,
    /// and nothing for an exponent the number may not carry.
    fn exponent(&mut self, place: usize, flags: &mut u8, seen: bool) -> Option<usize> {
        let text = self.text;
        if byte_at(text, place.saturating_sub(1)) < b'0' {
            if !ends_on_point(text, place) {
                return None;
            }
            self.nonstandard = true;
            *flags |= 0x01;
        }
        if seen {
            return None;
        }
        *flags |= 0x02;
        let mut after = place.saturating_add(1);
        let sign = byte_at(text, after);
        if sign == b'+' || sign == b'-' {
            after = after.saturating_add(1);
        }
        if !digit_at(text, after) {
            return None;
        }
        Some(after.saturating_sub(1))
    }

    /// The number the text holds from `from` to `end`, written under
    /// the type the flags name, which is `parse_number_finish`.
    fn written(&self, at: usize, from: usize, end: usize, out: &mut Vec<u8>, flags: u8) -> Step {
        let _ = at;
        let payload = self.text.get(from..end).unwrap_or_default();
        append(out, INT.saturating_add(flags), payload);
        Step::Ended(end)
    }
}

/// Whether the number that ends before `place` ends on a point with a
/// digit before it, which is the shape JSON5 allows and JSON does not.
///
/// The reader asks this only where the byte before `place` is below a
/// digit, and the only such byte a number carries is its point.
fn ends_on_point(text: &[u8], place: usize) -> bool {
    let last = place.saturating_sub(1);
    digit_at(text, last.saturating_sub(1))
}

impl Reading<'_> {
    /// An array: the elements one after another, with the trailing
    /// comma JSON5 allows.
    fn array(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        self.depth = self.depth.saturating_add(1);
        if self.depth > DEPTH {
            return self.gave_up(Step::Wrong(at));
        }
        let mut body = Vec::new();
        let mut place = at.saturating_add(1);
        let end = loop {
            match self.value(place, &mut body) {
                Step::Ended(next) => place = next,
                Step::Mark(b']', mark) => {
                    if !body.is_empty() {
                        self.nonstandard = true;
                    }
                    break mark;
                }
                Step::Wrong(mark) => return self.gave_up(Step::Wrong(mark)),
                Step::Done | Step::Mark(_, _) => return self.gave_up(Step::Wrong(place)),
            }
            match self.between(place, b']', &mut body) {
                Between::Next(next) => place = next,
                Between::Last(mark) => break mark,
                Between::Wrong(mark) => return self.gave_up(Step::Wrong(mark)),
            }
        };
        append(out, ARRAY, &body);
        self.depth = self.depth.saturating_sub(1);
        Step::Ended(end.saturating_add(1))
    }

    /// The comma or the bracket after one element of an array or after
    /// one pair of an object, which may carry whitespace or a comment
    /// on either side of it.
    fn between(&mut self, at: usize, closer: u8, body: &mut Vec<u8>) -> Between {
        let text = self.text;
        let byte = byte_at(text, at);
        if byte == b',' {
            return Between::Next(at.saturating_add(1));
        }
        if byte == closer {
            return Between::Last(at);
        }
        let mut place = at;
        if matches!(byte, 0x09 | 0x0a | 0x0d | 0x20) {
            place = skipped(text, at);
            let byte = byte_at(text, place);
            if byte == b',' {
                return Between::Next(place.saturating_add(1));
            }
            if byte == closer {
                return Between::Last(place);
            }
        }
        match self.value(place, body) {
            Step::Mark(b',', mark) => Between::Next(mark.saturating_add(1)),
            Step::Mark(mark, place) if mark == closer => Between::Last(place),
            _ => Between::Wrong(place),
        }
    }

    /// Takes one level off the depth and answers the step that gave up.
    const fn gave_up(&mut self, step: Step) -> Step {
        self.depth = self.depth.saturating_sub(1);
        step
    }
}

/// What stood between two elements of an array or two pairs of an
/// object.
enum Between {
    /// A comma, with the next element beginning here.
    Next(usize),
    /// The bracket that closes them, standing here.
    Last(usize),
    /// Something neither of those, standing here.
    Wrong(usize),
}

impl Reading<'_> {
    /// An object: label and value one pair after another, where a
    /// label is a string or, which JSON5 allows, a name written with
    /// no quotes around it.
    fn object(&mut self, at: usize, out: &mut Vec<u8>) -> Step {
        self.depth = self.depth.saturating_add(1);
        if self.depth > DEPTH {
            return self.gave_up(Step::Wrong(at));
        }
        let mut body = Vec::new();
        let mut place = at.saturating_add(1);
        let end = loop {
            let began = body.len();
            let after = match self.labelled(place, &mut body) {
                Ok(next) => next,
                Err(Labelled::Closed(mark)) => {
                    if !body.is_empty() {
                        self.nonstandard = true;
                    }
                    break mark;
                }
                Err(Labelled::Wrong(mark)) => return self.gave_up(Step::Wrong(mark)),
            };
            let kind = body.get(began).map_or(0, |byte| byte & 0x0f);
            if !(TEXT..=TEXTRAW).contains(&kind) {
                return self.gave_up(Step::Wrong(place));
            }
            place = match self.colon(after, &mut body) {
                Some(next) => next,
                None => return self.gave_up(Step::Wrong(after)),
            };
            match self.value(place, &mut body) {
                Step::Ended(next) => place = next,
                Step::Wrong(mark) => return self.gave_up(Step::Wrong(mark)),
                Step::Done | Step::Mark(_, _) => return self.gave_up(Step::Wrong(place)),
            }
            match self.between(place, b'}', &mut body) {
                Between::Next(next) => place = next,
                Between::Last(mark) => break mark,
                Between::Wrong(mark) => return self.gave_up(Step::Wrong(mark)),
            }
        };
        append(out, OBJECT, &body);
        self.depth = self.depth.saturating_sub(1);
        Step::Ended(end.saturating_add(1))
    }

    /// The label of one pair, written into `body`: a string, or a name
    /// with no quotes around it, which JSON5 allows.
    fn labelled(&mut self, at: usize, body: &mut Vec<u8>) -> Result<usize, Labelled> {
        match self.value(at, body) {
            Step::Ended(next) => return Ok(next),
            Step::Mark(b'}', mark) => return Err(Labelled::Closed(mark)),
            Step::Wrong(mark) => {
                let name = spaced(self.text, at);
                return self.named(name, body).ok_or(Labelled::Wrong(mark));
            }
            Step::Done | Step::Mark(_, _) => {}
        }
        let name = spaced(self.text, at);
        self.named(name, body).ok_or(Labelled::Wrong(name))
    }

    /// A name with no quotes around it, written into `body`, and
    /// nothing where the text holds no such name at `at`.
    fn named(&mut self, at: usize, body: &mut Vec<u8>) -> Option<usize> {
        let text = self.text;
        let mut kind = TEXT;
        if !begins_name(byte_at(text, at)) && !escaped_hex(text, at, &mut kind) {
            return None;
        }
        let mut end = at.saturating_add(1);
        loop {
            let byte = byte_at(text, end);
            if holds_name(byte) && spaced(text, end) == end {
                end = end.saturating_add(1);
                continue;
            }
            if escaped_hex(text, end, &mut kind) {
                end = end.saturating_add(1);
                continue;
            }
            break;
        }
        append(body, kind, text.get(at..end).unwrap_or_default());
        self.nonstandard = true;
        Some(end)
    }

    /// The colon between a label and its value, which may carry
    /// whitespace or a comment on either side of it.
    fn colon(&mut self, at: usize, body: &mut Vec<u8>) -> Option<usize> {
        let text = self.text;
        if byte_at(text, at) == b':' {
            return Some(at.saturating_add(1));
        }
        let mut place = at;
        if matches!(byte_at(text, at), 0x09 | 0x0a | 0x0d | 0x20) {
            place = skipped(text, at);
            if byte_at(text, place) == b':' {
                return Some(place.saturating_add(1));
            }
        }
        match self.value(place, body) {
            Step::Mark(b':', mark) => Some(mark.saturating_add(1)),
            _ => None,
        }
    }
}

/// Whether the text at `at` holds `\uXXXX`, which JSON5 allows inside
/// a name with no quotes around it, and which makes the label one with
/// the escapes of JSON in it.
fn escaped_hex(text: &[u8], at: usize, kind: &mut u8) -> bool {
    if byte_at(text, at) != b'\\' || byte_at(text, at.saturating_add(1)) != b'u' {
        return false;
    }
    if !hex_at(text, at.saturating_add(2), 4) {
        return false;
    }
    *kind = TEXTJ;
    true
}

/// What reading the label of one pair answered where it read none.
enum Labelled {
    /// The bracket that closes the object stood here.
    Closed(usize),
    /// The text holds no label at this place.
    Wrong(usize),
}

/// The text of the element at `at`, written into `out`, and where the
/// element ends. Answers nothing where the bytes hold no whole
/// element, which is what `JSTRING_MALFORMED` marks.
///
/// Writing `n` bytes of blob costs O(n).
pub fn write(blob: &[u8], at: usize, out: &mut Vec<u8>) -> Option<usize> {
    let (kind, start, length) = element(blob, at)?;
    let payload = blob.get(start..start.saturating_add(length))?;
    let end = start.saturating_add(length);
    match kind {
        NULL => out.extend_from_slice(b"null"),
        TRUE => out.extend_from_slice(b"true"),
        FALSE => out.extend_from_slice(b"false"),
        INT | FLOAT => {
            if payload.is_empty() {
                return None;
            }
            out.extend_from_slice(payload);
        }
        INT5 => write_hex(payload, out)?,
        FLOAT5 => write_loose(payload, out)?,
        TEXT | TEXTJ => {
            out.push(b'"');
            out.extend_from_slice(payload);
            out.push(b'"');
        }
        TEXT5 => write_five(payload, out)?,
        TEXTRAW => write_raw(payload, out),
        ARRAY => return write_items(blob, start, end, out, b'[').map(|()| end),
        // `element` answers nothing for a type past an object, so the
        // rest is one.
        _ => return write_items(blob, start, end, out, b'{').map(|()| end),
    }
    Some(end)
}

/// The elements of an array or an object, with the commas and colons
/// between them and the brackets around them.
fn write_items(blob: &[u8], start: usize, end: usize, out: &mut Vec<u8>, open: u8) -> Option<()> {
    let object = open == b'{';
    out.push(open);
    let mut place = start;
    let mut count = 0_usize;
    while place < end {
        place = write(blob, place, out)?;
        // A label is followed by a colon and a value by a comma, which
        // makes every other one of an object a colon.
        out.push(if object && count.is_multiple_of(2) {
            b':'
        } else {
            b','
        });
        count = count.saturating_add(1);
    }
    if place > end || (object && !count.is_multiple_of(2)) {
        return None;
    }
    if count > 0 {
        out.pop();
    }
    out.push(if object { b'}' } else { b']' });
    Some(())
}

/// A whole number written in hexadecimal, written out in decimal, and
/// the largest number a float holds where it counts past one.
fn write_hex(payload: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut at = 2_usize;
    let mut held = 0_u64;
    let mut over = false;
    match payload.first() {
        Some(b'-') => {
            out.push(b'-');
            at = 3;
        }
        Some(b'+') => at = 3,
        Some(_) => {}
        None => return None,
    }
    for byte in payload.get(at..).unwrap_or_default() {
        let digit = u64::from(char::from(*byte).to_digit(16)?);
        if held >> 60 != 0 {
            over = true;
        } else {
            held = held.saturating_mul(16).saturating_add(digit);
        }
    }
    if over {
        out.extend_from_slice(b"9.0e999");
    } else {
        out.extend_from_slice(&crate::number::unsigned_text(held));
    }
    Some(())
}

/// A number JSON5 allows, written out with a digit on either side of
/// its point.
fn write_loose(payload: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut at = 0_usize;
    if payload.first() == Some(&b'-') {
        out.push(b'-');
        if payload.len() <= 1 {
            return None;
        }
        at = 1;
    }
    if payload.get(at) == Some(&b'.') {
        out.push(b'0');
    }
    while let Some(byte) = payload.get(at) {
        out.push(*byte);
        let next = payload.get(at.saturating_add(1));
        if *byte == b'.' && !next.is_some_and(u8::is_ascii_digit) {
            out.push(b'0');
        }
        at = at.saturating_add(1);
    }
    Some(())
}

/// Whether a byte stands in JSON text as it is, which is `jsonIsOk`:
/// every byte but a control character, a quote of either kind and a
/// backslash.
const fn plain(byte: u8) -> bool {
    byte > 0x1f && byte != b'"' && byte != b'\'' && byte != b'\\'
}

/// One control character as JSON writes it, which is
/// `jsonAppendControlChar`: the six it has a letter for, and `\u00xx`
/// for the rest.
fn write_control(byte: u8, out: &mut Vec<u8>) {
    let letter = match byte {
        0x08 => b'b',
        0x09 => b't',
        0x0a => b'n',
        0x0c => b'f',
        0x0d => b'r',
        _ => 0,
    };
    out.push(b'\\');
    if letter != 0 {
        out.push(letter);
        return;
    }
    out.extend_from_slice(b"u00");
    let digits = b"0123456789abcdef";
    out.push(digits.get(usize::from(byte >> 4)).copied().unwrap_or(b'0'));
    out.push(
        digits
            .get(usize::from(byte & 0x0f))
            .copied()
            .unwrap_or(b'0'),
    );
}

/// Text as SQL holds it, written with the quotes and escapes JSON
/// wants, which is `jsonAppendString`.
pub fn write_raw(payload: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for byte in payload {
        if plain(*byte) || *byte == b'\'' {
            out.push(*byte);
        } else if *byte == b'"' || *byte == b'\\' {
            out.push(b'\\');
            out.push(*byte);
        } else {
            write_control(*byte, out);
        }
    }
    out.push(b'"');
}

/// One value written as the JSON text of it, which is
/// `jsonAppendSqlValue`: what a JSON aggregate puts together.
///
/// # Errors
///
/// [`Refused::Blob`] for a blob that is not the binary form.
pub fn write_value(value: &Value, json: bool, out: &mut Vec<u8>) -> Result<(), Refused> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Int(_) => out.extend_from_slice(&value.stringify().unwrap_or_default()),
        Value::Real(number) => {
            let text = crate::fp::text(*number, crate::fp::DIGITS);
            match text.first() {
                Some(b'I') => out.extend_from_slice(b"9.0e+999"),
                Some(b'-') if text.get(1) == Some(&b'I') => {
                    out.extend_from_slice(b"-9.0e+999");
                }
                _ => out.extend_from_slice(&text),
            }
        }
        Value::Text(text) if json => out.extend_from_slice(text),
        Value::Text(text) => write_raw(text, out),
        Value::Blob(bytes) => {
            let blob = held_blob(bytes).ok_or(Refused::Blob)?;
            out.extend_from_slice(&text_of(&blob, 0)?);
        }
    }
    Ok(())
}

/// Text with the escapes JSON5 allows, written with the escapes JSON
/// allows in their place./// Text with the escapes JSON5 allows, written with the escapes JSON
/// allows in their place.
fn write_five(payload: &[u8], out: &mut Vec<u8>) -> Option<()> {
    out.push(b'"');
    let mut at = 0_usize;
    while let Some(byte) = payload.get(at).copied() {
        if plain(byte) || byte == b'\'' {
            out.push(byte);
            at = at.saturating_add(1);
            continue;
        }
        if byte == b'"' {
            out.extend_from_slice(b"\\\"");
            at = at.saturating_add(1);
            continue;
        }
        if byte <= 0x1f {
            write_control(byte, out);
            at = at.saturating_add(1);
            continue;
        }
        at = write_escape(payload, at, out)?;
    }
    out.push(b'"');
    Some(())
}

/// One escape JSON5 allows, written as JSON writes it, and where the
/// text goes on.
fn write_escape(payload: &[u8], at: usize, out: &mut Vec<u8>) -> Option<usize> {
    let after = at.saturating_add(1);
    let byte = *payload.get(after)?;
    let mut step = 2_usize;
    match byte {
        b'\'' => out.push(b'\''),
        b'v' => out.extend_from_slice(b"\\u000b"),
        b'x' => {
            let digits = payload.get(at.saturating_add(2)..at.saturating_add(4))?;
            out.extend_from_slice(b"\\u00");
            out.extend_from_slice(digits);
            step = 4;
        }
        b'0' => out.extend_from_slice(b"\\u0000"),
        // A break after a backslash carries no character of its own.
        b'\r' => {
            if payload.get(at.saturating_add(2)) == Some(&b'\n') {
                step = 3;
            }
        }
        b'\n' => {}
        0xe2 => {
            let held = payload.get(at.saturating_add(2)..at.saturating_add(4))?;
            if held != [0x80, 0xa8] && held != [0x80, 0xa9] {
                return None;
            }
            step = 4;
        }
        _ => {
            out.push(b'\\');
            out.push(byte);
        }
    }
    Some(at.saturating_add(step))
}

/// What a path did not find, and what a path may not be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wrong {
    /// The path is not one this engine reads, which is
    /// `JSON_LOOKUP_PATHERROR`.
    Path,
    /// The value runs deeper than a value may nest, which is
    /// `JSON_LOOKUP_TOODEEP`.
    Deep,
    /// The bytes hold no whole element, which is `JSON_LOOKUP_ERROR`.
    Blob,
}

/// Where the value the path names begins, and nothing where the value
/// the path names is not there.
///
/// The path is what follows the `$` that begins one. Each step of it
/// costs O(n) in the elements of the value it steps into.
///
/// # Errors
///
/// [`Wrong`] names what the path could not be read as.
pub fn lookup(blob: &[u8], at: usize, path: &[u8]) -> Result<Option<usize>, Wrong> {
    lookup_deep(blob, at, path, 0)
}

/// [`lookup`], counting how deep it stands.
fn lookup_deep(blob: &[u8], at: usize, path: &[u8], depth: usize) -> Result<Option<usize>, Wrong> {
    if depth >= DEPTH {
        return Err(Wrong::Deep);
    }
    let Some(byte) = path.first().copied() else {
        return Ok(Some(at));
    };
    let (kind, start, length) = element(blob, at).ok_or(Wrong::Blob)?;
    let end = start.saturating_add(length);
    if byte == b'.' {
        let (key, raw, rest) = keyed(path)?;
        if kind != OBJECT {
            return Ok(None);
        }
        let Some(found) = in_object(blob, start, end, &key, raw)? else {
            return Ok(None);
        };
        return lookup_deep(blob, found, rest, depth.saturating_add(1));
    }
    if byte == b'[' {
        // The value must be an array before the number is read, which
        // is the order `jsonLookupStep` reads them in.
        if kind != ARRAY {
            return Ok(None);
        }
        let (count, rest) = indexed(path, blob, at)?;
        let Some(count) = count else {
            return Ok(None);
        };
        let Some(found) = in_array(blob, start, end, count)? else {
            return Ok(None);
        };
        return lookup_deep(blob, found, rest, depth.saturating_add(1));
    }
    Err(Wrong::Path)
}

/// The label one step of a path names, whether it holds no escape, and
/// the rest of the path after it.
fn keyed(path: &[u8]) -> Result<(Vec<u8>, bool, &[u8]), Wrong> {
    let rest = path.get(1..).unwrap_or_default();
    if rest.first() == Some(&b'"') {
        let mut at = 1_usize;
        while let Some(byte) = rest.get(at) {
            if *byte == b'"' {
                break;
            }
            if *byte == b'\\' && rest.get(at.saturating_add(1)).is_some() {
                at = at.saturating_add(1);
            }
            at = at.saturating_add(1);
        }
        if rest.get(at) != Some(&b'"') {
            return Err(Wrong::Path);
        }
        let key = rest.get(1..at).unwrap_or_default().to_vec();
        let raw = !key.contains(&b'\\');
        return Ok((
            key,
            raw,
            rest.get(at.saturating_add(1)..).unwrap_or_default(),
        ));
    }
    let mut at = 0_usize;
    while let Some(byte) = rest.get(at) {
        if *byte == b'.' || *byte == b'[' {
            break;
        }
        at = at.saturating_add(1);
    }
    if at == 0 {
        return Err(Wrong::Path);
    }
    let key = rest.get(..at).unwrap_or_default().to_vec();
    Ok((key, true, rest.get(at..).unwrap_or_default()))
}

/// Where the value of the label `key` begins, among the pairs from
/// `start` to `end`.
fn in_object(
    blob: &[u8],
    start: usize,
    end: usize,
    key: &[u8],
    raw: bool,
) -> Result<Option<usize>, Wrong> {
    let mut at = start;
    while at < end {
        let (kind, text, length) = element(blob, at).ok_or(Wrong::Blob)?;
        if !(TEXT..=TEXTRAW).contains(&kind) {
            return Err(Wrong::Blob);
        }
        let value = text.saturating_add(length);
        if value >= end {
            return Err(Wrong::Blob);
        }
        let label = blob.get(text..value).ok_or(Wrong::Blob)?;
        if same_label(key, raw, label, kind == TEXT || kind == TEXTRAW) {
            // The value of the pair ends inside the object, which
            // `jsonLookupStep` holds the pair it answers to.
            if after(blob, value).ok_or(Wrong::Blob)? > end {
                return Err(Wrong::Blob);
            }
            return Ok(Some(value));
        }
        at = after(blob, value).ok_or(Wrong::Blob)?;
    }
    if at > end {
        return Err(Wrong::Blob);
    }
    Ok(None)
}

/// Where the element `count` of an array begins, among the elements
/// from `start` to `end`.
fn in_array(blob: &[u8], start: usize, end: usize, count: u64) -> Result<Option<usize>, Wrong> {
    let mut at = start;
    let mut left = count;
    while at < end {
        if left == 0 {
            return Ok(Some(at));
        }
        left = left.saturating_sub(1);
        at = after(blob, at).ok_or(Wrong::Blob)?;
    }
    if at > end {
        return Err(Wrong::Blob);
    }
    Ok(None)
}

/// How many elements the array at `at` holds.
fn counted(blob: &[u8], at: usize) -> Option<u64> {
    let (kind, start, length) = element(blob, at)?;
    if kind != ARRAY {
        return Some(0);
    }
    let end = start.saturating_add(length);
    let mut place = start;
    let mut count = 0_u64;
    while place < end {
        place = after(blob, place)?;
        count = count.saturating_add(1);
    }
    Some(count)
}

/// Which element of an array one step of a path names, and the rest of
/// the path after it. Answers no number where `[#-n]` counts back past
/// the first element.
fn indexed<'a>(path: &'a [u8], blob: &[u8], at: usize) -> Result<(Option<u64>, &'a [u8]), Wrong> {
    let mut place = 1_usize;
    let mut count = 0_u64;
    while path.get(place).is_some_and(u8::is_ascii_digit) {
        // A number past every array answers nothing rather than
        // refusing the path, which is what the C library keeps it
        // under 0xffffffff for.
        if count < 0xffff_ffff {
            let digit = u64::from(path.get(place).copied().unwrap_or(b'0').wrapping_sub(b'0'));
            count = count.saturating_mul(10).saturating_add(digit);
        }
        place = place.saturating_add(1);
    }
    if place < 2 || path.get(place) != Some(&b']') {
        if path.get(1) != Some(&b'#') {
            return Err(Wrong::Path);
        }
        count = counted(blob, at).ok_or(Wrong::Blob)?;
        place = 2;
        if path.get(2) == Some(&b'-') && path.get(3).is_some_and(u8::is_ascii_digit) {
            let mut back = 0_u64;
            place = 3;
            while path.get(place).is_some_and(u8::is_ascii_digit) {
                if back < 0xffff_ffff {
                    let digit =
                        u64::from(path.get(place).copied().unwrap_or(b'0').wrapping_sub(b'0'));
                    back = back.saturating_mul(10).saturating_add(digit);
                }
                place = place.saturating_add(1);
            }
            if back > count {
                return Ok((None, path.get(place..).unwrap_or_default()));
            }
            count = count.saturating_sub(back);
        }
        if path.get(place) != Some(&b']') {
            return Err(Wrong::Path);
        }
    }
    Ok((
        Some(count),
        path.get(place.saturating_add(1)..).unwrap_or_default(),
    ))
}

/// Whether two labels name the same thing, which is
/// `jsonLabelCompare`: bytes where neither holds an escape, and the
/// characters the escapes stand for otherwise.
fn same_label(key: &[u8], key_raw: bool, label: &[u8], label_raw: bool) -> bool {
    if key_raw && label_raw {
        return key == label;
    }
    unescaped(key, key_raw) == unescaped(label, label_raw)
}

/// The characters of a label, with every escape read as the character
/// it stands for.
fn unescaped(text: &[u8], raw: bool) -> Vec<u8> {
    if raw {
        return text.to_vec();
    }
    let mut out = Vec::new();
    let mut at = 0_usize;
    while let Some(byte) = text.get(at).copied() {
        if byte != b'\\' {
            out.push(byte);
            at = at.saturating_add(1);
            continue;
        }
        at = read_escape(text, at, &mut out);
    }
    out
}

/// One escape of a label, written into `out` as the character it
/// stands for, and where the label goes on, which is
/// `jsonUnescapeOneChar`.
fn read_escape(text: &[u8], at: usize, out: &mut Vec<u8>) -> usize {
    let after = at.saturating_add(1);
    let Some(byte) = text.get(after).copied() else {
        out.push(0xff);
        return text.len();
    };
    let plain = match byte {
        b'b' => Some(0x08),
        b'f' => Some(0x0c),
        b'n' => Some(0x0a),
        b'r' => Some(0x0d),
        b't' => Some(0x09),
        b'v' => Some(0x0b),
        b'\'' | b'"' | b'/' | b'\\' => Some(u32::from(byte)),
        b'0' if !text
            .get(at.saturating_add(2))
            .is_some_and(u8::is_ascii_digit) =>
        {
            Some(0)
        }
        _ => None,
    };
    if let Some(code) = plain {
        push_char(out, code);
        return at.saturating_add(2);
    }
    if byte == b'u' {
        return read_hex(text, at, out);
    }
    if byte == b'x' {
        let Some(digits) = text.get(at.saturating_add(2)..at.saturating_add(4)) else {
            out.push(0xff);
            return text.len();
        };
        push_char(out, hex_value(digits));
        return at.saturating_add(4);
    }
    // A break after a backslash carries no character of its own.
    if byte == b'\n' {
        return at.saturating_add(2);
    }
    if byte == b'\r' {
        let long = text.get(at.saturating_add(2)) == Some(&b'\n');
        return at.saturating_add(if long { 3 } else { 2 });
    }
    if byte == 0xe2 && text.get(at.saturating_add(2)) == Some(&0x80) {
        return at.saturating_add(4);
    }
    out.push(0xff);
    text.len()
}

/// The `\uXXXX` escape at `at`, with the pair of them that stands for
/// one character past the first plane.
fn read_hex(text: &[u8], at: usize, out: &mut Vec<u8>) -> usize {
    let Some(digits) = text.get(at.saturating_add(2)..at.saturating_add(6)) else {
        out.push(0xff);
        return text.len();
    };
    let first = hex_value(digits);
    if first & 0xfc00 == 0xd800
        && text.get(at.saturating_add(6)) == Some(&b'\\')
        && text.get(at.saturating_add(7)) == Some(&b'u')
        && let Some(low) = text.get(at.saturating_add(8)..at.saturating_add(12))
    {
        let second = hex_value(low);
        if second & 0xfc00 == 0xdc00 {
            let code = ((first & 0x3ff) << 10)
                .saturating_add(second & 0x3ff)
                .saturating_add(0x10000);
            push_char(out, code);
            return at.saturating_add(12);
        }
    }
    push_char(out, first);
    at.saturating_add(6)
}

/// The number the hex digits hold.
fn hex_value(digits: &[u8]) -> u32 {
    let mut held = 0_u32;
    for byte in digits {
        let digit = char::from(*byte).to_digit(16).unwrap_or(0);
        held = held.saturating_mul(16).saturating_add(digit);
    }
    held
}

/// One character written as the bytes UTF-8 holds it in.
fn push_char(out: &mut Vec<u8>, code: u32) {
    match char::from_u32(code) {
        Some(held) => {
            let mut bytes = [0_u8; 4];
            out.extend_from_slice(held.encode_utf8(&mut bytes).as_bytes());
        }
        // A number no character stands for, which compares against
        // nothing a label holds.
        None => out.push(0xff),
    }
}

/// What a JSON function refuses, which is the message it raises.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Refused {
    /// The text is not JSON.
    Malformed,
    /// The path is not one, with the path as it was written.
    Path(Vec<u8>),
    /// The value the path names is not an element of an array, with
    /// the path as it was written.
    NotArray(Vec<u8>),
    /// The path steps deeper than the walk goes.
    Deep,
    /// The value nests deeper than a value may.
    Nested,
    /// A blob stands where JSON is wanted.
    Blob,
    /// `json_object` was given a label that is not text.
    Labels,
    /// `json_object` was given an odd number of arguments.
    Pairs,
    /// `json_insert`, `json_replace` or `json_set` was given an even
    /// number of arguments, with the name of the function.
    Odd(Vec<u8>),
    /// The flags of `json_valid` are not flags.
    Flags,
}

/// The JSON a value holds where the value stands for a document, which
/// is `jsonParseFuncArg`: text is read as JSON and a blob is read as
/// the binary form.
///
/// Reading `n` bytes costs O(n).
///
/// # Errors
///
/// [`Refused::Malformed`] where the text is not JSON.
pub fn document(value: &Value) -> Result<Vec<u8>, Refused> {
    match value {
        Value::Text(text) => read(text)
            .map(|(blob, _)| blob)
            .map_err(|_| Refused::Malformed),
        // A blob that is not the binary form is read as the text it
        // holds, which is the behaviour applications came to depend on
        // and which `jsonParseFuncArg` keeps.
        Value::Blob(bytes) => match held_blob(bytes) {
            Some(blob) => Ok(blob),
            None => read(bytes)
                .map(|(blob, _)| blob)
                .map_err(|_| Refused::Malformed),
        },
        // Nothing stringifies to no text at all, which is no JSON.
        Value::Null | Value::Int(_) | Value::Real(_) => {
            let text = value.stringify().unwrap_or_default();
            read(&text)
                .map(|(blob, _)| blob)
                .map_err(|_| Refused::Malformed)
        }
    }
}

/// The blob as the binary form of JSON, where it holds one whole
/// element and nothing after it, which is `jsonArgIsJsonb`.
fn held_blob(bytes: &[u8]) -> Option<Vec<u8>> {
    let first = *bytes.first()?;
    let (kind, start, length) = element(bytes, 0)?;
    if start.saturating_add(length) != bytes.len() {
        return None;
    }
    // `null`, `true` and `false` carry no payload.
    if kind <= FALSE && length != 0 {
        return None;
    }
    // A short blob that begins with a byte JSON text begins with is
    // read whole before it is taken for the binary form, so that text
    // is not mistaken for it.
    let alike = first == b'{' || first == b'[' || first.is_ascii_digit();
    if length <= 7 && alike && !checked(bytes, (kind, start, length), 0) {
        return None;
    }
    Some(bytes.to_vec())
}

/// Whether the element `held` is a whole one, with everything under it
/// whole as well, which is `jsonbValidityCheck`. The caller read the
/// element out of the blob, so that a walk never reads one twice.
///
/// Walking `n` bytes costs O(n).
fn checked(blob: &[u8], held: (u8, usize, usize), depth: usize) -> bool {
    if depth >= DEPTH {
        return false;
    }
    let (kind, start, length) = held;
    let last = start.saturating_add(length);
    if kind <= FALSE {
        return length == 0;
    }
    if kind < ARRAY {
        return length > 0;
    }
    let mut place = start;
    let mut count = 0_usize;
    while place < last {
        let Some(under) = element(blob, place) else {
            return false;
        };
        // The label of a pair is text and nothing else.
        if kind == OBJECT && count.is_multiple_of(2) && !(TEXT..=TEXTRAW).contains(&under.0) {
            return false;
        }
        if !checked(blob, under, depth.saturating_add(1)) {
            return false;
        }
        count = count.saturating_add(1);
        place = under.1.saturating_add(under.2);
    }
    place == last && (kind == ARRAY || count.is_multiple_of(2))
}

/// The JSON a value holds where the value stands for a value to write
/// into a document, which is `jsonFunctionArgToBlob`: text carries
/// JSON only where the call that answered it said so.
///
/// # Errors
///
/// [`Refused::Malformed`] where the text says it is JSON and is not,
/// and [`Refused::Blob`] for a blob that is not the binary form.
pub fn written(value: &Value, json: bool) -> Result<Vec<u8>, Refused> {
    let mut out = Vec::new();
    match value {
        Value::Null => append(&mut out, NULL, &[]),
        Value::Int(_) => append(&mut out, INT, &value.stringify().unwrap_or_default()),
        Value::Real(number) => written_real(*number, &mut out),
        Value::Text(text) => {
            if json {
                return read(text)
                    .map(|(blob, _)| blob)
                    .map_err(|_| Refused::Malformed);
            }
            append(&mut out, TEXTRAW, text);
        }
        Value::Blob(bytes) => return held_blob(bytes).ok_or(Refused::Blob),
    }
    Ok(out)
}

/// A real written as JSON holds it, where an infinity is written as
/// the largest number a float holds.
fn written_real(number: f64, out: &mut Vec<u8>) {
    let text = crate::fp::text(number, crate::fp::DIGITS);
    // SQLite writes an infinity as the largest number its own
    // formatting holds, which is what `sqlite3_value_text` of one
    // answers here.
    let held: &[u8] = match text.first() {
        Some(b'I') => b"9.0e+999",
        Some(b'-') if text.get(1) == Some(&b'I') => b"-9.0e+999",
        _ => &text,
    };
    append(out, FLOAT, held);
}

/// The text of the whole value the blob holds.
///
/// # Errors
///
/// [`Refused::Malformed`] where the bytes hold no whole element.
pub fn text_of(blob: &[u8], at: usize) -> Result<Vec<u8>, Refused> {
    let mut out = Vec::new();
    write(blob, at, &mut out).ok_or(Refused::Malformed)?;
    Ok(out)
}

/// The element at `at` as an SQL value, with whether that value is
/// JSON of its own, which is `jsonReturnFromBlob`: an array and an
/// object answer their text and everything else answers what SQL holds
/// it as.
///
/// # Errors
///
/// [`Refused::Malformed`] where the bytes hold no whole element.
pub fn value_of(blob: &[u8], at: usize) -> Result<(Value, bool), Refused> {
    let (kind, start, length) = element(blob, at).ok_or(Refused::Malformed)?;
    let payload = blob
        .get(start..start.saturating_add(length))
        .ok_or(Refused::Malformed)?;
    let value = match kind {
        NULL => Value::Null,
        TRUE => Value::Int(1),
        FALSE => Value::Int(0),
        INT | INT5 => whole_number(payload)?,
        FLOAT | FLOAT5 => Value::Real(crate::number::real(payload).value),
        TEXT | TEXTRAW => Value::Text(payload.to_vec()),
        TEXT5 | TEXTJ => Value::Text(unescaped(payload, false)),
        // `element` answers nothing for a type past an object, so the
        // rest is an array or an object.
        _ => return Ok((Value::Text(text_of(blob, at)?), true)),
    };
    Ok((value, false))
}

/// A number with no sign as the real nearest to it.
fn unsigned_as_real(held: u64) -> f64 {
    let half = held.wrapping_div(2);
    let rest = held.wrapping_sub(half);
    crate::value::integer_as_real(i64::try_from(half).unwrap_or(i64::MAX))
        + crate::value::integer_as_real(i64::try_from(rest).unwrap_or(i64::MAX))
}

/// A whole number as SQL holds it, which is a real where the digits
/// count past what a whole number holds.
fn whole_number(payload: &[u8]) -> Result<Value, Refused> {
    let negative = payload.first() == Some(&b'-');
    let digits = if negative {
        payload.get(1..).ok_or(Refused::Malformed)?
    } else {
        payload
    };
    if digits
        .get(..2)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"0x"))
    {
        let mut held = 0_u64;
        for byte in digits.get(2..).unwrap_or_default() {
            let digit = u64::from(char::from(*byte).to_digit(16).ok_or(Refused::Malformed)?);
            // `sqlite3DecOrHexToI64` keeps the low sixteen digits of a
            // longer number and drops the rest.
            held = held.wrapping_mul(16).wrapping_add(digit);
        }
        return Ok(match i64::try_from(held) {
            Ok(number) if negative => Value::Int(number.saturating_neg()),
            Ok(number) => Value::Int(number),
            // A number past what a whole number holds is read as the
            // real the bits stand for with no sign, which is what
            // `jsonReturnFromBlob` reads it as.
            Err(_) => {
                let real = unsigned_as_real(held);
                Value::Real(if negative { -real } else { real })
            }
        });
    }
    let read = crate::number::integer(payload);
    if read.outcome == crate::number::Outcome::Exact {
        return Ok(Value::Int(read.value));
    }
    Ok(Value::Real(crate::number::real(payload).value))
}

/// `json(X)`: the JSON of `X`, written with nothing left out and
/// nothing added.
///
/// # Errors
///
/// [`Refused`] names what the value could not be read as.
pub fn minified(value: &Value) -> Result<Value, Refused> {
    let blob = document(value)?;
    Ok(Value::Text(text_of(&blob, 0)?))
}

/// The value a JSON function answers with: the text of the blob, or
/// the blob itself where the call was one of the `jsonb` family.
///
/// # Errors
///
/// [`Refused::Malformed`] where the bytes hold no whole element.
pub fn answered(blob: Vec<u8>, binary: bool) -> Result<Value, Refused> {
    if binary {
        return Ok(Value::Blob(blob));
    }
    Ok(Value::Text(text_of(&blob, 0)?))
}

/// `json_quote(X)`: the value written as JSON writes it.
///
/// # Errors
///
/// [`Refused`] names what the value could not be read as.
pub fn quoted(value: &Value, json: bool) -> Result<Value, Refused> {
    let blob = written(value, json)?;
    Ok(Value::Text(text_of(&blob, 0)?))
}

/// `json_array(...)`: the values as the elements of an array.
///
/// # Errors
///
/// [`Refused`] names what an argument could not be read as.
pub fn array(args: &[Value], carried: &[bool]) -> Result<Vec<u8>, Refused> {
    let mut body = Vec::new();
    for (at, value) in args.iter().enumerate() {
        body.extend_from_slice(&written(value, carried.get(at).copied().unwrap_or(false))?);
    }
    let mut blob = Vec::new();
    append(&mut blob, ARRAY, &body);
    Ok(blob)
}

/// `json_object(...)`: the values as the labels and values of an
/// object, one pair after another.
///
/// # Errors
///
/// [`Refused::Pairs`] for an odd number of arguments,
/// [`Refused::Labels`] for a label that is not text, and whatever a
/// value could not be read as.
pub fn object(args: &[Value], carried: &[bool]) -> Result<Vec<u8>, Refused> {
    if !args.len().is_multiple_of(2) {
        return Err(Refused::Pairs);
    }
    let mut body = Vec::new();
    for (at, pair) in args.chunks(2).enumerate() {
        let Some(Value::Text(label)) = pair.first() else {
            return Err(Refused::Labels);
        };
        append(&mut body, TEXTRAW, label);
        let place = at.saturating_mul(2).saturating_add(1);
        let value = pair.get(1).unwrap_or(&Value::Null);
        body.extend_from_slice(&written(
            value,
            carried.get(place).copied().unwrap_or(false),
        )?);
    }
    let mut blob = Vec::new();
    append(&mut blob, OBJECT, &body);
    Ok(blob)
}

/// Where the path names a value in the blob, and nothing where the
/// value it names is not there.
fn at_path(blob: &[u8], path: &[u8]) -> Result<Option<usize>, Refused> {
    if path.first() != Some(&b'$') {
        return Err(Refused::Path(path.to_vec()));
    }
    let rest = path.get(1..).unwrap_or_default();
    match lookup(blob, 0, rest) {
        Ok(found) => Ok(found),
        Err(Wrong::Path) => Err(Refused::Path(path.to_vec())),
        Err(Wrong::Deep) => Err(Refused::Deep),
        Err(Wrong::Blob) => Err(Refused::Malformed),
    }
}

/// `json_extract(X, P, ...)`: the value at each path, as one value
/// where the call named one path and as an array where it named more.
///
/// # Errors
///
/// [`Refused`] names what the value or a path could not be read as.
pub fn extract(args: &[Value]) -> Result<(Value, bool), Refused> {
    let blob = document(args.first().unwrap_or(&Value::Null))?;
    let paths = args.get(1..).unwrap_or_default();
    if let ([one], true) = (paths, paths.len() == 1) {
        let path = one.text().ok_or(Refused::Path(Vec::new()))?;
        return match at_path(&blob, &path)? {
            Some(found) => value_of(&blob, found),
            None => Ok((Value::Null, false)),
        };
    }
    let mut out = Vec::new();
    out.push(b'[');
    for (at, one) in paths.iter().enumerate() {
        if at > 0 {
            out.push(b',');
        }
        let path = one.text().ok_or(Refused::Path(Vec::new()))?;
        match at_path(&blob, &path)? {
            Some(found) => out.extend_from_slice(&text_of(&blob, found)?),
            None => out.extend_from_slice(b"null"),
        }
    }
    out.push(b']');
    Ok((Value::Text(out), true))
}

/// `json_type(X[,P])`: the name of what the value at the path is.
///
/// # Errors
///
/// [`Refused`] names what the value or the path could not be read as.
pub fn type_of(args: &[Value]) -> Result<Value, Refused> {
    let blob = document(args.first().unwrap_or(&Value::Null))?;
    let at = match args.get(1) {
        None => Some(0),
        Some(one) => {
            let path = one.text().ok_or(Refused::Path(Vec::new()))?;
            at_path(&blob, &path)?
        }
    };
    let Some(at) = at else {
        return Ok(Value::Null);
    };
    let (kind, _, _) = element(&blob, at).ok_or(Refused::Malformed)?;
    let name: &[u8] = match kind {
        NULL => b"null",
        TRUE => b"true",
        FALSE => b"false",
        INT | INT5 => b"integer",
        FLOAT | FLOAT5 => b"real",
        TEXT | TEXTJ | TEXT5 | TEXTRAW => b"text",
        ARRAY => b"array",
        // `element` answers nothing for a type past an object.
        _ => b"object",
    };
    Ok(Value::Text(name.to_vec()))
}

/// `json_array_length(X[,P])`: how many elements the array at the path
/// holds, and nought where the value there is no array.
///
/// # Errors
///
/// [`Refused`] names what the value or the path could not be read as.
pub fn array_length(args: &[Value]) -> Result<Value, Refused> {
    let blob = document(args.first().unwrap_or(&Value::Null))?;
    let at = match args.get(1) {
        None => Some(0),
        Some(one) => {
            let path = one.text().ok_or(Refused::Path(Vec::new()))?;
            at_path(&blob, &path)?
        }
    };
    let Some(at) = at else {
        return Ok(Value::Null);
    };
    let count = counted(&blob, at).ok_or(Refused::Malformed)?;
    Ok(Value::Int(i64::try_from(count).unwrap_or(i64::MAX)))
}

/// `json_valid(X[,F])`: whether the value is JSON under the flags,
/// which are 1 for JSON, 2 for JSON5, 4 for the binary form read
/// lightly and 8 for it read whole.
///
/// # Errors
///
/// [`Refused::Flags`] for flags outside 1 to 15.
pub fn valid(args: &[Value]) -> Result<Value, Refused> {
    let mut flags = 1_i64;
    if let Some(given) = args.get(1) {
        flags = given.to_integer();
        if !(1..=15).contains(&flags) {
            return Err(Refused::Flags);
        }
    }
    let value = args.first().unwrap_or(&Value::Null);
    if *value == Value::Null {
        return Ok(Value::Null);
    }
    if let Value::Blob(bytes) = value
        && held_blob(bytes).is_some()
    {
        // Flag 4 reads the binary form lightly, which `held_blob` has
        // done, and flag 8 reads every element of it.
        if flags & 0x04 != 0 {
            return Ok(Value::Int(1));
        }
        let whole =
            flags & 0x08 != 0 && element(bytes, 0).is_some_and(|held| checked(bytes, held, 0));
        return Ok(Value::Int(i64::from(whole)));
    }
    if flags.trailing_zeros() >= 2 {
        return Ok(Value::Int(0));
    }
    let text = value.text().unwrap_or_default();
    let held = match read(&text) {
        Ok((_, nonstandard)) => flags & 0x02 != 0 || !nonstandard,
        Err(_) => false,
    };
    Ok(Value::Int(i64::from(held)))
}

/// `json_error_position(X)`: where the text stopped being JSON,
/// counting characters from one, and nought where the text is JSON.
#[must_use]
pub fn error_position(value: &Value) -> Value {
    if *value == Value::Null {
        return Value::Null;
    }
    if let Value::Blob(bytes) = value {
        return Value::Int(i64::from(held_blob(bytes).is_none()));
    }
    let text = value.text().unwrap_or_default();
    match read(&text) {
        Ok(_) => Value::Int(0),
        Err(at) => {
            // The count is of characters and not of bytes, which is
            // what `jsonErrorFunc` counts with `sqlite3Utf8CharLen`.
            let held = text.get(..at).unwrap_or_default();
            let count = crate::utf8::count(held);
            Value::Int(i64::try_from(count).unwrap_or(i64::MAX).saturating_add(1))
        }
    }
}

/// `json_pretty(X[,I])`: the JSON written with one element per line
/// and `indent` before each level of it.
///
/// # Errors
///
/// [`Refused`] names what the value could not be read as.
pub fn pretty(args: &[Value]) -> Result<Value, Refused> {
    let blob = document(args.first().unwrap_or(&Value::Null))?;
    let indent = match args.get(1) {
        Some(Value::Null) | None => b"    ".to_vec(),
        Some(given) => given.text().unwrap_or_default(),
    };
    let mut out = Vec::new();
    write_pretty(&blob, 0, &indent, 0, &mut out).ok_or(Refused::Malformed)?;
    Ok(Value::Text(out))
}

/// The text of the element at `at`, written with one element per line,
/// which is `jsonTranslateBlobToPrettyText`.
fn write_pretty(
    blob: &[u8],
    at: usize,
    indent: &[u8],
    level: usize,
    out: &mut Vec<u8>,
) -> Option<usize> {
    let (kind, start, length) = element(blob, at)?;
    let end = start.saturating_add(length);
    if kind != ARRAY && kind != OBJECT {
        return write(blob, at, out);
    }
    let object = kind == OBJECT;
    out.push(if object { b'{' } else { b'[' });
    if length == 0 {
        out.push(if object { b'}' } else { b']' });
        return Some(end);
    }
    let inner = level.saturating_add(1);
    let mut place = start;
    let mut first = true;
    while place < end {
        if !first {
            out.push(b',');
        }
        first = false;
        out.push(b'\n');
        write_indent(indent, inner, out);
        if object {
            place = write(blob, place, out)?;
            out.extend_from_slice(b": ");
        }
        place = write_pretty(blob, place, indent, inner, out)?;
    }
    out.push(b'\n');
    write_indent(indent, level, out);
    out.push(if object { b'}' } else { b']' });
    Some(end)
}

/// The indent of one level, written `level` times.
fn write_indent(indent: &[u8], level: usize, out: &mut Vec<u8>) {
    for _ in 0..level {
        out.extend_from_slice(indent);
    }
}

/// What an edit does to the value a path names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Takes the value out, which is `json_remove`.
    Remove,
    /// Writes the value where one is already, which is
    /// `json_replace`.
    Replace,
    /// Writes the value where none is, which is `json_insert`.
    Insert,
    /// Writes the value either way, which is `json_set`.
    Set,
    /// Writes the value before the element the path names, which is
    /// `json_array_insert`.
    ArrayInsert,
}

/// What an edit writes and how.
struct Writing<'a> {
    /// What the edit does.
    edit: Edit,
    /// The value it writes, in the binary form.
    value: &'a [u8],
    /// Whether the step that reached this element named an element of
    /// an array, which `json_array_insert` is held to.
    indexed: bool,
}

/// What an edit left of an element.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Left {
    /// The element holds these bytes now.
    Held(Vec<u8>),
    /// The element goes, which is what `json_remove` does.
    Gone,
    /// The edit found nothing to write, so the element stands as it
    /// was, which is `JSON_LOOKUP_NOTFOUND` under an edit.
    Same,
}

/// The element at `at` after the edit at `path`, which is
/// `jsonLookupStep` under an `eEdit`.
///
/// Each step costs O(n) in the elements of the value it steps into,
/// and the value is written again as it goes.
fn edited(
    blob: &[u8],
    at: usize,
    path: &[u8],
    writing: &Writing<'_>,
    depth: usize,
) -> Result<Left, Refused> {
    if depth >= DEPTH {
        return Err(Refused::Deep);
    }
    let (kind, start, length) = element(blob, at).ok_or(Refused::Malformed)?;
    let end = start.saturating_add(length);
    let whole = blob
        .get(at..after(blob, at).ok_or(Refused::Malformed)?)
        .ok_or(Refused::Malformed)?;
    let Some(byte) = path.first().copied() else {
        return match writing.edit {
            Edit::Remove => Ok(Left::Gone),
            Edit::Insert => Ok(Left::Same),
            Edit::Replace | Edit::Set => Ok(Left::Held(writing.value.to_vec())),
            // The value goes before the element the path names, and
            // the path must name an element of an array.
            Edit::ArrayInsert => {
                if !writing.indexed {
                    return Err(Refused::NotArray(Vec::new()));
                }
                let mut out = writing.value.to_vec();
                out.extend_from_slice(whole);
                Ok(Left::Held(out))
            }
        };
    };
    if byte == b'.' {
        if kind != OBJECT {
            return Ok(Left::Same);
        }
        return edited_object(blob, (start, end), path, writing, depth);
    }
    if byte == b'[' {
        if kind != ARRAY {
            return Ok(Left::Same);
        }
        return edited_array(blob, at, path, writing, depth);
    }
    Err(Refused::Path(Vec::new()))
}

/// The object from `start` to `end` after the edit at `path`.
fn edited_object(
    blob: &[u8],
    bounds: (usize, usize),
    path: &[u8],
    writing: &Writing<'_>,
    depth: usize,
) -> Result<Left, Refused> {
    let (start, end) = bounds;
    let inner = Writing {
        edit: writing.edit,
        value: writing.value,
        indexed: false,
    };
    let (key, raw, rest) = keyed(path).ok().ok_or(Refused::Path(Vec::new()))?;
    let mut body = Vec::new();
    let mut at = start;
    let mut found = false;
    while at < end {
        let (kind, text, width) = element(blob, at).ok_or(Refused::Malformed)?;
        let value_at = text.saturating_add(width);
        let label = blob.get(text..value_at).ok_or(Refused::Malformed)?;
        let after_value = after(blob, value_at).ok_or(Refused::Malformed)?;
        if !found && same_label(&key, raw, label, kind == TEXT || kind == TEXTRAW) {
            found = true;
            match edited(blob, value_at, rest, &inner, depth.saturating_add(1))? {
                Left::Held(held) => {
                    body.extend_from_slice(blob.get(at..value_at).ok_or(Refused::Malformed)?);
                    body.extend_from_slice(&held);
                }
                Left::Gone => {}
                Left::Same => return Ok(Left::Same),
            }
        } else {
            body.extend_from_slice(blob.get(at..after_value).ok_or(Refused::Malformed)?);
        }
        at = after_value;
    }
    if !found {
        if !matches!(writing.edit, Edit::Insert | Edit::Set | Edit::ArrayInsert) {
            return Ok(Left::Same);
        }
        // `json_array_insert` writes a new pair only where the rest of
        // the path names an element of an array.
        if writing.edit == Edit::ArrayInsert && rest.last() != Some(&b']') {
            return Err(Refused::NotArray(Vec::new()));
        }
        let Left::Held(held) = substructure(rest, writing, depth)? else {
            return Ok(Left::Same);
        };
        append(&mut body, if raw { TEXTRAW } else { TEXT5 }, &key);
        body.extend_from_slice(&held);
    }
    let mut out = Vec::new();
    append(&mut out, OBJECT, &body);
    Ok(Left::Held(out))
}

/// The array at `at` after the edit at `path`.
fn edited_array(
    blob: &[u8],
    at: usize,
    path: &[u8],
    writing: &Writing<'_>,
    depth: usize,
) -> Result<Left, Refused> {
    let inner = Writing {
        edit: writing.edit,
        value: writing.value,
        indexed: true,
    };
    let (kind, start, length) = element(blob, at).ok_or(Refused::Malformed)?;
    let end = start.saturating_add(length);
    let (count, rest) = indexed(path, blob, at)
        .ok()
        .ok_or(Refused::Path(Vec::new()))?;
    let mut body = Vec::new();
    let mut place = start;
    let mut left = count;
    let mut found = false;
    while place < end {
        let next = after(blob, place).ok_or(Refused::Malformed)?;
        if left == Some(0) {
            found = true;
            match edited(blob, place, rest, &inner, depth.saturating_add(1))? {
                Left::Held(held) => body.extend_from_slice(&held),
                Left::Gone => {}
                Left::Same => return Ok(Left::Same),
            }
            // The element was found, so no later one is it.
            left = None;
        } else {
            body.extend_from_slice(blob.get(place..next).ok_or(Refused::Malformed)?);
            left = left.map(|held| held.saturating_sub(1));
        }
        place = next;
    }
    // A number that counts to the end of the array writes a new
    // element there, which is what `jsonLookupStep` does where the
    // walk ran out.
    if !found {
        if left != Some(0) || !matches!(writing.edit, Edit::Insert | Edit::Set | Edit::ArrayInsert)
        {
            return Ok(Left::Same);
        }
        let Left::Held(held) = substructure(rest, writing, depth)? else {
            return Ok(Left::Same);
        };
        body.extend_from_slice(&held);
    }
    let _ = kind;
    let mut out = Vec::new();
    append(&mut out, ARRAY, &body);
    Ok(Left::Held(out))
}

/// The value written at the end of a path that runs deeper than the
/// value goes, which is `jsonCreateEditSubstructure`: the steps left
/// over make objects and arrays to hold it.
fn substructure(tail: &[u8], writing: &Writing<'_>, depth: usize) -> Result<Left, Refused> {
    if tail.is_empty() {
        return Ok(Left::Held(writing.value.to_vec()));
    }
    let kind = if tail.first() == Some(&b'.') {
        OBJECT
    } else {
        ARRAY
    };
    let mut blob = Vec::new();
    append(&mut blob, kind, &[]);
    edited(&blob, 0, tail, writing, depth.saturating_add(1))
}

/// `json_remove(X, P, ...)`, `json_insert`, `json_replace` and
/// `json_set`: the document with each path written in turn.
///
/// `json_remove` takes one path per argument and the other three take
/// a path and a value per pair of arguments.
///
/// # Errors
///
/// [`Refused`] names what the value or a path could not be read as.
pub fn changed(args: &[Value], carried: &[bool], edit: Edit) -> Result<Option<Vec<u8>>, Refused> {
    let mut blob = document(args.first().unwrap_or(&Value::Null))?;
    let step = if edit == Edit::Remove { 1 } else { 2 };
    // A document and a pair per path make an odd number of arguments.
    if edit != Edit::Remove && args.len().is_multiple_of(2) {
        return Err(Refused::Odd(match edit {
            Edit::Insert => b"insert".to_vec(),
            Edit::Replace => b"replace".to_vec(),
            Edit::ArrayInsert => b"array_insert".to_vec(),
            _ => b"set".to_vec(),
        }));
    }
    let mut at = 1_usize;
    while at < args.len() {
        let path = args
            .get(at)
            .and_then(Value::text)
            .ok_or(Refused::Path(Vec::new()))?;
        if path.first() != Some(&b'$') {
            return Err(Refused::Path(path));
        }
        let value = if edit == Edit::Remove {
            Vec::new()
        } else {
            let place = at.saturating_add(1);
            let given = args.get(place).unwrap_or(&Value::Null);
            written(given, carried.get(place).copied().unwrap_or(false))?
        };
        let rest = path.get(1..).unwrap_or_default();
        let writing = Writing {
            edit,
            value: &value,
            indexed: false,
        };
        let held = edited(&blob, 0, rest, &writing, 0).map_err(|refused| match refused {
            Refused::Path(_) => Refused::Path(path.clone()),
            Refused::NotArray(_) => Refused::NotArray(path.clone()),
            other => other,
        })?;
        blob = match held {
            Left::Held(held) => held,
            // A `json_remove` that names the whole document answers
            // nothing, which is what the C library answers for a blob
            // it has emptied.
            Left::Gone => return Ok(None),
            Left::Same => blob,
        };
        at = at.saturating_add(step);
    }
    Ok(Some(blob))
}

/// `json_patch(T, P)`: the target with the patch merged into it, which
/// is the `MergePatch` of RFC 7396.
///
/// Merging costs O(n·m) in the pairs of the two objects.
///
/// # Errors
///
/// [`Refused`] names what either value could not be read as.
pub fn patched(args: &[Value]) -> Result<Vec<u8>, Refused> {
    let target = document(args.first().unwrap_or(&Value::Null))?;
    let patch = document(args.get(1).unwrap_or(&Value::Null))?;
    merge(&target, 0, &patch, 0, 0)
}

/// The target merged with the patch, which is `jsonMergePatch`.
///
/// The pairs of the patch are read in the order they were written, so
/// a label the patch holds twice is read twice and the later one wins.
fn merge(
    target: &[u8],
    at: usize,
    patch: &[u8],
    from: usize,
    depth: usize,
) -> Result<Vec<u8>, Refused> {
    if depth >= DEPTH {
        return Err(Refused::Nested);
    }
    let (kind, start, length) = element(patch, from).ok_or(Refused::Malformed)?;
    let end = start.saturating_add(length);
    if kind != OBJECT {
        return held_bytes(patch, from);
    }
    // A target that is no object is read as an empty one, which is
    // line 06 of the algorithm.
    let mut pairs = pairs_of(target, at)?;
    let mut place = start;
    while place < end {
        let (label, value_at, next) = pair(patch, place)?;
        place = next;
        let (value_kind, _, _) = element(patch, value_at).ok_or(Refused::Malformed)?;
        let held = at_label(&pairs, patch, label)?;
        if value_kind == NULL {
            if let Some(held) = held {
                pairs.remove(held);
            }
            continue;
        }
        let label_bytes = patch.get(label.0..label.1).ok_or(Refused::Malformed)?;
        match held {
            Some(held) => {
                let was = pairs
                    .get(held)
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default();
                let merged = merge(&was, 0, patch, value_at, depth.saturating_add(1))?;
                for slot in pairs.iter_mut().skip(held).take(1) {
                    slot.1.clone_from(&merged);
                }
            }
            None if value_kind == OBJECT => {
                let mut empty = Vec::new();
                append(&mut empty, OBJECT, &[]);
                let merged = merge(&empty, 0, patch, value_at, depth.saturating_add(1))?;
                pairs.push((label_bytes.to_vec(), merged));
            }
            None => pairs.push((label_bytes.to_vec(), held_bytes(patch, value_at)?)),
        }
    }
    let mut body = Vec::new();
    for (label, value) in pairs {
        body.extend_from_slice(&label);
        body.extend_from_slice(&value);
    }
    let mut out = Vec::new();
    append(&mut out, OBJECT, &body);
    Ok(out)
}

/// The bytes of the whole element at `at`.
fn held_bytes(blob: &[u8], at: usize) -> Result<Vec<u8>, Refused> {
    let end = after(blob, at).ok_or(Refused::Malformed)?;
    Ok(blob.get(at..end).ok_or(Refused::Malformed)?.to_vec())
}

/// One pair of an object: the bytes of its label and the bytes of its
/// value, each a whole element.
type Pair = (Vec<u8>, Vec<u8>);

/// The pairs of the object at `at`, each as the bytes of its label and
/// the bytes of its value, and none where the element is no object.
fn pairs_of(blob: &[u8], at: usize) -> Result<Vec<Pair>, Refused> {
    let mut out = Vec::new();
    let (kind, start, length) = element(blob, at).ok_or(Refused::Malformed)?;
    if kind != OBJECT {
        return Ok(out);
    }
    let end = start.saturating_add(length);
    let mut place = start;
    while place < end {
        let (label, value_at, next) = pair(blob, place)?;
        out.push((
            blob.get(label.0..label.1)
                .ok_or(Refused::Malformed)?
                .to_vec(),
            blob.get(value_at..next).ok_or(Refused::Malformed)?.to_vec(),
        ));
        place = next;
    }
    Ok(out)
}

/// Which pair carries the label the other blob holds, and nothing
/// where none does.
fn at_label(pairs: &[Pair], other: &[u8], label: (usize, usize)) -> Result<Option<usize>, Refused> {
    let (kind, text, width) = element(other, label.0).ok_or(Refused::Malformed)?;
    let key = other
        .get(text..text.saturating_add(width))
        .ok_or(Refused::Malformed)?;
    let raw = kind == TEXT || kind == TEXTRAW;
    for (at, (held, _)) in pairs.iter().enumerate() {
        let (held_kind, held_text, held_width) = element(held, 0).ok_or(Refused::Malformed)?;
        let name = held
            .get(held_text..held_text.saturating_add(held_width))
            .ok_or(Refused::Malformed)?;
        if same_label(key, raw, name, held_kind == TEXT || held_kind == TEXTRAW) {
            return Ok(Some(at));
        }
    }
    Ok(None)
}

/// The label of the pair at `at`, where its value begins, and where
/// the pair after it begins.
fn pair(blob: &[u8], at: usize) -> Result<((usize, usize), usize, usize), Refused> {
    let value_at = after(blob, at).ok_or(Refused::Malformed)?;
    let next = after(blob, value_at).ok_or(Refused::Malformed)?;
    Ok(((at, value_at), value_at, next))
}

impl Refused {
    /// The text the C library writes for this refusal, which is what a
    /// `catchsql` of SQLite's own test files compares.
    #[must_use]
    pub fn message(&self) -> alloc::string::String {
        use alloc::string::ToString as _;
        let shown = |text: &[u8]| alloc::string::String::from_utf8_lossy(text).into_owned();
        match self {
            Refused::Malformed => "malformed JSON".to_string(),
            // `%Q` of the C library writes the text in single quotes
            // with every quote inside it doubled.
            Refused::Path(path) => alloc::format!("bad JSON path: '{}'", shown(&doubled(path))),
            Refused::NotArray(path) => {
                alloc::format!("not an array element: '{}'", shown(&doubled(path)))
            }
            Refused::Deep => "JSON path too deep".to_string(),
            Refused::Nested => "JSON nested too deep".to_string(),
            Refused::Blob => "JSON cannot hold BLOB values".to_string(),
            Refused::Labels => "json_object() labels must be TEXT".to_string(),
            Refused::Pairs => "json_object() requires an even number of arguments".to_string(),
            Refused::Odd(name) => {
                alloc::format!("json_{}() needs an odd number of arguments", shown(name))
            }
            Refused::Flags => {
                "FLAGS parameter to json_valid() must be between 1 and 15".to_string()
            }
        }
    }
}

/// Text with every quote in it doubled, which is what `%Q` writes
/// inside the quotes it puts around the text.
fn doubled(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for byte in text {
        if *byte == b'\'' {
            out.push(b'\'');
        }
        out.push(*byte);
    }
    out
}

/// `X -> P` and `X ->> P`: the value the path names, as JSON for the
/// first and as SQL holds it for the second.
///
/// The path may be written short, which is `JSON_ABPATH`: a number
/// names an element of an array, a word names a label, and anything
/// else is a label written with quotes around it.
///
/// # Errors
///
/// [`Refused`] names what the value or the path could not be read as.
pub fn arrow(value: &Value, path: &Value, as_text: bool) -> Result<(Value, bool), Refused> {
    let blob = document(value)?;
    let written = path.text().ok_or(Refused::Path(Vec::new()))?;
    let whole = if written.first() == Some(&b'$') {
        written
    } else {
        short_path(path, &written)
    };
    let Some(found) = at_path(&blob, &whole)? else {
        return Ok((Value::Null, false));
    };
    if as_text {
        let (held, _) = value_of(&blob, found)?;
        return Ok((held, false));
    }
    Ok((Value::Text(text_of(&blob, found)?), true))
}

/// A path written short, written out whole.
fn short_path(value: &Value, written: &[u8]) -> Vec<u8> {
    let mut out = b"$".to_vec();
    if matches!(value, Value::Int(_)) {
        out.push(b'[');
        // A number below nought counts from the end of the array.
        if written.first() == Some(&b'-') {
            out.push(b'#');
        }
        out.extend_from_slice(written);
        out.push(b']');
        return out;
    }
    if written
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        out.push(b'.');
        out.extend_from_slice(written);
        return out;
    }
    if written.first() == Some(&b'[') && written.len() >= 3 && written.last() == Some(&b']') {
        out.extend_from_slice(written);
        return out;
    }
    out.extend_from_slice(b".\"");
    out.extend_from_slice(written);
    out.push(b'"');
    out
}

/// `jsonb_extract(X, P, ...)`: the value at each path, as the binary
/// form where it is an array or an object.
///
/// # Errors
///
/// [`Refused`] names what the value or a path could not be read as.
pub fn extracted_blob(args: &[Value]) -> Result<Value, Refused> {
    let blob = document(args.first().unwrap_or(&Value::Null))?;
    let paths = args.get(1..).unwrap_or_default();
    if let [one] = paths {
        let path = one.text().ok_or(Refused::Path(Vec::new()))?;
        return match at_path(&blob, &path)? {
            Some(found) => binary_of(&blob, found),
            None => Ok(Value::Null),
        };
    }
    let mut body = Vec::new();
    for one in paths {
        let path = one.text().ok_or(Refused::Path(Vec::new()))?;
        match at_path(&blob, &path)? {
            Some(found) => {
                let end = after(&blob, found).ok_or(Refused::Malformed)?;
                body.extend_from_slice(blob.get(found..end).ok_or(Refused::Malformed)?);
            }
            None => append(&mut body, NULL, &[]),
        }
    }
    let mut out = Vec::new();
    append(&mut out, ARRAY, &body);
    Ok(Value::Blob(out))
}

/// The element at `at` as a blob where it is an array or an object,
/// and as the value SQL holds otherwise.
fn binary_of(blob: &[u8], at: usize) -> Result<Value, Refused> {
    let (kind, _, _) = element(blob, at).ok_or(Refused::Malformed)?;
    if kind == ARRAY || kind == OBJECT {
        let end = after(blob, at).ok_or(Refused::Malformed)?;
        let held = blob.get(at..end).ok_or(Refused::Malformed)?;
        return Ok(Value::Blob(held.to_vec()));
    }
    let (value, _) = value_of(blob, at)?;
    Ok(value)
}
