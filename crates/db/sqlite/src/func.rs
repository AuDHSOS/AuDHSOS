// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The scalar functions, and the pattern matching `LIKE` and `GLOB` are.
//!
//! `src/func.c` is the document. What the prose leaves out is where the
//! answers are: `substr` counts characters for text and bytes for a blob
//! and has four rules for a start before the first character; `length`
//! counts characters and `octet_length` bytes; `instr` compares blobs as
//! blobs and everything else as text. Each is the routine it is named
//! after.
//!
//! What is not here: the functions that read a clock or a random source,
//! the ones that answer something about the connection, and the
//! mathematical ones, which want a library this repository does not
//! have. Each refuses by name. `format` and its whole format language
//! are in [`crate::format`].

use alloc::vec::Vec;

use crate::eval::Error;
use crate::fp;
use crate::header::Encoding;
use crate::number;
use crate::utf8;
use crate::value::{Collation, Value, apply_numeric, compare, stored};

/// A function this engine has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    /// `abs(X)`.
    Abs,
    /// `ceil(X)` and `ceiling(X)`.
    Ceil,
    /// `char(...)`.
    Char,
    /// `degrees(X)`.
    Degrees,
    /// `floor(X)`.
    Floor,
    /// `format(F,...)` and `printf(F,...)`.
    Format,
    /// `coalesce(X,Y,...)` and `ifnull(X,Y)`.
    Coalesce,
    /// `concat(...)`.
    Concat,
    /// `concat_ws(S,...)`.
    ConcatWs,
    /// `glob(P,X)`, and the operator.
    Glob,
    /// `hex(X)`.
    Hex,
    /// `iif(...)` and `if(...)`.
    Iif,
    /// `instr(X,Y)`.
    Instr,
    /// `pi()`.
    Pi,
    /// `radians(X)`.
    Radians,
    /// `trunc(X)`.
    Trunc,
    /// `zeroblob(N)`.
    Zeroblob,
    /// `random()`.
    Random,
    /// `randomblob(N)`.
    Randomblob,
    /// `changes()`, `total_changes()` and `last_insert_rowid()`, each
    /// of which is nought for a connection that has written nothing.
    Written,
    /// `mod(X,Y)`.
    Modulo,
    /// `length(X)`.
    Length,
    /// `like(P,X)` and `like(P,X,E)`, and the operator.
    Like,
    /// `lower(X)`.
    Lower,
    /// `ltrim(X)` and `ltrim(X,Y)`.
    Ltrim,
    /// `max(X,Y,...)`.
    Max,
    /// `min(X,Y,...)`.
    Min,
    /// `nullif(X,Y)`.
    Nullif,
    /// `octet_length(X)`.
    OctetLength,
    /// `quote(X)`.
    Quote,
    /// `replace(X,Y,Z)`.
    Replace,
    /// `round(X)` and `round(X,Y)`.
    Round,
    /// `rtrim(X)` and `rtrim(X,Y)`.
    Rtrim,
    /// `sign(X)`.
    Sign,
    /// `substr(X,Y)` and `substr(X,Y,Z)`, and `substring`.
    Substr,
    /// `trim(X)` and `trim(X,Y)`.
    Trim,
    /// `typeof(X)`.
    Typeof,
    /// `unhex(X)` and `unhex(X,Y)`.
    Unhex,
    /// `unicode(X)`.
    Unicode,
    /// `unistr(X)`.
    Unistr,
    /// `unistr_quote(X)`.
    UnistrQuote,
    /// `likely(X)`, `unlikely(X)` and `likelihood(X,Y)`, which answer
    /// their first argument and tell the planner what to expect.
    Unlikely,
    /// `upper(X)`.
    Upper,
}

/// One row of the table: a name, how many arguments it takes, and which
/// function it is. A count of `None` is any number at or above `least`.
struct Entry {
    /// The name, in lower case.
    name: &'static [u8],
    /// The fewest arguments it takes.
    least: usize,
    /// The most, where there is a most.
    most: Option<usize>,
    /// Which function.
    function: Function,
}

/// What a radian is in degrees, which is the constant `radToDeg`
/// multiplies by.
const DEGREES: f64 = 180.0 / core::f64::consts::PI;

/// What a degree is in radians, which is the constant `degToRad`
/// multiplies by.
const RADIANS: f64 = core::f64::consts::PI / 180.0;

/// The longest blob a value holds, which is `SQLITE_MAX_LENGTH`.
pub const MAX_LENGTH: usize = 1_000_000_000;

