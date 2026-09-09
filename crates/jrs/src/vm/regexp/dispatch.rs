// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Observable string/symbol dispatch. No custom exec result bypasses validation.
use super::{Builtin, Error, Execution, Property, Rc, Value, intrinsics::require_object};
use crate::object::same_value;

impl Execution<'_> {
    pub(super) fn string_regexp_dispatch(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        pattern: &Value,
    ) -> Result<Value, Error> {
        if matches!(receiver, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "string method receiver is nullish",
            });
        }
        let name = if kind == Builtin::StringMatch {
            "match"
        } else {
            "search"
        };
        let key = Value::Symbol(self.well_known(name)?);
        // The loaded ECMA draft consults hooks only on Objects, not primitives.
        if matches!(pattern, Value::Object(_) | Value::Function(_)) {
            let method = self.get_key(pattern, &key)?;
            if !matches!(method, Value::Null | Value::Undefined) {
                return self.call_sync(method, pattern.clone(), core::slice::from_ref(receiver));
            }
        }
        let text = Value::String(self.string_units(receiver)?);
        // RegExpCreate differs from calling the public constructor: no IsRegExp
        // or constructor/source/flags hook, and never a same-instance return.
        let proto = self.regexp_prototype()?;
        let regex = self.allocate_regexp(proto)?;
        self.native_roots.push(regex.clone());
        self.initialize_regexp(&regex, pattern, &Value::Undefined)?;
        let method = self.get_key(&regex, &key)?;
        self.call_sync(method, regex, &[text])
    }

    pub(super) fn regexp_exec_value(
        &mut self,
        receiver: &Value,
        text: &Rc<[u16]>,
    ) -> Result<Value, Error> {
        let exec = self.get(receiver, &Value::string("exec").units())?;
        if matches!(exec, Value::Function(_)) {
            let result = self.call_sync(exec, receiver.clone(), &[Value::String(text.clone())])?;
            if !matches!(result, Value::Object(_) | Value::Function(_) | Value::Null) {
                return Err(Error::Type {
                    message: "RegExp exec returned neither object nor null",
                });
            }
            return Ok(result);
        }
        let indices = self
            .regexp_data(receiver)
            .ok_or(Error::Type {
                message: "RegExp receiver required",
            })?
            .indices;
        self.regexp_exec(receiver, text)?
            .map_or(Ok(Value::Null), |m| self.match_array(text, &m, indices))
    }

    pub(super) fn regexp_symbol_search(
        &mut self,
        receiver: &Value,
        input: &Value,
    ) -> Result<Value, Error> {
        require_object(receiver)?;
        let text = self.string_units(input)?;
        let key = Value::string("lastIndex").units();
        let previous = self.get(receiver, &key)?;
        self.native_roots.push(previous.clone());
        if !same_value(&previous, &Value::Number(0.0)) {
            self.set(receiver, key.clone(), Value::Number(0.0), true)?;
        }
        // Restore only on normal completion, not as a finally action after throws.
        let result = self.regexp_exec_value(receiver, &text)?;
        self.native_roots.push(result.clone());
        let current = self.get(receiver, &key)?;
        if !same_value(&current, &previous) {
            self.set(receiver, key, previous, true)?;
        }
        if matches!(result, Value::Null) {
            Ok(Value::Number(-1.0))
        } else {
            self.get(&result, &Value::string("index").units())
        }
    }

    pub(super) fn regexp_symbol_match(
        &mut self,
        receiver: &Value,
        input: &Value,
    ) -> Result<Value, Error> {
        require_object(receiver)?;
        let text = self.string_units(input)?;
        let flags = self.get(receiver, &Value::string("flags").units())?;
        let flags = self.string_units(&flags)?;
        self.charge(u64::try_from(flags.len()).unwrap_or(u64::MAX))?;
        if !flags.contains(&103) {
            return self.regexp_exec_value(receiver, &text);
        }
        let unicode = flags.contains(&117) || flags.contains(&118);
        let key = Value::string("lastIndex").units();
        self.set(receiver, key.clone(), Value::Number(0.0), true)?;
        let array = self.new_array(0)?;
        self.native_roots.push(array.clone());
        let mut count = 0usize;
        loop {
            self.charge(1)?;
            let result = self.regexp_exec_value(receiver, &text)?;
            if matches!(result, Value::Null) {
                return Ok(if count == 0 { Value::Null } else { array });
            }
            let roots = self.native_roots.len();
            self.native_roots.push(result.clone());
            let iteration = (|| {
                let matched = self.get(&result, &Value::string("0").units())?;
                let matched = self.string_units(&matched)?;
                // The result Array also owns its non-configurable length property.
                if count >= self.limits.properties.saturating_sub(1) {
                    return Err(Error::Limit {
                        resource: "RegExp match results",
                    });
                }
                let empty = matched.is_empty();
                self.define(
                    &array,
                    Value::string(&alloc::format!("{count}")).units(),
                    Property::data(Value::String(matched)),
                )?;
                if empty {
                    let value = self.get(receiver, &key)?;
                    let index = crate::value::integer_or_infinity(self.numeric(&value)?)
                        .clamp(0.0, 9_007_199_254_740_991.0);
                    let next = advance(&text, index, unicode);
                    self.set(receiver, key.clone(), Value::Number(next), true)?;
                }
                Ok(())
            })();
            self.native_roots.truncate(roots);
            iteration?;
            count = count.saturating_add(1);
        }
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "index is a ToLength integer; usize conversion is used only for bounded slice lookup"
)]
pub(super) fn advance(text: &[u16], index: f64, unicode: bool) -> f64 {
    let width = if unicode {
        audhsos_utf16::code_point_at(text, index as usize).map_or(1, |(_, width, _)| width)
    } else {
        1
    };
    index + if width == 2 { 2.0 } else { 1.0 }
}
