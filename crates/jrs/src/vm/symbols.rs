// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Symbol identities, symbol-keyed properties, and well-known protocol hooks.

use super::Execution;
use crate::{
    Error, SymbolValue, Value, bytecode::Builtin, heap::Node, object::Property,
    value::FunctionValue,
};
use alloc::{rc::Rc, vec::Vec};

const WELL_KNOWN: &[&str] = &[
    "asyncIterator",
    "hasInstance",
    "isConcatSpreadable",
    "iterator",
    "match",
    "matchAll",
    "replace",
    "search",
    "species",
    "split",
    "toPrimitive",
    "toStringTag",
    "unscopables",
    "dispose",
    "asyncDispose",
];

impl Execution<'_> {
    pub(super) fn tagged_object_string(&mut self, value: &Value) -> Result<Value, Error> {
        if matches!(value, Value::Null | Value::Undefined) {
            return Ok(self.object_tag(value));
        }
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let fallback = self.object_tag(value);
            let object = self.box_value(value)?;
            self.native_roots.push(object.clone());
            let key = Value::Symbol(self.well_known("toStringTag")?);
            let tag = self.get_key(&object, &key)?;
            let Value::String(tag) = tag else {
                return Ok(fallback);
            };
            if tag.len().saturating_add(9) > self.limits.string_units {
                return Err(Error::Limit {
                    resource: "string units",
                });
            }
            let mut text = Value::string("[object ").units().to_vec();
            text.extend_from_slice(&tag);
            text.push(93);
            Ok(Value::String(text.into()))
        })();
        self.native_roots.truncate(roots);
        result
    }
    fn new_symbol(
        &mut self,
        description: Option<Rc<[u16]>>,
        registered: bool,
    ) -> Result<SymbolValue, Error> {
        self.reserve()?;
        let handle = self.heap.allocate(Node::Symbol, self.limits.heap_entries)?;
        Ok(SymbolValue {
            handle,
            owner: self.owner.clone(),
            description,
            registered,
        })
    }
    pub(super) fn well_known(&mut self, name: &'static str) -> Result<SymbolValue, Error> {
        if let Some(symbol) = self.well_known.get(name) {
            return Ok(symbol.clone());
        }
        let symbol = self.new_symbol(
            Some(Value::string(&alloc::format!("Symbol.{name}")).units()),
            false,
        )?;
        self.well_known.insert(name, symbol.clone());
        Ok(symbol)
    }
    pub(super) fn symbol_prototype(&mut self) -> Result<Value, Error> {
        if let Some(value) = &self.symbol_proto {
            return Ok(value.clone());
        }
        let proto = self.prototype()?;
        let value = self.allocate_object(proto)?;
        self.symbol_proto = Some(value.clone());
        for (key, builtin) in [
            ("constructor", Builtin::Symbol),
            ("toString", Builtin::SymbolToString),
            ("valueOf", Builtin::SymbolValueOf),
        ] {
            self.define(
                &value,
                Value::string(key).units(),
                Property {
                    enumerable: false,
                    ..Property::data(native(builtin))
                },
            )?;
        }
        self.define(
            &value,
            Value::string("description").units(),
            Property {
                value: Value::Undefined,
                writable: false,
                enumerable: false,
                configurable: true,
                accessor: Some((native(Builtin::SymbolDescription), Value::Undefined)),
            },
        )?;
        let key = Value::Symbol(self.well_known("toPrimitive")?);
        self.define_key(
            &value,
            key,
            Property {
                writable: false,
                enumerable: false,
                ..Property::data(native(Builtin::SymbolToPrimitive))
            },
        )?;
        self.install_tag(&value, "Symbol")?;
        Ok(value)
    }
    pub(super) fn install_tag(&mut self, object: &Value, tag: &str) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(object.clone());
        let result = (|| {
            let key = Value::Symbol(self.well_known("toStringTag")?);
            self.define_key(
                object,
                key,
                Property {
                    writable: false,
                    enumerable: false,
                    ..Property::data(Value::string(tag))
                },
            )
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn symbol_property(
        &mut self,
        builtin: Builtin,
        key: &[u16],
    ) -> Result<Option<Property>, Error> {
        if builtin != Builtin::Symbol {
            return Ok(None);
        }
        let mut readonly = true;
        let mut configurable = false;
        let value = if key == Value::string("prototype").units().as_ref() {
            self.symbol_prototype()?
        } else if key == Value::string("for").units().as_ref() {
            readonly = false;
            configurable = true;
            native(Builtin::SymbolFor)
        } else if key == Value::string("keyFor").units().as_ref() {
            readonly = false;
            configurable = true;
            native(Builtin::SymbolKeyFor)
        } else if key == Value::string("name").units().as_ref() {
            configurable = true;
            Value::string("Symbol")
        } else if key == Value::string("length").units().as_ref() {
            configurable = true;
            Value::Number(0.0)
        } else if let Some(name) = WELL_KNOWN
            .iter()
            .find(|name| key == Value::string(name).units().as_ref())
        {
            Value::Symbol(self.well_known(name)?)
        } else {
            return Ok(None);
        };
        Ok(Some(Property {
            value,
            writable: !readonly,
            enumerable: false,
            configurable,
            accessor: None,
        }))
    }
    pub(super) fn symbol_call(
        &mut self,
        builtin: Builtin,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Option<Value>, Error> {
        let first = args.first().unwrap_or(&Value::Undefined);
        match builtin {
            Builtin::Symbol => {
                let description = if matches!(first, Value::Undefined) {
                    None
                } else {
                    Some(self.string_units(first)?)
                };
                Ok(Some(Value::Symbol(self.new_symbol(description, false)?)))
            }
            Builtin::SymbolFor => {
                let key = self.string_units(first)?;
                if let Some(value) = self.symbol_registry.get(&key) {
                    return Ok(Some(Value::Symbol(value.clone())));
                }
                if self.symbol_registry.len() >= self.limits.properties {
                    return Err(Error::Limit {
                        resource: "symbol registry",
                    });
                }
                let symbol = self.new_symbol(Some(key.clone()), true)?;
                self.symbol_registry.insert(key, symbol.clone());
                Ok(Some(Value::Symbol(symbol)))
            }
            Builtin::SymbolKeyFor => {
                let Value::Symbol(symbol) = first else {
                    return Err(Error::Type {
                        message: "Symbol.keyFor requires a symbol",
                    });
                };
                Ok(Some(if symbol.registered {
                    symbol
                        .description
                        .clone()
                        .map_or(Value::Undefined, Value::String)
                } else {
                    Value::Undefined
                }))
            }
            Builtin::String if matches!(first, Value::Symbol(_)) => {
                let Value::Symbol(symbol) = first else {
                    return Err(Error::InvalidBytecode);
                };
                self.symbol_text(symbol).map(Some)
            }
            Builtin::SymbolValueOf
            | Builtin::SymbolToPrimitive
            | Builtin::SymbolToString
            | Builtin::SymbolDescription => {
                let symbol = if let Value::Symbol(symbol) = receiver {
                    symbol.clone()
                } else {
                    let Some(Value::Symbol(symbol)) = self
                        .object_ref(receiver)
                        .ok()
                        .and_then(|o| o.primitive.as_ref())
                    else {
                        return Err(Error::Type {
                            message: "Symbol receiver required",
                        });
                    };
                    symbol.clone()
                };
                Ok(Some(match builtin {
                    Builtin::SymbolToString => self.symbol_text(&symbol)?,
                    Builtin::SymbolDescription => {
                        symbol.description.map_or(Value::Undefined, Value::String)
                    }
                    _ => Value::Symbol(symbol),
                }))
            }
            Builtin::GetOwnPropertySymbols => {
                let object = self.box_value(first)?;
                let roots = self.native_roots.len();
                self.native_roots.push(object.clone());
                let result = (|| {
                    let keys = self.symbol_keys(&object)?;
                    self.array_from_values(&keys)
                })();
                self.native_roots.truncate(roots);
                result.map(Some)
            }
            _ => Ok(None),
        }
    }
    fn symbol_text(&self, symbol: &SymbolValue) -> Result<Value, Error> {
        if symbol
            .description
            .as_ref()
            .map_or(0, |s| s.len())
            .saturating_add(8)
            > self.limits.string_units
        {
            return Err(Error::Limit {
                resource: "string units",
            });
        }
        Ok(Value::String(symbol.descriptive()))
    }
    pub(super) fn property_key(&mut self, value: &Value) -> Result<Value, Error> {
        let primitive = self.primitive(value, true)?;
        Ok(if matches!(primitive, Value::Symbol(_)) {
            primitive
        } else {
            Value::String(primitive.units())
        })
    }
    pub(super) fn symbol_keys(&mut self, object: &Value) -> Result<Vec<Value>, Error> {
        if super::properties::has_constructor_storage(object) {
            self.constructor_storage(object)?;
        }
        let object = self.object_ref(object)?;
        object
            .symbol_order
            .iter()
            .map(|h| {
                object
                    .symbols
                    .get(h)
                    .map(|(s, _)| Value::Symbol(s.clone()))
                    .ok_or(Error::InvalidBytecode)
            })
            .collect()
    }
    pub(super) fn own_key(
        &mut self,
        object: &Value,
        key: &Value,
    ) -> Result<Option<Property>, Error> {
        if super::properties::has_constructor_storage(object) {
            self.constructor_storage(object)?;
        }
        if let Value::Symbol(symbol) = key {
            if matches!(
                object,
                Value::Function(FunctionValue(crate::value::Callable::Native(
                    Builtin::Promise
                )))
            ) && *symbol == self.well_known("species")?
            {
                return Ok(Some(Property {
                    value: Value::Undefined,
                    writable: false,
                    enumerable: false,
                    configurable: true,
                    accessor: Some((native(Builtin::PromiseSpecies), Value::Undefined)),
                }));
            }
            if matches!(object, Value::Null | Value::Undefined) {
                return Err(Error::Type {
                    message: "property access on nullish value",
                });
            }
            Ok(self
                .object_ref(object)
                .ok()
                .and_then(|o| o.symbols.get(&symbol.handle))
                .map(|(_, p)| p.clone()))
        } else {
            self.own(object, &key.units())
        }
    }
    pub(super) fn define_key(
        &mut self,
        object: &Value,
        key: Value,
        property: Property,
    ) -> Result<(), Error> {
        if let Value::Symbol(symbol) = key {
            let limit = self.limits.properties;
            if self
                .object_mut(object)?
                .define_symbol(symbol, property, limit)?
            {
                Ok(())
            } else {
                Err(Error::Type {
                    message: "symbol property definition rejected",
                })
            }
        } else {
            self.define(object, key.units(), property)
        }
    }
    pub(super) fn get_key(&mut self, object: &Value, key: &Value) -> Result<Value, Error> {
        if !matches!(key, Value::Symbol(_)) {
            return self.get(object, &key.units());
        }
        let roots = self.native_roots.len();
        self.native_roots.extend([object.clone(), key.clone()]);
        let result = (|| {
            let mut current = object.clone();
            for _ in 0..=self.limits.heap_entries {
                self.charge(1)?;
                if let Some(property) = self.own_key(&current, key)? {
                    if let Some((get, _)) = property.accessor {
                        return if matches!(get, Value::Undefined) {
                            Ok(Value::Undefined)
                        } else {
                            self.call_sync(get, object.clone(), &[])
                        };
                    }
                    return Ok(property.value);
                }
                current = self.prototype_of(&current)?;
                if matches!(current, Value::Null) {
                    return Ok(Value::Undefined);
                }
            }
            Err(Error::Limit {
                resource: "prototype walk",
            })
        })();
        self.native_roots.truncate(roots);
        result
    }
    pub(super) fn set_key(
        &mut self,
        object: &Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<(), Error> {
        if !matches!(key, Value::Symbol(_)) {
            return self.set(object, key.units(), value, strict);
        }
        let roots = self.native_roots.len();
        self.native_roots
            .extend([object.clone(), key.clone(), value.clone()]);
        let result = self.set_symbol_rooted(object, key, value, strict);
        self.native_roots.truncate(roots);
        result
    }
    fn set_symbol_rooted(
        &mut self,
        object: &Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<(), Error> {
        let mut current = object.clone();
        for _ in 0..=self.limits.heap_entries {
            self.charge(1)?;
            if let Some(property) = self.own_key(&current, &key)? {
                if let Some((_, set)) = property.accessor {
                    if matches!(set, Value::Undefined) {
                        return rejected(strict);
                    }
                    self.call_sync(set, object.clone(), &[value])?;
                    return Ok(());
                }
                if !property.writable {
                    return rejected(strict);
                }
                break;
            }
            current = self.prototype_of(&current)?;
            if matches!(current, Value::Null) {
                break;
            }
        }
        if !matches!(object, Value::Object(_) | Value::Function(_)) {
            return rejected(strict);
        }
        let property = self.own_key(object, &key)?.map_or_else(
            || Property::data(value.clone()),
            |p| Property {
                value: value.clone(),
                ..p
            },
        );
        let Value::Symbol(symbol) = key else {
            return Err(Error::InvalidBytecode);
        };
        let limit = self.limits.properties;
        if self
            .object_mut(object)?
            .define_symbol(symbol, property, limit)?
        {
            Ok(())
        } else {
            rejected(strict)
        }
    }
    pub(super) fn has_key(&mut self, object: &Value, key: &Value) -> Result<bool, Error> {
        if !matches!(key, Value::Symbol(_)) {
            return self.has_property(object, &key.units());
        }
        let roots = self.native_roots.len();
        self.native_roots.extend([object.clone(), key.clone()]);
        let result = self.has_symbol_rooted(object, key);
        self.native_roots.truncate(roots);
        result
    }
    fn has_symbol_rooted(&mut self, object: &Value, key: &Value) -> Result<bool, Error> {
        if !matches!(object, Value::Object(_) | Value::Function(_)) {
            return Err(Error::Type {
                message: "in requires an object",
            });
        }
        let mut current = object.clone();
        for _ in 0..=self.limits.heap_entries {
            self.charge(1)?;
            if self.own_key(&current, key)?.is_some() {
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
}
const fn rejected(strict: bool) -> Result<(), Error> {
    if strict {
        Err(Error::Type {
            message: "symbol property assignment rejected",
        })
    } else {
        Ok(())
    }
}
const fn native(builtin: Builtin) -> Value {
    Value::Function(FunctionValue::native(builtin))
}