/// The table, which is `aBuiltinFunc` for what is written here.
const TABLE: &[Entry] = &[
    Entry {
        name: b"abs",
        least: 1,
        most: Some(1),
        function: Function::Abs,
    },
    Entry {
        name: b"ceil",
        least: 1,
        most: Some(1),
        function: Function::Ceil,
    },
    Entry {
        name: b"ceiling",
        least: 1,
        most: Some(1),
        function: Function::Ceil,
    },
    Entry {
        name: b"char",
        least: 0,
        most: None,
        function: Function::Char,
    },
    Entry {
        name: b"degrees",
        least: 1,
        most: Some(1),
        function: Function::Degrees,
    },
    Entry {
        name: b"floor",
        least: 1,
        most: Some(1),
        function: Function::Floor,
    },
    Entry {
        name: b"format",
        least: 0,
        most: None,
        function: Function::Format,
    },
    Entry {
        name: b"printf",
        least: 0,
        most: None,
        function: Function::Format,
    },
    Entry {
        name: b"coalesce",
        least: 2,
        most: None,
        function: Function::Coalesce,
    },
    Entry {
        name: b"concat",
        least: 1,
        most: None,
        function: Function::Concat,
    },
    Entry {
        name: b"concat_ws",
        least: 2,
        most: None,
        function: Function::ConcatWs,
    },
    Entry {
        name: b"glob",
        least: 2,
        most: Some(2),
        function: Function::Glob,
    },
    Entry {
        name: b"hex",
        least: 1,
        most: Some(1),
        function: Function::Hex,
    },
    Entry {
        name: b"if",
        least: 2,
        most: None,
        function: Function::Iif,
    },
    Entry {
        name: b"ifnull",
        least: 2,
        most: Some(2),
        function: Function::Coalesce,
    },
    Entry {
        name: b"iif",
        least: 2,
        most: None,
        function: Function::Iif,
    },
    Entry {
        name: b"instr",
        least: 2,
        most: Some(2),
        function: Function::Instr,
    },
    Entry {
        name: b"length",
        least: 1,
        most: Some(1),
        function: Function::Length,
    },
    Entry {
        name: b"like",
        least: 2,
        most: Some(3),
        function: Function::Like,
    },
    Entry {
        name: b"likelihood",
        least: 2,
        most: Some(2),
        function: Function::Unlikely,
    },
    Entry {
        name: b"likely",
        least: 1,
        most: Some(1),
        function: Function::Unlikely,
    },
    Entry {
        name: b"lower",
        least: 1,
        most: Some(1),
        function: Function::Lower,
    },
    Entry {
        name: b"ltrim",
        least: 1,
        most: Some(2),
        function: Function::Ltrim,
    },
    Entry {
        name: b"max",
        least: 1,
        most: None,
        function: Function::Max,
    },
    Entry {
        name: b"min",
        least: 1,
        most: None,
        function: Function::Min,
    },
    Entry {
        name: b"nullif",
        least: 2,
        most: Some(2),
        function: Function::Nullif,
    },
    Entry {
        name: b"octet_length",
        least: 1,
        most: Some(1),
        function: Function::OctetLength,
    },
    Entry {
        name: b"quote",
        least: 1,
        most: Some(1),
        function: Function::Quote,
    },
    Entry {
        name: b"unistr",
        least: 1,
        most: Some(1),
        function: Function::Unistr,
    },
    Entry {
        name: b"unistr_quote",
        least: 1,
        most: Some(1),
        function: Function::UnistrQuote,
    },
    Entry {
        name: b"replace",
        least: 3,
        most: Some(3),
        function: Function::Replace,
    },
    Entry {
        name: b"round",
        least: 1,
        most: Some(2),
        function: Function::Round,
    },
    Entry {
        name: b"rtrim",
        least: 1,
        most: Some(2),
        function: Function::Rtrim,
    },
    Entry {
        name: b"changes",
        least: 0,
        most: Some(0),
        function: Function::Written,
    },
    Entry {
        name: b"last_insert_rowid",
        least: 0,
        most: Some(0),
        function: Function::Written,
    },
    Entry {
        name: b"mod",
        least: 2,
        most: Some(2),
        function: Function::Modulo,
    },
    Entry {
        name: b"pi",
        least: 0,
        most: Some(0),
        function: Function::Pi,
    },
    Entry {
        name: b"radians",
        least: 1,
        most: Some(1),
        function: Function::Radians,
    },
    Entry {
        name: b"sign",
        least: 1,
        most: Some(1),
        function: Function::Sign,
    },
    Entry {
        name: b"substr",
        least: 2,
        most: Some(3),
        function: Function::Substr,
    },
    Entry {
        name: b"substring",
        least: 2,
        most: Some(3),
        function: Function::Substr,
    },
    Entry {
        name: b"trim",
        least: 1,
        most: Some(2),
        function: Function::Trim,
    },
    Entry {
        name: b"typeof",
        least: 1,
        most: Some(1),
        function: Function::Typeof,
    },
    Entry {
        name: b"unhex",
        least: 1,
        most: Some(2),
        function: Function::Unhex,
    },
    Entry {
        name: b"unicode",
        least: 1,
        most: Some(1),
        function: Function::Unicode,
    },
    Entry {
        name: b"unlikely",
        least: 1,
        most: Some(1),
        function: Function::Unlikely,
    },
    Entry {
        name: b"total_changes",
        least: 0,
        most: Some(0),
        function: Function::Written,
    },
    Entry {
        name: b"trunc",
        least: 1,
        most: Some(1),
        function: Function::Trunc,
    },
    Entry {
        name: b"upper",
        least: 1,
        most: Some(1),
        function: Function::Upper,
    },
    Entry {
        name: b"random",
        least: 0,
        most: Some(0),
        function: Function::Random,
    },
    Entry {
        name: b"randomblob",
        least: 1,
        most: Some(1),
        function: Function::Randomblob,
    },
    Entry {
        name: b"zeroblob",
        least: 1,
        most: Some(1),
        function: Function::Zeroblob,
    },
];

