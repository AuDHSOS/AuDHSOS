// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ordinary data properties (ECMA-262 10.1). Keys preserve UTF-16; creation
//! order is separate from lookup order. No direct ownership links form cycles.

use crate::{Error, Value};
use alloc::{collections::BTreeMap, rc::Rc, vec::Vec};

#[derive(Clone, Debug)]
pub(crate) struct Property {
    pub(crate) value: Value,
    pub(crate) writable: bool,
    pub(crate) enumerable: bool,
    pub(crate) configurable: bool,
    pub(crate) accessor: Option<(Value, Value)>,
}

impl Property {
    pub(crate) const fn data(value: Value) -> Self {
        Self {
            value,
            writable: true,
            enumerable: true,
            configurable: true,
            accessor: None,
        }
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent intrinsic brands and extensibility are internal slots, not states of one state machine"
)]
pub(crate) struct Object {
    pub(crate) prototype: Value,
    pub(crate) extensible: bool,
    pub(crate) properties: BTreeMap<Rc<[u16]>, Property>,
    order: Vec<Rc<[u16]>>,
    pub(crate) array: bool,
    pub(crate) regexp: Option<Rc<crate::regexp::RegExp>>,
    pub(crate) promise: Option<crate::promise::Promise>,
    pub(crate) error: bool,
    pub(crate) argument_map: BTreeMap<Rc<[u16]>, crate::heap::Handle>,
    pub(crate) arguments: bool,
    pub(crate) primitive: Option<Value>,
    pub(crate) weakmap: Option<crate::weakmap::WeakMap>,
    pub(crate) symbols: BTreeMap<crate::heap::Handle, (crate::SymbolValue, Property)>,
    pub(crate) symbol_order: Vec<crate::heap::Handle>,
    pub(crate) iterator: Option<crate::iterator::State>,
    pub(crate) home: Option<Value>,
    pub(crate) lexical_this_cell: Option<crate::heap::Handle>,
    pub(crate) lexical_new_target: Value,
    pub(crate) lexical_super: Option<Value>,
    pub(crate) dynamic_source: Option<Rc<[u16]>>,
    pub(crate) function_initialized: bool,
    pub(crate) event: Option<crate::event::Event>,
    pub(crate) listeners: Option<audhsos_event_target::Listeners<Value>>,
    pub(crate) dom_exception: Option<crate::event::DomException>,
    pub(crate) raw_json: Option<Rc<[u16]>>,
    pub(crate) abort_signal: Option<crate::event::AbortSignal>,
    pub(crate) controller_signal: Option<Value>,
    pub(crate) listener_signals: BTreeMap<u64, Value>,
}

impl Object {
    pub(crate) const fn new(prototype: Value) -> Self {
        Self {
            prototype,
            extensible: true,
            properties: BTreeMap::new(),
            order: Vec::new(),
            array: false,
            regexp: None,
            promise: None,
            error: false,
            argument_map: BTreeMap::new(),
            arguments: false,
            primitive: None,
            weakmap: None,
            symbols: BTreeMap::new(),
            symbol_order: Vec::new(),
            iterator: None,
            home: None,
            lexical_this_cell: None,
            lexical_new_target: Value::Undefined,
            lexical_super: None,
            dynamic_source: None,
            function_initialized: false,
            event: None,
            listeners: None,
            dom_exception: None,
            raw_json: None,
            abort_signal: None,
            controller_signal: None,
            listener_signals: BTreeMap::new(),
        }
    }

    pub(crate) fn define(
        &mut self,
        key: Rc<[u16]>,
        property: Property,
        limit: usize,
    ) -> Result<bool, Error> {
        if let Some(current) = self.string_property(&key) {
            return Ok(!property.writable
                && !property.configurable
                && property.enumerable
                && property.accessor.is_none()
                && same_value(&current.value, &property.value));
        }
        if self.array {
            if key.as_ref() == length_key().as_ref() {
                return self.array_length(property, limit);
            }
            if let Some(index) = array_index(&key) {
                let length = self
                    .properties
                    .get(length_key().as_ref())
                    .ok_or(Error::InvalidBytecode)?;
                let old = length.value.to_uint32();
                if index >= old && !length.writable {
                    return Ok(false);
                }
                if !self.ordinary_define(key, property, limit)? {
                    return Ok(false);
                }
                if index >= old {
                    self.properties
                        .get_mut(length_key().as_ref())
                        .ok_or(Error::InvalidBytecode)?
                        .value = Value::Number(f64::from(index.saturating_add(1)));
                }
                return Ok(true);
            }
        }
        self.ordinary_define(key, property, limit)
    }

    pub(crate) fn new_array(prototype: Value, length: u32, limit: usize) -> Result<Self, Error> {
        let mut object = Self::new(prototype);
        object.ordinary_define(
            length_key(),
            Property {
                value: Value::Number(f64::from(length)),
                writable: true,
                enumerable: false,
                configurable: false,
                accessor: None,
            },
            limit,
        )?;
        object.array = true;
        Ok(object)
    }

