// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! UTF-16 string operations keep lone surrogates and charge native loops to fuel.

use super::Execution;
use crate::{Error, Value, bytecode::Builtin};
use alloc::{rc::Rc, vec::Vec};
mod basic;
pub(super) mod substitution;

impl Execution<'_> {
    pub(super) fn string_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if let Some(value) = self.basic_string_call(builtin, receiver, args)? {
            return Ok(Some(value));
        }
        match builtin {
            Builtin::StringFromCharCode => {
                if args.len() > self.limits.string_units {
                    return Err(Error::Limit {
                        resource: "string units",
                    });
                }
                let mut units = Vec::new();
                for value in args {
                    self.charge(1)?;
                    let number = Value::Number(self.numeric(value)?).to_uint32();
                    units.push(u16::try_from(number & 0xffff).map_err(|_| Error::InvalidBytecode)?);
                }
                Ok(Some(Value::String(units.into())))
            }
            Builtin::StringSplit => self.string_split(receiver, args).map(Some),
            Builtin::StringReplace => self.string_replace(receiver, args).map(Some),
            _ => Ok(None),
        }
    }
    fn string_split(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.string_split_rooted(receiver, args);
        self.native_roots.truncate(roots);
        result
    }
    fn string_split_rooted(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        if matches!(receiver, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "String.split receiver is nullish",
            });
        }
        let separator = args.first().unwrap_or(&Value::Undefined);
        if matches!(separator, Value::Object(_) | Value::Function(_)) {
            let key = Value::Symbol(self.well_known("split")?);
            let method = self.get_key(separator, &key)?;
            if !matches!(method, Value::Null | Value::Undefined) {
                return self.call_sync(
                    method,
                    separator.clone(),
                    &[
                        receiver.clone(),
                        args.get(1).cloned().unwrap_or(Value::Undefined),
                    ],
                );
            }
        }
        let text = self.string_units(receiver)?;
        let limit = if args.get(1).is_none_or(|v| matches!(v, Value::Undefined)) {
            u32::MAX
        } else {
            Value::Number(self.numeric(args.get(1).ok_or(Error::InvalidBytecode)?)?).to_uint32()
        };
        let separator_text = self.string_units(separator)?;
        if limit == 0 {
            return self.new_array(0);
        }
        if matches!(separator, Value::Undefined) {
            return self.array_from_values(&[Value::String(text)]);
        }
        let mut values = Vec::new();
        if separator_text.is_empty() {
            for unit in text
                .iter()
                .take(usize::try_from(limit).unwrap_or(usize::MAX))
            {
                self.charge(1)?;
                self.split_push(&mut values, Value::String(Rc::from([*unit])))?;
            }
        } else {
            let mut start = 0usize;
            let mut position = 0usize;
            let pattern = audhsos_utf16::Pattern::new(
                &separator_text,
                self.limits.string_units,
                &mut self.fuel,
            )
            .map_err(basic::search_error)?;
            while position.saturating_add(separator_text.len()) <= text.len() {
                if let Some(found) = pattern
                    .find(&text, position, &mut self.fuel)
                    .map_err(basic::search_error)?
                {
                    position = found;
                    self.split_push(
                        &mut values,
                        Value::String(Rc::from(
                            text.get(start..position).ok_or(Error::InvalidBytecode)?,
                        )),
                    )?;
                    if values.len() >= usize::try_from(limit).unwrap_or(usize::MAX) {
                        return self.array_from_values(&values);
                    }
                    position = position.saturating_add(separator_text.len());
                    start = position;
                } else {
                    break;
                }
            }
            self.split_push(
                &mut values,
                Value::String(Rc::from(text.get(start..).ok_or(Error::InvalidBytecode)?)),
            )?;
        }
        self.array_from_values(&values)
    }
    pub(super) fn split_push(&self, values: &mut Vec<Value>, value: Value) -> Result<(), Error> {
        if values.len() >= self.limits.properties {
            return Err(Error::Limit {
                resource: "split results",
            });
        }
        values.push(value);
        Ok(())
    }

    fn string_replace(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        if matches!(receiver, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "String.replace receiver is nullish",
            });
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.replace_rooted(receiver, args);
        self.native_roots.truncate(roots);
        result
    }
    fn replace_rooted(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let search = args.first().unwrap_or(&Value::Undefined);
        let replacement = args.get(1).unwrap_or(&Value::Undefined);
        if matches!(search, Value::Object(_) | Value::Function(_)) {
            let key = Value::Symbol(self.well_known("replace")?);
            let method = self.get_key(search, &key)?;
            if !matches!(method, Value::Null | Value::Undefined) {
                return self.call_sync(
                    method,
                    search.clone(),
                    &[receiver.clone(), replacement.clone()],
                );
            }
        }
        let text = self.string_units(receiver)?;
        let search = self.string_units(search)?;
        let template = if matches!(replacement, Value::Function(_)) {
            None
        } else {
            Some(self.string_units(replacement)?)
        };
        let position = if search.is_empty() {
            Some(0)
        } else {
            let pattern =
                audhsos_utf16::Pattern::new(&search, self.limits.string_units, &mut self.fuel)
                    .map_err(basic::search_error)?;
            pattern
                .find(&text, 0, &mut self.fuel)
                .map_err(basic::search_error)?
        };
        let Some(position) = position else {
            return Ok(Value::String(text));
        };
        let substituted = if let Some(template) = template {
            self.get_substitution(
                &template,
                substitution::Input {
                    matched: &search,
                    text: &text,
                    position,
                    captures: &[],
                    named: &Value::Undefined,
                },
            )?
        } else {
            let value = self.call_sync(
                replacement.clone(),
                Value::Undefined,
                &[
                    Value::String(search.clone()),
                    index_value(position),
                    Value::String(text.clone()),
                ],
            )?;
            self.string_units(&value)?.to_vec()
        };
        let mut out = Vec::new();
        self.append_string(
            &mut out,
            text.get(..position).ok_or(Error::InvalidBytecode)?,
        )?;
        self.append_string(&mut out, &substituted)?;
        self.append_string(
            &mut out,
            text.get(position.saturating_add(search.len())..)
                .ok_or(Error::InvalidBytecode)?,
        )?;
        Ok(Value::String(out.into()))
    }
    pub(super) fn append_string(&mut self, out: &mut Vec<u16>, units: &[u16]) -> Result<(), Error> {
        if out
            .len()
            .checked_add(units.len())
            .is_none_or(|n| n > self.limits.string_units)
        {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        self.charge(u64::try_from(units.len()).unwrap_or(u64::MAX))?;
        out.extend_from_slice(units);
        Ok(())
    }
}

fn index_value(index: usize) -> Value {
    Value::Number(f64::from(u32::try_from(index).unwrap_or(u32::MAX)))
}
