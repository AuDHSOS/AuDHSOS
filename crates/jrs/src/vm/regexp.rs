// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `RegExpBuiltinExec` adapter. NFA work is charged to VM fuel; no matcher lives
//! here and no unsupported expression is handed to a fallback engine.

use super::Execution;
use crate::{Error, Value, bytecode::Builtin, object::Property, regexp::RegExp};
use alloc::{rc::Rc, string::String, vec::Vec};
mod dispatch;
mod intrinsics;
mod replace;
mod split;

impl Execution<'_> {
    pub(super) fn new_regexp(&mut self, regex: Rc<RegExp>) -> Result<Value, Error> {
        let proto = self.regexp_prototype()?;
        let object = self.allocate_regexp(proto)?;
        self.object_mut(&object)?.regexp = Some(regex);
        Ok(object)
    }
    fn allocate_regexp(&mut self, proto: Value) -> Result<Value, Error> {
        let object = self.allocate_object(proto)?;
        self.define(
            &object,
            Value::string("lastIndex").units(),
            Property {
                value: Value::Number(0.0),
                writable: true,
                enumerable: false,
                configurable: false,
                accessor: None,
            },
        )?;
        Ok(object)
    }
    pub(super) fn regexp_data(&self, value: &Value) -> Option<Rc<RegExp>> {
        self.object_ref(value)
            .ok()
            .and_then(|object| object.regexp.clone())
    }
    pub(super) fn regexp_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if !matches!(
            builtin,
            Builtin::RegExp
                | Builtin::RegExpTest
                | Builtin::RegExpExec
                | Builtin::RegExpToString
                | Builtin::StringMatch
                | Builtin::StringSearch
                | Builtin::RegExpMatch
                | Builtin::RegExpSearch
                | Builtin::RegExpSplit
                | Builtin::RegExpReplace
        ) {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.regexp_call_rooted(builtin, receiver, args);
        self.native_roots.truncate(roots);
        result
    }
    fn regexp_call_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let first = args.first().unwrap_or(&Value::Undefined);
        match builtin {
            Builtin::RegExp => self.regexp_constructor_call(args, None).map(Some),
            Builtin::RegExpTest => {
                intrinsics::require_object(receiver)?;
                let text = self.string_units(first)?;
                let result = self.regexp_exec_value(receiver, &text)?;
                Ok(Some(Value::Boolean(!matches!(result, Value::Null))))
            }
            Builtin::RegExpExec => {
                let indices = self
                    .regexp_data(receiver)
                    .ok_or(Error::Type {
                        message: "RegExp receiver required",
                    })?
                    .indices;
                let text = self.string_units(first)?;
                let result = self.regexp_exec(receiver, &text)?;
                Ok(Some(if let Some(result) = result {
                    self.match_array(&text, &result, indices)?
                } else {
                    Value::Null
                }))
            }
            Builtin::RegExpToString => {
                intrinsics::require_object(receiver)?;
                let source = self.get(receiver, &Value::string("source").units())?;
                let source = self.string_units(&source)?;
                let flags = self.get(receiver, &Value::string("flags").units())?;
                let flags = self.string_units(&flags)?;
                let length = source
                    .len()
                    .checked_add(flags.len())
                    .and_then(|n| n.checked_add(2));
                if length.is_none_or(|n| n > self.limits.string_units) {
                    return Err(Error::Limit {
                        resource: "string units",
                    });
                }
                self.charge(u64::try_from(length.unwrap_or(0)).unwrap_or(u64::MAX))?;
                let mut text = alloc::vec![47];
                text.extend_from_slice(&source);
                text.push(47);
                text.extend_from_slice(&flags);
                Ok(Some(Value::String(text.into())))
            }
            Builtin::StringMatch | Builtin::StringSearch => self
                .string_regexp_dispatch(builtin, receiver, first)
                .map(Some),
            Builtin::RegExpMatch => self.regexp_symbol_match(receiver, first).map(Some),
            Builtin::RegExpSearch => self.regexp_symbol_search(receiver, first).map(Some),
            Builtin::RegExpSplit => self
                .regexp_symbol_split(receiver, first, args.get(1).unwrap_or(&Value::Undefined))
                .map(Some),
            Builtin::RegExpReplace => self
                .regexp_symbol_replace(receiver, first, args.get(1).unwrap_or(&Value::Undefined))
                .map(Some),
            _ => Ok(None),
        }
    }
    pub(super) fn regexp_exec(
        &mut self,
        object: &Value,
        text: &[u16],
    ) -> Result<Option<audhsos_regex::Match>, Error> {
        let regex = self.regexp_data(object).ok_or(Error::Type {
            message: "RegExp receiver required",
        })?;
        let stateful = regex.global || regex.sticky;
        let value = self.get(object, &Value::string("lastIndex").units())?;
        let index = crate::value::integer_or_infinity(self.numeric(&value)?);
        let from = if !stateful || index.is_nan() || index <= 0.0 {
            0
        } else if index > f64::from(u32::try_from(text.len()).unwrap_or(u32::MAX)) {
            text.len().saturating_add(1)
        } else {
            usize::try_from(Value::Number(index).to_uint32()).unwrap_or(usize::MAX)
        };
        let limits = audhsos_regex::Limits {
            input_units: self.limits.string_units,
            work: self.fuel,
            ..audhsos_regex::Limits::default()
        };
        let report = regex
            .regex
            .find(text, from, regex.sticky, limits)
            .map_err(crate::regexp::error)?;
        self.charge(report.work)?;
        if stateful {
            let index = report.matched.as_ref().map_or(0, |m| m.range.end);
            self.set(
                object,
                Value::string("lastIndex").units(),
                number(index),
                true,
            )?;
        }
        Ok(report.matched)
    }
    fn match_array(
        &mut self,
        text: &[u16],
        m: &audhsos_regex::Match,
        indices: bool,
    ) -> Result<Value, Error> {
        let mut values = Vec::new();
        values.push(Value::String(Rc::from(
            text.get(m.range.clone()).ok_or(Error::InvalidBytecode)?,
        )));
        for capture in &m.captures {
            values.push(if let Some(range) = capture {
                Value::String(Rc::from(
                    text.get(range.clone()).ok_or(Error::InvalidBytecode)?,
                ))
            } else {
                Value::Undefined
            });
        }
        let array = self.array_from_values(&values)?;
        let roots = self.native_roots.len();
        self.native_roots.push(array.clone());
        let result = (|| {
            for (name, value) in [
                ("index", number(m.range.start)),
                ("input", Value::String(Rc::from(text))),
                ("groups", Value::Undefined),
            ] {
                self.define(&array, Value::string(name).units(), Property::data(value))?;
            }
            if indices {
                let indices = self.new_array(0)?;
                self.native_roots.push(indices.clone());
                for (index, range) in core::iter::once(Some(m.range.clone()))
                    .chain(m.captures.iter().cloned())
                    .enumerate()
                {
                    let value = if let Some(range) = range {
                        self.array_from_values(&[number(range.start), number(range.end)])?
                    } else {
                        Value::Undefined
                    };
                    self.define(
                        &indices,
                        Value::string(&alloc::format!("{index}")).units(),
                        Property::data(value),
                    )?;
                }
                self.define(
                    &indices,
                    Value::string("groups").units(),
                    Property::data(Value::Undefined),
                )?;
                self.define(
                    &array,
                    Value::string("indices").units(),
                    Property::data(indices),
                )?;
            }
            Ok(array)
        })();
        self.native_roots.truncate(roots);
        result
    }
}

fn number(index: usize) -> Value {
    Value::Number(f64::from(u32::try_from(index).unwrap_or(u32::MAX)))
}
fn source_text(source: &[u16], limit: usize) -> Result<Rc<[u16]>, Error> {
    if source.is_empty() {
        if limit < 4 {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        return Ok(Value::string("(?:)").units());
    }
    let mut result = Vec::new();
    let mut escaped = false;
    for unit in source {
        if matches!(*unit, 10 | 13 | 0x2028 | 0x2029) {
            let escape = match *unit {
                10 => "\\n",
                13 => "\\r",
                0x2028 => "\\u2028",
                _ => "\\u2029",
            };
            // An existing backslash before a literal line terminator must be
            // consumed as part of its replacement escape, not escaped twice.
            if escaped {
                result.pop();
            }
            if result.len().saturating_add(escape.len()) > limit {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            result.extend(escape.encode_utf16());
        } else {
            let extra = 1usize.saturating_add(usize::from(*unit == 47 && !escaped));
            if result.len().saturating_add(extra) > limit {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            if *unit == 47 && !escaped {
                result.push(92);
            }
            result.push(*unit);
        }
        escaped = *unit == 92 && !escaped;
    }
    Ok(result.into())
}
