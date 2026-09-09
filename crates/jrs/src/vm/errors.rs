// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Error and `NativeError` intrinsics with distinct prototype identities.

use super::Execution;
use crate::{Error, Value, bytecode::Builtin, object::Property, value::FunctionValue};

pub(super) const fn error_name(kind: Builtin) -> Option<&'static str> {
    match kind {
        Builtin::Error => Some("Error"),
        Builtin::TypeError => Some("TypeError"),
        Builtin::RangeError => Some("RangeError"),
        Builtin::ReferenceError => Some("ReferenceError"),
        Builtin::SyntaxError => Some("SyntaxError"),
        Builtin::EvalError => Some("EvalError"),
        Builtin::URIError => Some("URIError"),
        _ => None,
    }
}
impl Execution<'_> {
    pub(super) fn error_prototype(&mut self, kind: Builtin) -> Result<Value, Error> {
        if let Some((_, value)) = self.error_prototypes.iter().find(|(id, _)| *id == kind) {
            return Ok(value.clone());
        }
        let name = error_name(kind).ok_or(Error::InvalidBytecode)?;
        let parent = if kind == Builtin::Error {
            self.prototype()?
        } else {
            self.error_prototype(Builtin::Error)?
        };
        let value = self.allocate_object(parent)?;
        self.error_prototypes.push((kind, value.clone()));
        for (key, item) in [
            ("name", Value::string(name)),
            ("message", Value::string("")),
            ("constructor", Value::Function(FunctionValue::native(kind))),
        ] {
            self.define(
                &value,
                Value::string(key).units(),
                Property {
                    enumerable: false,
                    ..Property::data(item)
                },
            )?;
        }
        if kind == Builtin::Error {
            self.define(
                &value,
                Value::string("toString").units(),
                Property {
                    enumerable: false,
                    ..Property::data(Value::Function(FunctionValue::native(
                        Builtin::ErrorToString,
                    )))
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn new_error(&mut self, kind: Builtin, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            let proto = self.error_prototype(kind)?;
            let object = self.allocate_object(proto)?;
            self.object_mut(&object)?.error = true;
            self.native_roots.push(object.clone());
            if let Some(message) = args.first().filter(|v| !matches!(v, Value::Undefined)) {
                let message = Value::String(self.string_units(message)?);
                self.define(
                    &object,
                    Value::string("message").units(),
                    Property {
                        enumerable: false,
                        ..Property::data(message)
                    },
                )?;
            }
            if let Some(options) = args
                .get(1)
                .filter(|v| matches!(v, Value::Object(_) | Value::Function(_)))
                && self.has_property(options, &Value::string("cause").units())?
            {
                let cause = self.get(options, &Value::string("cause").units())?;
                self.define(
                    &object,
                    Value::string("cause").units(),
                    Property {
                        enumerable: false,
                        ..Property::data(cause)
                    },
                )?;
            }
            Ok(object)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn error_call(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if error_name(kind).is_some() {
            return self.new_error(kind, args).map(Some);
        }
        if kind == Builtin::ErrorIsError {
            return Ok(Some(Value::Boolean(
                args.first()
                    .is_some_and(|v| self.object_ref(v).is_ok_and(|o| o.error)),
            )));
        }
        if kind != Builtin::ErrorToString {
            return Ok(None);
        }
        self.object_ref(receiver)?;
        let name = self.get(receiver, &Value::string("name").units())?;
        let name = if matches!(name, Value::Undefined) {
            Value::string("Error").units()
        } else {
            self.string_units(&name)?
        };
        let message = self.get(receiver, &Value::string("message").units())?;
        let message = if matches!(message, Value::Undefined) {
            Value::string("").units()
        } else {
            self.string_units(&message)?
        };
        if name.is_empty() {
            return Ok(Some(Value::String(message)));
        }
        if message.is_empty() {
            return Ok(Some(Value::String(name)));
        }
        let mut text = name.to_vec();
        text.extend([58, 32]);
        text.extend_from_slice(&message);
        if text.len() > self.limits.string_units {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        Ok(Some(Value::String(text.into())))
    }
}
