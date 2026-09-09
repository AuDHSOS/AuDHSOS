// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Object operations never collect while a newly allocated value is unrooted.
//! Prototype walks are iterative and bounded by the heap quota.

use super::Execution;
use crate::{
    Error, ObjectValue, Value,
    bytecode::{Builtin, Op},
    heap::Node,
    object::{Object, Property, same_value},
    value::{Callable, FunctionValue},
};
use alloc::{rc::Rc, string::String};

impl Execution<'_> {
    pub(super) fn constructor_storage(&mut self, value: &Value) -> Result<Value, Error> {
        if matches!(
            value,
            Value::Function(FunctionValue(Callable::Native(Builtin::Array)))
        ) {
            return self.array_constructor();
        }
        if matches!(
            value,
            Value::Function(FunctionValue(Callable::Native(Builtin::RegExp)))
        ) {
            return self.regexp_constructor();
        }
        if matches!(
            value,
            Value::Function(FunctionValue(Callable::Native(Builtin::Number)))
        ) {
            self.number_constructor()
        } else {
            self.object_constructor()
        }
    }
    pub(super) fn object_constructor(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.object_constructor_storage {
            return Ok(value.clone());
        }
        let proto = self.function_prototype()?;
        let storage = self.allocate_object(proto)?;
        self.object_constructor_storage = Some(storage.clone());
        for (name, value, configurable) in [
            ("length", Value::Number(1.0), true),
            ("name", Value::string("Object"), true),
            ("prototype", self.prototype()?, false),
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
        for name in [
            "create",
            "getPrototypeOf",
            "setPrototypeOf",
            "defineProperty",
            "defineProperties",
            "getOwnPropertyDescriptor",
            "hasOwn",
            "preventExtensions",
            "isExtensible",
            "freeze",
            "seal",
            "is",
            "isFrozen",
            "isSealed",
            "keys",
            "getOwnPropertyNames",
            "getOwnPropertySymbols",
            "entries",
            "values",
        ] {
            let value = match name {
                "entries" => self.object_enumerator(true)?,
                "values" => self.object_enumerator(false)?,
                _ => native_property(Builtin::Object, &Value::string(name).units())
                    .ok_or(Error::InvalidBytecode)?,
            };
            self.define(
                &storage,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(value)
                },
            )?;
        }
        Ok(storage)
    }
    pub(super) fn math_object(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.math {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.math = Some(value.clone());
        self.install_tag(&value, "Math")?;
        let pow = self.new_host_behavior(
            crate::heap::HostBehavior::Intrinsic(Builtin::MathPow),
            "pow",
            2,
        )?;
        self.define(
            &value,
            Value::string("pow").units(),
            Property {
                enumerable: false,
                ..Property::data(pow)
            },
        )?;
        for (name, builtin) in [("max", Builtin::MathMax), ("min", Builtin::MathMin)] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(builtin))
                },
            )?;
        }
        for (name, number) in [
            ("PI", core::f64::consts::PI),
            ("E", core::f64::consts::E),
            ("LN10", core::f64::consts::LN_10),
            ("LN2", core::f64::consts::LN_2),
            ("LOG10E", core::f64::consts::LOG10_E),
            ("LOG2E", core::f64::consts::LOG2_E),
            ("SQRT1_2", core::f64::consts::FRAC_1_SQRT_2),
            ("SQRT2", core::f64::consts::SQRT_2),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: Value::Number(number),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn error_object(&mut self, error: &Error) -> Result<Value, Error> {
        let kind = match error {
            Error::Type { .. } => Builtin::TypeError,
            Error::Reference { .. } => Builtin::ReferenceError,
            Error::Range { .. } => Builtin::RangeError,
            Error::Syntax { .. } => Builtin::SyntaxError,
            _ => Builtin::Error,
        };
        self.new_error(kind, &[Value::string(&alloc::format!("{error}"))])
    }
    pub(super) fn object_ref(&self, value: &Value) -> Result<&Object, Error> {
        if has_constructor_storage(value) {
            let storage = if matches!(
                value,
                Value::Function(FunctionValue(Callable::Native(Builtin::Array)))
            ) {
                &self.array_constructor_storage
            } else if matches!(
                value,
                Value::Function(FunctionValue(Callable::Native(Builtin::RegExp)))
            ) {
                &self.regexp_constructor_storage
            } else if matches!(
                value,
                Value::Function(FunctionValue(Callable::Native(Builtin::Number)))
            ) {
                &self.number_constructor_storage
            } else {
                &self.object_constructor_storage
            };
            return self.object_ref(storage.as_ref().ok_or(Error::InvalidBytecode)?);
        }
        let handle = value.heap_handle().ok_or(Error::Type {
            message: "expected an object",
        })?;
        match self.heap.get(handle)? {
            Node::Symbol => Err(Error::Type {
                message: "Symbol is not an object",
            }),
            Node::Object(object)
            | Node::HostFunction { object, .. }
            | Node::Function { object, .. }
            | Node::Resolving { object, .. }
            | Node::Bound { object, .. } => Ok(object),
            Node::Cell { .. } | Node::Suspended(_) => Err(Error::InvalidBytecode),
        }
    }
    pub(super) fn object_mut(&mut self, value: &Value) -> Result<&mut Object, Error> {
        if has_constructor_storage(value) {
            let storage = self.constructor_storage(value)?;
            return self.object_mut(&storage);
        }
        let handle = value.heap_handle().ok_or(Error::Type {
            message: "expected an object",
        })?;
        match self.heap.get_mut(handle)? {
            Node::Symbol => Err(Error::Type {
                message: "Symbol is not an object",
            }),
            Node::Object(object)
            | Node::HostFunction { object, .. }
            | Node::Function { object, .. }
            | Node::Resolving { object, .. }
            | Node::Bound { object, .. } => Ok(object),
            Node::Cell { .. } | Node::Suspended(_) => Err(Error::InvalidBytecode),
        }
    }
    pub(super) fn allocate_object(&mut self, prototype: Value) -> Result<Value, Error> {
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Object(Object::new(prototype)),
            self.limits.heap_entries,
        )?;
        Ok(Value::Object(ObjectValue {
            handle,
            owner: self.owner.clone(),
        }))
    }
    pub(super) fn prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.object_prototype {
            return Ok(value.clone());
        }
        let value = self.allocate_object(Value::Null)?;
        self.object_prototype = Some(value.clone());
        for (name, builtin) in [
            ("hasOwnProperty", Builtin::HasOwnProperty),
            ("constructor", Builtin::Object),
            ("toString", Builtin::ObjectToString),
            ("valueOf", Builtin::ObjectValueOf),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(builtin))
                },
            )?;
        }
        for (name, kind) in [
            ("propertyIsEnumerable", Builtin::PropertyIsEnumerable),
            ("isPrototypeOf", Builtin::IsPrototypeOf),
        ] {
            let f = self.new_host_behavior(crate::heap::HostBehavior::Intrinsic(kind), name, 1)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(f)
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn global_object(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.global {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.global = Some(value.clone());
        self.define(
            &value,
            Value::string("globalThis").units(),
            Property::data(value.clone()),
        )?;
        let json = self.json_object()?;
        for (name, item) in [
            ("JSON", json),
            ("self", value.clone()),
            ("Promise", native(Builtin::Promise)),
            ("WeakMap", native(Builtin::WeakMap)),
            ("Symbol", native(Builtin::Symbol)),
            ("Array", native(Builtin::Array)),
            ("Object", native(Builtin::Object)),
            ("String", native(Builtin::String)),
            ("Number", native(Builtin::Number)),
            ("Boolean", native(Builtin::Boolean)),
            ("RegExp", native(Builtin::RegExp)),
        ] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(item)
                },
            )?;
        }
        for (name, integer) in [("parseInt", true), ("parseFloat", false)] {
            let function = self.number_parser(integer)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn define(
        &mut self,
        object: &Value,
        key: Rc<[u16]>,
        mut property: Property,
    ) -> Result<(), Error> {
        if self.object_ref(object)?.array
            && key == crate::object::length_key()
            && property.accessor.is_none()
        {
            property.value = self.array_length_value(&property.value)?;
        }
        let limit = self.limits.properties;
        let mapping = self
            .object_ref(object)?
            .argument_map
            .get(key.as_ref())
            .copied();
        let detach = property.accessor.is_some() || !property.writable;
        let mapped_value = if property.accessor.is_none() {
            Some(property.value.clone())
        } else {
            None
        };
        let removed_key = key.clone();
        if !self.object_mut(object)?.define(key, property, limit)? {
            return Err(Error::Type {
                message: "property definition rejected",
            });
        }
        if let Some(handle) = mapping {
            if let Some(new) = mapped_value {
                let Node::Cell { value, .. } = self.heap.get_mut(handle)? else {
                    return Err(Error::InvalidBytecode);
                };
                *value = Some(new);
            }
            if detach {
                self.object_mut(object)?
                    .argument_map
                    .remove(removed_key.as_ref());
            }
        }
        Ok(())
    }
    pub(super) fn own(&mut self, object: &Value, key: &[u16]) -> Result<Option<Property>, Error> {
        if has_constructor_storage(object) {
            let storage = self.constructor_storage(object)?;
            return self.own(&storage, key);
        }
        if matches!(
            object,
            Value::Number(_) | Value::Boolean(_) | Value::Symbol(_)
        ) {
            return Ok(None);
        }
        if let Value::String(units) = object {
            if key == Value::string("length").units().as_ref() {
                return Ok(Some(Property {
                    value: length(units.len()),
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                }));
            }
            return Ok(crate::object::string_index(units, key));
        }
        if let Value::Function(FunctionValue(Callable::Native(builtin))) = object {
            if *builtin == Builtin::Function {
                return self.function_constructor_property(key);
            }
            if let Some(property) = Self::json_property(*builtin, key) {
                return Ok(Some(property));
            }
            if *builtin == Builtin::DomException {
                return self.dom_exception_property(key);
            }
            if let Some(property) = self.event_property(*builtin, key)? {
                return Ok(Some(property));
            }
            if let Some(property) = self.abort_property(*builtin, key)? {
                return Ok(Some(property));
            }
            if *builtin == Builtin::ArrayConcat
                && let Some(value) = native_property(*builtin, key)
            {
                return Ok(Some(Property {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                }));
            }
            if let Some(property) = self.symbol_property(*builtin, key)? {
                return Ok(Some(property));
            }
            if let Some(property) = self.weakmap_property(*builtin, key)? {
                return Ok(Some(property));
            }
            if matches!(
                builtin,
                Builtin::Number | Builtin::Boolean | Builtin::String
            ) {
                return self.primitive_constructor_property(*builtin, key);
            }
            return Ok(native_property(*builtin, key).map(Property::data));
        }
        let object = self.object_ref(object)?;
        let mut property = object
            .properties
            .get(key)
            .cloned()
            .or_else(|| object.string_property(key));
        if let Some(handle) = object.argument_map.get(key)
            && let Some(property) = &mut property
        {
            let Node::Cell { value, .. } = self.heap.get(*handle)? else {
                return Err(Error::InvalidBytecode);
            };
            property.value = value.clone().unwrap_or(Value::Undefined);
        }
        Ok(property)
    }
    pub(super) fn get(&mut self, object: &Value, key: &[u16]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(object.clone());
        let result = self.get_rooted(object, key);
        self.native_roots.truncate(roots);
        result
    }
    fn primitive_constructor_property(
        &mut self,
        builtin: Builtin,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        let name = match builtin {
            Builtin::Number => "Number",
            Builtin::Boolean => "Boolean",
            _ => "String",
        };
        let (value, configurable) = if key == Value::string("prototype").units().as_ref() {
            (self.primitive_prototype(builtin)?, false)
        } else if key == Value::string("name").units().as_ref() {
            (Value::string(name), true)
        } else if key == Value::string("length").units().as_ref() {
            (Value::Number(1.0), true)
        } else {
            return Ok(native_property(builtin, key).map(Property::data));
        };
        Ok(Some(Property {
            value,
            writable: false,
            enumerable: false,
            configurable,
            accessor: None,
        }))
    }

    fn get_rooted(&mut self, object: &Value, key: &[u16]) -> Result<Value, Error> {
        if has_constructor_storage(object) {
            self.constructor_storage(object)?;
            return self.get_inherited(object, key);
        }
        if let Value::Function(FunctionValue(Callable::Native(
            kind @ (Builtin::String | Builtin::Number | Builtin::Boolean),
        ))) = object
            && key == Value::string("prototype").units().as_ref()
        {
            return self.primitive_prototype(*kind);
        }
        if matches!(
            object,
            Value::Function(FunctionValue(Callable::Native(Builtin::Function)))
        ) && key == Value::string("prototype").units().as_ref()
        {
            return self.function_prototype();
        }
        if let Value::Function(FunctionValue(Callable::Native(kind))) = object
            && super::errors::error_name(*kind).is_some()
            && key == Value::string("prototype").units().as_ref()
        {
            return self.error_prototype(*kind);
        }
        if matches!(
            object,
            Value::Function(FunctionValue(Callable::Native(Builtin::Promise)))
        ) && key == Value::string("prototype").units().as_ref()
        {
            return self.promise_prototype();
        }
        if key == Value::string("prototype").units().as_ref()
            && matches!(
                object,
                Value::Function(FunctionValue(Callable::Script { .. }))
            )
        {
            self.ensure_function_prototype(object)?;
        }
        if matches!(object, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "property access on null or undefined",
            });
        }
        if matches!(object, Value::Function(FunctionValue(Callable::Native(_)))) {
            if key == Value::string("call").units().as_ref() {
                return Ok(native(Builtin::FunctionCall));
            }
            if key == Value::string("apply").units().as_ref() {
                return Ok(native(Builtin::FunctionApply));
            }
        }
        if matches!(object, Value::Function(FunctionValue(Callable::Native(_))))
            && self.own(object, key)?.is_none()
            && key == Value::string("valueOf").units().as_ref()
        {
            return Ok(native(Builtin::ObjectValueOf));
        }
        if matches!(
            object,
            Value::Function(FunctionValue(Callable::Native(Builtin::Object)))
        ) && key == Value::string("prototype").units().as_ref()
        {
            return self.prototype();
        }
        self.get_inherited(object, key)
    }

    fn get_inherited(&mut self, object: &Value, key: &[u16]) -> Result<Value, Error> {
        let mut current = object.clone();
        for _ in 0..=self.limits.heap_entries {
            if let Some(property) = self.own(&current, key)? {
                if let Some((getter, _)) = property.accessor {
                    if matches!(getter, Value::Undefined) {
                        return Ok(Value::Undefined);
                    }
                    return self.call_sync(getter, object.clone(), &[]);
                }
                return Ok(property.value);
            }
            let prototype = if let Some(kind) = super::boxing::kind(&current) {
                self.primitive_prototype(kind)?
            } else if matches!(current, Value::Function(FunctionValue(Callable::Native(_)))) {
                self.prototype_of(&current)?
            } else {
                self.object_ref(&current)?.prototype.clone()
            };
            if matches!(prototype, Value::Null) {
                return Ok(Value::Undefined);
            }
            current = prototype;
        }
        Err(Error::Limit {
            resource: "prototype walk",
        })
    }
    pub(super) fn has_property(&mut self, object: &Value, key: &[u16]) -> Result<bool, Error> {
        if !matches!(object, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "in requires an object",
            });
        }
        let mut current = object.clone();
        for _ in 0..=self.limits.heap_entries {
            if self.own(&current, key)?.is_some() {
                return Ok(true);
            }
            current = self.prototype_of(&current)?;
            if matches!(current, Value::Null) {
                return Ok(false);
            }
        }
        Err(Error::Limit {
            resource: "prototype walk",
        })
    }
    pub(super) fn set(
        &mut self,
        object: &Value,
        key: Rc<[u16]>,
        value: Value,
        strict: bool,
    ) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend([object.clone(), value.clone()]);
        let result = self.set_rooted(object, key, value, strict);
        self.native_roots.truncate(roots);
        result
    }

    fn set_rooted(
        &mut self,
        object: &Value,
        key: Rc<[u16]>,
        mut value: Value,
        strict: bool,
    ) -> Result<(), Error> {
        if matches!(object, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "property assignment on null or undefined",
            });
        }
        if self.object_ref(object).is_ok_and(|o| o.array) && key == crate::object::length_key() {
            value = self.array_length_value(&value)?;
        }
        let mut current = object.clone();
        for _ in 0..=self.limits.heap_entries {
            if let Some(property) = self.own(&current, &key)? {
                if let Some((_, setter)) = property.accessor {
                    if matches!(setter, Value::Undefined) {
                        return rejection(strict);
                    }
                    self.call_sync(setter, object.clone(), &[value])?;
                    return Ok(());
                }
                if !property.writable {
                    return rejection(strict);
                }
                break;
            }
            current = self.prototype_of(&current)?;
            if matches!(current, Value::Null) {
                break;
            }
        }
        if !matches!(object, Value::Object(_) | Value::Function(_)) {
            return rejection(strict);
        }
        let limit = self.limits.properties;
        let own = self.own(object, &key)?;
        let property = match own {
            Some(p) => Property { value, ..p },
            None => Property::data(value),
        };
        let mapping = self
            .object_ref(object)?
            .argument_map
            .get(key.as_ref())
            .copied();
        let mapped_value = property.value.clone();
        if !self.object_mut(object)?.define(key, property, limit)? {
            return rejection(strict);
        }
        if let Some(handle) = mapping {
            let Node::Cell { value, .. } = self.heap.get_mut(handle)? else {
                return Err(Error::InvalidBytecode);
            };
            *value = Some(mapped_value);
        }
        Ok(())
    }
    fn set_prototype(&mut self, object: &Value, prototype: &Value) -> Result<(), Error> {
        if matches!(object, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "prototype target is nullish",
            });
        }
        if !matches!(
            prototype,
            Value::Object(_) | Value::Function(_) | Value::Null
        ) {
            return Err(Error::Type {
                message: "prototype must be an object or null",
            });
        }
        if super::boxing::kind(object).is_some() {
            return Ok(());
        }
        let own = self.object_ref(object)?;
        if own.prototype.strictly_equals(prototype) {
            return Ok(());
        }
        if !own.extensible {
            return Err(Error::Type {
                message: "object is not extensible",
            });
        }
        let mut current = prototype.clone();
        for _ in 0..=self.limits.heap_entries {
            if matches!(current, Value::Null) {
                self.object_mut(object)?.prototype = prototype.clone();
                return Ok(());
            }
            if current.strictly_equals(object) {
                return Err(Error::Type {
                    message: "cyclic prototype",
                });
            }
            if matches!(current, Value::Function(FunctionValue(Callable::Native(_)))) {
                current = Value::Null;
            } else {
                current = self.object_ref(&current)?.prototype.clone();
            }
        }
        Err(Error::Limit {
            resource: "prototype walk",
        })
    }

    pub(super) fn property_op(&mut self, op: &Op) -> Result<(), Error> {
        match op {
            Op::Array(length) => {
                let value = self.new_array(*length)?;
                self.push(value)?;
            }
            Op::Object => {
                let prototype = self.prototype()?;
                let value = self.allocate_object(prototype)?;
                self.push(value)?;
            }
            Op::This => {
                let value = self.current_this()?;
                self.push(value)?;
            }
            Op::Global => {
                let value = self.global_object()?;
                self.push(value)?;
            }
            Op::Key => {
                let value = self.pop()?;
                let key = self.property_key(&value)?;
                self.push(key)?;
            }
            Op::DupPair => {
                let at = self
                    .stack
                    .len()
                    .checked_sub(2)
                    .ok_or(Error::InvalidBytecode)?;
                let a = self.stack.get(at).ok_or(Error::InvalidBytecode)?.clone();
                let b = self.stack.last().ok_or(Error::InvalidBytecode)?.clone();
                self.push(a)?;
                self.push(b)?;
            }
            Op::Get(method) => {
                let key = self.pop()?;
                let object = self.pop()?;
                // Keep the base rooted across lazy intrinsic allocation.
                self.push(object.clone())?;
                let value = self.get_key(&object, &key)?;
                self.pop()?;
                self.push(value)?;
                if *method {
                    self.push(object)?;
                }
            }
            Op::Define(prototype) => {
                let value = self.pop()?;
                let key = self.pop()?;
                let object = self.stack.last().ok_or(Error::InvalidBytecode)?.clone();
                if *prototype {
                    if matches!(value, Value::Object(_) | Value::Function(_) | Value::Null) {
                        self.set_prototype(&object, &value)?;
                    }
                } else {
                    self.define_key(&object, key, Property::data(value))?;
                }
            }
            Op::Accessor(setter) => {
                self.accessor_literal(*setter)?;
            }
            Op::Set(strict) => {
                let value = self.pop()?;
                let key = self.pop()?;
                let object = self.pop()?;
                self.set_key(&object, key, value.clone(), *strict)?;
                self.push(value)?;
            }
            Op::Delete(strict) => {
                let key = self.pop()?;
                let object = self.pop()?;
                let result = self.delete_key(&object, &key)?;
                if !result {
                    rejection(*strict)?;
                }
                self.push(Value::Boolean(result))?;
            }
            Op::UpdateProperty(add, prefix, strict) => {
                let key = self.pop()?;
                let object = self.pop()?;
                self.push(object.clone())?;
                self.push(key.clone())?;
                let old_value = self.get_key(&object, &key)?;
                let old = self.numeric(&old_value)?;
                self.pop()?;
                self.pop()?;
                let value = if *add { old + 1.0 } else { old - 1.0 };
                self.set_key(&object, key, Value::Number(value), *strict)?;
                self.push(Value::Number(if *prefix { value } else { old }))?;
            }
            _ => return Err(Error::InvalidBytecode),
        }
        Ok(())
    }

    pub(super) fn object_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let first = args.first().unwrap_or(&Value::Undefined);
        for value in [first, receiver] {
            if has_constructor_storage(value) {
                self.constructor_storage(value)?;
            }
        }
        let second = args.get(1).unwrap_or(&Value::Undefined);
        let value = match builtin {
            Builtin::Object => {
                if matches!(first, Value::Object(_) | Value::Function(_)) {
                    first.clone()
                } else if matches!(first, Value::Null | Value::Undefined) {
                    let proto = self.prototype()?;
                    self.allocate_object(proto)?
                } else {
                    self.box_value(first)?
                }
            }
            Builtin::ObjectCreate => {
                if !matches!(first, Value::Null | Value::Object(_) | Value::Function(_)) {
                    return Err(Error::Type {
                        message: "invalid Object.create prototype",
                    });
                }
                let object = self.allocate_object(first.clone())?;
                if !matches!(second, Value::Undefined) {
                    self.define_properties(&object, second)?;
                }
                object
            }
            Builtin::DefineProperties => {
                self.define_properties(first, second)?;
                first.clone()
            }
            Builtin::GetPrototypeOf => self.prototype_of(first)?,
            Builtin::SetPrototypeOf => {
                self.set_prototype(first, second)?;
                first.clone()
            }
            Builtin::PreventExtensions | Builtin::Freeze | Builtin::Seal => {
                self.integrity(first, builtin)?;
                first.clone()
            }
            Builtin::IsExtensible => Value::Boolean(
                matches!(first, Value::Object(_) | Value::Function(_))
                    && (first.heap_handle().is_some() || has_constructor_storage(first))
                    && self.object_ref(first)?.extensible,
            ),
            Builtin::ObjectIs => Value::Boolean(same_value(first, second)),
            Builtin::ObjectValueOf => self.box_value(receiver)?,
            Builtin::FunctionToString => self.function_text(receiver)?,
            Builtin::IsFrozen | Builtin::IsSealed => Value::Boolean(
                if matches!(first, Value::Object(_) | Value::Function(_))
                    && (first.heap_handle().is_some() || has_constructor_storage(first))
                {
                    let object = self.object_ref(first)?;
                    !object.extensible
                        && object
                            .properties
                            .values()
                            .chain(object.symbols.values().map(|(_, p)| p))
                            .all(|p| {
                                !p.configurable
                                    && (builtin != Builtin::IsFrozen
                                        || p.accessor.is_some()
                                        || !p.writable)
                            })
                } else {
                    !matches!(first, Value::Function(_))
                },
            ),
            Builtin::HasOwn | Builtin::HasOwnProperty => {
                let (object, key) = if builtin == Builtin::HasOwn {
                    (first, second)
                } else {
                    (receiver, first)
                };
                if builtin == Builtin::HasOwn && matches!(object, Value::Null | Value::Undefined) {
                    return Err(Error::Type {
                        message: "hasOwn target is nullish",
                    });
                }
                let key = self.property_key(key)?;
                Value::Boolean(self.own_key(object, &key)?.is_some())
            }
            Builtin::PropertyIsEnumerable | Builtin::IsPrototypeOf => {
                return self.object_predicate(builtin, receiver, first).map(Some);
            }
            Builtin::DefineProperty => {
                self.object_ref(first)?;
                let desc = args.get(2).unwrap_or(&Value::Undefined);
                let key = self.property_key(second)?;
                self.define_descriptor(first, key, desc)?;
                first.clone()
            }
            Builtin::GetOwnPropertyDescriptor => self.boxed_descriptor(first, second)?,
            Builtin::ObjectToString => self.tagged_object_string(receiver)?,
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    fn object_predicate(
        &mut self,
        kind: Builtin,
        receiver: &Value,
        value: &Value,
    ) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend([receiver.clone(), value.clone()]);
        let result = (|| {
            if kind == Builtin::PropertyIsEnumerable {
                let key = self.property_key(value)?;
                self.native_roots.push(key.clone());
                let object = self.box_value(receiver)?;
                self.native_roots.push(object.clone());
                return Ok(Value::Boolean(
                    self.own_key(&object, &key)?.is_some_and(|p| p.enumerable),
                ));
            }
            if !matches!(value, Value::Object(_) | Value::Function(_)) {
                return Ok(Value::Boolean(false));
            }
            let object = self.box_value(receiver)?;
            self.native_roots.push(object.clone());
            let mut current = value.clone();
            for _ in 0..=self.limits.heap_entries {
                self.charge(1)?;
                current = self.prototype_of(&current)?;
                if current == Value::Null {
                    return Ok(Value::Boolean(false));
                }
                if same_value(&object, &current) {
                    return Ok(Value::Boolean(true));
                }
            }
            Err(Error::Limit {
                resource: "prototype traversal",
            })
        })();
        self.native_roots.truncate(roots);
        result
    }

    fn boxed_descriptor(&mut self, target: &Value, key: &Value) -> Result<Value, Error> {
        let target = self.box_value(target)?;
        let roots = self.native_roots.len();
        self.native_roots.push(target.clone());
        let result = (|| {
            let key = self.property_key(key)?;
            self.native_roots.push(key.clone());
            self.descriptor(&target, &key)
        })();
        self.native_roots.truncate(roots);
        result
    }

    fn delete_key(&mut self, object: &Value, key: &Value) -> Result<bool, Error> {
        if matches!(object, Value::Null | Value::Undefined) {
            return Err(Error::Type {
                message: "delete on null or undefined",
            });
        }
        if matches!(object, Value::Object(_) | Value::Function(_))
            && (object.heap_handle().is_some() || has_constructor_storage(object))
        {
            Ok(if let Value::Symbol(symbol) = key {
                self.object_mut(object)?.delete_symbol(symbol)
            } else {
                self.object_mut(object)?.delete(&key.units())
            })
        } else {
            Ok(self.own_key(object, key)?.is_none_or(|p| p.configurable))
        }
    }
    fn integrity(&mut self, object: &Value, builtin: Builtin) -> Result<(), Error> {
        if !matches!(object, Value::Object(_) | Value::Function(_))
            || (object.heap_handle().is_none() && !has_constructor_storage(object))
        {
            return Ok(());
        }
        self.object_mut(object)?.extensible = false;
        if builtin == Builtin::PreventExtensions {
            return Ok(());
        }
        let mut keys: alloc::vec::Vec<_> = self
            .object_ref(object)?
            .keys()
            .into_iter()
            .map(Value::String)
            .collect();
        keys.extend(self.symbol_keys(object)?);
        for key in keys {
            let mut property = self.own_key(object, &key)?.ok_or(Error::InvalidBytecode)?;
            property.configurable = false;
            if builtin == Builtin::Freeze && property.accessor.is_none() {
                property.writable = false;
            }
            self.define_key(object, key, property)?;
        }
        Ok(())
    }

    pub(super) fn prototype_of(&mut self, value: &Value) -> Result<Value, Error> {
        if has_constructor_storage(value) {
            let storage = self.constructor_storage(value)?;
            return Ok(self.object_ref(&storage)?.prototype.clone());
        }
        if let Some(kind) = super::boxing::kind(value) {
            return self.primitive_prototype(kind);
        }
        if let Value::Function(FunctionValue(Callable::Native(kind))) = value {
            if *kind == Builtin::AbortSignal {
                return Ok(native(Builtin::EventTarget));
            }
            if super::errors::error_name(*kind).is_some() && *kind != Builtin::Error {
                return Ok(native(Builtin::Error));
            }
            return self.function_prototype();
        }
        Ok(self.object_ref(value)?.prototype.clone())
    }

    pub(super) fn object_tag(&self, value: &Value) -> Value {
        if let Some(primitive) = self
            .object_ref(value)
            .ok()
            .and_then(|o| o.primitive.as_ref())
        {
            return Value::string(match primitive {
                Value::Boolean(_) => "[object Boolean]",
                Value::Number(_) => "[object Number]",
                Value::Symbol(_) => "[object Object]",
                _ => "[object String]",
            });
        }
        Value::string(if self.object_ref(value).is_ok_and(|o| o.error) {
            "[object Error]"
        } else if self.object_ref(value).is_ok_and(|o| o.arguments) {
            "[object Arguments]"
        } else if self.is_array(value) {
            "[object Array]"
        } else if self.regexp_data(value).is_some() {
            "[object RegExp]"
        } else {
            match value {
                Value::Undefined => "[object Undefined]",
                Value::Null => "[object Null]",
                Value::Function(_) => "[object Function]",
                Value::String(_) => "[object String]",
                Value::Number(_) => "[object Number]",
                Value::Boolean(_) => "[object Boolean]",
                Value::Object(_) | Value::Symbol(_) => "[object Object]",
            }
        })
    }

    fn accessor_literal(&mut self, setter: bool) -> Result<(), Error> {
        let function = self.pop()?;
        let key = self.pop()?;
        let object = self.stack.last().ok_or(Error::InvalidBytecode)?.clone();
        let mut pair = self
            .own_key(&object, &key)?
            .and_then(|p| p.accessor)
            .unwrap_or((Value::Undefined, Value::Undefined));
        if setter {
            pair.1 = function;
        } else {
            pair.0 = function;
        }
        self.define_key(
            &object,
            key,
            Property {
                value: Value::Undefined,
                writable: false,
                enumerable: true,
                configurable: true,
                accessor: Some(pair),
            },
        )
    }

    fn current_this(&mut self) -> Result<Value, Error> {
        self.this_binding()
    }

    fn descriptor(&mut self, target: &Value, key: &Value) -> Result<Value, Error> {
        if *key == Value::string("prototype")
            && matches!(
                target,
                Value::Function(FunctionValue(Callable::Script { .. }))
            )
        {
            self.ensure_function_prototype(target)?;
        }
        let Some(property) = self.own_key(target, key)? else {
            return Ok(Value::Undefined);
        };
        let proto = self.prototype()?;
        let object = self.allocate_object(proto)?;
        let (first, second) = if let Some((get, set)) = property.accessor {
            (("get", get), ("set", set))
        } else {
            (
                ("value", property.value),
                ("writable", Value::Boolean(property.writable)),
            )
        };
        for (name, value) in [
            first,
            second,
            ("enumerable", Value::Boolean(property.enumerable)),
            ("configurable", Value::Boolean(property.configurable)),
        ] {
            self.define(&object, Value::string(name).units(), Property::data(value))?;
        }
        Ok(object)
    }

    pub(super) fn ensure_function_prototype(&mut self, function: &Value) -> Result<(), Error> {
        let handle = function.heap_handle().ok_or(Error::InvalidBytecode)?;
        if self.object_ref(function)?.function_initialized {
            return Ok(());
        }
        let Node::Function { code, object, .. } = self.heap.get(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        if !object
            .properties
            .contains_key(Value::string("length").units().as_ref())
        {
            let length = code.length;
            self.define(
                function,
                Value::string("length").units(),
                Property {
                    value: Value::Number(f64::from(u32::try_from(length).unwrap_or(u32::MAX))),
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
        }
        let Node::Function { code, object, .. } = self.heap.get(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        if !code.constructible
            || code.constructor_kind != crate::parser::ConstructorKind::Ordinary
            || object
                .properties
                .contains_key(Value::string("prototype").units().as_ref())
        {
            self.object_mut(function)?.function_initialized = true;
            return Ok(());
        }
        let proto = self.prototype()?;
        let object = self.allocate_object(proto)?;
        self.define(
            &object,
            Value::string("constructor").units(),
            Property {
                enumerable: false,
                ..Property::data(function.clone())
            },
        )?;
        self.define(
            function,
            Value::string("prototype").units(),
            Property {
                value: object,
                writable: true,
                enumerable: false,
                configurable: false,
                accessor: None,
            },
        )?;
        self.object_mut(function)?.function_initialized = true;
        Ok(())
    }

    fn define_descriptor(&mut self, object: &Value, key: Value, desc: &Value) -> Result<(), Error> {
        self.object_ref(object)?;
        self.object_ref(desc)?;
        let roots = self.native_roots.len();
        self.native_roots.extend([object.clone(), desc.clone()]);
        self.native_roots.push(key.clone());
        let result = (|| {
            let fields = self.read_descriptor(desc)?;
            self.apply_descriptor(object, key, fields)
        })();
        self.native_roots.truncate(roots);
        result
    }

    fn define_properties(&mut self, object: &Value, descriptors: &Value) -> Result<(), Error> {
        self.object_ref(object)?;
        let roots = self.native_roots.len();
        self.native_roots
            .extend([object.clone(), descriptors.clone()]);
        let result = (|| {
            // Conversion of every descriptor precedes the first definition.
            let descriptors = self.box_value(descriptors)?;
            self.native_roots.push(descriptors.clone());
            let mut keys: alloc::vec::Vec<_> = self
                .own_keys(&descriptors)?
                .into_iter()
                .map(Value::String)
                .collect();
            keys.extend(self.symbol_keys(&descriptors)?);
            self.native_roots.extend_from_slice(&keys);
            let mut converted = alloc::vec::Vec::new();
            for key in keys {
                self.charge(1)?;
                if self
                    .own_key(&descriptors, &key)?
                    .is_some_and(|p| p.enumerable)
                {
                    let desc = self.get_key(&descriptors, &key)?;
                    self.native_roots.push(desc.clone());
                    let fields = self.read_descriptor(&desc)?;
                    converted.push((key, fields));
                }
            }
            for (key, fields) in converted {
                self.charge(1)?;
                self.apply_descriptor(object, key, fields)?;
            }
            Ok(())
        })();
        self.native_roots.truncate(roots);
        result
    }

    fn read_descriptor(
        &mut self,
        desc: &Value,
    ) -> Result<alloc::vec::Vec<(&'static str, Value)>, Error> {
        self.object_ref(desc)?;
        let mut fields = alloc::vec::Vec::new();
        for name in [
            "enumerable",
            "configurable",
            "value",
            "writable",
            "get",
            "set",
        ] {
            let field = Value::string(name).units();
            if self.has_property(desc, &field)? {
                let value = self.get(desc, &field)?;
                if matches!(name, "get" | "set")
                    && !matches!(value, Value::Function(_) | Value::Undefined)
                {
                    return Err(Error::Type {
                        message: "accessor must be callable or undefined",
                    });
                }
                self.native_roots.push(value.clone());
                fields.push((name, value));
            }
        }
        let accessor = fields.iter().any(|(n, _)| matches!(*n, "get" | "set"));
        let data = fields
            .iter()
            .any(|(n, _)| matches!(*n, "value" | "writable"));
        if accessor && data {
            return Err(Error::Type {
                message: "descriptor mixes data and accessors",
            });
        }
        Ok(fields)
    }

    fn apply_descriptor(
        &mut self,
        object: &Value,
        key: Value,
        fields: alloc::vec::Vec<(&'static str, Value)>,
    ) -> Result<(), Error> {
        let accessor = fields.iter().any(|(n, _)| matches!(*n, "get" | "set"));
        let data = fields
            .iter()
            .any(|(n, _)| matches!(*n, "value" | "writable"));
        let mut property = self.own_key(object, &key)?.unwrap_or(Property {
            value: Value::Undefined,
            writable: false,
            enumerable: false,
            configurable: false,
            accessor: None,
        });
        if accessor && property.accessor.is_none() {
            property.value = Value::Undefined;
            property.writable = false;
            property.accessor = Some((Value::Undefined, Value::Undefined));
        }
        if data && property.accessor.is_some() {
            property.accessor = None;
            property.value = Value::Undefined;
            property.writable = false;
        }
        for (name, value) in fields {
            match name {
                "value" => property.value = value,
                "writable" => property.writable = value.to_boolean(),
                "enumerable" => property.enumerable = value.to_boolean(),
                "configurable" => property.configurable = value.to_boolean(),
                "get" => property.accessor.as_mut().ok_or(Error::InvalidBytecode)?.0 = value,
                "set" => property.accessor.as_mut().ok_or(Error::InvalidBytecode)?.1 = value,
                _ => return Err(Error::InvalidBytecode),
            }
        }
        self.define_key(object, key, property)
    }
}

const fn rejection(strict: bool) -> Result<(), Error> {
    if strict {
        Err(Error::Type {
            message: "property assignment or deletion rejected",
        })
    } else {
        Ok(())
    }
}
const fn native(builtin: Builtin) -> Value {
    Value::Function(FunctionValue::native(builtin))
}
pub(super) const fn has_constructor_storage(value: &Value) -> bool {
    matches!(
        value,
        Value::Function(FunctionValue(Callable::Native(
            Builtin::Object | Builtin::Number | Builtin::RegExp | Builtin::Array
        )))
    )
}
fn length(value: usize) -> Value {
    Value::Number(f64::from(u32::try_from(value).unwrap_or(u32::MAX)))
}
fn native_property(builtin: Builtin, key: &[u16]) -> Option<Value> {
    if builtin == Builtin::ArrayConcat {
        if key == Value::string("length").units().as_ref() {
            return Some(Value::Number(1.0));
        }
        if key == Value::string("name").units().as_ref() {
            return Some(Value::string("concat"));
        }
    }
    if let Some(name) = super::errors::error_name(builtin)
        && key == Value::string("name").units().as_ref()
    {
        return Some(Value::string(name));
    }
    if builtin == Builtin::Error && key == Value::string("isError").units().as_ref() {
        return Some(native(Builtin::ErrorIsError));
    }
    if builtin == Builtin::String && key == Value::string("fromCharCode").units().as_ref() {
        return Some(native(Builtin::StringFromCharCode));
    }
    if builtin == Builtin::Promise {
        return match String::from_utf16(key).ok()?.as_str() {
            "resolve" => Some(native(Builtin::PromiseResolve)),
            "reject" => Some(native(Builtin::PromiseReject)),
            "withResolvers" => Some(native(Builtin::PromiseWithResolvers)),
            "all" => Some(native(Builtin::PromiseAll)),
            "allSettled" => Some(native(Builtin::PromiseAllSettled)),
            "race" => Some(native(Builtin::PromiseRace)),
            _ => None,
        };
    }
    if builtin != Builtin::Object {
        return None;
    }
    let name = String::from_utf16(key).ok()?;
    Some(native(match name.as_str() {
        "create" => Builtin::ObjectCreate,
        "getPrototypeOf" => Builtin::GetPrototypeOf,
        "setPrototypeOf" => Builtin::SetPrototypeOf,
        "defineProperty" => Builtin::DefineProperty,
        "defineProperties" => Builtin::DefineProperties,
        "getOwnPropertyDescriptor" => Builtin::GetOwnPropertyDescriptor,
        "hasOwn" => Builtin::HasOwn,
        "preventExtensions" => Builtin::PreventExtensions,
        "isExtensible" => Builtin::IsExtensible,
        "freeze" => Builtin::Freeze,
        "seal" => Builtin::Seal,
        "is" => Builtin::ObjectIs,
        "isFrozen" => Builtin::IsFrozen,
        "isSealed" => Builtin::IsSealed,
        "keys" => Builtin::ObjectKeys,
        "getOwnPropertyNames" => Builtin::GetOwnPropertyNames,
        "getOwnPropertySymbols" => Builtin::GetOwnPropertySymbols,
        "entries" => Builtin::ObjectEntries,
        "values" => Builtin::ObjectValues,
        _ => return None,
    }))
}
