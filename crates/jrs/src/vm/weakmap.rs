// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `WeakMap` algorithms with rooted native intermediates and quota-aware inserts.

use super::Execution;
use crate::{
    Error, Value,
    bytecode::Builtin,
    object::Property,
    value::FunctionValue,
    weakmap::{Key, WeakMap},
};

const METHODS: &[(&str, Builtin, usize)] = &[
    ("get", Builtin::WeakMapGet, 1),
    ("set", Builtin::WeakMapSet, 2),
    ("has", Builtin::WeakMapHas, 1),
    ("delete", Builtin::WeakMapDelete, 1),
    ("getOrInsert", Builtin::WeakMapGetOrInsert, 2),
    (
        "getOrInsertComputed",
        Builtin::WeakMapGetOrInsertComputed,
        2,
    ),
];

impl Execution<'_> {
    pub(super) fn weakmap_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.weakmap_proto {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.weakmap_proto = Some(value.clone());
        self.install_tag(&value, "WeakMap")?;
        for (name, builtin) in core::iter::once(("constructor", Builtin::WeakMap))
            .chain(METHODS.iter().map(|(name, builtin, _)| (*name, *builtin)))
        {
            self.define(
                &value,
                Value::string(name).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(builtin))
                },
            )?;
        }
        Ok(value)
    }

    pub(super) fn weakmap_property(
        &mut self,
        builtin: Builtin,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        if builtin == Builtin::WeakMap && key == Value::string("prototype").units().as_ref() {
            return Ok(Some(Property {
                value: self.weakmap_prototype()?,
                writable: false,
                enumerable: false,
                configurable: false,
                accessor: None,
            }));
        }
        let (name, length) = if builtin == Builtin::WeakMap {
            ("WeakMap", 0)
        } else if let Some((name, _, length)) = METHODS.iter().find(|(_, id, _)| *id == builtin) {
            (*name, *length)
        } else {
            return Ok(None);
        };
        let value = if key == Value::string("name").units().as_ref() {
            Value::string(name)
        } else if key == Value::string("length").units().as_ref() {
            Value::Number(f64::from(u32::try_from(length).unwrap_or(u32::MAX)))
        } else {
            return Ok(None);
        };
        Ok(Some(Property {
            value,
            writable: false,
            enumerable: false,
            configurable: true,
            accessor: None,
        }))
    }

    pub(super) fn weakmap_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        if builtin != Builtin::WeakMap
            && builtin != Builtin::WeakMapConstruct
            && !METHODS.iter().any(|(_, id, _)| *id == builtin)
        {
            return Ok(None);
        }
        let roots = self.native_roots.len();
        self.native_roots.push(receiver.clone());
        self.native_roots.extend_from_slice(args);
        let result = self.weakmap_rooted(builtin, receiver, args);
        self.native_roots.truncate(roots);
        result.map(Some)
    }

    fn weakmap_rooted(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let first = args.first().unwrap_or(&Value::Undefined);
        let second = args.get(1).unwrap_or(&Value::Undefined);
        if builtin == Builtin::WeakMap {
            return Err(Error::Type {
                message: "WeakMap requires new",
            });
        }
        if builtin == Builtin::WeakMapConstruct {
            return self.construct_weakmap(first);
        }
        self.map_data(receiver)?;
        let Some(key) = Key::of(first) else {
            return match builtin {
                Builtin::WeakMapGet => Ok(Value::Undefined),
                Builtin::WeakMapHas | Builtin::WeakMapDelete => Ok(Value::Boolean(false)),
                _ => Err(Error::Type {
                    message: "invalid WeakMap key",
                }),
            };
        };
        self.charge(1)?;
        if builtin == Builtin::WeakMapGetOrInsertComputed && !matches!(second, Value::Function(_)) {
            return Err(Error::Type {
                message: "WeakMap callback must be callable",
            });
        }
        let old = self.map_data(receiver)?.entries.get(&key).cloned();
        match builtin {
            Builtin::WeakMapGet => Ok(old.unwrap_or(Value::Undefined)),
            Builtin::WeakMapHas => Ok(Value::Boolean(old.is_some())),
            Builtin::WeakMapDelete => {
                let removed = self
                    .object_mut(receiver)?
                    .weakmap
                    .as_mut()
                    .ok_or(Error::InvalidBytecode)?
                    .entries
                    .remove(&key)
                    .is_some();
                self.weak_entries = self.weak_entries.saturating_sub(usize::from(removed));
                Ok(Value::Boolean(removed))
            }
            Builtin::WeakMapSet => {
                self.map_insert(receiver, key, second.clone())?;
                Ok(receiver.clone())
            }
            Builtin::WeakMapGetOrInsert | Builtin::WeakMapGetOrInsertComputed => {
                if let Some(old) = old {
                    return Ok(old);
                }
                let value = if builtin == Builtin::WeakMapGetOrInsertComputed {
                    self.call_sync(
                        second.clone(),
                        Value::Undefined,
                        core::slice::from_ref(first),
                    )?
                } else {
                    second.clone()
                };
                self.native_roots.push(value.clone());
                self.map_insert(receiver, key, value.clone())?;
                Ok(value)
            }
            _ => Err(Error::InvalidBytecode),
        }
    }

    fn map_data(&self, value: &Value) -> Result<&WeakMap, Error> {
        self.object_ref(value)?.weakmap.as_ref().ok_or(Error::Type {
            message: "WeakMap receiver required",
        })
    }

    fn map_insert(&mut self, map: &Value, key: Key, value: Value) -> Result<(), Error> {
        let exists = self.map_data(map)?.entries.contains_key(&key);
        if !exists && self.weak_entries >= self.limits.weak_entries {
            self.collect_garbage()?;
        }
        if !exists && self.weak_entries >= self.limits.weak_entries {
            return Err(Error::Limit {
                resource: "WeakMap entries",
            });
        }
        if !exists {
            self.weak_entries = self.weak_entries.saturating_add(1);
        }
        self.object_mut(map)?
            .weakmap
            .as_mut()
            .ok_or(Error::InvalidBytecode)?
            .entries
            .insert(key, value);
        Ok(())
    }

    fn construct_weakmap(&mut self, input: &Value) -> Result<Value, Error> {
        let proto = self.weakmap_prototype()?;
        self.construct_weakmap_with_proto(input, proto)
    }
    pub(super) fn construct_weakmap_with_proto(
        &mut self,
        input: &Value,
        proto: Value,
    ) -> Result<Value, Error> {
        let map = self.allocate_object(proto)?;
        self.object_mut(&map)?.weakmap = Some(WeakMap::default());
        self.native_roots.push(map.clone());
        if matches!(input, Value::Null | Value::Undefined) {
            return Ok(map);
        }
        let adder = self.get(&map, &Value::string("set").units())?;
        self.native_roots.push(adder.clone());
        if !matches!(adder, Value::Function(_)) {
            return Err(Error::Type {
                message: "WeakMap set is not callable",
            });
        }
        let mut iterator = self.get_iterator(input)?;
        self.native_roots
            .extend([iterator.object.clone(), iterator.next.clone()]);
        while let Some(entry) = self.iterator_step(&mut iterator)? {
            let roots = self.native_roots.len();
            self.native_roots.push(entry.clone());
            let result = (|| {
                if !matches!(entry, Value::Object(_) | Value::Function(_)) {
                    return Err(Error::Type {
                        message: "WeakMap entry is not an object",
                    });
                }
                let key = self.get(&entry, &Value::string("0").units())?;
                self.native_roots.push(key.clone());
                let value = self.get(&entry, &Value::string("1").units())?;
                self.call_sync(adder.clone(), map.clone(), &[key, value])?;
                Ok(())
            })();
            if let Err(error) = result {
                if !super::iterators::language_error(&error) {
                    return Err(error);
                }
                if let Error::Thrown { value } = &error {
                    self.native_roots.push(value.clone());
                }
                self.iterator_close(&mut iterator, true)?;
                return Err(error);
            }
            self.native_roots.truncate(roots);
        }
        Ok(map)
    }
}

const fn native(builtin: Builtin) -> Value {
    Value::Function(FunctionValue::native(builtin))
}
