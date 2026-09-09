// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Array.from's iterable/array-like branches share constructor and iterator
//! machinery. Only mapper/definition errors close an acquired iterator.
use super::{
    Error, Execution, Property, Value,
    indexing::{index_number, wide_key},
    length_key,
};
use crate::vm::iterators::language_error;

impl Execution<'_> {
    pub(super) fn array_from(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let items = args.first().unwrap_or(&Value::Undefined);
        let mapper = args.get(1).unwrap_or(&Value::Undefined);
        if !matches!(mapper, Value::Undefined | Value::Function(_)) {
            return Err(Error::Type {
                message: "Array.from mapper is not callable",
            });
        }
        let this = args.get(2).unwrap_or(&Value::Undefined);
        let key = Value::Symbol(self.well_known("iterator")?);
        let method = self.get_key(items, &key)?;
        if !matches!(method, Value::Undefined | Value::Null) {
            if !matches!(method, Value::Function(_)) {
                return Err(Error::Type {
                    message: "Array.from iterator method is not callable",
                });
            }
            self.native_roots.push(method.clone());
            let array = self.array_from_create(receiver, None)?;
            self.native_roots.push(array.clone());
            let mut iterator = self.get_iterator_from(items, method)?;
            self.native_roots
                .extend([iterator.object.clone(), iterator.next.clone()]);
            let mut index = 0u64;
            loop {
                self.charge(1)?;
                if index >= 9_007_199_254_740_991 {
                    self.iterator_close(&mut iterator, true)?;
                    return Err(Error::Type {
                        message: "Array.from exceeds maximum length",
                    });
                }
                let Some(value) = self.iterator_step(&mut iterator)? else {
                    self.set(
                        &array,
                        length_key(),
                        Value::Number(index_number(index)),
                        true,
                    )?;
                    return Ok(array);
                };
                let result = self.array_from_element(&array, index, value, mapper, this);
                if let Err(error) = result {
                    if !language_error(&error) {
                        return Err(error);
                    }
                    // The thrown object may be held only in this completion
                    // while a return getter/callback triggers garbage collection.
                    if let Error::Thrown { value } = &error {
                        self.native_roots.push(value.clone());
                    }
                    self.iterator_close(&mut iterator, true)?;
                    return Err(error);
                }
                index = index.saturating_add(1);
            }
        }
        let object = self.box_value(items)?;
        self.native_roots.push(object.clone());
        let length = self.array_length_wide(&object)?;
        let array = self.array_from_create(receiver, Some(length))?;
        self.native_roots.push(array.clone());
        for index in 0..length {
            self.charge(1)?;
            // Array.from materializes holes and never calls HasProperty here.
            let value = self.get(&object, &wide_key(index))?;
            self.array_from_element(&array, index, value, mapper, this)?;
        }
        self.set(
            &array,
            length_key(),
            Value::Number(index_number(length)),
            true,
        )?;
        Ok(array)
    }
    fn array_from_create(&mut self, receiver: &Value, length: Option<u64>) -> Result<Value, Error> {
        if self.is_constructor(receiver) {
            match length {
                None => self.construct_sync(receiver.clone(), &[], receiver.clone()),
                Some(length) => self.construct_sync(
                    receiver.clone(),
                    &[Value::Number(index_number(length))],
                    receiver.clone(),
                ),
            }
        } else {
            self.array_create_wide(length.unwrap_or(0))
        }
    }
    fn array_from_element(
        &mut self,
        array: &Value,
        index: u64,
        value: Value,
        mapper: &Value,
        this: &Value,
    ) -> Result<(), Error> {
        let roots = self.native_roots.len();
        self.native_roots.push(value.clone());
        let result = (|| {
            let element = if matches!(mapper, Value::Undefined) {
                value
            } else {
                self.call_sync(
                    mapper.clone(),
                    this.clone(),
                    &[value, Value::Number(index_number(index))],
                )?
            };
            self.define(array, wide_key(index), Property::data(element))
        })();
        self.native_roots.truncate(roots);
        result
    }
}
