// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Species-aware splitting. Only proven hook-free failed sticky starts can be
//! merged into a Thompson search; custom exec/prototype mutations stay observable.
use super::{Builtin, Error, Execution, Property, Rc, Value, intrinsics::require_object};
use crate::{
    heap::{HostBehavior, Node},
    value::{Callable, FunctionValue},
};

impl Execution<'_> {
    fn split_species(&mut self, receiver: &Value) -> Result<Value, Error> {
        let default = Value::Function(FunctionValue::native(Builtin::RegExp));
        let ctor = self.get(receiver, &Value::string("constructor").units())?;
        if matches!(ctor, Value::Undefined) {
            return Ok(default);
        }
        require_object(&ctor)?;
        self.native_roots.push(ctor.clone());
        let key = Value::Symbol(self.well_known("species")?);
        let species = self.get_key(&ctor, &key)?;
        if matches!(species, Value::Null | Value::Undefined) {
            return Ok(default);
        }
        if !self.is_constructor(&species) {
            return Err(Error::Type {
                message: "RegExp split species is not a constructor",
            });
        }
        Ok(species)
    }
    pub(super) fn regexp_symbol_split(
        &mut self,
        receiver: &Value,
        input: &Value,
        limit: &Value,
    ) -> Result<Value, Error> {
        require_object(receiver)?;
        let text = self.string_units(input)?;
        let ctor = self.split_species(receiver)?;
        self.native_roots.push(ctor.clone());
        let flags = self.get(receiver, &Value::string("flags").units())?;
        let mut flags = self.string_units(&flags)?.to_vec();
        self.charge(u64::try_from(flags.len()).unwrap_or(u64::MAX))?;
        let unicode = flags.contains(&117) || flags.contains(&118);
        if !flags.contains(&121) {
            if flags.len() >= self.limits.string_units {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            flags.push(121);
        }
        let splitter = self.construct_sync(
            ctor.clone(),
            &[receiver.clone(), Value::String(flags.into())],
            ctor,
        )?;
        self.native_roots.push(splitter.clone());
        let array = self.new_array(0)?;
        self.native_roots.push(array.clone());
        let limit = if matches!(limit, Value::Undefined) {
            u32::MAX
        } else {
            Value::Number(self.numeric(limit)?).to_uint32()
        };
        if limit == 0 {
            return Ok(array);
        }
        let mut count = 0u32;
        if text.is_empty() {
            if matches!(self.regexp_exec_value(&splitter, &text)?, Value::Null) {
                self.split_append(&array, &mut count, Value::String(text))?;
            }
            return Ok(array);
        }
        self.split_loop(&splitter, &text, &array, limit, unicode)?;
        Ok(array)
    }
    fn split_loop(
        &mut self,
        splitter: &Value,
        text: &Rc<[u16]>,
        array: &Value,
        limit: u32,
        unicode: bool,
    ) -> Result<(), Error> {
        let key = Value::string("lastIndex").units();
        let (mut previous, mut position, mut count) = (0usize, 0usize, 0u32);
        while position < text.len() {
            self.charge(1)?;
            self.set(splitter, key.clone(), super::number(position), true)?;
            let result = if !unicode && self.split_can_search(splitter)? {
                let (start, result) = self.split_search(splitter, text, position)?;
                position = start;
                result
            } else {
                self.regexp_exec_value(splitter, text)?
            };
            if matches!(result, Value::Null) {
                position = next_position(text, position, unicode);
                continue;
            }
            let roots = self.native_roots.len();
            self.native_roots.push(result.clone());
            let iteration = (|| {
                let end = self.get(splitter, &key)?;
                let end = crate::value::integer_or_infinity(self.numeric(&end)?)
                    .clamp(0.0, 9_007_199_254_740_991.0);
                let end = bounded_index(end, text.len());
                if end == previous {
                    position = next_position(text, position, unicode);
                    return Ok(false);
                }
                self.split_append(
                    array,
                    &mut count,
                    Value::String(Rc::from(
                        text.get(previous..position).ok_or(Error::InvalidBytecode)?,
                    )),
                )?;
                if count == limit {
                    return Ok(true);
                }
                previous = end;
                let length = self.get(&result, &Value::string("length").units())?;
                let length = crate::value::integer_or_infinity(self.numeric(&length)?)
                    .clamp(0.0, 9_007_199_254_740_991.0);
                let mut capture = 1u64;
                while capture_number(capture) < length {
                    self.charge(1)?;
                    let value = self.get(
                        &result,
                        &Value::string(&alloc::format!("{capture}")).units(),
                    )?;
                    self.split_append(array, &mut count, value)?;
                    if count == limit {
                        return Ok(true);
                    }
                    capture = capture.saturating_add(1);
                }
                position = previous;
                Ok(false)
            })();
            self.native_roots.truncate(roots);
            if iteration? {
                return Ok(());
            }
        }
        self.split_append(
            array,
            &mut count,
            Value::String(Rc::from(
                text.get(previous..).ok_or(Error::InvalidBytecode)?,
            )),
        )
    }
    fn split_append(&mut self, array: &Value, count: &mut u32, value: Value) -> Result<(), Error> {
        if usize::try_from(*count).unwrap_or(usize::MAX) >= self.limits.properties.saturating_sub(1)
        {
            return Err(Error::Limit {
                resource: "split results",
            });
        }
        self.define(
            array,
            Value::string(&alloc::format!("{count}")).units(),
            Property::data(value),
        )?;
        *count = count.saturating_add(1);
        Ok(())
    }
    fn split_can_search(&mut self, splitter: &Value) -> Result<bool, Error> {
        if !self.regexp_data(splitter).is_some_and(|r| r.sticky) {
            return Ok(false);
        }
        let object = self.object_ref(splitter)?;
        if Some(&object.prototype) != self.regexp_proto.as_ref() {
            return Ok(false);
        }
        if self
            .own(splitter, &Value::string("exec").units())?
            .is_some()
        {
            return Ok(false);
        }
        let proto = self.regexp_proto.clone().ok_or(Error::InvalidBytecode)?;
        let Some(property) = self.own(&proto, &Value::string("exec").units())? else {
            return Ok(false);
        };
        if property.accessor.is_some() {
            return Ok(false);
        }
        let Value::Function(FunctionValue(Callable::Host { handle, .. })) = property.value else {
            return Ok(false);
        };
        Ok(matches!(
            self.heap.get(handle)?,
            Node::HostFunction {
                behavior: HostBehavior::Intrinsic(Builtin::RegExpExec),
                ..
            }
        ))
    }
    fn split_search(
        &mut self,
        splitter: &Value,
        text: &Rc<[u16]>,
        from: usize,
    ) -> Result<(usize, Value), Error> {
        let regex = self.regexp_data(splitter).ok_or(Error::InvalidBytecode)?;
        let report = regex
            .regex
            .find(
                text,
                from,
                false,
                audhsos_regex::Limits {
                    input_units: self.limits.string_units,
                    work: self.fuel,
                    ..audhsos_regex::Limits::default()
                },
            )
            .map_err(crate::regexp::error)?;
        self.charge(report.work)?;
        let found = report.matched.filter(|m| m.range.start < text.len());
        self.set(
            splitter,
            Value::string("lastIndex").units(),
            super::number(found.as_ref().map_or(0, |m| m.range.end)),
            true,
        )?;
        match found {
            Some(m) => Ok((m.range.start, self.match_array(text, &m, regex.indices)?)),
            None => Ok((text.len(), Value::Null)),
        }
    }
}
fn next_position(text: &[u16], position: usize, unicode: bool) -> usize {
    position.saturating_add(if unicode {
        audhsos_utf16::code_point_at(text, position).map_or(1, |(_, width, _)| width)
    } else {
        1
    })
}
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "ToLength bounded index; usize conversion saturates before min with actual slice length"
)]
fn bounded_index(index: f64, length: usize) -> usize {
    (index as usize).min(length)
}
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "capture index is bounded by the 53-bit ToLength of a custom exec result"
)]
const fn capture_number(index: u64) -> f64 {
    index as f64
}
