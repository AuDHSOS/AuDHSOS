// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Class evaluation, method home objects and constructor this initialization.

use super::{Execution, Frame};
use crate::{
    Error, Value,
    bytecode::{Builtin, Op, Slot},
    heap::Node,
    object::Property,
    value::{Callable, FunctionValue},
};

impl Execution<'_> {
    pub(super) fn class_op(&mut self, op: &Op) -> Result<(), Error> {
        match op {
            Op::Class(derived) => self.make_class(*derived),
            Op::ClassMethod(is_static, accessor) => self.class_method(*is_static, *accessor),
            Op::MethodHome => {
                let len = self.stack.len();
                let function = self.stack.last().cloned().ok_or(Error::InvalidBytecode)?;
                let home = self
                    .stack
                    .get(len.saturating_sub(3))
                    .cloned()
                    .ok_or(Error::InvalidBytecode)?;
                self.object_mut(&function)?.home = Some(home);
                Ok(())
            }
            Op::NewTarget => {
                let v = self
                    .frames
                    .last()
                    .ok_or(Error::InvalidBytecode)?
                    .new_target
                    .clone();
                self.push(v)
            }
            Op::SuperBase => {
                self.this_binding()?;
                let home = self
                    .frames
                    .last()
                    .and_then(|f| self.object_ref(&f.callee).ok())
                    .and_then(|o| o.home.clone())
                    .ok_or(Error::Reference {
                        name: "super".into(),
                    })?;
                let base = self.prototype_of(&home)?;
                self.push(base)
            }
            Op::SuperGet(method) => {
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.this_binding()?;
                let value = self.get_super(&base, &key, &receiver)?;
                self.push(value)?;
                if *method {
                    self.push(receiver)?;
                }
                Ok(())
            }
            Op::SuperSet => {
                let value = self.pop()?;
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.this_binding()?;
                self.set_super(&base, &key, &receiver, value.clone())?;
                self.push(value)
            }
            Op::SuperUpdate(add, prefix) => {
                let key = self.pop()?;
                let base = self.pop()?;
                let receiver = self.this_binding()?;
                let roots = self.native_roots.len();
                self.native_roots
                    .extend([key.clone(), base.clone(), receiver.clone()]);
                let result = (|| {
                    let old = self.get_super(&base, &key, &receiver)?;
                    let number = self.numeric(&old)?;
                    let next = if *add { number + 1.0 } else { number - 1.0 };
                    self.set_super(&base, &key, &receiver, Value::Number(next))?;
                    self.push(Value::Number(if *prefix { next } else { number }))
                })();
                self.native_roots.truncate(roots);
                result
            }
            Op::SuperCall(_) | Op::SuperCallExpanded => {
                let count = if let Op::SuperCall(count) = op {
                    *count
                } else {
                    self.expanded_count()?
                };
                self.explicit_super_call(count)
            }
            Op::PrepareSuperCall => {
                let parent = self.super_constructor()?;
                self.push(parent)
            }
            Op::DefaultSuper => {
                let args = self
                    .frames
                    .last()
                    .ok_or(Error::InvalidBytecode)?
                    .arguments
                    .clone();
                let parent = self.super_constructor()?;
                let value = self.super_construct(parent, &args)?;
                self.push(value)
            }
            _ => Err(Error::InvalidBytecode),
        }
    }
    fn explicit_super_call(&mut self, count: usize) -> Result<(), Error> {
        let args = self.stack.split_off(
            self.stack
                .len()
                .checked_sub(count)
                .ok_or(Error::InvalidBytecode)?,
        );
        let parent = self.pop()?;
        let value = self.super_construct(parent, &args)?;
        self.push(value)
    }
    fn make_class(&mut self, derived: bool) -> Result<(), Error> {
        let constructor = self.pop()?;
        let heritage = self.pop()?;
        let roots = self.native_roots.len();
        self.native_roots
            .extend([constructor.clone(), heritage.clone()]);
        let result = (|| {
            let (parent, ctor_parent) = if !derived {
                (self.prototype()?, self.function_prototype()?)
            } else if matches!(heritage, Value::Null) {
                (Value::Null, self.function_prototype()?)
            } else {
                if !self.is_constructor(&heritage) {
                    return Err(Error::Type {
                        message: "class extends non-constructor",
                    });
                }
                let proto = self.get(&heritage, &Value::string("prototype").units())?;
                if !matches!(proto, Value::Null | Value::Object(_) | Value::Function(_)) {
                    return Err(Error::Type {
                        message: "invalid superclass prototype",
                    });
                }
                (proto, heritage)
            };
            let proto = self.allocate_object(parent)?;
            self.native_roots.push(proto.clone());
            self.object_mut(&constructor)?.prototype = ctor_parent;
            self.object_mut(&constructor)?.home = Some(proto.clone());
            self.define(
                &proto,
                Value::string("constructor").units(),
                Property {
                    enumerable: false,
                    ..Property::data(constructor.clone())
                },
            )?;
            self.define(
                &constructor,
                Value::string("prototype").units(),
                Property {
                    value: proto,
                    writable: false,
                    enumerable: false,
                    configurable: false,
                    accessor: None,
                },
            )?;
            self.push(constructor)
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn class_method(&mut self, is_static: bool, accessor: Option<bool>) -> Result<(), Error> {
        let function = self.pop()?;
        let key = self.pop()?;
        let constructor = self.stack.last().cloned().ok_or(Error::InvalidBytecode)?;
        let roots = self.native_roots.len();
        self.native_roots.extend([function.clone(), key.clone()]);
        let result = (|| {
            let home = if is_static {
                constructor
            } else {
                self.get(&constructor, &Value::string("prototype").units())?
            };
            self.object_mut(&function)?.home = Some(home.clone());
            let name = match &key {
                Value::Symbol(s) => Value::string(&alloc::format!(
                    "[{}]",
                    s.description
                        .clone()
                        .map_or(Value::string(""), Value::String)
                ))
                .units(),
                _ => key.units(),
            };
            let mut text = if let Some(setter) = accessor {
                Value::string(if setter { "set " } else { "get " })
                    .units()
                    .to_vec()
            } else {
                alloc::vec::Vec::new()
            };
            text.extend_from_slice(&name);
            self.define(
                &function,
                Value::string("name").units(),
                Property {
                    value: Value::String(text.into()),
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
            let property = if let Some(setter) = accessor {
                let mut pair = self
                    .own_key(&home, &key)?
                    .and_then(|p| p.accessor)
                    .unwrap_or((Value::Undefined, Value::Undefined));
                if setter {
                    pair.1 = function;
                } else {
                    pair.0 = function;
                }
                Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: Some(pair),
                }
            } else {
                Property {
                    enumerable: false,
                    ..Property::data(function)
                }
            };
            self.define_key(&home, key, property)
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn is_constructor(&self, value: &Value) -> bool {
        if self
            .function_proto
            .as_ref()
            .is_some_and(|p| p.strictly_equals(value))
        {
            return false;
        }
        match value {
            Value::Function(FunctionValue(Callable::Host { handle, .. })) => matches!(
                self.heap.get(*handle),
                Ok(Node::HostFunction {
                    behavior: crate::heap::HostBehavior::AsyncFunctionConstructor,
                    ..
                })
            ),
            Value::Function(FunctionValue(Callable::Bound { handle, .. })) => matches!(
                self.heap.get(*handle),
                Ok(Node::Bound {
                    constructible: true,
                    ..
                })
            ),
            Value::Function(FunctionValue(Callable::Script { handle, .. })) => {
                matches!(self.heap.get(*handle),Ok(Node::Function{code,..}) if code.constructible)
            }
            Value::Function(FunctionValue(Callable::Native(kind))) => {
                matches!(
                    kind,
                    Builtin::Object
                        | Builtin::Function
                        | Builtin::Event
                        | Builtin::CustomEvent
                        | Builtin::DomException
                        | Builtin::EventTarget
                        | Builtin::AbortController
                        | Builtin::AbortSignal
                        | Builtin::Array
                        | Builtin::Number
                        | Builtin::String
                        | Builtin::Boolean
                        | Builtin::Promise
                        | Builtin::RegExp
                        | Builtin::WeakMap
                ) || super::errors::error_name(*kind).is_some()
            }
            _ => false,
        }
    }
    pub(super) fn construct_sync(
        &mut self,
        callee: Value,
        args: &[Value],
        new_target: Value,
    ) -> Result<Value, Error> {
        if !self.is_constructor(&callee) || !self.is_constructor(&new_target) {
            return Err(Error::Type {
                message: "value is not a constructor",
            });
        }
        if self.native_depth >= 12 {
            return Err(Error::Limit {
                resource: "constructor reentry",
            });
        }
        let roots = self.native_roots.len();
        self.native_roots
            .extend([callee.clone(), new_target.clone()]);
        self.native_roots.extend_from_slice(args);
        self.native_depth = self.native_depth.saturating_add(1);
        let result = self.construct_rooted(callee, args, new_target);
        self.native_depth = self.native_depth.saturating_sub(1);
        self.native_roots.truncate(roots);
        result
    }
    fn construct_rooted(
        &mut self,
        callee: Value,
        args: &[Value],
        new_target: Value,
    ) -> Result<Value, Error> {
        if matches!(
            callee,
            Value::Function(FunctionValue(Callable::Bound { .. }))
        ) {
            let (callee, _, args, new_target) =
                self.flatten_bound(&callee, args, Some(new_target))?;
            let new_target = new_target.ok_or(Error::InvalidBytecode)?;
            self.native_roots
                .extend([callee.clone(), new_target.clone()]);
            self.native_roots.extend_from_slice(&args);
            return self.construct_unbound(callee, &args, new_target);
        }
        self.construct_unbound(callee, args, new_target)
    }
    fn construct_unbound(
        &mut self,
        callee: Value,
        args: &[Value],
        new_target: Value,
    ) -> Result<Value, Error> {
        if matches!(
            callee,
            Value::Function(FunctionValue(Callable::Host { .. }))
        ) {
            return self.dynamic_function_kind(args, &new_target, crate::parser::AsyncKind::Async);
        }
        if let Value::Function(FunctionValue(Callable::Native(kind))) = &callee {
            return self.construct_native(*kind, args, &new_target);
        }
        let (receiver, cell) = self.constructor_receiver(&callee, &new_target)?;
        let stack = self.stack.len();
        let frames = self.frames.len();
        let boundary = self.unwind_boundary;
        self.unwind_boundary = frames;
        let program = self.top_program.clone().ok_or(Error::InvalidBytecode)?;
        let result = (|| {
            self.push(callee)?;
            self.push(receiver)?;
            for value in args {
                self.push(value.clone())?;
            }
            self.invoke_kind(args.len(), &program, Some((new_target, cell)))?;
            self.execute(&program, frames)
        })();
        self.unwind_boundary = boundary;
        while self.frames.len() > frames {
            let frame = self.frames.pop().ok_or(Error::InvalidBytecode)?;
            self.binding_slots = self.binding_slots.saturating_sub(frame.locals.len());
        }
        self.stack.truncate(stack);
        result
    }
    pub(super) fn construct_dispatch(
        &mut self,
        mut count: usize,
        program: &crate::Program,
    ) -> Result<(), Error> {
        let start = self
            .stack
            .len()
            .checked_sub(count.saturating_add(2))
            .ok_or(Error::InvalidBytecode)?;
        let mut callee = self
            .stack
            .get(start)
            .cloned()
            .ok_or(Error::InvalidBytecode)?;
        if !self.is_constructor(&callee) {
            return Err(Error::Type {
                message: "value is not a constructor",
            });
        }
        let mut new_target = callee.clone();
        if matches!(
            callee,
            Value::Function(FunctionValue(Callable::Bound { .. }))
        ) {
            let args = self.arguments_from(start.saturating_add(2))?;
            let (target, _, args, adjusted) =
                self.flatten_bound(&callee, &args, Some(new_target))?;
            callee = target;
            new_target = adjusted.ok_or(Error::InvalidBytecode)?;
            count = args.len();
            self.stack.truncate(start);
            self.push(callee.clone())?;
            self.push(Value::Undefined)?;
            for arg in args {
                self.push(arg)?;
            }
        }
        if matches!(
            callee,
            Value::Function(FunctionValue(Callable::Native(_) | Callable::Host { .. }))
        ) {
            let args = self
                .stack
                .get(start.saturating_add(2)..)
                .ok_or(Error::InvalidBytecode)?
                .to_vec();
            let value = self.construct_sync(callee, &args, new_target)?;
            self.stack.truncate(start);
            return self.push(value);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(new_target.clone());
        let result = (|| {
            let (receiver, cell) = self.constructor_receiver(&callee, &new_target)?;
            *self
                .stack
                .get_mut(start.saturating_add(1))
                .ok_or(Error::InvalidBytecode)? = receiver;
            self.invoke_kind(count, program, Some((new_target, cell)))
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn constructor_receiver(
        &mut self,
        callee: &Value,
        new_target: &Value,
    ) -> Result<(Value, Option<crate::heap::Handle>), Error> {
        let derived = matches!(self.heap.get(callee.heap_handle().ok_or(Error::InvalidBytecode)?)?,Node::Function{code,..} if code.constructor_kind==crate::parser::ConstructorKind::DerivedClass);
        let receiver = if derived {
            Value::Undefined
        } else {
            let fallback = self.prototype()?;
            let proto = self.constructor_prototype(new_target, fallback)?;
            self.allocate_object(proto)?
        };
        self.native_roots.push(receiver.clone());
        let cell = if derived {
            self.reserve()?;
            let handle = self.heap.allocate(
                Node::Cell {
                    slot: Slot {
                        name: "this".into(),
                        mutable: true,
                        captured: true,
                    },
                    value: None,
                },
                self.limits.heap_entries,
            )?;
            self.native_roots.push(Value::Object(crate::ObjectValue {
                handle,
                owner: self.owner.clone(),
            }));
            Some(handle)
        } else {
            None
        };
        Ok((receiver, cell))
    }
    pub(super) fn constructor_prototype(
        &mut self,
        target: &Value,
        fallback: Value,
    ) -> Result<Value, Error> {
        let proto = self.get(target, &Value::string("prototype").units())?;
        Ok(if matches!(proto, Value::Object(_) | Value::Function(_)) {
            proto
        } else {
            fallback
        })
    }
    fn construct_native(
        &mut self,
        kind: Builtin,
        args: &[Value],
        target: &Value,
    ) -> Result<Value, Error> {
        if kind == Builtin::RegExp {
            return self.regexp_constructor_call(args, Some(target));
        }
        if kind == Builtin::Function {
            return self.dynamic_function(args, target);
        }
        if kind == Builtin::DomException {
            return self.construct_dom_exception(args, target);
        }
        if matches!(kind, Builtin::AbortController | Builtin::AbortSignal) {
            return self.construct_abort(kind, target);
        }
        if matches!(
            kind,
            Builtin::Event | Builtin::CustomEvent | Builtin::EventTarget
        ) {
            return self.construct_event(kind, args, target);
        }
        if kind == Builtin::Object
            && target.strictly_equals(&Value::Function(FunctionValue::native(Builtin::Object)))
        {
            return self.native_call(kind, &Value::Undefined, args);
        }
        let default = match kind {
            Builtin::Promise => self.promise_prototype()?,
            Builtin::Array => self.array_prototype()?,
            Builtin::WeakMap => self.weakmap_prototype()?,
            Builtin::Number | Builtin::Boolean | Builtin::String => {
                self.primitive_prototype(kind)?
            }
            k if super::errors::error_name(k).is_some() => self.error_prototype(k)?,
            _ => self.prototype()?,
        };
        let proto = self.constructor_prototype(target, default)?;
        self.native_roots.push(proto.clone());
        if kind == Builtin::Promise {
            return self.construct_promise(args, proto);
        }
        if kind == Builtin::WeakMap {
            return self
                .construct_weakmap_with_proto(args.first().unwrap_or(&Value::Undefined), proto);
        }
        let mapped = match kind {
            Builtin::WeakMap => Builtin::WeakMapConstruct,
            Builtin::Number => Builtin::NumberConstruct,
            Builtin::String => Builtin::StringConstruct,
            Builtin::Boolean => Builtin::BooleanConstruct,
            k => k,
        };
        let value = if kind == Builtin::Object
            && !target.strictly_equals(&Value::Function(FunctionValue::native(Builtin::Object)))
        {
            self.allocate_object(proto.clone())?
        } else {
            self.native_call(mapped, &Value::Undefined, args)?
        };
        if matches!(value, Value::Object(_) | Value::Function(_)) {
            self.object_mut(&value)?.prototype = proto;
        }
        Ok(value)
    }
    fn super_constructor(&mut self) -> Result<Value, Error> {
        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
        let callee = self
            .object_ref(&frame.callee)?
            .lexical_super
            .clone()
            .unwrap_or_else(|| frame.callee.clone());
        self.prototype_of(&callee)
    }
    fn super_construct(&mut self, parent: Value, args: &[Value]) -> Result<Value, Error> {
        let frame = self.frames.last().ok_or(Error::InvalidBytecode)?;
        let target = frame.new_target.clone();
        let cell = frame.this_cell.ok_or(Error::Reference {
            name: "this".into(),
        })?;
        let result = self.construct_sync(parent, args, target)?;
        let Node::Cell { value, .. } = self.heap.get_mut(cell)? else {
            return Err(Error::InvalidBytecode);
        };
        if value.is_some() {
            return Err(Error::Reference {
                name: "this already initialized".into(),
            });
        }
        *value = Some(result.clone());
        Ok(result)
    }
    pub(super) fn this_binding(&mut self) -> Result<Value, Error> {
        if self.frames.last().is_some_and(|frame| frame.code.is_none()) {
            return self.global_object();
        }
        self.frame_this(self.frames.last().ok_or(Error::InvalidBytecode)?)
    }
    pub(super) fn frame_this(&self, frame: &Frame) -> Result<Value, Error> {
        if let Some(cell) = frame.this_cell {
            let Node::Cell { value, .. } = self.heap.get(cell)? else {
                return Err(Error::InvalidBytecode);
            };
            value.clone().ok_or(Error::Reference {
                name: "this".into(),
            })
        } else {
            Ok(frame.this_value.clone())
        }
    }
    fn get_super(&mut self, base: &Value, key: &Value, receiver: &Value) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots
            .extend([base.clone(), key.clone(), receiver.clone()]);
        let result = (|| {
            let mut current = base.clone();
            for _ in 0..=self.limits.heap_entries {
                self.charge(1)?;
                if let Some(p) = self.own_key(&current, key)? {
                    return if let Some((get, _)) = p.accessor {
                        if matches!(get, Value::Undefined) {
                            Ok(Value::Undefined)
                        } else {
                            self.call_sync(get, receiver.clone(), &[])
                        }
                    } else {
                        Ok(p.value)
                    };
                }
                current = self.prototype_of(&current)?;
                if matches!(current, Value::Null) {
                    return Ok(Value::Undefined);
                }
            }
            Err(Error::Limit {
                resource: "super prototype walk",
            })
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn set_super(
        &mut self,
        base: &Value,
        key: &Value,
        receiver: &Value,
        value: Value,
    ) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots
            .extend([base.clone(), key.clone(), receiver.clone(), value.clone()]);
        let result = (|| {
            let mut current = base.clone();
            for _ in 0..=self.limits.heap_entries {
                self.charge(1)?;
                if let Some(p) = self.own_key(&current, key)? {
                    if let Some((_, set)) = p.accessor {
                        if matches!(set, Value::Undefined) {
                            return Err(Error::Type {
                                message: "super setter missing",
                            });
                        }
                        self.call_sync(set, receiver.clone(), &[value])?;
                        return Ok(());
                    }
                    if !p.writable {
                        return Err(Error::Type {
                            message: "super property not writable",
                        });
                    }
                    break;
                }
                current = self.prototype_of(&current)?;
                if matches!(current, Value::Null) {
                    break;
                }
            }
            let p = if let Some(p) = self.own_key(receiver, key)? {
                if p.accessor.is_some() || !p.writable {
                    return Err(Error::Type {
                        message: "super receiver property rejected",
                    });
                }
                Property { value, ..p }
            } else {
                Property::data(value)
            };
            self.define_key(receiver, key.clone(), p)
        })();
        self.native_roots.truncate(roots);
        result
    }
}
