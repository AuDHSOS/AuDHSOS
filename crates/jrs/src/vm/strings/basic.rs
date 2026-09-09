// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Generic string methods. Conversion order is observable; all callback inputs
//! stay rooted, and copied/searched code units consume shared execution fuel.
use super::{Builtin, Error, Execution, Rc, Value, Vec, index_value};
use crate::{heap::HostBehavior, object::Property};
const METHODS: &[(&str, Builtin, u32)] = &[
    ("at", Builtin::StringAt, 1),
    ("charAt", Builtin::StringCharAt, 1),
    ("charCodeAt", Builtin::StringCharCodeAt, 1),
    ("codePointAt", Builtin::StringCodePointAt, 1),
    ("concat", Builtin::StringConcat, 1),
    ("indexOf", Builtin::StringIndexOf, 1),
    ("lastIndexOf", Builtin::StringLastIndexOf, 1),
    ("includes", Builtin::StringIncludes, 1),
    ("startsWith", Builtin::StringStartsWith, 1),
    ("endsWith", Builtin::StringEndsWith, 1),
    ("slice", Builtin::StringSlice, 2),
    ("substring", Builtin::StringSubstring, 2),
    ("repeat", Builtin::StringRepeat, 1),
    ("padStart", Builtin::StringPadStart, 1),
    ("padEnd", Builtin::StringPadEnd, 1),
    ("trim", Builtin::StringTrim, 0),
    ("trimStart", Builtin::StringTrimStart, 0),
    ("trimEnd", Builtin::StringTrimEnd, 0),
    ("isWellFormed", Builtin::StringIsWellFormed, 0),
    ("toWellFormed", Builtin::StringToWellFormed, 0),
];
impl Execution<'_> {
    pub(in super::super) fn install_string_methods(&mut self, object: &Value) -> Result<(), Error> {
        for (name, kind, length) in METHODS {
            let f = self.new_host_behavior(HostBehavior::Intrinsic(*kind), name, *length)?;
            self.define(
                object,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(f)
                },
            )?;
            let alias = match kind {
                Builtin::StringTrimStart => Some("trimLeft"),
                Builtin::StringTrimEnd => Some("trimRight"),
                _ => None,
            };
            if let Some(alias) = alias {
                let f = self.get(object, &Value::string(name).units())?;
                self.define(
                    object,
                    Value::string(alias).units(),
                    Property {
                        enumerable: false,
                        ..Property::data(f)
                    },
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn basic_string_call(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !METHODS.iter().any(|(_, k, _)| *k == kind) {
            return Ok(None);
        }
        if matches!(receiver, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "String method receiver is nullish",
            });
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            let text = self.string_units(receiver)?;
            let first = args.first().unwrap_or(&Value::Undefined);
            let second = args.get(1).unwrap_or(&Value::Undefined);
            self.string_basic_rooted(kind, &text, first, second, args)
        })();
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn string_basic_rooted(
        &mut self,
        kind: Builtin,
        text: &Rc<[u16]>,
        first: &Value,
        second: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        match kind {
            Builtin::StringRepeat => return self.string_repeat(text, first),
            Builtin::StringPadStart | Builtin::StringPadEnd => {
                return self.string_pad(kind, text, first, second);
            }
            _ => {}
        }
        match kind {
            Builtin::StringAt
            | Builtin::StringCharAt
            | Builtin::StringCharCodeAt
            | Builtin::StringCodePointAt => {
                let mut position = integer(self.numeric(first)?);
                if kind == Builtin::StringAt && position < 0.0 {
                    position += length_number(text.len());
                }
                if position < 0.0 || position >= length_number(text.len()) {
                    return Ok(match kind {
                        Builtin::StringCharAt => Value::string(""),
                        Builtin::StringCharCodeAt => Value::Number(f64::NAN),
                        _ => Value::Undefined,
                    });
                }
                let index = clamp(position, text.len());
                let unit = *text.get(index).ok_or(Error::InvalidBytecode)?;
                Ok(match kind {
                    Builtin::StringCharCodeAt => Value::Number(f64::from(unit)),
                    Builtin::StringCodePointAt => Value::Number(f64::from(
                        audhsos_utf16::code_point_at(text, index)
                            .ok_or(Error::InvalidBytecode)?
                            .0,
                    )),
                    _ => Value::String(Rc::from([unit])),
                })
            }
            Builtin::StringConcat => {
                let mut out = Vec::new();
                self.string_copy(&mut out, text)?;
                for arg in args {
                    let units = self.string_units(arg)?;
                    self.string_copy(&mut out, &units)?;
                }
                Ok(Value::String(out.into()))
            }
            Builtin::StringIndexOf
            | Builtin::StringLastIndexOf
            | Builtin::StringIncludes
            | Builtin::StringStartsWith
            | Builtin::StringEndsWith => self.string_find(kind, text, first, second),
            Builtin::StringSlice | Builtin::StringSubstring => {
                let a = integer(self.numeric(first)?);
                let b = if matches!(second, Value::Undefined) {
                    length_number(text.len())
                } else {
                    integer(self.numeric(second)?)
                };
                let (a, b) = if kind == Builtin::StringSlice {
                    (relative(a, text.len()), relative(b, text.len()))
                } else {
                    (clamp(a, text.len()), clamp(b, text.len()))
                };
                let (start, end) = if kind == Builtin::StringSubstring {
                    (a.min(b), a.max(b))
                } else {
                    (a, a.max(b))
                };
                self.string_part(text, start, end)
            }
            Builtin::StringTrim | Builtin::StringTrimStart | Builtin::StringTrimEnd => {
                self.string_trim(kind, text)
            }
            Builtin::StringIsWellFormed | Builtin::StringToWellFormed => {
                self.string_well_formed(kind, text)
            }
            _ => Err(Error::InvalidBytecode),
        }
    }
    fn string_repeat(&mut self, text: &Rc<[u16]>, first: &Value) -> Result<Value, Error> {
        let n = integer(self.numeric(first)?);
        if n < 0.0 || n == f64::INFINITY {
            return Err(Error::Range {
                message: "invalid String.repeat count",
            });
        }
        if text.is_empty() || n == 0.0 {
            return Ok(Value::string(""));
        }
        let length = length_number(text.len()) * n;
        self.string_output_size(length)?;
        let length = clamp(length, self.limits.string_units);
        self.charge(u64::try_from(length).unwrap_or(u64::MAX))?;
        Ok(Value::String(repeat_prefix(text, length).into()))
    }
    fn string_pad(
        &mut self,
        kind: Builtin,
        text: &Rc<[u16]>,
        first: &Value,
        second: &Value,
    ) -> Result<Value, Error> {
        let n = integer(self.numeric(first)?).clamp(0.0, 9_007_199_254_740_991.0);
        if n <= length_number(text.len()) {
            return Ok(Value::String(text.clone()));
        }
        let filler = if matches!(second, Value::Undefined) {
            Value::string(" ").units()
        } else {
            self.string_units(second)?
        };
        if filler.is_empty() {
            return Ok(Value::String(text.clone()));
        }
        self.string_output_size(n)?;
        let size = clamp(n, self.limits.string_units);
        let count = size.saturating_sub(text.len());
        self.charge(u64::try_from(size).unwrap_or(u64::MAX))?;
        let mut out = if kind == Builtin::StringPadStart {
            repeat_prefix(&filler, count)
        } else {
            text.to_vec()
        };
        if kind == Builtin::StringPadStart {
            out.extend_from_slice(text);
        } else {
            let begin = out.len();
            out.extend_from_slice(
                filler
                    .get(..filler.len().min(count))
                    .ok_or(Error::InvalidBytecode)?,
            );
            while out.len() < size {
                let n = out
                    .len()
                    .saturating_sub(begin)
                    .min(size.saturating_sub(out.len()));
                out.extend_from_within(begin..begin.saturating_add(n));
            }
        }
        Ok(Value::String(out.into()))
    }
    fn string_trim(&mut self, kind: Builtin, text: &Rc<[u16]>) -> Result<Value, Error> {
        let mut start = 0usize;
        let mut end = text.len();
        if kind != Builtin::StringTrimEnd {
            while start < end {
                self.charge(1)?;
                if !space(*text.get(start).ok_or(Error::InvalidBytecode)?) {
                    break;
                }
                start = start.saturating_add(1);
            }
        }
        if kind != Builtin::StringTrimStart {
            while end > start {
                self.charge(1)?;
                if !space(
                    *text
                        .get(end.saturating_sub(1))
                        .ok_or(Error::InvalidBytecode)?,
                ) {
                    break;
                }
                end = end.saturating_sub(1);
            }
        }
        self.string_part(text, start, end)
    }
    fn string_well_formed(&mut self, kind: Builtin, text: &Rc<[u16]>) -> Result<Value, Error> {
        self.charge(u64::try_from(text.len()).unwrap_or(u64::MAX))?;
        let mut out = None;
        let mut i = 0usize;
        while let Some((_, width, lone)) = audhsos_utf16::code_point_at(text, i) {
            if lone {
                if kind == Builtin::StringIsWellFormed {
                    return Ok(Value::Boolean(false));
                }
                self.string_output_size(length_number(text.len()))?;
                let buffer = out.get_or_insert_with(|| text.to_vec());
                *buffer.get_mut(i).ok_or(Error::InvalidBytecode)? = 0xfffd;
            }
            i = i.saturating_add(width);
        }
        Ok(if kind == Builtin::StringIsWellFormed {
            Value::Boolean(true)
        } else {
            Value::String(out.map_or_else(|| text.clone(), Rc::from))
        })
    }
    fn string_find(
        &mut self,
        kind: Builtin,
        text: &[u16],
        search: &Value,
        position: &Value,
    ) -> Result<Value, Error> {
        if matches!(
            kind,
            Builtin::StringIncludes | Builtin::StringStartsWith | Builtin::StringEndsWith
        ) && self.is_regexp_value(search)?
        {
            return Err(Error::Type {
                message: "String search does not accept RegExp",
            });
        }
        let needle = self.string_units(search)?;
        let mut pos = if kind == Builtin::StringEndsWith && matches!(position, Value::Undefined) {
            length_number(text.len())
        } else {
            self.numeric(position)?
        };
        if kind == Builtin::StringLastIndexOf && pos.is_nan() {
            pos = f64::INFINITY;
        }
        let start = clamp(integer(pos), text.len());
        if matches!(kind, Builtin::StringStartsWith | Builtin::StringEndsWith) {
            self.charge(u64::try_from(needle.len()).unwrap_or(u64::MAX))?;
            let range = if kind == Builtin::StringStartsWith {
                Some(start..start.saturating_add(needle.len()))
            } else {
                start.checked_sub(needle.len()).map(|a| a..start)
            };
            return Ok(Value::Boolean(
                range.and_then(|r| text.get(r)) == Some(needle.as_ref()),
            ));
        }
        let pattern =
            audhsos_utf16::Pattern::new(&needle, self.limits.string_units, &mut self.fuel)
                .map_err(search_error)?;
        let found = if kind == Builtin::StringLastIndexOf {
            pattern.rfind(text, start, &mut self.fuel)
        } else {
            pattern.find(text, start, &mut self.fuel)
        }
        .map_err(search_error)?;
        Ok(if kind == Builtin::StringIncludes {
            Value::Boolean(found.is_some())
        } else {
            found.map_or(Value::Number(-1.0), index_value)
        })
    }
    pub(in crate::vm) fn is_regexp_value(&mut self, value: &Value) -> Result<bool, Error> {
        if !matches!(value, Value::Object(_) | Value::Function(_)) {
            return Ok(false);
        }
        let key = Value::Symbol(self.well_known("match")?);
        let matcher = self.get_key(value, &key)?;
        if matches!(matcher, Value::Undefined) {
            Ok(self.regexp_data(value).is_some())
        } else {
            Ok(matcher.to_boolean())
        }
    }
    fn string_part(&mut self, text: &Rc<[u16]>, start: usize, end: usize) -> Result<Value, Error> {
        if start == 0 && end == text.len() {
            return Ok(Value::String(text.clone()));
        }
        let mut out = Vec::new();
        self.string_copy(
            &mut out,
            text.get(start..end).ok_or(Error::InvalidBytecode)?,
        )?;
        Ok(Value::String(out.into()))
    }
    fn string_output_size(&self, size: f64) -> Result<(), Error> {
        if size > length_number(self.limits.string_units) {
            Err(Error::Limit {
                resource: "string units",
            })
        } else {
            Ok(())
        }
    }
    fn string_copy(&mut self, out: &mut Vec<u16>, units: &[u16]) -> Result<(), Error> {
        self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
        self.append_string(out, units)
    }
}
fn repeat_prefix(units: &[u16], size: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(units.get(..units.len().min(size)).unwrap_or(&[]));
    while out.len() < size {
        let n = out.len().min(size.saturating_sub(out.len()));
        out.extend_from_within(..n);
    }
    out
}
fn integer(n: f64) -> f64 {
    crate::value::integer_or_infinity(n)
}
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    reason = "clamped nonnegative integral index bounded by a real slice length"
)]
fn clamp(n: f64, length: usize) -> usize {
    if n <= 0.0 {
        0
    } else if n >= length_number(length) {
        length
    } else {
        n as usize
    }
}
fn relative(n: f64, length: usize) -> usize {
    clamp(
        if n < 0.0 {
            length_number(length) + n
        } else {
            n
        },
        length,
    )
}
fn length_number(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}
fn space(u: u16) -> bool {
    char::from_u32(u32::from(u))
        .is_some_and(|c| crate::value::whitespace(c) || crate::value::line_terminator(c))
}
pub(super) const fn search_error(_: audhsos_utf16::Limit) -> Error {
    Error::Limit {
        resource: "UTF-16 search work or storage",
    }
}
