// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Same-realm `RegExp` intrinsic objects, brands and observable construction.
use super::{Builtin, Error, Execution, Property, Rc, RegExp, String, Value};
use crate::{heap::HostBehavior, regexp::Field, value::FunctionValue};

const FLAGS: &[(&str, char)] = &[
    ("hasIndices", 'd'),
    ("global", 'g'),
    ("ignoreCase", 'i'),
    ("multiline", 'm'),
    ("dotAll", 's'),
    ("unicode", 'u'),
    ("unicodeSets", 'v'),
    ("sticky", 'y'),
];

impl Execution<'_> {
    pub(in crate::vm) fn regexp_constructor(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.regexp_constructor_storage {
            return Ok(value.clone());
        }
        let proto = self.function_prototype()?;
        let value = self.allocate_object(proto)?;
        self.regexp_constructor_storage = Some(value.clone());
        for (name, item, configurable) in [
            ("length", Value::Number(2.0), true),
            ("name", Value::string("RegExp"), true),
            ("prototype", self.regexp_prototype()?, false),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: item,
                    writable: false,
                    enumerable: false,
                    configurable,
                    accessor: None,
                },
            )?;
        }
        let key = Value::Symbol(self.well_known("species")?);
        let getter = self.new_host_behavior(
            HostBehavior::Intrinsic(Builtin::ArraySpecies),
            "get [Symbol.species]",
            0,
        )?;
        self.define_key(
            &value,
            key,
            Property {
                value: Value::Undefined,
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: Some((getter, Value::Undefined)),
            },
        )?;
        Ok(value)
    }
    pub(in crate::vm) fn regexp_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.regexp_proto {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.regexp_proto = Some(value.clone());
        self.define(
            &value,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(Value::Function(FunctionValue::native(Builtin::RegExp)))
            },
        )?;
        for (name, kind, length) in [
            ("exec", Builtin::RegExpExec, 1),
            ("test", Builtin::RegExpTest, 1),
            ("toString", Builtin::RegExpToString, 0),
        ] {
            let method = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, length)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(method)
                },
            )?;
        }
        for (name, kind) in [
            ("match", Builtin::RegExpMatch),
            ("search", Builtin::RegExpSearch),
            ("split", Builtin::RegExpSplit),
            ("replace", Builtin::RegExpReplace),
        ] {
            let key = Value::Symbol(self.well_known(name)?);
            let method = self.new_host_behavior(
                HostBehavior::Intrinsic(kind),
                &alloc::format!("[Symbol.{name}]"),
                if matches!(kind, Builtin::RegExpSplit | Builtin::RegExpReplace) {
                    2
                } else {
                    1
                },
            )?;
            self.define_key(
                &value,
                key,
                Property {
                    enumerable: false,
                    ..Property::data(method)
                },
            )?;
        }
        for (name, field) in [("source", Field::Source), ("flags", Field::Flags)]
            .into_iter()
            .chain(FLAGS.iter().map(|(name, flag)| (*name, Field::Flag(*flag))))
        {
            let getter = self.new_host_behavior(
                HostBehavior::RegExpGet(field),
                &alloc::format!("get {name}"),
                0,
            )?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: Some((getter, Value::Undefined)),
                },
            )?;
        }
        Ok(value)
    }
    pub(in crate::vm) fn regexp_get(
        &mut self,
        receiver: &Value,
        field: Field,
    ) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        let result = (|| {
            require_object(receiver)?;
            if matches!(field, Field::Flags) {
                let mut flags = String::new();
                for (name, flag) in FLAGS {
                    if self
                        .get(receiver, &Value::string(name).units())?
                        .to_boolean()
                    {
                        flags.push(*flag);
                    }
                }
                if flags.len() > self.limits.string_units {
                    return Err(Error::Limit {
                        resource: "string units",
                    });
                }
                return Ok(Value::string(&flags));
            }
            if let Some(data) = self.regexp_data(receiver) {
                return Ok(match field {
                    Field::Source => {
                        self.charge(u64::try_from(data.source.len()).unwrap_or(u64::MAX))?;
                        Value::String(super::source_text(&data.source, self.limits.string_units)?)
                    }
                    Field::Flag(flag) => Value::Boolean(data.flags.contains(flag)),
                    Field::Flags => return Err(Error::InvalidBytecode),
                });
            }
            if self.regexp_proto.as_ref() == Some(receiver) {
                return Ok(if matches!(field, Field::Source) {
                    Value::String(super::source_text(&[], self.limits.string_units)?)
                } else {
                    Value::Undefined
                });
            }
            Err(Error::Type {
                message: "RegExp getter requires an initialized instance",
            })
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(in crate::vm) fn regexp_constructor_call(
        &mut self,
        args: &[Value],
        target: Option<&Value>,
    ) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        if let Some(target) = target {
            self.native_roots.push(target.clone());
        }
        let result = (|| {
            let input = args.first().unwrap_or(&Value::Undefined);
            let mut flags = args.get(1).cloned().unwrap_or(Value::Undefined);
            let is_regex = self.is_regexp_value(input)?;
            let intrinsic = Value::Function(FunctionValue::native(Builtin::RegExp));
            if target.is_none()
                && is_regex
                && matches!(flags, Value::Undefined)
                && self.get(input, &Value::string("constructor").units())? == intrinsic
            {
                return Ok(input.clone());
            }
            let target = target.unwrap_or(&intrinsic);
            let source = if let Some(old) = self.regexp_data(input) {
                if matches!(flags, Value::Undefined) {
                    flags = Value::string(&old.flags);
                }
                Value::String(old.source.clone())
            } else if is_regex {
                let source = self.get(input, &Value::string("source").units())?;
                self.native_roots.push(source.clone());
                if matches!(flags, Value::Undefined) {
                    flags = self.get(input, &Value::string("flags").units())?;
                }
                source
            } else {
                input.clone()
            };
            self.native_roots.extend([source.clone(), flags.clone()]);
            let default = self.regexp_prototype()?;
            let proto = self.constructor_prototype(target, default)?;
            self.native_roots.push(proto.clone());
            let object = self.allocate_regexp(proto)?;
            self.native_roots.push(object.clone());
            self.initialize_regexp(&object, &source, &flags)?;
            Ok(object)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn initialize_regexp(
        &mut self,
        object: &Value,
        source: &Value,
        flags: &Value,
    ) -> Result<(), Error> {
        let source = if matches!(source, Value::Undefined) {
            Rc::from([])
        } else {
            self.string_units(source)?
        };
        let flags = if matches!(flags, Value::Undefined) {
            String::new()
        } else {
            String::from_utf16(&self.string_units(flags)?).map_err(|_| Error::Syntax {
                offset: 0,
                message: "invalid RegExp flags",
            })?
        };
        self.charge(u64::try_from(source.len().saturating_add(flags.len())).unwrap_or(u64::MAX))?;
        self.object_mut(object)?.regexp = Some(Rc::new(RegExp::compile(source, &flags)?));
        Ok(())
    }
}
pub(super) const fn require_object(value: &Value) -> Result<(), Error> {
    if matches!(value, Value::Object(_) | Value::Function(_)) {
        Ok(())
    } else {
        Err(Error::Type {
            message: "RegExp method requires an object",
        })
    }
}
