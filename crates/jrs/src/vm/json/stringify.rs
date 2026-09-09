// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Serialization streams into one quota-checked UTF-16 buffer. Member prefixes
//! are emitted only after hooks determine inclusion. Hooks use the real VM/host.
use super::{Error, Execution, Property, Rc, Value, Vec, index_key, json_error};
use alloc::collections::BTreeSet;
struct State {
    replacer: Value,
    keys: Option<Vec<Rc<[u16]>>>,
    gap: Rc<[u16]>,
    ancestors: Vec<Value>,
    out: Vec<u16>,
}
impl Execution<'_> {
    pub(super) fn json_stringify(
        &mut self,
        value: &Value,
        replacer: &Value,
        space: &Value,
    ) -> Result<Value, Error> {
        let keys = self.replacer_keys(replacer)?;
        let gap = self.json_gap(space)?;
        let proto = self.prototype()?;
        let holder = self.allocate_object(proto)?;
        self.native_roots.push(holder.clone());
        self.define(
            &holder,
            Value::string("").units(),
            Property::data(value.clone()),
        )?;
        let mut state = State {
            replacer: if matches!(replacer, Value::Function(_)) {
                replacer.clone()
            } else {
                Value::Undefined
            },
            keys,
            gap,
            ancestors: Vec::new(),
            out: Vec::new(),
        };
        if self.stringify_property(&mut state, &holder, &Value::string("").units(), 0, None)? {
            Ok(Value::String(state.out.into()))
        } else {
            Ok(Value::Undefined)
        }
    }
    fn replacer_keys(&mut self, replacer: &Value) -> Result<Option<Vec<Rc<[u16]>>>, Error> {
        if !self.is_array(replacer) {
            return Ok(None);
        }
        let length = self.array_length(replacer)?;
        let mut keys = Vec::new();
        let mut seen = BTreeSet::new();
        for i in 0..length {
            self.charge(1)?;
            let value = self.get(
                replacer,
                &index_key(usize::try_from(i).map_err(|_| Error::InvalidBytecode)?),
            )?;
            let accepted = matches!(value, Value::String(_) | Value::Number(_))
                || self.object_ref(&value).is_ok_and(|o| {
                    matches!(o.primitive, Some(Value::String(_) | Value::Number(_)))
                });
            if accepted {
                let key = self.string_units(&value)?;
                if seen.insert(key.clone()) {
                    if keys.len() >= self.limits.properties {
                        return Err(Error::Limit {
                            resource: "JSON replacer keys",
                        });
                    }
                    keys.push(key);
                }
            }
        }
        Ok(Some(keys))
    }
    fn json_gap(&mut self, space: &Value) -> Result<Rc<[u16]>, Error> {
        let space = if self
            .object_ref(space)
            .is_ok_and(|o| matches!(o.primitive, Some(Value::Number(_))))
        {
            Value::Number(self.numeric(space)?)
        } else if self
            .object_ref(space)
            .is_ok_and(|o| matches!(o.primitive, Some(Value::String(_))))
        {
            Value::String(self.string_units(space)?)
        } else {
            space.clone()
        };
        Ok(match space {
            Value::String(s) => Rc::from(s.get(..s.len().min(10)).ok_or(Error::InvalidBytecode)?),
            Value::Number(n) => {
                let n = if n.is_nan() || n < 1.0 {
                    0
                } else if n >= 10.0 {
                    10
                } else {
                    Value::Number(n).to_uint32()
                };
                alloc::vec![32;usize::try_from(n).unwrap_or(10)].into()
            }
            _ => Rc::from([]),
        })
    }
    fn json_append(&mut self, state: &mut State, units: &[u16]) -> Result<(), Error> {
        audhsos_json::append(
            &mut state.out,
            units,
            self.limits.string_units,
            &mut self.fuel,
        )
        .map_err(json_error)
    }
    fn json_quote(&mut self, state: &mut State, units: &[u16]) -> Result<(), Error> {
        audhsos_json::quote(
            &mut state.out,
            units,
            self.limits.string_units,
            &mut self.fuel,
        )
        .map_err(json_error)
    }
    fn json_indent(&mut self, state: &mut State, depth: usize) -> Result<(), Error> {
        if state.gap.is_empty() {
            return Ok(());
        }
        self.json_append(state, &[10])?;
        let gap = state.gap.clone();
        for _ in 0..depth {
            self.json_append(state, &gap)?;
        }
        Ok(())
    }
    fn stringify_property(
        &mut self,
        state: &mut State,
        holder: &Value,
        key: &Rc<[u16]>,
        depth: usize,
        prefix: Option<(bool, usize)>,
    ) -> Result<bool, Error> {
        if self.json_depth >= 48 {
            return Err(Error::Limit {
                resource: "JSON serialization nesting",
            });
        }
        self.charge(1)?;
        self.json_depth = self.json_depth.saturating_add(1);
        let roots = self.native_roots.len();
        self.native_roots.push(holder.clone());
        let result = (|| {
            let mut value = self.get(holder, key)?;
            self.native_roots.push(value.clone());
            if matches!(value, Value::Object(_) | Value::Function(_)) {
                let method = self.get(&value, &Value::string("toJSON").units())?;
                if matches!(method, Value::Function(_)) {
                    value = self.call_sync(method, value, &[Value::String(key.clone())])?;
                    self.native_roots.push(value.clone());
                }
            }
            if matches!(state.replacer, Value::Function(_)) {
                value = self.call_sync(
                    state.replacer.clone(),
                    holder.clone(),
                    &[Value::String(key.clone()), value],
                )?;
                self.native_roots.push(value.clone());
            }
            let raw = self
                .object_ref(&value)
                .ok()
                .and_then(|o| o.raw_json.clone());
            let wrapper = self
                .object_ref(&value)
                .ok()
                .and_then(|o| o.primitive.clone());
            value = match wrapper {
                Some(Value::Number(_)) => Value::Number(self.numeric(&value)?),
                Some(Value::String(_)) => Value::String(self.string_units(&value)?),
                Some(v @ Value::Boolean(_)) => v,
                _ => value,
            };
            if matches!(
                value,
                Value::Undefined | Value::Symbol(_) | Value::Function(_)
            ) {
                if prefix.is_some_and(|(array, _)| array) {
                    value = Value::Null;
                } else {
                    return Ok(false);
                }
            }
            if let Some((array, count)) = prefix {
                self.json_member_prefix(state, key, depth, array, count)?;
            }
            if let Some(raw) = raw {
                self.json_append(state, &raw)?;
                return Ok(true);
            }
            match value {
                Value::Undefined | Value::Symbol(_) | Value::Function(_) => return Ok(false),
                Value::Null => self.json_append(state, &Value::string("null").units())?,
                Value::Boolean(v) => self.json_append(state, &Value::Boolean(v).units())?,
                Value::String(s) => self.json_quote(state, &s)?,
                Value::Number(n) => self.json_append(
                    state,
                    &if n.is_finite() {
                        Value::Number(n).units()
                    } else {
                        Value::string("null").units()
                    },
                )?,
                value @ Value::Object(_) => self.stringify_container(state, &value, depth)?,
            }
            Ok(true)
        })();
        self.native_roots.truncate(roots);
        self.json_depth = self.json_depth.saturating_sub(1);
        result
    }
    fn json_member_prefix(
        &mut self,
        state: &mut State,
        key: &[u16],
        depth: usize,
        array: bool,
        count: usize,
    ) -> Result<(), Error> {
        if count > 0 {
            self.json_append(state, &[44])?;
        }
        self.json_indent(state, depth)?;
        if !array {
            self.json_quote(state, key)?;
            self.json_append(
                state,
                if state.gap.is_empty() {
                    &[58]
                } else {
                    &[58, 32]
                },
            )?;
        }
        Ok(())
    }
    fn stringify_container(
        &mut self,
        state: &mut State,
        value: &Value,
        depth: usize,
    ) -> Result<(), Error> {
        if state.ancestors.contains(value) {
            return Err(Error::Type {
                message: "cyclic JSON value",
            });
        }
        state.ancestors.push(value.clone());
        let array = self.is_array(value);
        self.json_append(state, &[if array { 91 } else { 123 }])?;
        let keys = if array {
            None
        } else {
            Some(if let Some(keys) = &state.keys {
                keys.clone()
            } else {
                self.json_enumerable_keys(value)?
            })
        };
        let length = if array {
            usize::try_from(self.array_length(value)?).map_err(|_| Error::InvalidBytecode)?
        } else {
            keys.as_ref().map_or(0, Vec::len)
        };
        let mut count = 0usize;
        for i in 0..length {
            self.charge(1)?;
            let key = if let Some(keys) = &keys {
                keys.get(i).cloned().ok_or(Error::InvalidBytecode)?
            } else {
                index_key(i)
            };
            if self.stringify_property(
                state,
                value,
                &key,
                depth.saturating_add(1),
                Some((array, count)),
            )? {
                count = count.saturating_add(1);
            }
        }
        if count > 0 {
            self.json_indent(state, depth)?;
        }
        self.json_append(state, &[if array { 93 } else { 125 }])?;
        state.ancestors.pop();
        Ok(())
    }
}
