// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Primitive wrappers and their realm prototypes. Character properties are
//! virtual; ordinary primitive reads need no transient wrapper allocation.

use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    object::{Property, length_key},
    value::FunctionValue,
};
use alloc::{rc::Rc, vec::Vec};

pub(super) const fn kind(value: &Value) -> Option<Builtin> {
    match value {
        Value::String(_) => Some(Builtin::String),
        Value::Number(_) => Some(Builtin::Number),
        Value::Boolean(_) => Some(Builtin::Boolean),
        Value::Symbol(_) => Some(Builtin::Symbol),
        _ => None,
    }
}

impl Execution<'_> {
    pub(super) fn primitive_prototype(&mut self, kind: Builtin) -> Result<Value, Error> {
        if kind == Builtin::Symbol {
            return self.symbol_prototype();
        }
        if let Some((_, value)) = self.primitive_protos.iter().find(|(id, _)| *id == kind) {
            return Ok(value.clone());
        }
        let (primitive, value_of, to_string) = match kind {
            Builtin::Boolean => (
                Value::Boolean(false),
                Builtin::BooleanValueOf,
                Builtin::BooleanToString,
            ),
            Builtin::Number => (
                Value::Number(0.0),
                Builtin::NumberValueOf,
                Builtin::NumberToString,
            ),
            Builtin::String => (
                Value::string(""),
                Builtin::StringValueOf,
                Builtin::StringToString,
            ),
            _ => return Err(Error::InvalidBytecode),
        };
        let parent = self.prototype()?;
        let value = self.allocate_object(parent)?;
        self.primitive_protos.push((kind, value.clone()));
        self.initialize_wrapper(&value, primitive)?;
        for (name, builtin) in [
            ("constructor", kind),
            ("valueOf", value_of),
            ("toString", to_string),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(Value::Function(FunctionValue::native(builtin)))
                },
            )?;
        }
        if kind == Builtin::String {
            self.install_string_methods(&value)?;
            self.install_iterator(&value, Builtin::StringIterator)?;
            for (name, kind) in [
                ("match", Builtin::StringMatch),
                ("search", Builtin::StringSearch),
                ("split", Builtin::StringSplit),
                ("replace", Builtin::StringReplace),
            ] {
                let function = self.new_host_behavior(
                    crate::heap::HostBehavior::Intrinsic(kind),
                    name,
                    if matches!(kind, Builtin::StringSplit | Builtin::StringReplace) {
                        2
                    } else {
                        1
                    },
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
        }
        Ok(value)
    }

    fn initialize_wrapper(&mut self, object: &Value, primitive: Value) -> Result<(), Error> {
        if let Value::String(units) = &primitive {
            if units.len() > self.limits.string_units
                || u32::try_from(units.len()).is_err()
                || units.len() == usize::try_from(u32::MAX).unwrap_or(usize::MAX)
            {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            self.define(
                object,
                length_key(),
                Property {
                    value: Value::Number(f64::from(
                        u32::try_from(units.len()).map_err(|_| Error::InvalidBytecode)?,
                    )),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
        }
        self.object_mut(object)?.primitive = Some(primitive);
        Ok(())
    }

    pub(super) fn box_value(&mut self, value: &Value) -> Result<Value, Error> {
        if matches!(value, Value::Object(_) | Value::Function(_)) {
            return Ok(value.clone());
        }
        let kind = kind(value).ok_or(Error::Type {
            message: "cannot box null or undefined",
        })?;
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let proto = self.primitive_prototype(kind)?;
            let object = self.allocate_object(proto)?;
            self.initialize_wrapper(&object, value.clone())?;
            Ok(object)
        })();
        self.native_roots.truncate(roots);
        result
    }

    pub(super) fn boxing_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let value = match builtin {
            Builtin::BooleanConstruct => {
                Value::Boolean(args.first().is_some_and(Value::to_boolean))
            }
            Builtin::NumberConstruct => Value::Number(if let Some(value) = args.first() {
                self.numeric(value)?
            } else {
                0.0
            }),
            Builtin::StringConstruct => {
                if let Some(value) = args.first() {
                    Value::String(self.string_units(value)?)
                } else {
                    Value::string("")
                }
            }
            _ => return self.wrapper_method(builtin, receiver, args),
        };
        self.box_value(&value).map(Some)
    }

    fn wrapper_method(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let (expected, stringify) = match builtin {
            Builtin::BooleanValueOf => (Builtin::Boolean, false),
            Builtin::NumberValueOf => (Builtin::Number, false),
            Builtin::StringValueOf => (Builtin::String, false),
            Builtin::BooleanToString => (Builtin::Boolean, true),
            Builtin::NumberToString => (Builtin::Number, true),
            Builtin::StringToString => (Builtin::String, true),
            _ => return Ok(None),
        };
        let value = if kind(receiver) == Some(expected) {
            receiver.clone()
        } else {
            self.object_ref(receiver)
                .ok()
                .and_then(|o| o.primitive.clone())
                .filter(|v| kind(v) == Some(expected))
                .ok_or(Error::Type {
                    message: "incompatible primitive wrapper receiver",
                })?
        };
        if builtin == Builtin::NumberToString
            && let Some(radix) = args.first().filter(|v| !matches!(v, Value::Undefined))
        {
            let n = self.numeric(radix)?;
            let n = n - n % 1.0;
            if !(2.0..=36.0).contains(&n) {
                return Err(Error::Range {
                    message: "invalid number radix",
                });
            }
            if !Value::Number(n).strictly_equals(&Value::Number(10.0)) {
                return Err(Error::Type {
                    message: "non-decimal number formatting is not implemented",
                });
            }
        }
        Ok(Some(if stringify {
            Value::String(value.units())
        } else {
            value
        }))
    }

    pub(super) fn own_keys(&mut self, value: &Value) -> Result<Vec<Rc<[u16]>>, Error> {
        if super::properties::has_constructor_storage(value) {
            let storage = self.constructor_storage(value)?;
            return self.own_keys(&storage);
        }
        if matches!(
            value,
            Value::Function(FunctionValue(crate::value::Callable::Native(
                Builtin::Function
            )))
        ) {
            self.charge(3)?;
            return Ok(["length", "name", "prototype"]
                .iter()
                .map(|s| Value::string(s).units())
                .collect());
        }
        let object = self.object_ref(value)?;
        let mut keys = Vec::new();
        if let Some(Value::String(units)) = &object.primitive {
            if units.len().saturating_add(object.properties.len()) > self.limits.properties {
                return Err(Error::Limit {
                    resource: "enumeration keys",
                });
            }
            for index in 0..units.len() {
                keys.push(Value::string(&alloc::format!("{index}")).units());
            }
        }
        keys.extend(object.keys());
        self.charge(u64::try_from(keys.len()).unwrap_or(u64::MAX))?;
        Ok(keys)
    }
}

pub(super) const NUMBER_CONSTANTS: &[(&str, f64)] = &[
    ("EPSILON", f64::EPSILON),
    ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
    ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    ("MAX_VALUE", f64::MAX),
    ("MIN_VALUE", f64::from_bits(1)),
    ("NaN", f64::NAN),
    ("POSITIVE_INFINITY", f64::INFINITY),
    ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
];