/// The function `name` names, taking `count` arguments.
///
/// # Errors
///
/// [`Error::NoFunction`] where no function has the name, and
/// [`Error::WrongArguments`] where one does and takes another number.
pub fn lookup(name: &[u8], count: usize) -> Result<Function, Error> {
    let mut found = false;
    for entry in TABLE {
        if !name.eq_ignore_ascii_case(entry.name) {
            continue;
        }
        found = true;
        if count >= entry.least && entry.most.is_none_or(|most| count <= most) {
            return Ok(entry.function);
        }
    }
    if found {
        Err(Error::WrongArguments)
    } else {
        Err(Error::NoFunction)
    }
}

/// What `function` answers for `args`, under `collation` where it
/// compares.
///
/// # Errors
///
/// [`Error`] names what it could not answer and why.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per function, each of them short, kept in one table so that the list reads as a list"
)]
pub fn call(
    function: Function,
    args: &[Value],
    collation: Collation,
    encoding: Encoding,
    random: Option<&crate::random::Source>,
) -> Result<Value, Error> {
    let arg = |at: usize| args.get(at).cloned().unwrap_or(Value::Null);
    let first = arg(0);
    Ok(match function {
        Function::Typeof => Value::Text(type_name(&first).to_vec()),
        Function::Length => match &first {
            Value::Null => Value::Null,
            Value::Text(bytes) => Value::Int(count_of(utf8::count(bytes))),
            other => Value::Int(count_of(other.text().unwrap_or_default().len())),
        },
        Function::OctetLength => match &first {
            Value::Null => Value::Null,
            // Bytes as the database holds them, not as this engine does.
            Value::Blob(bytes) => Value::Int(count_of(bytes.len())),
            other => Value::Int(count_of(
                stored(&other.text().unwrap_or_default(), encoding).len(),
            )),
        },
        Function::Pi => Value::Real(core::f64::consts::PI),
        Function::Format => crate::format::format(args)?,
        // This engine reads and does not write, so no statement of it
        // has ever changed a row or made a rowid. Q7 of document 16 is
        // where these stop being nought.
        Function::Written => Value::Int(0),
        // `math2Func`: either value not a number after the numeric
        // affinity answers nothing, and a remainder that is not a number
        // answers nothing as well.
        Function::Modulo => match (numeric(first), numeric(arg(1))) {
            (Some(left), Some(right)) => real(remainder(left, right)),
            _ => Value::Null,
        },
        // `math1Func`: a value that is not a number after the numeric
        // affinity is one the function answers nothing for.
        Function::Degrees => {
            numeric(first).map_or(Value::Null, |number| Value::Real(number * DEGREES))
        }
        Function::Radians => {
            numeric(first).map_or(Value::Null, |number| Value::Real(number * RADIANS))
        }
        // `ceilingFunc`: an integer is answered as it stands, because
        // rounding it changes nothing and would lose its width.
        Function::Ceil | Function::Floor | Function::Trunc => {
            let mut value = first;
            apply_numeric(&mut value, false);
            match value {
                Value::Int(number) => Value::Int(number),
                Value::Real(number) => Value::Real(match function {
                    Function::Ceil => ceiling(number),
                    Function::Floor => flooring(number),
                    _ => truncated(number),
                }),
                Value::Null | Value::Text(_) | Value::Blob(_) => Value::Null,
            }
        }
        Function::Random => {
            let source = random.ok_or(Error::NoRandom)?;
            Value::Int(i64::from_ne_bytes(source.word().to_ne_bytes()))
        }
        Function::Randomblob => {
            let source = random.ok_or(Error::NoRandom)?;
            // `randomBlob` answers one byte where the count is less
            // than one, which `randomblob(NULL)` is.
            let count = first.to_integer().max(1);
            let count = usize::try_from(count).map_err(|_| Error::TooBig)?;
            if count > MAX_LENGTH {
                return Err(Error::TooBig);
            }
            Value::Blob(source.bytes(count))
        }
        Function::Zeroblob => {
            // `sqlite3_value_int64` of a `NULL` is nought, so a blob of
            // no bytes is what `zeroblob(NULL)` answers.
            let count = first.to_integer().max(0);
            let count = usize::try_from(count).map_err(|_| Error::TooBig)?;
            if count > MAX_LENGTH {
                return Err(Error::TooBig);
            }
            Value::Blob(alloc::vec![0u8; count])
        }
        Function::Abs => match first {
            Value::Null => Value::Null,
            Value::Int(number) => Value::Int(number.checked_abs().ok_or(Error::Overflow)?),
            other => Value::Real(other.to_real().abs()),
        },
        Function::Sign => {
            let mut value = first;
            apply_numeric(&mut value, false);
            match value {
                Value::Int(_) | Value::Real(_) => {
                    let number = value.to_real();
                    Value::Int(if number < 0.0 {
                        -1
                    } else {
                        i64::from(number > 0.0)
                    })
                }
                _ => Value::Null,
            }
        }
        Function::Coalesce => args
            .iter()
            .find(|value| **value != Value::Null)
            .cloned()
            .unwrap_or(Value::Null),
        Function::Iif => {
            // `iif(a,b,c,d,e)` is `CASE WHEN a THEN b WHEN c THEN d ELSE
            // e END`, and with an even number of arguments there is no
            // `ELSE`.
            let mut at = 0;
            loop {
                let Some(condition) = args.get(at) else {
                    break Value::Null;
                };
                let Some(result) = args.get(at.saturating_add(1)) else {
                    break condition.clone();
                };
                if condition.truth(false) {
                    break result.clone();
                }
                at = at.saturating_add(2);
            }
        }
        Function::Unlikely => first,
        Function::Nullif => {
            if compare(&first, &arg(1), collation) == core::cmp::Ordering::Equal {
                Value::Null
            } else {
                first
            }
        }
        Function::Min | Function::Max => {
            let wants_greater = function == Function::Max;
            let mut best = first;
            for value in args.iter().skip(1) {
                if best == Value::Null || *value == Value::Null {
                    return Ok(Value::Null);
                }
                // `min` takes the later of two that compare equal and
                // `max` keeps the earlier, which is what the mask in
                // `minmaxFunc` comes to.
                let order = compare(&best, value, collation);
                let take = if wants_greater {
                    order == core::cmp::Ordering::Less
                } else {
                    order != core::cmp::Ordering::Less
                };
                if take {
                    best = value.clone();
                }
            }
            best
        }
        Function::Lower | Function::Upper => match first.text() {
            None => Value::Null,
            Some(bytes) => Value::Text(
                bytes
                    .iter()
                    .map(|byte| {
                        if function == Function::Lower {
                            byte.to_ascii_lowercase()
                        } else {
                            byte.to_ascii_uppercase()
                        }
                    })
                    .collect(),
            ),
        },
        Function::Trim | Function::Ltrim | Function::Rtrim => {
            let left = function != Function::Rtrim;
            let right = function != Function::Ltrim;
            match (first.text(), args.len()) {
                (None, _) => Value::Null,
                (Some(bytes), 1) => Value::Text(trim(&bytes, b" ", left, right)),
                (Some(bytes), _) => match arg(1).text() {
                    None => Value::Null,
                    Some(set) => Value::Text(trim(&bytes, &set, left, right)),
                },
            }
        }
        Function::Replace => replace(&first, &arg(1), &arg(2)),
        Function::Instr => instr(&first, &arg(1)),
        Function::Substr => substr(&first, &arg(1), args.get(2)),
        Function::Hex => match first {
            Value::Null => Value::Text(Vec::new()),
            other => {
                let mut out = Vec::new();
                let bytes = match &other {
                    Value::Blob(bytes) => bytes.clone(),
                    _ => stored(&other.text().unwrap_or_default(), encoding),
                };
                for byte in bytes {
                    out.push(hex_digit(byte >> 4));
                    out.push(hex_digit(byte & 0x0f));
                }
                Value::Text(out)
            }
        },
        Function::Unhex => unhex(&first, args.get(1)),
        Function::Char => {
            let mut out = Vec::new();
            for value in args {
                let point = value.to_integer();
                let point = if (0..=0x10_ffff).contains(&point) {
                    u32::try_from(point).unwrap_or(utf8::REPLACEMENT)
                } else {
                    utf8::REPLACEMENT
                };
                utf8::write(&mut out, point & 0x1f_ffff);
            }
            Value::Text(out)
        }
        Function::Unicode => match first.text() {
            Some(bytes) if bytes.first().is_some_and(|byte| *byte != 0) => {
                Value::Int(i64::from(utf8::read(&bytes, 0).0))
            }
            _ => Value::Null,
        },
        Function::Quote => Value::Text(quote(&first, false)?),
        Function::UnistrQuote => Value::Text(quote(&first, true)?),
        // A value with no text answers nothing, and a `\` that names no
        // character refuses.
        Function::Unistr => match first.text() {
            Some(text) => Value::Text(crate::format::unistr(&text)?),
            None => Value::Null,
        },
        Function::Round => round(&first, args.get(1)),
        Function::Concat => {
            let mut out = Vec::new();
            for value in args {
                out.extend(value.text().unwrap_or_default());
            }
            Value::Text(out)
        }
        Function::ConcatWs => match first.text() {
            None => Value::Null,
            Some(separator) => {
                let mut out = Vec::new();
                let mut written = false;
                for value in args.iter().skip(1) {
                    let Some(bytes) = value.text() else {
                        continue;
                    };
                    if written {
                        out.extend(separator.iter());
                    }
                    out.extend(bytes);
                    written = true;
                }
                Value::Text(out)
            }
        },
        Function::Like | Function::Glob => {
            return pattern(function, args);
        }
    })
}

