// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bound calls are flattened by the VM dispatch loop, not recursive Rust calls.

use super::Execution;
use crate::{
    Error, Value,
    heap::{Handle, Node},
    object::{Object, Property},
    value::{Callable, FunctionValue},
};

impl Execution<'_> {
    pub(super) fn function_constructor_property(
        &mut self,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        let (value, configurable) = if key == Value::string("prototype").units().as_ref() {
            (self.function_prototype()?, false)
        } else if key == Value::string("length").units().as_ref() {
            (Value::Number(1.0), true)
        } else if key == Value::string("name").units().as_ref() {
            (Value::string("Function"), true)
        } else {
            return Ok(None);
        };
        Ok(Some(Property {
            value,
            writable: false,
            enumerable: false,
            configurable,
            accessor: None,
        }))
    }
    pub(super) fn function_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.function_proto {
            return Ok(value.clone());
        }
        let program = crate::compile("(function(){})", crate::Limits::default())?;
        let code = program
            .functions
            .first()
            .ok_or(Error::InvalidBytecode)?
            .clone();
        let parent = self.prototype()?;
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Function {
                code,
                captures: alloc::vec::Vec::new(),
                lexical_this: None,
                object: Object::new(parent),
            },
            self.limits.heap_entries,
        )?;
        let value = Value::Function(FunctionValue(Callable::Script {
            handle,
            owner: self.owner.clone(),
        }));
        self.function_proto = Some(value.clone());
        let method = self.new_host_behavior(
            crate::heap::HostBehavior::Intrinsic(crate::bytecode::Builtin::FunctionHasInstance),
            "[Symbol.hasInstance]",
            1,
        )?;
        self.native_roots.push(method.clone());
        let key = Value::Symbol(self.well_known("hasInstance")?);
        self.define_key(
            &value,
            key,
            Property {
                value: method,
                writable: false,
                enumerable: false,
                configurable: false,
                accessor: None,
            },
        )?;
        self.object_mut(&value)?.function_initialized = true;
        for (name, v) in [("length", Value::Number(0.0)), ("name", Value::string(""))] {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    value: v,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
        }
        for (name, builtin) in [
            ("call", crate::bytecode::Builtin::FunctionCall),
            ("apply", crate::bytecode::Builtin::FunctionApply),
            ("constructor", crate::bytecode::Builtin::Function),
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
        for (name, kind, length) in [
            ("toString", crate::bytecode::Builtin::FunctionToString, 0),
            ("bind", crate::bytecode::Builtin::FunctionBind, 1),
        ] {
            let method =
                self.new_host_behavior(crate::heap::HostBehavior::Intrinsic(kind), name, length)?;
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(method)
                },
            )?;
        }
        self.restrict_function_properties(&value)?;
        Ok(value)
    }
    fn restrict_function_properties(&mut self, value: &Value) -> Result<(), Error> {
        let thrower = self.thrower()?;
        for name in ["caller", "arguments"] {
            self.define(
                value,
                Value::string(name).units(),
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: Some((thrower.clone(), thrower.clone())),
                },
            )?;
        }
        Ok(())
    }
    pub(super) fn thrower(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.throw_type_error {
            return Ok(value.clone());
        }
        let value = self.new_host_behavior(
            crate::heap::HostBehavior::Intrinsic(crate::bytecode::Builtin::ThrowTypeError),
            "",
            0,
        )?;
        self.throw_type_error = Some(value.clone());
        for (key, property_value) in [("length", Value::Number(0.0)), ("name", Value::string(""))] {
            self.define(
                &value,
                Value::string(key).units(),
                Property {
                    value: property_value,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
        }
        self.object_mut(&value)?.extensible = false;
        Ok(value)
    }
    pub(super) fn arguments_object(
        &mut self,
        code: &crate::bytecode::FunctionCode,
        callee: Value,
        start: usize,
        count: usize,
    ) -> Result<Value, Error> {
        let proto = self.prototype()?;
        let object = self.allocate_object(proto)?;
        self.object_mut(&object)?.arguments = true;
        for index in 0..count {
            let value = self
                .stack
                .get(start.saturating_add(index))
                .ok_or(Error::InvalidBytecode)?
                .clone();
            self.define(
                &object,
                Value::string(&alloc::format!("{index}")).units(),
                Property::data(value),
            )?;
        }
        self.define(
            &object,
            Value::string("length").units(),
            Property {
                enumerable: false,
                ..Property::data(Value::Number(f64::from(
                    u32::try_from(count).unwrap_or(u32::MAX),
                )))
            },
        )?;
        let mapped =
            !code.strict && code.initialization == crate::bytecode::ParameterInitialization::Direct;
        if mapped {
            let mut visited = alloc::collections::BTreeSet::new();
            for (position, slot) in code.parameters.iter().enumerate().rev() {
                if visited.insert(*slot) && position < count {
                    let handle = self
                        .frames
                        .last()
                        .ok_or(Error::InvalidBytecode)?
                        .handle(crate::bytecode::Access::Local(*slot))?;
                    self.object_mut(&object)?
                        .argument_map
                        .insert(Value::string(&alloc::format!("{position}")).units(), handle);
                }
            }
            self.define(
                &object,
                Value::string("callee").units(),
                Property {
                    enumerable: false,
                    ..Property::data(callee)
                },
            )?;
        } else {
            let roots = self.native_roots.len();
            self.native_roots.push(object.clone());
            let thrower = self.thrower()?;
            self.define(
                &object,
                Value::string("callee").units(),
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: Some((thrower.clone(), thrower)),
                },
            )?;
            self.native_roots.truncate(roots);
        }
        self.install_iterator(&object, crate::bytecode::Builtin::ArrayValues)?;
        Ok(object)
    }
    pub(super) fn bind_function(&mut self, target: &Value, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(target.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.bind_rooted(target, args);
        self.native_roots.truncate(roots);
        result
    }
    fn bind_rooted(&mut self, target: &Value, args: &[Value]) -> Result<Value, Error> {
        if !matches!(target, Value::Function(_)) {
            return Err(Error::Type {
                message: "bind requires a callable target",
            });
        }
        let receiver = args.first().cloned().unwrap_or(Value::Undefined);
        if args.len().saturating_sub(1) > self.limits.stack {
            return Err(Error::Limit {
                resource: "bound arguments",
            });
        }
        let arguments = args.get(1..).unwrap_or_default().to_vec();
        let prototype = self.prototype_of(target)?;
        self.native_roots.push(prototype.clone());
        let constructible = self.is_constructor(target);
        let length = if self
            .own(target, &Value::string("length").units())?
            .is_some()
        {
            self.get(target, &Value::string("length").units())?
        } else {
            Value::Undefined
        };
        let length = if let Value::Number(n) = length {
            // Every positive binary64 value below 2^52 can be truncated by
            // subtracting its remainder. Larger finite values are integral.
            let integer = if n.is_nan() {
                0.0
            } else if n.is_infinite() {
                n
            } else {
                n - n % 1.0
            };
            (integer - f64::from(u32::try_from(arguments.len()).unwrap_or(u32::MAX))).max(0.0)
        } else {
            0.0
        };
        let name = self.get(target, &Value::string("name").units())?;
        let mut name_units = Value::string("bound ").units().to_vec();
        if let Value::String(units) = name {
            name_units.extend_from_slice(&units);
        }
        if name_units.len() > self.limits.string_units {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::Bound {
                target: target.clone(),
                constructible,
                receiver,
                arguments,
                object: Object::new(prototype),
            },
            self.limits.heap_entries,
        )?;
        let value = Value::Function(FunctionValue(Callable::Bound {
            handle,
            owner: self.owner.clone(),
        }));
        for (key, item) in [
            ("length", Value::Number(length)),
            ("name", Value::String(name_units.into())),
        ] {
            self.define(
                &value,
                Value::string(key).units(),
                Property {
                    value: item,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
        }
        Ok(value)
    }
    pub(super) fn forward_bound(
        &mut self,
        handle: Handle,
        start: usize,
        args_start: usize,
    ) -> Result<usize, Error> {
        let callee = Value::Function(FunctionValue(Callable::Bound {
            handle,
            owner: self.owner.clone(),
        }));
        let args = self.arguments_from(args_start)?;
        let (target, receiver, args, _) = self.flatten_bound(&callee, &args, None)?;
        let count = args.len();
        self.stack.truncate(start);
        self.push(target)?;
        self.push(receiver)?;
        for value in args {
            self.push(value)?;
        }
        Ok(count)
    }
    /// Iterative bound chain traversal; argument chunks are copied once in
    /// inside-out order, avoiding quadratic repeated prepending.
    pub(super) fn flatten_bound(
        &mut self,
        callee: &Value,
        args: &[Value],
        mut new_target: Option<Value>,
    ) -> Result<(Value, Value, alloc::vec::Vec<Value>, Option<Value>), Error> {
        let mut current = callee.clone();
        let mut receiver = Value::Undefined;
        let mut groups = alloc::vec::Vec::new();
        let mut length = args.len();
        while let Value::Function(FunctionValue(Callable::Bound { handle, .. })) = &current {
            self.charge(1)?;
            let Node::Bound {
                target,
                receiver: bound_this,
                arguments,
                ..
            } = self.heap.get(*handle)?
            else {
                return Err(Error::InvalidBytecode);
            };
            length = length
                .checked_add(arguments.len())
                .filter(|n| *n <= self.limits.stack)
                .ok_or(Error::Limit {
                    resource: "bound argument expansion",
                })?;
            if new_target.as_ref() == Some(&current) {
                new_target = Some(target.clone());
            }
            receiver = bound_this.clone();
            groups.push(*handle);
            current = target.clone();
        }
        if length > self.limits.stack {
            return Err(Error::Limit {
                resource: "bound argument expansion",
            });
        }
        self.charge(u64::try_from(length).unwrap_or(u64::MAX))?;
        let mut result = alloc::vec::Vec::with_capacity(length);
        for handle in groups.into_iter().rev() {
            let Node::Bound { arguments, .. } = self.heap.get(handle)? else {
                return Err(Error::InvalidBytecode);
            };
            result.extend_from_slice(arguments);
        }
        result.extend_from_slice(args);
        Ok((current, receiver, result, new_target))
    }
    pub(super) fn forward_apply(
        &mut self,
        target: Value,
        start: usize,
        args_start: usize,
    ) -> Result<usize, Error> {
        if !matches!(target, Value::Function(_)) {
            return Err(Error::Type {
                message: "apply requires a callable target",
            });
        }
        let receiver = self
            .stack
            .get(args_start)
            .cloned()
            .unwrap_or(Value::Undefined);
        let list = self
            .stack
            .get(args_start.saturating_add(1))
            .cloned()
            .unwrap_or(Value::Undefined);
        let roots = self.native_roots.len();
        self.native_roots
            .extend([target.clone(), receiver.clone(), list.clone()]);
        let result = (|| {
            let mut args = alloc::vec::Vec::new();
            if !matches!(list, Value::Null | Value::Undefined) {
                self.object_ref(&list)?;
                let length_value = self.get(&list, &Value::string("length").units())?;
                let length = self.numeric(&length_value)?;
                if length > f64::from(u32::try_from(self.limits.stack).unwrap_or(u32::MAX)) {
                    return Err(Error::Limit {
                        resource: "apply arguments",
                    });
                }
                let length = if length.is_nan() || length < 0.0 {
                    0
                } else {
                    Value::Number(length).to_uint32()
                };
                for i in 0..length {
                    self.charge(1)?;
                    let value = self.get(&list, &Value::string(&alloc::format!("{i}")).units())?;
                    self.native_roots.push(value.clone());
                    args.push(value);
                }
            }
            let count = args.len();
            self.stack.truncate(start);
            self.push(target)?;
            self.push(receiver)?;
            for value in args {
                self.push(value)?;
            }
            Ok(count)
        })();
        self.native_roots.truncate(roots);
        result
    }
}
