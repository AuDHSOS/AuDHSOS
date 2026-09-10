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
            ("get", Builtin::ReflectGet, 2),
            ("set", Builtin::ReflectSet, 3),
            ("has", Builtin::ReflectHas, 2),
            ("deleteProperty", Builtin::ReflectDeleteProperty, 2),
            ("getPrototypeOf", Builtin::ReflectGetPrototypeOf, 1),
            ("setPrototypeOf", Builtin::ReflectSetPrototypeOf, 2),
            ("isExtensible", Builtin::ReflectIsExtensible, 1),
            ("preventExtensions", Builtin::ReflectPreventExtensions, 1),
            (
                "getOwnPropertyDescriptor",
                Builtin::ReflectGetOwnPropertyDescriptor,
                2,
            ),
            ("defineProperty", Builtin::ReflectDefineProperty, 3),
            ("ownKeys", Builtin::ReflectOwnKeys, 1),
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
    #[expect(
        clippy::too_many_lines,
        reason = "dispatch table for all Reflect static methods"
    )]
    pub(super) fn reflect_call(&mut self, kind: Builtin, args: &[Value]) -> Result<Value, Error> {
        let roots = self.native_roots.len();
        self.native_roots.extend_from_slice(args);
        let result = (|| {
            let target = args.first().unwrap_or(&Value::Undefined);
            match kind {
                Builtin::ReflectGet => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    let key = self.property_key(key)?;
                    self.get_key(target, &key)
                }
                Builtin::ReflectSet => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    let val = args.get(2).unwrap_or(&Value::Undefined);
                    let key = self.property_key(key)?;
                    let success = self.set_key(target, key, val.clone(), false).is_ok();
                    Ok(Value::Boolean(success))
                }
                Builtin::ReflectHas => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    let key = self.property_key(key)?;
                    let has = self.has_key(target, &key)?;
                    Ok(Value::Boolean(has))
                }
                Builtin::ReflectDeleteProperty => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    let key = self.property_key(key)?;
                    let deleted = self.delete_key(target, &key).unwrap_or(false);
                    Ok(Value::Boolean(deleted))
                }
                Builtin::ReflectGetPrototypeOf => {
                    self.object_ref(target)?;
                    self.prototype_of(target)
                }
                Builtin::ReflectSetPrototypeOf => {
                    self.object_ref(target)?;
                    let proto = args.get(1).unwrap_or(&Value::Undefined);
                    if !matches!(proto, Value::Object(_) | Value::Null) {
                        return Err(Error::Type {
                            message: "prototype must be an object or null",
                        });
                    }
                    let success = self.set_prototype(target, proto).is_ok();
                    Ok(Value::Boolean(success))
                }
                Builtin::ReflectIsExtensible => {
                    self.object_ref(target)?;
                    let ext = self.object_ref(target)?.extensible;
                    Ok(Value::Boolean(ext))
                }
                Builtin::ReflectPreventExtensions => {
                    self.object_ref(target)?;
                    self.object_mut(target)?.extensible = false;
                    Ok(Value::Boolean(true))
                }
                Builtin::ReflectGetOwnPropertyDescriptor => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    self.boxed_descriptor(target, key)
                }
                Builtin::ReflectDefineProperty => {
                    self.object_ref(target)?;
                    let key = args.get(1).unwrap_or(&Value::Undefined);
                    let desc = args.get(2).unwrap_or(&Value::Undefined);
                    let key = self.property_key(key)?;
                    let success = self.define_descriptor(target, key, desc).is_ok();
                    Ok(Value::Boolean(success))
                }
                Builtin::ReflectOwnKeys => {
                    self.object_ref(target)?;
                    let mut keys: Vec<Value> = self
                        .own_keys(target)?
                        .into_iter()
                        .map(Value::String)
                        .collect();
                    keys.extend(self.symbol_keys(target)?);
                    self.array_from_values(&keys)
                }
                Builtin::ReflectApply | Builtin::ReflectConstruct => {
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
                }
                _ => Err(Error::InvalidBytecode),
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