/// The name `typeof` answers with.
const fn type_name(value: &Value) -> &'static [u8] {
    match value {
        Value::Null => b"null",
        Value::Int(_) => b"integer",
        Value::Real(_) => b"real",
        Value::Text(_) => b"text",
        Value::Blob(_) => b"blob",
    }
}

/// A count as the integer a function answers with.
fn count_of(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// A hex digit, upper case, as `hex` writes them.
const fn hex_digit(value: u8) -> u8 {
    if value < 10 {
        b'0'.saturating_add(value)
    } else {
        b'A'.saturating_add(value.saturating_sub(10))
    }
}

/// The characters of `set`, as the byte runs they are.
fn characters(set: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while set.get(at).is_some_and(|byte| *byte != 0) {
        let next = utf8::skip(set, at);
        out.push((at, next));
        at = next;
    }
    out
}

/// `bytes` with the characters of `set` taken off either end.
fn trim(bytes: &[u8], set: &[u8], left: bool, right: bool) -> Vec<u8> {
    let runs = characters(set);
    let mut start = 0;
    let mut end = bytes.len();
    if left {
        while let Some(run) = runs.iter().find(|(from, to)| {
            let len = to.saturating_sub(*from);
            len <= end.saturating_sub(start)
                && bytes.get(start..start.saturating_add(len)) == set.get(*from..*to)
        }) {
            start = start.saturating_add(run.1.saturating_sub(run.0));
        }
    }
    if right {
        while let Some(run) = runs.iter().find(|(from, to)| {
            let len = to.saturating_sub(*from);
            len <= end.saturating_sub(start)
                && bytes.get(end.saturating_sub(len)..end) == set.get(*from..*to)
        }) {
            end = end.saturating_sub(run.1.saturating_sub(run.0));
        }
    }
    bytes.get(start..end).unwrap_or_default().to_vec()
}

/// `replace(X,Y,Z)`, which works in bytes and answers text.
fn replace(subject: &Value, pattern: &Value, with: &Value) -> Value {
    let (Some(bytes), Some(needle)) = (subject.text(), pattern.text()) else {
        return Value::Null;
    };
    // An empty pattern answers before the replacement is even read.
    if needle.first().is_none_or(|byte| *byte == 0) {
        return Value::Text(bytes);
    }
    let Some(replacement) = with.text() else {
        return Value::Null;
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes.get(at..at.saturating_add(needle.len())) == Some(needle.as_slice()) {
            out.extend(replacement.iter());
            at = at.saturating_add(needle.len());
        } else {
            out.push(bytes.get(at).copied().unwrap_or(0));
            at = at.saturating_add(1);
        }
    }
    Value::Text(out)
}

/// `instr(X,Y)`, which counts characters for text and bytes for blobs.
fn instr(haystack: &Value, needle: &Value) -> Value {
    if *haystack == Value::Null || *needle == Value::Null {
        return Value::Null;
    }
    let blobs = matches!(haystack, Value::Blob(_)) && matches!(needle, Value::Blob(_));
    let (hay, pin) = (
        haystack.text().unwrap_or_default(),
        needle.text().unwrap_or_default(),
    );
    if pin.is_empty() {
        return Value::Int(1);
    }
    let mut at: usize = 0;
    let mut found = 1i64;
    while at.saturating_add(pin.len()) <= hay.len() {
        if hay.get(at..at.saturating_add(pin.len())) == Some(pin.as_slice()) {
            return Value::Int(found);
        }
        at = if blobs {
            at.saturating_add(1)
        } else {
            utf8::skip(&hay, at)
        };
        found = found.saturating_add(1);
    }
    Value::Int(0)
}

/// `substr(X,Y)` and `substr(X,Y,Z)`.
fn substr(subject: &Value, from: &Value, length: Option<&Value>) -> Value {
    if *subject == Value::Null {
        return Value::Null;
    }
    let blob = matches!(subject, Value::Blob(_));
    let bytes = subject.text().unwrap_or_default();
    // An empty blob has no pointer to read from, so SQLite answers
    // nothing rather than an empty blob.
    if blob && bytes.is_empty() {
        return Value::Null;
    }
    let mut start = from.to_integer();
    let mut count = match length {
        Some(Value::Null) => return Value::Null,
        Some(value) => value.to_integer(),
        // With no third argument the count is the largest a value may be.
        None => 1_000_000_000,
    };
    if start == 0 && *from == Value::Null {
        return Value::Null;
    }
    let characters = if start < 0 && !blob {
        count_of(utf8::count(&bytes))
    } else {
        count_of(bytes.len())
    };
    if start < 0 {
        start = start.saturating_add(characters);
        if start < 0 {
            if count < 0 {
                count = 0;
            } else {
                count = count.saturating_add(start);
            }
            start = 0;
        }
    } else if start > 0 {
        start = start.saturating_sub(1);
    } else if count > 0 {
        count = count.saturating_sub(1);
    }
    if count < 0 {
        if count < start.saturating_neg() {
            count = start;
        } else {
            count = count.saturating_neg();
        }
        start = start.saturating_sub(count);
    }
    let start = usize::try_from(start).unwrap_or(0);
    let count = usize::try_from(count).unwrap_or(0);
    if blob {
        let start = start.min(bytes.len());
        let end = start.saturating_add(count).min(bytes.len());
        return Value::Blob(bytes.get(start..end).unwrap_or_default().to_vec());
    }
    let mut at = 0;
    for _ in 0..start {
        if bytes.get(at).is_none_or(|byte| *byte == 0) {
            break;
        }
        at = utf8::skip(&bytes, at);
    }
    let mut end = at;
    for _ in 0..count {
        if bytes.get(end).is_none_or(|byte| *byte == 0) {
            break;
        }
        end = utf8::skip(&bytes, end);
    }
    Value::Text(bytes.get(at..end).unwrap_or_default().to_vec())
}

/// `unhex(X)` and `unhex(X,Y)`: the hex digits of `X` as bytes, with the
/// characters of `Y` allowed between them and nothing else.
fn unhex(subject: &Value, allowed: Option<&Value>) -> Value {
    let (Some(bytes), Some(pass)) = (
        subject.text(),
        match allowed {
            None => Some(Vec::new()),
            Some(value) => value.text(),
        },
    ) else {
        return Value::Null;
    };
    let mut out = Vec::new();
    let mut at = 0;
    while bytes.get(at).is_some_and(|byte| *byte != 0) {
        while bytes
            .get(at)
            .is_some_and(|byte| !byte.is_ascii_hexdigit() && *byte != 0)
        {
            let (character, next) = utf8::read(&bytes, at);
            if !characters(&pass)
                .iter()
                .any(|(from, _)| utf8::read(&pass, *from).0 == character)
            {
                return Value::Null;
            }
            at = next;
        }
        let Some(high) = bytes.get(at).copied().filter(u8::is_ascii_hexdigit) else {
            break;
        };
        let Some(low) = bytes
            .get(at.saturating_add(1))
            .copied()
            .filter(u8::is_ascii_hexdigit)
        else {
            return Value::Null;
        };
        out.push((nibble(high) << 4) | nibble(low));
        at = at.saturating_add(2);
    }
    Value::Blob(out)
}

/// A hex digit's value.
fn nibble(digit: u8) -> u8 {
    u8::try_from(char::from(digit).to_digit(16).unwrap_or(0)).unwrap_or(0)
}

/// `quote(X)`, which is what `sqlite3QuoteValue` writes. With `escapes`
/// a control character in text is written as the escape `unistr` reads,
/// which is `unistr_quote(X)`.
fn quote(value: &Value, escapes: bool) -> Result<Vec<u8>, Error> {
    Ok(match value {
        Value::Null => b"NULL".to_vec(),
        Value::Int(number) => number::integer_text(*number),
        Value::Real(number) => fp::quoted(*number),
        Value::Text(bytes) => crate::format::quoted_text(bytes, escapes)?,
        Value::Blob(bytes) => {
            let mut out = alloc::vec![b'X', b'\''];
            for byte in bytes {
                out.push(hex_digit(byte >> 4));
                out.push(hex_digit(byte & 0x0f));
            }
            out.push(b'\'');
            out
        }
    })
}

/// `round(X)` and `round(X,Y)`.
fn round(subject: &Value, decimals: Option<&Value>) -> Value {
    let places = match decimals {
        Some(Value::Null) => return Value::Null,
        Some(value) => value.to_integer().clamp(0, 30),
        None => 0,
    };
    if *subject == Value::Null {
        return Value::Null;
    }
    let number = subject.to_real();
    if !(-4_503_599_627_370_496.0..=4_503_599_627_370_496.0).contains(&number) {
        // Nothing below the point to round away.
        return Value::Real(number);
    }
    if places == 0 {
        let half = if number < 0.0 { -0.5 } else { 0.5 };
        return Value::Real(crate::value::integer_as_real(
            crate::value::real_as_integer(number + half),
        ));
    }
    let written = fp::fixed(number, i32::try_from(places).unwrap_or(0));
    Value::Real(number::real(&written).value)
}

/// The longest pattern `LIKE` or `GLOB` takes, which is
/// `SQLITE_LIMIT_LIKE_PATTERN_LENGTH`. It is what bounds how deep the
/// comparison recurses.
pub const MAX_PATTERN: usize = 50_000;

/// Which of the two pattern operators, which differ in their wildcards
/// and in whether case matters.
struct Pattern {
    /// The character that matches any run, `%` or `*`.
    many: u32,
    /// The character that matches one, `_` or `?`.
    one: u32,
    /// The character that opens a set, `[` for `GLOB` and none for
    /// `LIKE`.
    set: u32,
    /// Whether the twenty-six letters match either way.
    fold: bool,
}

/// `like(P,X[,E])` and `glob(P,X)`.
fn pattern(function: Function, args: &[Value]) -> Result<Value, Error> {
    let mut info = if function == Function::Glob {
        Pattern {
            many: u32::from(b'*'),
            one: u32::from(b'?'),
            set: u32::from(b'['),
            fold: false,
        }
    } else {
        Pattern {
            many: u32::from(b'%'),
            one: u32::from(b'_'),
            set: 0,
            fold: true,
        }
    };
    if args.first().and_then(Value::text).unwrap_or_default().len() > MAX_PATTERN {
        return Err(Error::PatternTooBig);
    }
    let mut other = info.set;
    if let Some(value) = args.get(2) {
        let Some(text) = value.text() else {
            return Ok(Value::Null);
        };
        if utf8::count(&text) != 1 {
            return Err(Error::BadEscape);
        }
        other = utf8::read(&text, 0).0;
        if other == info.many {
            info.many = 0;
        }
        if other == info.one {
            info.one = 0;
        }
    }
    let (Some(pattern), Some(subject)) = (
        args.first().and_then(Value::text),
        args.get(1).and_then(Value::text),
    ) else {
        return Ok(Value::Null);
    };
    Ok(Value::Int(i64::from(
        compare_pattern(&pattern, 0, &subject, 0, &info, other) == Match::Yes,
    )))
}

/// What one comparison of a pattern against a string came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Match {
    /// The pattern matches.
    Yes,
    /// It does not.
    No,
    /// It does not, and no run of characters ahead of it would help.
    NoWildcard,
}

