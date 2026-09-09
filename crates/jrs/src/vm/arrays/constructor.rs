// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Mutable Array constructor storage and generic Array.of. The immediate
//! callable identity delegates properties to a realm-owned traced object.
use super::{Builtin, Error, Execution, Property, Value, length_key};
use crate::heap::HostBehavior;

impl Execution<'_> {
    pub(in crate::vm) fn array_constructor(&mut self) -> Result<Value, Error> {
        if let Some(storage) = &self.array_constructor_storage {
            return Ok(storage.clone());
        }
        let proto = self.function_prototype()?;
        let storage = self.allocate_object(proto)?;
        self.array_constructor_storage = Some(storage.clone());
        for (name, value, configurable) in [
            ("length", Value::Number(1.0), true),
            ("name", Value::string("Array"), true),
            ("prototype", self.array_prototype()?, false),
        ] {
            self.define(
                &storage,
                Value::string(name).units(),
                Property {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable,
                    accessor: None,
                },
            )?;
        }
        for (name, kind, length) in [
            ("isArray", Builtin::ArrayIsArray, 1),
            ("of", Builtin::ArrayOf, 0),
            ("from", Builtin::ArrayFrom, 1),
        ] {
            let function = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, length)?;
            self.define(
                &storage,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
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
            &storage,
            key,
            Property {
                value: Value::Undefined,
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: Some((getter, Value::Undefined)),
            },
        )?;
        Ok(storage)
    }
    pub(super) fn array_of(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let length = u32::try_from(args.len()).map_err(|_| Error::Limit {
            resource: "Array.of arguments",
        })?;
        let length_number = Value::Number(f64::from(length));
        let array = if self.is_constructor(receiver) {
            self.construct_sync(
                receiver.clone(),
                core::slice::from_ref(&length_number),
                receiver.clone(),
            )?
        } else {
            self.new_array(length)?
        };
        self.native_roots.push(array.clone());
        for (index, value) in args.iter().enumerate() {
            self.charge(1)?;
            self.define(
                &array,
                Value::string(&alloc::format!("{index}")).units(),
                Property::data(value.clone()),
            )?;
        }
        self.set(&array, length_key(), length_number, true)?;
        Ok(array)
    }
}
