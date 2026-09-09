// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Array exotic storage and generic array-like operations. Callback algorithms
//! run as private bytecode functions, sharing VM fuel, frames and unwinding.

use super::Execution;
use crate::{
    Error, Limits, ObjectValue, Value,
    bytecode::Builtin,
    compile,
    heap::Node,
    object::{Object, Property, length_key},
    value::{Callable, FunctionValue},
};
use alloc::{format, vec::Vec};
use indexing::{index_number, wide_key};

mod concat;
mod constructor;
mod copying;
mod flatten;
mod from;
pub(super) mod indexing;
mod mutate;
mod reverse;
mod search;
mod sort;
mod species;
mod splice;

const METHODS: &[(&str, Builtin)] = &[
    ("constructor", Builtin::Array),
    ("toString", Builtin::ArrayToString),
];

impl Execution<'_> {
    pub(super) fn object_enumerator(&mut self, pairs: bool) -> Result<Value, Error> {
        if self.object_enumerators.is_none() {
            let entries = self.new_host_behavior(
                crate::heap::HostBehavior::Intrinsic(Builtin::ObjectEntries),
                "entries",
                1,
            )?;
            let roots = self.native_roots.len();
            self.native_roots.push(entries.clone());
            let values = self.new_host_behavior(
                crate::heap::HostBehavior::Intrinsic(Builtin::ObjectValues),
                "values",
                1,
            )?;
            self.object_enumerators = Some((entries, values));
            self.native_roots.truncate(roots);
        }
        let (entries, values) = self
            .object_enumerators
            .as_ref()
            .ok_or(Error::InvalidBytecode)?;
        Ok(if pairs {
            entries.clone()
        } else {
            values.clone()
        })
    }
    pub(super) fn is_array(&self, value: &Value) -> bool {
        value.heap_handle().is_some() && self.object_ref(value).is_ok_and(|o| o.array)
    }
    pub(super) fn array_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.array_proto {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_array(proto, 0)?;
        self.array_proto = Some(value.clone());
        for (name, builtin, length) in [
            ("sort", Builtin::ArraySort, 1),
            ("toSorted", Builtin::ArrayToSorted, 1),
            ("toReversed", Builtin::ArrayToReversed, 0),
            ("with", Builtin::ArrayWith, 2),
            ("toSpliced", Builtin::ArrayToSpliced, 2),
            ("flat", Builtin::ArrayFlat, 0),
            ("flatMap", Builtin::ArrayFlatMap, 1),
            ("concat", Builtin::ArrayConcat, 1),
            ("push", Builtin::ArrayPush, 1),
            ("pop", Builtin::ArrayPop, 0),
            ("join", Builtin::ArrayJoin, 1),
            ("forEach", Builtin::ArrayForEach, 1),
            ("some", Builtin::ArraySome, 1),
            ("every", Builtin::ArrayEvery, 1),
            ("reduce", Builtin::ArrayReduce, 1),
            ("indexOf", Builtin::ArrayIndexOf, 1),
            ("includes", Builtin::ArrayIncludes, 1),
            ("map", Builtin::ArrayMap, 1),
            ("filter", Builtin::ArrayFilter, 1),
            ("slice", Builtin::ArraySlice, 2),
            ("reverse", Builtin::ArrayReverse, 0),
            ("lastIndexOf", Builtin::ArrayLastIndexOf, 1),
            ("fill", Builtin::ArrayFill, 1),
            ("copyWithin", Builtin::ArrayCopyWithin, 2),
            ("splice", Builtin::ArraySplice, 2),
            ("at", Builtin::ArrayAt, 1),
            ("find", Builtin::ArrayFind, 1),
            ("findIndex", Builtin::ArrayFindIndex, 1),
            ("findLast", Builtin::ArrayFindLast, 1),
            ("findLastIndex", Builtin::ArrayFindLastIndex, 1),
        ] {
            let function = self.new_host_behavior(
                crate::heap::HostBehavior::Intrinsic(builtin),
                name,
                length,
            )?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        for (name, builtin) in METHODS {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(*builtin))
                },
            )?;
        }
        self.install_iterator(&value, Builtin::ArrayValues)?;
        for (name, kind) in [
            ("keys", Builtin::ArrayKeys),
            ("values", Builtin::ArrayValues),
            ("entries", Builtin::ArrayEntries),
        ] {
            let function = self.iterator_method(kind)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        self.array_unscopables(&value)?;
        Ok(value)
    }
    fn array_unscopables(&mut self, prototype: &Value) -> Result<(), Error> {
        let roots = self.native_roots.len();
        let result = (|| {
            let object = self.allocate_object(Value::Null)?;
            self.native_roots.push(object.clone());
            for name in [
                "at",
                "copyWithin",
                "entries",
                "fill",
                "find",
                "findIndex",
                "findLast",
                "findLastIndex",
                "flat",
                "flatMap",
                "includes",
                "keys",
                "toReversed",
                "toSorted",
                "toSpliced",
                "values",
            ] {
                self.define(
                    &object,
                    Value::string(name).units(),
                    Property::data(Value::Boolean(true)),
                )?;
            }
            let key = Value::Symbol(self.well_known("unscopables")?);
            self.define_key(
                prototype,
                key,
                Property {
                    value: object,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn allocate_array(&mut self, proto: Value, length: u32) -> Result<Value, Error> {
        self.reserve()?;
        let object = Object::new_array(proto, length, self.limits.properties)?;
        let handle = self
            .heap
            .allocate(Node::Object(object), self.limits.heap_entries)?;
        Ok(Value::Object(ObjectValue {
            handle,
            owner: self.owner.clone(),
        }))
    }
    pub(super) fn new_array(&mut self, length: u32) -> Result<Value, Error> {
        let proto = self.array_prototype()?;
        self.allocate_array(proto, length)
    }
    pub(super) fn array_from_values(&mut self, values: &[Value]) -> Result<Value, Error> {
        let size = u32::try_from(values.len()).map_err(|_| Error::Range {
            message: "array is too long",
        })?;
        let array = self.new_array(size)?;
        for (index, value) in values.iter().enumerate() {
            self.define(
                &array,
                Value::string(&format!("{index}")).units(),
                Property::data(value.clone()),
            )?;
        }
        Ok(array)
    }
    pub(super) fn array_length(&mut self, receiver: &Value) -> Result<u32, Error> {
        if matches!(receiver, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "array method needs a receiver",
            });
        }
        let length = self.get(receiver, &length_key())?;
        let value = self.numeric(&length)?;
        if value.is_nan() || value <= 0.0 {
            return Ok(0);
        }
        // Large array-like lengths need wider index arithmetic; do not wrap.
        if value > f64::from(u32::MAX) {
            return Err(Error::Limit {
                resource: "array-like length",
            });
        }
        Ok(Value::Number(value).to_uint32())
    }

    pub(super) fn array_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            if matches!(builtin, Builtin::ArraySort | Builtin::ArrayToSorted)
                && args
                    .first()
                    .is_some_and(|v| !matches!(v, Value::Undefined | Value::Function(_)))
            {
                return Err(Error::Type {
                    message: "sort comparator must be callable or undefined",
                });
            }
            let boxed;
            let receiver = if matches!(
                builtin,
                Builtin::ArrayPush
                    | Builtin::ArrayPop
                    | Builtin::ArrayJoin
                    | Builtin::ArraySlice
                    | Builtin::ArraySort
                    | Builtin::ArrayToSorted
                    | Builtin::ArrayToReversed
                    | Builtin::ArrayWith
                    | Builtin::ArrayToSpliced
                    | Builtin::ArrayFlat
                    | Builtin::ArrayFlatMap
                    | Builtin::ArrayIndexOf
                    | Builtin::ArrayIncludes
                    | Builtin::ArrayConcat
                    | Builtin::ArrayReverse
                    | Builtin::ArrayLastIndexOf
                    | Builtin::ArrayFill
                    | Builtin::ArrayCopyWithin
                    | Builtin::ArrayAt
                    | Builtin::ArraySplice
            ) {
                boxed = self.box_value(receiver)?;
                self.native_roots.push(boxed.clone());
                &boxed
            } else {
                receiver
            };
            self.array_call_rooted(builtin, receiver, args)
        })();
        self.native_roots.truncate(roots);
        result
    }

    fn array_call_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let first = args.first().unwrap_or(&Value::Undefined);
        if matches!(builtin, Builtin::ObjectEntries | Builtin::ObjectValues) {
            return self
                .object_entries(first, builtin == Builtin::ObjectEntries)
                .map(Some);
        }
        let value = match builtin {
            Builtin::ArraySort | Builtin::ArrayToSorted => {
                self.array_sort(receiver, first, builtin == Builtin::ArrayToSorted)?
            }
            Builtin::ArrayConcat => self.array_concat(receiver, args)?,
            Builtin::ArrayReverse => self.array_reverse(receiver)?,
            Builtin::ArrayFill => self.array_fill(receiver, args)?,
            Builtin::ArrayCopyWithin => self.array_copy_within(receiver, args)?,
            Builtin::ArrayLastIndexOf => self.array_last_index_of(receiver, first, args.get(1))?,
            Builtin::ArrayAt => self.array_at(receiver, first)?,
            Builtin::ArraySplice => self.array_splice(receiver, args)?,
            Builtin::ArrayFlat | Builtin::ArrayFlatMap => {
                self.array_flatten(receiver, args, builtin == Builtin::ArrayFlatMap)?
            }
            Builtin::ArrayToReversed => self.array_to_reversed(receiver)?,
            Builtin::ArrayWith => {
                self.array_with(receiver, first, args.get(1).unwrap_or(&Value::Undefined))?
            }
            Builtin::ArrayToSpliced => self.array_to_spliced(receiver, args)?,
            Builtin::Array => {
                if args.len() == 1 && matches!(first, Value::Number(_)) {
                    let length = first.to_uint32();
                    if !same_number(first, length) {
                        return Err(Error::Range {
                            message: "invalid array length",
                        });
                    }
                    self.new_array(length)?
                } else {
                    self.array_from_values(args)?
                }
            }
            Builtin::ArrayOf => self.array_of(receiver, args)?,
            Builtin::ArrayFrom => self.array_from(receiver, args)?,
            Builtin::ArrayIsArray => Value::Boolean(self.is_array(first)),
            Builtin::ArrayPush => self.array_push(receiver, args)?,
            Builtin::ArrayPop => {
                let length = self.array_length_wide(receiver)?;
                let value = if length == 0 {
                    Value::Undefined
                } else {
                    let key = wide_key(length.saturating_sub(1));
                    let value = self.get(receiver, &key)?;
                    self.array_delete_or_throw(receiver, &key)?;
                    value
                };
                self.native_roots.push(value.clone());
                self.set(
                    receiver,
                    length_key(),
                    Value::Number(index_number(length.saturating_sub(1))),
                    true,
                )?;
                value
            }
            Builtin::ArrayJoin => self.array_join(receiver, first)?,
            Builtin::ArraySlice => {
                self.array_slice(receiver, first, args.get(1).unwrap_or(&Value::Undefined))?
            }
            Builtin::ArrayIndexOf | Builtin::ArrayIncludes => self.array_search(
                builtin,
                receiver,
                first,
                args.get(1).unwrap_or(&Value::Undefined),
            )?,
            Builtin::ObjectKeys | Builtin::GetOwnPropertyNames => {
                return self
                    .object_keys(first, builtin == Builtin::GetOwnPropertyNames)
                    .map(Some);
            }
            Builtin::InternalDefine => {
                self.define(
                    first,
                    args.get(1).ok_or(Error::InvalidBytecode)?.units(),
                    Property::data(args.get(2).ok_or(Error::InvalidBytecode)?.clone()),
                )?;
                Value::Undefined
            }
            Builtin::InternalHas => Value::Boolean(
                self.present(first, &args.get(1).ok_or(Error::InvalidBytecode)?.units())?,
            ),
            Builtin::InternalReduceEmpty => {
                return Err(Error::Type {
                    message: "reduce of empty array without an initial value",
                });
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }

    fn array_join(&mut self, receiver: &Value, separator: &Value) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let separator = if matches!(separator, Value::Undefined) {
            Value::string(",").units()
        } else {
            self.string_units(separator)?
        };
        let mut units = Vec::new();
        for index in 0..length {
            self.charge(1)?;
            if index > 0 {
                append(&mut units, &separator, self.limits.string_units)?;
            }
            let value = self.get(receiver, &wide_key(index))?;
            if !matches!(value, Value::Null | Value::Undefined) {
                let element = self.string_units(&value)?;
                append(&mut units, &element, self.limits.string_units)?;
            }
        }
        Ok(Value::String(units.into()))
    }

    fn array_push(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let mut length = self.array_length_wide(receiver)?;
        let count = u64::try_from(args.len()).map_err(|_| Error::Type {
            message: "push exceeds maximum array-like length",
        })?;
        if length
            .checked_add(count)
            .is_none_or(|n| n > 9_007_199_254_740_991)
        {
            return Err(Error::Type {
                message: "push exceeds maximum array-like length",
            });
        }
        for value in args {
            self.charge(1)?;
            self.set(receiver, wide_key(length), value.clone(), true)?;
            length = length.saturating_add(1);
        }
        let length = Value::Number(index_number(length));
        self.set(receiver, length_key(), length.clone(), true)?;
        Ok(length)
    }

    fn object_entries(&mut self, object: &Value, pairs: bool) -> Result<Value, Error> {
        let object = self.box_value(object)?;
        self.native_roots.push(object.clone());
        let keys = self.own_keys(&object)?;
        let mut values = Vec::new();
        for key in keys {
            self.charge(1)?;
            if self.own(&object, &key)?.is_some_and(|p| p.enumerable) {
                let value = self.get(&object, &key)?;
                self.native_roots.push(value.clone());
                let value = if pairs {
                    self.array_from_values(&[Value::String(key), value])?
                } else {
                    value
                };
                self.native_roots.push(value.clone());
                values.push(value);
            }
        }
        self.array_from_values(&values)
    }

    fn object_keys(&mut self, object: &Value, all: bool) -> Result<Value, Error> {
        let object = self.box_value(object)?;
        self.native_roots.push(object.clone());
        let mut values = Vec::new();
        for key in self.own_keys(&object)? {
            if all || self.own(&object, &key)?.is_some_and(|p| p.enumerable) {
                values.push(Value::String(key));
            }
        }
        self.array_from_values(&values)
    }

    fn array_slice(
        &mut self,
        receiver: &Value,
        start: &Value,
        end: &Value,
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let start = self.array_clamped_index(start, length)?;
        let end = if matches!(end, Value::Undefined) {
            length
        } else {
            self.array_clamped_index(end, length)?
        };
        let count = end.saturating_sub(start);
        let array = self.array_species_create(receiver, count)?;
        self.native_roots.push(array.clone());
        // No heap allocation occurs between creation and publication here.
        for index in start..end {
            self.charge(1)?;
            if self.present(receiver, &wide_key(index))? {
                let value = self.get(receiver, &wide_key(index))?;
                self.define(
                    &array,
                    wide_key(index.saturating_sub(start)),
                    Property::data(value),
                )?;
            }
        }
        self.set(
            &array,
            length_key(),
            Value::Number(index_number(count)),
            true,
        )?;
        Ok(array)
    }
    fn present(&mut self, receiver: &Value, key: &[u16]) -> Result<bool, Error> {
        if matches!(receiver, Value::String(_)) {
            Ok(self.own(receiver, key)?.is_some())
        } else {
            self.has_property(receiver, key)
        }
    }
    fn array_search(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        value: &Value,
        from: &Value,
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        if length == 0 {
            return Ok(if builtin == Builtin::ArrayIncludes {
                Value::Boolean(false)
            } else {
                Value::Number(-1.0)
            });
        }
        let start = self.array_clamped_index(from, length)?;
        for index in start..length {
            self.charge(1)?;
            if builtin == Builtin::ArrayIncludes || self.present(receiver, &wide_key(index))? {
                let element = self.get(receiver, &wide_key(index))?;
                let equal = element.strictly_equals(value)
                    || (builtin == Builtin::ArrayIncludes
                        && matches!((&element, value), (Value::Number(a), Value::Number(b)) if a.is_nan() && b.is_nan()));
                if equal {
                    return Ok(if builtin == Builtin::ArrayIncludes {
                        Value::Boolean(true)
                    } else {
                        Value::Number(index_number(index))
                    });
                }
            }
        }
        Ok(if builtin == Builtin::ArrayIncludes {
            Value::Boolean(false)
        } else {
            Value::Number(-1.0)
        })
    }

    pub(super) fn prepare_array_callback(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
        start: usize,
    ) -> Result<Option<usize>, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.prepare_callback_rooted(builtin, receiver, args, start);
        self.native_roots.truncate(roots);
        result
    }

    fn prepare_callback_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
        start: usize,
    ) -> Result<Option<usize>, Error> {
        if builtin == Builtin::ArrayToString {
            let receiver = self.box_value(receiver)?;
            self.native_roots.push(receiver.clone());
            let join = self.get(&receiver, &Value::string("join").units())?;
            let function = if matches!(join, Value::Function(_)) {
                join
            } else {
                native(Builtin::ObjectToString)
            };
            self.stack.truncate(start);
            self.push(function)?;
            self.push(receiver.clone())?;
            return Ok(Some(0));
        }
        let Some(body) = callback_body(builtin) else {
            return Ok(None);
        };
        let receiver = self.box_value(receiver)?;
        self.native_roots.push(receiver.clone());
        let length = self.array_length_wide(&receiver)?;
        let callback = args.first().unwrap_or(&Value::Undefined);
        if !matches!(callback, Value::Function(_)) {
            return Err(Error::Type {
                message: "array callback is not callable",
            });
        }
        // Species construction follows callable validation but precedes any
        // element read. Keep even aliased/non-array results rooted across hooks.
        let output = if matches!(builtin, Builtin::ArrayMap | Builtin::ArrayFilter) {
            let output = self.array_species_create(
                &receiver,
                if builtin == Builtin::ArrayMap {
                    length
                } else {
                    0
                },
            )?;
            self.native_roots.push(output.clone());
            output
        } else {
            Value::Undefined
        };
        // Internal functions are compiled once per Runtime. Their private
        // parameters, not globals, supply non-overridable abstract operations.
        let code = if let Some((_, code)) =
            self.intrinsic_code.iter().find(|(id, _)| *id == builtin)
        {
            code.clone()
        } else {
            let source = format!(
                "function(callback,receiver,length,thisArg,initial,hasInitial,invoke,out,define,has,empty){{'use strict';{body}}}"
            );
            let program = compile(&format!("({source})"), Limits::default())?;
            let code = program
                .functions
                .first()
                .ok_or(Error::InvalidBytecode)?
                .clone();
            self.intrinsic_code.push((builtin, code.clone()));
            code
        };
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: Vec::new(),
                lexical_this: None,
                object: Object::new(Value::Null),
            },
            self.limits.heap_entries,
        )?;
        let function = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        let callback = callback.clone();
        let this = args.get(1).cloned().unwrap_or(Value::Undefined);
        let arguments = [
            callback,
            receiver.clone(),
            Value::Number(index_number(length)),
            this.clone(),
            this,
            Value::Boolean(args.len() > 1),
            native(Builtin::InternalCall),
            output,
            native(Builtin::InternalDefine),
            native(Builtin::InternalHas),
            native(Builtin::InternalReduceEmpty),
        ];
        self.stack.truncate(start);
        self.push(function)?;
        self.push(Value::Undefined)?;
        for value in arguments {
            self.push(value)?;
        }
        Ok(Some(11))
    }
}