/// `patternCompare`, which is the whole of `LIKE` and `GLOB`.
///
/// How deep it goes is bounded by the pattern, which is why the pattern
/// has a length limit: every step of the recursion spends a wildcard and
/// at least one character after it.
fn compare_pattern(
    pattern: &[u8],
    from: usize,
    subject: &[u8],
    at: usize,
    info: &Pattern,
    other: u32,
) -> Match {
    let mut from = from;
    let mut at = at;
    // One past the last character the escape made ordinary, which is
    // what keeps an escaped `_` from matching anything else.
    let mut escaped = usize::MAX;
    loop {
        let (read, next) = utf8::read(pattern, from);
        let mut c = read;
        from = next;
        if c == 0 {
            break;
        }
        if c == info.many {
            // Every way out of a wildcard is an answer: what follows it
            // is matched by the recursion rather than by this loop.
            return wildcard(pattern, &mut from, subject, &mut at, info, other);
        }
        if c == other {
            if info.set == 0 {
                let (escape, next) = utf8::read(pattern, from);
                from = next;
                if escape == 0 {
                    return Match::No;
                }
                escaped = from;
                c = escape;
            } else if set(pattern, &mut from, subject, &mut at) {
                continue;
            } else {
                return Match::No;
            }
        }
        let (c2, after) = utf8::read(subject, at);
        at = after;
        if c == c2 {
            continue;
        }
        if info.fold && c < 0x80 && c2 < 0x80 && fold(c) == fold(c2) {
            continue;
        }
        if c == info.one && from != escaped && c2 != 0 {
            continue;
        }
        return Match::No;
    }
    if subject.get(at).is_none_or(|byte| *byte == 0) {
        Match::Yes
    } else {
        Match::No
    }
}

