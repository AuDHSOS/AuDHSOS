// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Synchronous iterator protocols. Cached next methods and completion-aware
//! closing share fuel and native roots with normal calls.

use super::{Execution, arrays::indexing::index_number, exceptions::Completion};
use crate::{
    Error, Value,
    bytecode::Builtin,
    iterator::{Kind, Record, State},
    object::Property,
};
use alloc::rc::Rc;

impl Execution<'_> {
    pub(super) fn iterator_method(&mut self, builtin: Builtin) -> Result<Value, Error> {
        if let Some((_, function)) = self
            .iterator_methods
            .iter()
            .find(|(kind, _)| *kind == builtin)
        {
            return Ok(function.clone());
        }
        let name = match builtin {
            Builtin::ArrayValues => "values",
            Builtin::ArrayKeys => "keys",
            Builtin::ArrayEntries => "entries",
            Builtin::StringIterator | Builtin::IteratorIdentity => "[Symbol.iterator]",
            Builtin::ArrayIteratorNext | Builtin::StringIteratorNext => "next",
            _ => return Err(Error::InvalidBytecode),
        };
        let function =
            self.new_host_behavior(crate::heap::HostBehavior::Intrinsic(builtin), name, 0)?;
        self.iterator_methods.push((builtin, function.clone()));
        Ok(function)
    }
    pub(super) fn iterator_prototype(&mut self, kind: Builtin) -> Result<Value, Error> {
        if let Some((_, v)) = self.iterator_protos.iter().find(|(k, _)| *k == kind) {
            return Ok(v.clone());
        }
        let parent = if kind == Builtin::IteratorIdentity {
            self.prototype()?
        } else {
            self.iterator_prototype(Builtin::IteratorIdentity)?
        };
        let object = self.allocate_object(parent)?;
        self.iterator_protos.push((kind, object.clone()));
        if kind == Builtin::IteratorIdentity {
            self.install_iterator(&object, Builtin::IteratorIdentity)?;
        } else {
            let function = self.iterator_method(kind)?;
            self.define(
                &object,
                Value::string("next").units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
            self.install_tag(
                &object,
                if kind == Builtin::StringIteratorNext {
                    "String Iterator"
                } else {
                    "Array Iterator"
                },
            )?;
        }
        Ok(object)
    }
    pub(super) fn install_iterator(
        &mut self,
        object: &Value,
        method: Builtin,
    ) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(object.clone());
        let result = (|| {
            let key = Value::Symbol(self.well_known("iterator")?);
            let function = self.iterator_method(method)?;
            self.define_key(
                object,
                key,
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn iterator_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
    ) -> Result<Option<Value>, Error> {
        if builtin == Builtin::IteratorIdentity {
            return Ok(Some(receiver.clone()));
        }
        let kind = match builtin {
            Builtin::ArrayKeys => Kind::Keys,
            Builtin::ArrayValues => Kind::Values,
            Builtin::ArrayEntries => Kind::Entries,
            Builtin::StringIterator => Kind::String,
            Builtin::ArrayIteratorNext | Builtin::StringIteratorNext => {
                let roots = self.native_roots.len();
                self.native_roots.push(receiver.clone());
                let result = self.builtin_iterator_next(receiver, builtin);
                self.native_roots.truncate(roots);
                return result.map(Some);
            }
            _ => return Ok(None),
        };
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        let result = (|| {
            let source = if kind == Kind::String {
                if matches!(receiver, Value::Null | Value::Undefined) {
                    return Err(Error::Type {
                        message: "string iterator receiver is nullish",
                    });
                }
                Value::String(self.string_units(receiver)?)
            } else {
                self.box_value(receiver)?
            };
            self.native_roots.push(source.clone());
            let proto = self.iterator_prototype(if kind == Kind::String {
                Builtin::StringIteratorNext
            } else {
                Builtin::ArrayIteratorNext
            })?;
            let object = self.allocate_object(proto)?;
            self.object_mut(&object)?.iterator = Some(State {
                source,
                index: 0,
                kind,
            });
            Ok(object)
        })();
        self.native_roots.truncate(roots);
        result.map(Some)
    }
    fn builtin_iterator_next(&mut self, object: &Value, builtin: Builtin) -> Result<Value, Error> {
        let state = self
            .object_ref(object)?
            .iterator
            .clone()
            .ok_or(Error::Type {
                message: "iterator receiver required",
            })?;
        if (state.kind == Kind::String) != (builtin == Builtin::StringIteratorNext) {
            return Err(Error::Type {
                message: "incompatible iterator brand",
            });
        }
        if matches!(state.source, Value::Undefined) {
            return self.iterator_result(Value::Undefined, true);
        }
        self.native_roots.push(state.source.clone());
        if state.kind == Kind::String {
            return self.string_iterator_next(object, &state);
        }
        let length = self.array_length_wide(&state.source)?;
        if state.index >= length {
            self.object_mut(object)?
                .iterator
                .as_mut()
                .ok_or(Error::InvalidBytecode)?
                .source = Value::Undefined;
            return self.iterator_result(Value::Undefined, true);
        }
        self.object_mut(object)?
            .iterator
            .as_mut()
            .ok_or(Error::InvalidBytecode)?
            .index = state.index.saturating_add(1);
        let index = Value::Number(index_number(state.index));
        let value = if state.kind == Kind::Keys {
            index
        } else {
            let value = self.get(&state.source, &index.units())?;
            self.native_roots.push(value.clone());
            if state.kind == Kind::Entries {
                self.array_from_values(&[index, value])?
            } else {
                value
            }
        };
        self.iterator_result(value, false)
    }
    fn string_iterator_next(&mut self, object: &Value, state: &State) -> Result<Value, Error> {
        let Value::String(units) = &state.source else {
            return Err(Error::InvalidBytecode);
        };
        let index = usize::try_from(state.index).map_err(|_| Error::InvalidBytecode)?;
        let Some(first) = units.get(index) else {
            self.object_mut(object)?
                .iterator
                .as_mut()
                .ok_or(Error::InvalidBytecode)?
                .source = Value::Undefined;
            return self.iterator_result(Value::Undefined, true);
        };
        let width = if (0xd800..=0xdbff).contains(first)
            && units
                .get(index.saturating_add(1))
                .is_some_and(|u| (0xdc00..=0xdfff).contains(u))
        {
            2
        } else {
            1
        };
        let end = index.saturating_add(width);
        self.object_mut(object)?
            .iterator
            .as_mut()
            .ok_or(Error::InvalidBytecode)?
            .index = u64::try_from(end).map_err(|_| Error::Limit {
            resource: "string iterator index",
        })?;
        let value = Value::String(Rc::from(
            units.get(index..end).ok_or(Error::InvalidBytecode)?,
        ));
        self.iterator_result(value, false)
    }
    fn iterator_result(&mut self, value: Value, done: bool) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let proto = self.prototype()?;
            let object = self.allocate_object(proto)?;
            self.define(
                &object,
                Value::string("value").units(),
                Property::data(value),
            )?;
            self.define(
                &object,
                Value::string("done").units(),
                Property::data(Value::Boolean(done)),
            )?;
            Ok(object)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn get_iterator(&mut self, input: &Value) -> Result<Record, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(input.clone());
        let result = (|| {
            let key = Value::Symbol(self.well_known("iterator")?);
            let method = self.get_key(input, &key)?;
            if !matches!(method, Value::Function(_)) {
                return Err(Error::Type {
                    message: "value is not iterable",
                });
            }
            self.get_iterator_from(input, method)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn get_iterator_from(
        &mut self,
        input: &Value,
        method: Value,
    ) -> Result<Record, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(input.clone());
        let result = (|| {
            let object = self.call_sync(method, input.clone(), &[])?;
            if !matches!(object, Value::Object(_) | Value::Function(_)) {
                return Err(Error::Type {
                    message: "iterator is not an object",
                });
            }
            self.native_roots.push(object.clone());
            let next = self.get(&object, &Value::string("next").units())?;
            Ok(Record {
                object,
                next,
                done: false,
            })
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn iterator_step(&mut self, record: &mut Record) -> Result<Option<Value>, Error> {
        self.iterator_step_value(record, true)
    }
    pub(super) fn iterator_step_value(
        &mut self,
        record: &mut Record,
        read_value: bool,
    ) -> Result<Option<Value>, Error> {
        if record.done {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots
            .extend([record.object.clone(), record.next.clone()]);
        let result = (|| {
            self.charge(1)?;
            let result = self.call_sync(record.next.clone(), record.object.clone(), &[])?;
            if !matches!(result, Value::Object(_) | Value::Function(_)) {
                return Err(Error::Type {
                    message: "iterator result is not an object",
                });
            }
            self.native_roots.push(result.clone());
            if self
                .get(&result, &Value::string("done").units())?
                .to_boolean()
            {
                record.done = true;
                return Ok(None);
            }
            if read_value {
                self.get(&result, &Value::string("value").units()).map(Some)
            } else {
                Ok(Some(Value::Undefined))
            }
        })();
        if result.is_err() {
            record.done = true;
        }
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn iterator_close(
        &mut self,
        record: &mut Record,
        throwing: bool,
    ) -> Result<(), Error> {
        if record.done {
            return Ok(());
        }
        record.done = true;
        let roots = self.native_roots.len();
        self.native_roots
            .extend([record.object.clone(), record.next.clone()]);
        let result = (|| {
            let method = self.get(&record.object, &Value::string("return").units())?;
            if matches!(method, Value::Undefined | Value::Null) {
                return Ok(());
            }
            let result = self.call_sync(method, record.object.clone(), &[])?;
            if !matches!(result, Value::Object(_) | Value::Function(_)) {
                return Err(Error::Type {
                    message: "iterator return result is not an object",
                });
            }
            Ok(())
        })();
        self.native_roots.truncate(roots);
        match result {
            Err(error) if throwing && language_error(&error) => Ok(()),
            other => other,
        }
    }
    pub(super) fn close_iteration(&mut self, id: usize, throwing: bool) -> Result<(), Error> {
        let state = self
            .frames
            .last_mut()
            .and_then(|f| f.enumerations.get_mut(id))
            .and_then(Option::take)
            .ok_or(Error::InvalidBytecode)?;
        if let Some(mut record) = state.iterator {
            self.iterator_close(&mut record, throwing)
        } else {
            Ok(())
        }
    }
    pub(super) fn close_iteration_completion(
        &mut self,
        id: usize,
        completion: Completion,
    ) -> Result<Completion, Error> {
        let roots = self.native_roots.len();
        if let Completion::Return(v)
        | Completion::Normal(v)
        | Completion::Throw(Error::Thrown { value: v }) = &completion
        {
            self.native_roots.push(v.clone());
        }
        let result = self.close_iteration(id, matches!(completion, Completion::Throw(_)));
        self.native_roots.truncate(roots);
        match result {
            Ok(()) => Ok(completion),
            Err(error) if language_error(&error) => Ok(Completion::Throw(error)),
            Err(error) => Err(error),
        }
    }
}
pub(super) const fn language_error(error: &Error) -> bool {
    matches!(
        error,
        Error::Thrown { .. }
            | Error::Type { .. }
            | Error::Range { .. }
            | Error::Reference { .. }
            | Error::Syntax { .. }
    )
}
#[cfg(test)]
#[path = "../tests/iterator_wide.rs"]
mod tests;
