// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Nonmutating array copies, with Get only for retained elements. No iterator,
//! species, source writes or `HasProperty`; holes become own undefined entries.
use super::{
    Error, Execution, Property, Value,
    indexing::{index_number, length_index, wide_key},
};
use crate::value::integer_or_infinity;

impl Execution<'_> {
    pub(super) fn array_to_reversed(&mut self, receiver: &Value) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let array = self.array_create_wide(length)?;
        self.native_roots.push(array.clone());
        for index in 0..length {
            self.copy_array_element(
                receiver,
                length.saturating_sub(index).saturating_sub(1),
                &array,
                index,
            )?;
        }
        Ok(array)
    }
    pub(super) fn array_with(
        &mut self,
        receiver: &Value,
        index: &Value,
        value: &Value,
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let index = integer_or_infinity(self.numeric(index)?);
        let size = index_number(length);
        let absolute = if index < 0.0 { size + index } else { index };
        if absolute < 0.0 || absolute >= size {
            return Err(Error::Range {
                message: "array replacement index out of bounds",
            });
        }
        let index = length_index(absolute);
        let array = self.array_create_wide(length)?;
        self.native_roots.push(array.clone());
        for k in 0..length {
            if k == index {
                self.charge(1)?;
                self.define(&array, wide_key(k), Property::data(value.clone()))?;
            } else {
                self.copy_array_element(receiver, k, &array, k)?;
            }
        }
        Ok(array)
    }
    pub(super) fn array_to_spliced(
        &mut self,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let start = self.array_clamped_index(args.first().unwrap_or(&Value::Undefined), length)?;
        let remaining = length.saturating_sub(start);
        let skipped = match args.get(1) {
            Some(skip) => length_index(
                integer_or_infinity(self.numeric(skip)?).clamp(0.0, index_number(remaining)),
            ),
            None if args.is_empty() => 0,
            None => remaining,
        };
        let items = args.get(2..).unwrap_or(&[]);
        let inserted = u64::try_from(items.len()).map_err(|_| Error::Type {
            message: "toSpliced exceeds maximum array-like length",
        })?;
        let new_length = length
            .saturating_sub(skipped)
            .checked_add(inserted)
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or(Error::Type {
                message: "toSpliced exceeds maximum array-like length",
            })?;
        let array = self.array_create_wide(new_length)?;
        self.native_roots.push(array.clone());
        let mut write = 0u64;
        while write < start {
            self.copy_array_element(receiver, write, &array, write)?;
            write = write.saturating_add(1);
        }
        for item in items {
            self.charge(1)?;
            self.define(&array, wide_key(write), Property::data(item.clone()))?;
            write = write.saturating_add(1);
        }
        let mut read = start.saturating_add(skipped);
        while write < new_length {
            self.copy_array_element(receiver, read, &array, write)?;
            read = read.saturating_add(1);
            write = write.saturating_add(1);
        }
        Ok(array)
    }
    fn copy_array_element(
        &mut self,
        source: &Value,
        from: u64,
        target: &Value,
        to: u64,
    ) -> Result<(), Error> {
        self.charge(1)?;
        let value = self.get(source, &wide_key(from))?;
        // Define data does not invoke inherited setters or allocate a heap node.
        self.define(target, wide_key(to), Property::data(value))
    }
}