/// A letter folded, which SQLite does for the twenty-six of them.
fn fold(character: u32) -> u32 {
    if (u32::from(b'A')..=u32::from(b'Z')).contains(&character) {
        character | 0x20
    } else {
        character
    }
}

/// A run of `%` or `*`, which is where the search branches.
fn wildcard(
    pattern: &[u8],
    from: &mut usize,
    subject: &[u8],
    at: &mut usize,
    info: &Pattern,
    other: u32,
) -> Match {
    let mut c;
    loop {
        let (next_c, next) = utf8::read(pattern, *from);
        c = next_c;
        if c != info.many && !(c == info.one && info.one != 0) {
            break;
        }
        *from = next;
        if c == info.one {
            let (read, after) = utf8::read(subject, *at);
            *at = after;
            if read == 0 {
                return Match::NoWildcard;
            }
        }
    }
    if c == 0 {
        return Match::Yes;
    }
    let after_c = utf8::read(pattern, *from).1;
    if c == other {
        if info.set == 0 {
            *from = after_c;
            let (escape, next) = utf8::read(pattern, *from);
            if escape == 0 {
                return Match::NoWildcard;
            }
            c = escape;
            *from = next;
        } else {
            // A set after the run: every place it could start is tried.
            while subject.get(*at).is_some_and(|byte| *byte != 0) {
                let answer = compare_pattern(pattern, *from, subject, *at, info, other);
                if answer != Match::No {
                    return answer;
                }
                *at = utf8::skip(subject, *at);
            }
            return Match::NoWildcard;
        }
    } else {
        *from = after_c;
    }
    // `c` is the first character past the run. Every place in the
    // subject where it appears is tried.
    while subject.get(*at).is_some_and(|byte| *byte != 0) {
        let (c2, after) = utf8::read(subject, *at);
        *at = after;
        if c2 == c || (info.fold && c < 0x80 && c2 < 0x80 && fold(c2) == fold(c)) {
            let answer = compare_pattern(pattern, *from, subject, *at, info, other);
            if answer != Match::No {
                return answer;
            }
        }
    }
    Match::NoWildcard
}