    #[expect(
        clippy::float_cmp,
        reason = "ArraySetLength requires exact equality of ToUint32 and ToNumber"
    )]
    fn array_length(&mut self, mut property: Property, limit: usize) -> Result<bool, Error> {
        if property.accessor.is_some() {
            return Ok(false);
        }
        let number = property.value.to_number();
        let length = property.value.to_uint32();
        if f64::from(length) != number {
            return Err(Error::Range {
                message: "invalid array length",
            });
        }
        property.value = Value::Number(f64::from(length));
        let key = length_key();
        let old = self
            .properties
            .get(key.as_ref())
            .ok_or(Error::InvalidBytecode)?;
        if length >= old.value.to_uint32() {
            return self.ordinary_define(key, property, limit);
        }
        if !old.writable {
            return Ok(false);
        }
        let writable = property.writable;
        property.writable = true;
        if !self.ordinary_define(key.clone(), property, limit)? {
            return Ok(false);
        }
        // Visit only existing own indices, never billions of missing elements.
        let keys = self.keys();
        for index_key in keys.iter().rev() {
            if let Some(index) = array_index(index_key)
                && index >= length
                && !self.delete(index_key)
            {
                let length = self
                    .properties
                    .get_mut(key.as_ref())
                    .ok_or(Error::InvalidBytecode)?;
                length.value = Value::Number(f64::from(index.saturating_add(1)));
                length.writable = writable;
                return Ok(false);
            }
        }
        self.properties
            .get_mut(key.as_ref())
            .ok_or(Error::InvalidBytecode)?
            .writable = writable;
        Ok(true)
    }

    fn ordinary_define(
        &mut self,
        key: Rc<[u16]>,
        property: Property,
        limit: usize,
    ) -> Result<bool, Error> {
        if let Some(current) = self.properties.get(key.as_ref()) {
            if !current.configurable
                && (property.configurable
                    || current.enumerable != property.enumerable
                    || current.accessor.is_some() != property.accessor.is_some()
                    || !accessors_equal(current.accessor.as_ref(), property.accessor.as_ref())
                    || (current.accessor.is_none()
                        && !current.writable
                        && (property.writable || !same_value(&current.value, &property.value))))
            {
                return Ok(false);
            }
        } else {
            if !self.extensible {
                return Ok(false);
            }
            if self.properties.len().saturating_add(self.symbols.len()) >= limit {
                return Err(Error::Limit {
                    resource: "object properties",
                });
            }
            self.order.push(key.clone());
        }
        self.properties.insert(key, property);
        Ok(true)
    }

    pub(crate) fn delete(&mut self, key: &[u16]) -> bool {
        if self.string_property(key).is_some() {
            return false;
        }
        if self.properties.get(key).is_some_and(|p| !p.configurable) {
            return false;
        }
        if self.properties.remove(key).is_some() {
            self.order.retain(|k| k.as_ref() != key);
            self.argument_map.remove(key);
        }
        true
    }

    pub(crate) fn keys(&self) -> Vec<Rc<[u16]>> {
        let mut indices: Vec<_> = self
            .order
            .iter()
            .filter_map(|key| array_index(key).map(|index| (index, key.clone())))
            .collect();
        indices.sort_unstable_by_key(|(index, _)| *index);
        let mut keys: Vec<_> = indices.into_iter().map(|(_, key)| key).collect();
        keys.extend(
            self.order
                .iter()
                .filter(|key| array_index(key).is_none())
                .cloned(),
        );
        keys
    }

    pub(crate) fn string_property(&self, key: &[u16]) -> Option<Property> {
        let Value::String(units) = self.primitive.as_ref()? else {
            return None;
        };
        string_index(units, key)
    }

    pub(crate) fn define_symbol(
        &mut self,
        key: crate::SymbolValue,
        property: Property,
        limit: usize,
    ) -> Result<bool, Error> {
        if let Some((_, current)) = self.symbols.get(&key.handle) {
            if !current.configurable
                && (property.configurable
                    || current.enumerable != property.enumerable
                    || current.accessor.is_some() != property.accessor.is_some()
                    || !accessors_equal(current.accessor.as_ref(), property.accessor.as_ref())
                    || (current.accessor.is_none()
                        && !current.writable
                        && (property.writable || !same_value(&current.value, &property.value))))
            {
                return Ok(false);
            }
        } else {
            if !self.extensible {
                return Ok(false);
            }
            if self.properties.len().saturating_add(self.symbols.len()) >= limit {
                return Err(Error::Limit {
                    resource: "object properties",
                });
            }
            self.symbol_order.push(key.handle);
        }
        self.symbols.insert(key.handle, (key, property));
        Ok(true)
    }

    pub(crate) fn delete_symbol(&mut self, key: &crate::SymbolValue) -> bool {
        if self
            .symbols
            .get(&key.handle)
            .is_some_and(|(_, p)| !p.configurable)
        {
            return false;
        }
        self.symbols.remove(&key.handle);
        self.symbol_order.retain(|h| *h != key.handle);
        true
    }
}

pub(crate) fn string_index(units: &[u16], key: &[u16]) -> Option<Property> {
    // String lengths are bounded below 2^32-1. Every in-range canonical numeric
    // string is thus an array index; -0 and 01 remain ordinary properties.
    let index = usize::try_from(array_index(key)?).ok()?;
    let unit = *units.get(index)?;
    Some(Property {
        value: Value::String(Rc::from([unit])),
        writable: false,
        enumerable: true,
        configurable: false,
        accessor: None,
    })
}

pub(crate) fn length_key() -> Rc<[u16]> {
    Value::string("length").units()
}

pub(crate) fn array_index(key: &[u16]) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key.first() == Some(&48)) {
        return None;
    }
    key.iter()
        .try_fold(0u32, |n, unit| {
            let digit = unit.checked_sub(48).filter(|d| *d < 10)?;
            n.checked_mul(10)?.checked_add(u32::from(digit))
        })
        .filter(|n| *n != u32::MAX)
}

pub(crate) fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits()
        }
        _ => a.strictly_equals(b),
    }
}

fn accessors_equal(a: Option<&(Value, Value)>, b: Option<&(Value, Value)>) -> bool {
    match (a, b) {
        (Some((get_a, set_a)), Some((get_b, set_b))) => {
            same_value(get_a, get_b) && same_value(set_a, set_b)
        }
        (None, None) => true,
        _ => false,
    }
}
