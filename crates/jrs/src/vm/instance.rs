// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! `InstanceofOperator` / `OrdinaryHasInstance`, with iterative default-bound
//! forwarding and real callbacks for custom Symbol.hasInstance methods.
use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    heap::{HostBehavior, Node},
    value::{Callable, FunctionValue},
};
impl Execution<'_> {
    pub(super) fn has_instance(
        &mut self,
        target: &Value,
        value: &Value,
        operator: bool,
    ) -> Result<bool, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend([target.clone(), value.clone()]);
        let result = self.instance_rooted(target.clone(), value, operator);
        self.native_roots.truncate(roots);
        result
    }
    fn instance_rooted(
        &mut self,
        mut target: Value,
        value: &Value,
        mut operator: bool,
    ) -> Result<bool, Error> {
        loop {
            self.charge(1)?;
            self.native_roots.push(target.clone());
            if operator {
                if !matches!(target, Value::Function(_) | Value::Object(_)) {
                    return Err(Error::Type {
                        message: "instanceof target is not an object",
                    });
                }
                let key = Value::Symbol(self.well_known("hasInstance")?);
                let handler = self.get_key(&target, &key)?;
                if !matches!(handler, Value::Undefined | Value::Null) {
                    if !matches!(handler, Value::Function(_)) {
                        return Err(Error::Type {
                            message: "Symbol.hasInstance is not callable",
                        });
                    }
                    // Fast-path only the exact builtin behavior; arbitrary hooks must run.
                    if !self.default_has_instance(&handler) {
                        return Ok(self
                            .call_sync(handler, target, core::slice::from_ref(value))?
                            .to_boolean());
                    }
                } else if !matches!(target, Value::Function(_)) {
                    return Err(Error::Type {
                        message: "instanceof target is not callable",
                    });
                }
            }
            if !matches!(target, Value::Function(_)) {
                return Ok(false);
            }
            if let Value::Function(FunctionValue(Callable::Bound { handle, .. })) = &target {
                let Node::Bound {
                    target: bound_target,
                    ..
                } = self.heap.get(*handle)?
                else {
                    return Err(Error::InvalidBytecode);
                };
                target = bound_target.clone();
                operator = true;
                continue;
            }
            if !matches!(value, Value::Object(_) | Value::Function(_)) {
                return Ok(false);
            }
            let prototype = self.get(&target, &Value::string("prototype").units())?;
            if !matches!(prototype, Value::Object(_) | Value::Function(_)) {
                return Err(Error::Type {
                    message: "constructor prototype is not an object",
                });
            }
            self.native_roots.push(prototype.clone());
            let mut current = value.clone();
            for _ in 0..=self.limits.heap_entries {
                self.charge(1)?;
                current = self.prototype_of(&current)?;
                if current == Value::Null {
                    return Ok(false);
                }
                if current.strictly_equals(&prototype) {
                    return Ok(true);
                }
            }
            return Err(Error::Limit {
                resource: "prototype walk",
            });
        }
    }
    fn default_has_instance(&self, handler: &Value) -> bool {
        match handler {
            Value::Function(FunctionValue(Callable::Host { handle, .. })) => matches!(
                self.heap.get(*handle),
                Ok(Node::HostFunction {
                    behavior: HostBehavior::Intrinsic(Builtin::FunctionHasInstance),
                    ..
                })
            ),
            _ => false,
        }
    }
}
