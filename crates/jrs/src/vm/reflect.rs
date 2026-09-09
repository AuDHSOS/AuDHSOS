// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Reflect invocation uses the VM's exact Call/Construct semantics. Array-like
//! arguments (not iterables) are captured once, rooted and bounded before calls.
use super::Execution;
use crate::{Error, Value, bytecode::Builtin, heap::HostBehavior, object::Property};
use alloc::vec::Vec;
impl Execution<'_> {
    pub(super) fn reflect_object(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.reflect {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let object = self.allocate_object(proto)?;
        self.reflect = Some(object.clone());
        self.install_tag(&object, "Reflect")?;
        for (name, kind, length) in [
            ("apply", Builtin::ReflectApply, 3),
            ("construct", Builtin::ReflectConstruct, 2),
        ] {
            let function = self.new_host_behavior(HostBehavior::Intrinsic(kind), name, length)?;
            self.define(
                &object,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(function)
                },
            )?;
        }
        Ok(object)
    }
    pub(super) fn reflect_call(&mut self, kind: Builtin, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            let target = args.first().unwrap_or(&Value::Undefined);
            let construct = kind == Builtin::ReflectConstruct;
            if construct && !self.is_constructor(target)
                || !construct && !matches!(target, Value::Function(_))
            {
                return Err(Error::Type {
                    message: "invalid Reflect target",
                });
            }
            let new_target = if construct {
                args.get(2).unwrap_or(target)
            } else {
                &Value::Undefined
            };
            if construct && !self.is_constructor(new_target) {
                return Err(Error::Type {
                    message: "invalid Reflect newTarget",
                });
            }
            let list = args
                .get(if construct { 1 } else { 2 })
                .unwrap_or(&Value::Undefined);
            let values = self.argument_list(list)?;
            if construct {
                self.construct_sync(target.clone(), &values, new_target.clone())
            } else {
                self.call_sync(
                    target.clone(),
                    args.get(1).cloned().unwrap_or(Value::Undefined),
                    &values,
                )
            }
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn argument_list(&mut self, list: &Value) -> Result<Vec<Value>, Error> {
        if !matches!(list, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "Reflect arguments must be an object",
            });
        }
        let length = self.get(list, &Value::string("length").units())?;
        let n = self.numeric(&length)?;
        let integer = if n.is_nan() || n <= 0.0 {
            0.0
        } else if n.is_infinite() {
            n
        } else {
            n - n % 1.0
        };
        if integer > f64::from(u32::try_from(self.limits.stack).unwrap_or(u32::MAX)) {
            return Err(Error::Limit {
                resource: "Reflect arguments",
            });
        }
        let n = Value::Number(integer).to_uint32();
        let mut args = Vec::new();
        for i in 0..n {
            self.charge(1)?;
            let value = self.get(list, &Value::string(&alloc::format!("{i}")).units())?;
            self.native_roots.push(value.clone());
            args.push(value);
        }
        Ok(args)
    }
}