/// `[...]`, which only `GLOB` has.
fn set(pattern: &[u8], from: &mut usize, subject: &[u8], at: &mut usize) -> bool {
    let (c, after) = utf8::read(subject, *at);
    *at = after;
    if c == 0 {
        return false;
    }
    let mut prior = 0;
    let mut seen = false;
    let mut invert = false;
    let (mut c2, mut next) = utf8::read(pattern, *from);
    *from = next;
    if c2 == u32::from(b'^') {
        invert = true;
        (c2, next) = utf8::read(pattern, *from);
        *from = next;
    }
    if c2 == u32::from(b']') {
        if c == u32::from(b']') {
            seen = true;
        }
        (c2, next) = utf8::read(pattern, *from);
        *from = next;
    }
    while c2 != 0 && c2 != u32::from(b']') {
        let ahead = pattern.get(*from).copied().unwrap_or(0);
        if c2 == u32::from(b'-') && ahead != b']' && ahead != 0 && prior > 0 {
            (c2, next) = utf8::read(pattern, *from);
            *from = next;
            if c >= prior && c <= c2 {
                seen = true;
            }
            prior = 0;
        } else {
            if c == c2 {
                seen = true;
            }
            prior = c2;
        }
        (c2, next) = utf8::read(pattern, *from);
        *from = next;
    }
    c2 != 0 && seen != invert
}

