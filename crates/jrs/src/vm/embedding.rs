// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Checked host/VM value boundary. Host callbacks have no direct heap access;
//! queued callbacks reenter only through a later explicit Realm operation.

use super::Execution;
use crate::{
    Error, Value,
    heap::{Handle, HostBehavior, Node},
    object::{Object, Property},
    value::{Callable, FunctionValue},
};
use alloc::rc::Rc;

impl Execution<'_> {
    pub(super) fn host_print(&mut self, args: &[Value]) -> Result<Value, Error> {
        for value in args {
            self.retain_result(value.clone())?;
        }
        let result = self.host.print(args).map(|()| Value::Undefined);
        self.validate_host_result(&result)?;
        result
    }
    pub(super) fn validate_host_result(&self, result: &Result<Value, Error>) -> Result<(), Error> {
        if let Ok(value) | Err(Error::Thrown { value }) = result {
            self.validate_host_value(value).map_err(|_| Error::Host)?;
        }
        Ok(())
    }
    pub(super) fn validate_host_value(&self, value: &Value) -> Result<(), Error> {
        if let Value::String(units) = value
            && units.len() > self.limits.string_units
        {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        let owner = match value {
            Value::Object(o) => Some(&o.owner),
            Value::Symbol(s) => Some(&s.owner),
            Value::Function(FunctionValue(
                Callable::Host { owner, .. }
                | Callable::Script { owner, .. }
                | Callable::Bound { owner, .. }
                | Callable::Resolver { owner, .. },
            )) => Some(owner),
            _ => None,
        };
        if owner.is_some_and(|owner| !Rc::ptr_eq(owner, &self.owner)) {
            return Err(Error::Type {
                message: "value belongs to another realm",
            });
        }
        if let Some(handle) = value.heap_handle() {
            let node = self.heap.get(handle).map_err(|_| Error::Type {
                message: "stale realm value",
            })?;
            let valid = matches!(
                (value, node),
                (Value::Object(_), Node::Object(_))
                    | (Value::Symbol(_), Node::Symbol)
                    | (
                        Value::Function(FunctionValue(Callable::Host { .. })),
                        Node::HostFunction { .. }
                    )
                    | (
                        Value::Function(FunctionValue(Callable::Script { .. })),
                        Node::Function { .. }
                    )
                    | (
                        Value::Function(FunctionValue(Callable::Bound { .. })),
                        Node::Bound { .. }
                    )
                    | (
                        Value::Function(FunctionValue(Callable::Resolver { .. })),
                        Node::Resolving { .. }
                    )
            );
            if !valid {
                return Err(Error::Type {
                    message: "invalid realm value kind",
                });
            }
        }
        Ok(())
    }
    pub(super) fn new_host_function(
        &mut self,
        id: u32,
        name: &str,
        length: u32,
    ) -> Result<Value, Error> {
        self.new_host_behavior(HostBehavior::Capability(id), name, length)
    }
    pub(super) fn new_host_behavior(
        &mut self,
        behavior: HostBehavior,
        name: &str,
        length: u32,
    ) -> Result<Value, Error> {
        let name = Value::string(name).units();
        // Intrinsic names are fixed engine metadata, like the names on immediate
        // Builtins. Caller-supplied embedding names remain quota checked.
        if name.len() > self.limits.string_units
            && !matches!(
                behavior,
                HostBehavior::Intrinsic(_) | HostBehavior::RegExpGet(_)
            )
        {
            return Err(Error::Limit {
                resource: "host function name",
            });
        }
        let proto = self.function_prototype()?;
        self.reserve()?;
        let handle = self.heap.allocate(
            Node::HostFunction {
                behavior,
                name: name.clone(),
                object: Object::new(proto),
            },
            self.limits.heap_entries,
        )?;
        let function = Value::Function(FunctionValue(Callable::Host {
            handle,
            owner: self.owner.clone(),
        }));
        for (key, value) in [
            ("length", Value::Number(f64::from(length))),
            ("name", Value::String(name)),
        ] {
            self.define(
                &function,
                Value::string(key).units(),
                Property {
                    value,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: None,
                },
            )?;
        }
        Ok(function)
    }
    pub(super) fn invoke_host(
        &mut self,
        handle: Handle,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let Node::HostFunction { behavior, .. } = self.heap.get(handle)? else {
            return Err(Error::InvalidBytecode);
        };
        let id = match *behavior {
            HostBehavior::RegExpGet(field) => return self.regexp_get(receiver, field),
            HostBehavior::AsyncFunctionConstructor => {
                let ctor = self
                    .async_function_ctor
                    .clone()
                    .ok_or(Error::InvalidBytecode)?;
                return self.dynamic_function_kind(args, &ctor, crate::parser::AsyncKind::Async);
            }
            HostBehavior::EvalScript => {
                return self.eval_global_source(args.first().unwrap_or(&Value::Undefined));
            }
            HostBehavior::Intrinsic(kind) => return self.native_call(kind, receiver, args),
            HostBehavior::CollectGarbage => {
                self.collect_garbage()?;
                return Ok(Value::Undefined);
            }
            HostBehavior::StringPrint => {
                let roots = self.native_roots.len();
                self.native_roots.extend_from_slice(args);
                let result = (|| {
                    let value = Value::String(
                        self.string_units(args.first().unwrap_or(&Value::Undefined))?,
                    );
                    let result = self.host.print(&[value]).map(|()| Value::Undefined);
                    self.validate_host_result(&result)?;
                    result
                })();
                self.native_roots.truncate(roots);
                return result;
            }
            HostBehavior::DomExceptionGet(field) => return self.dom_exception_get(receiver, field),
            HostBehavior::AbortGet(field) => return self.abort_get(receiver, field),
            HostBehavior::AbortSet => {
                self.abort_handler_set(receiver, args.first().unwrap_or(&Value::Undefined))?;
                return Ok(Value::Undefined);
            }
            HostBehavior::EventGet(field) => return self.event_get(receiver, field),
            HostBehavior::EventSet(field) => {
                self.event_set(receiver, field, args.first().unwrap_or(&Value::Undefined))?;
                return Ok(Value::Undefined);
            }
            HostBehavior::Capability(id) => id,
            HostBehavior::QueueMicrotask => {
                let callback = args.first().cloned().unwrap_or(Value::Undefined);
                self.enqueue_microtask(callback)?;
                return Ok(Value::Undefined);
            }
        };
        self.validate_host_value(receiver)?;
        self.retain_result(receiver.clone())?;
        for value in args {
            self.validate_host_value(value)?;
            self.retain_result(value.clone())?;
        }
        // No VM operation can run during Host::call: the realm's mutable borrow
        // is still held. Validate before publishing a host-owned value to the VM.
        let result = self.host.call(id, receiver, args);
        self.validate_host_result(&result)?;
        result
    }
    pub(super) fn function_text(&mut self, receiver: &Value) -> Result<Value, Error> {
        if !matches!(receiver, Value::Function(_)) {
            return Err(Error::Type {
                message: "Function.toString requires a callable",
            });
        }
        if matches!(
            receiver,
            Value::Function(FunctionValue(Callable::Bound { .. }))
        ) {
            return Ok(Value::string("function () { [native code] }"));
        }
        if self.function_proto.as_ref() == Some(receiver) {
            return Ok(Value::string("function () { [native code] }"));
        }
        if let Some(source) = self
            .object_ref(receiver)
            .ok()
            .and_then(|o| o.dynamic_source.clone())
        {
            return Ok(Value::String(source));
        }
        if let Value::Function(FunctionValue(Callable::Script { handle, .. })) = receiver {
            let Node::Function { code, .. } = self.heap.get(*handle)? else {
                return Err(Error::InvalidBytecode);
            };
            if let Some(source) = code.source.clone() {
                let text = source
                    .text
                    .get(source.range)
                    .ok_or(Error::InvalidBytecode)?;
                let count = text.encode_utf16().count();
                if count > self.limits.string_units {
                    return Err(Error::Limit {
                        resource: "function source string units",
                    });
                }
                self.charge(u64::try_from(count).unwrap_or(u64::MAX))?;
                return Ok(Value::String(text.encode_utf16().collect()));
            }
        }
        if let Value::Function(FunctionValue(Callable::Host { handle, .. })) = receiver {
            let Node::HostFunction { name, .. } = self.heap.get(*handle)? else {
                return Err(Error::InvalidBytecode);
            };
            let mut units = Value::string("function ").units().to_vec();
            units.extend_from_slice(name);
            units.extend("() { [native code] }".encode_utf16());
            if units.len() > self.limits.string_units {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            Ok(Value::String(units.into()))
        } else {
            Ok(Value::string("function () { [native code] }"))
        }
    }
}