const fn callback_body(builtin: Builtin) -> Option<&'static str> {
    if let Some(body) = search::find_body(builtin) {
        return Some(body);
    }
    Some(match builtin {
        Builtin::ArrayForEach => {
            "for(let k=0;k<length;k++){if(has(receiver,k)){invoke(callback,thisArg,receiver[k],k,receiver);}}"
        }
        Builtin::ArrayMap => {
            "for(let k=0;k<length;k++){if(has(receiver,k)){define(out,k,invoke(callback,thisArg,receiver[k],k,receiver));}}return out;"
        }
        Builtin::ArrayFilter => {
            "let to=0;for(let k=0;k<length;k++){if(has(receiver,k)){let value=receiver[k];if(invoke(callback,thisArg,value,k,receiver)){define(out,to++,value);}}}return out;"
        }
        Builtin::ArraySome => {
            "for(let k=0;k<length;k++){if(has(receiver,k)){if(invoke(callback,thisArg,receiver[k],k,receiver))return true;}}return false;"
        }
        Builtin::ArrayEvery => {
            "for(let k=0;k<length;k++){if(has(receiver,k)){if(!invoke(callback,thisArg,receiver[k],k,receiver))return false;}}return true;"
        }
        Builtin::ArrayReduce => {
            "let k=0;let acc=initial;if(!hasInitial){while(k<length && !has(receiver,k)){k++;}if(k>=length){empty();}acc=receiver[k++];}for(;k<length;k++){if(has(receiver,k)){acc=invoke(callback,undefined,acc,receiver[k],k,receiver);}}return acc;"
        }
        _ => return None,
    })
}
const fn native(builtin: Builtin) -> Value {
    Value::Function(FunctionValue::native(builtin))
}
#[expect(
    clippy::float_cmp,
    reason = "Array constructor length must equal its uint32 conversion exactly"
)]
fn same_number(value: &Value, length: u32) -> bool {
    value.to_number() == f64::from(length)
}
fn append(out: &mut Vec<u16>, units: &[u16], limit: usize) -> Result<(), Error> {
    if out.len().checked_add(units.len()).is_none_or(|n| n > limit) {
        return Err(Error::Limit {
            resource: "string units",
        });
    }
    out.extend_from_slice(units);
    Ok(())
}