/// A double as a value, which is nothing where the double is not a
/// number.
///
/// `sqlite3VdbeMemSetDouble` answers `NULL` for a NaN, so a function
/// whose answer is one answers nothing.
const fn real(number: f64) -> Value {
    if number.is_nan() {
        return Value::Null;
    }
    Value::Real(number)
}

/// A value as the double it is, where it is a number once the numeric
/// affinity has been applied to it.
///
/// This is `sqlite3_value_numeric_type` answering `SQLITE_INTEGER` or
/// `SQLITE_FLOAT`, which is what the math functions ask of an argument.
fn numeric(value: Value) -> Option<f64> {
    let mut value = value;
    apply_numeric(&mut value, false);
    match value {
        Value::Int(_) | Value::Real(_) => Some(value.to_real()),
        Value::Null | Value::Text(_) | Value::Blob(_) => None,
    }
}

/// A double with its fractional part dropped, which is `trunc`.
///
/// The bits are what is read rather than a conversion to an integer: a
/// double whose exponent is 52 or more is a whole number already, one
/// whose exponent is below zero is under one, and every other one is the
/// bits of its fraction masked off.
fn truncated(number: f64) -> f64 {
    let raw = number.to_bits();
    let exponent = i64::try_from((raw >> 52) & 0x7ff)
        .unwrap_or(0)
        .saturating_sub(1023);
    if exponent >= 52 {
        // A whole number already, or an infinity, or a NaN.
        return number;
    }
    if exponent < 0 {
        // Under one, so the whole part is a zero of the same sign.
        return f64::from_bits(raw & (1 << 63));
    }
    let shift = u32::try_from(52_i64.saturating_sub(exponent)).unwrap_or(52);
    let fraction = 1u64.checked_shl(shift).unwrap_or(0).wrapping_sub(1);
    f64::from_bits(raw & !fraction)
}

/// The smallest whole number that is not below `number`, which is
/// `ceil`.
fn ceiling(number: f64) -> f64 {
    let whole = truncated(number);
    if whole < number { whole + 1.0 } else { whole }
}

/// The largest whole number that is not above `number`, which is
/// `floor`.
fn flooring(number: f64) -> f64 {
    let whole = truncated(number);
    if whole > number { whole - 1.0 } else { whole }
}

/// What is left of `left` after taking `right` out of it as many whole
/// times as it goes, which is `fmod`.
///
/// The divisor is doubled until one more doubling would pass the
/// dividend, then halved back down, and what fits is taken out at every
/// step. Each subtraction is exact: the loop holds the divisor at no
/// more than what is left and more than half of it, which is where a
/// subtraction of two doubles rounds nothing.
fn remainder(left: f64, right: f64) -> f64 {
    // An infinite dividend has no remainder, and neither has a divisor
    // of nought. Neither operand is ever a NaN: SQLite keeps one as a
    // `NULL` and this engine holds no value that is one.
    if !left.is_finite() || right == 0.0 {
        return f64::NAN;
    }
    let mut rest = left.abs();
    let divisor = right.abs();
    if rest < divisor {
        // Under the divisor already, and an infinite divisor as well:
        // what is left is the whole of it.
        return left;
    }
    let mut scaled = divisor;
    let mut doublings = 0u32;
    while scaled * 2.0 <= rest {
        scaled *= 2.0;
        doublings = doublings.saturating_add(1);
    }
    loop {
        if scaled <= rest {
            rest -= scaled;
        }
        if doublings == 0 {
            break;
        }
        scaled /= 2.0;
        doublings = doublings.saturating_sub(1);
    }
    // What is left carries the sign of what it was taken out of, which a
    // zero carries as well.
    if left.is_sign_negative() { -rest } else { rest }
}
