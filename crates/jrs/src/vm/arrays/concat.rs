// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Concat preserves holes and uses species creation and spreadability hooks.

use super::{
    Error, Execution, Property, Value,
    indexing::{index_number, wide_key},
    length_key,
};

impl Execution<'_> {
    pub(super) fn array_concat(
        &mut self,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let result = self.array_species_create(receiver, 0)?;
        self.native_roots.push(result.clone());
        let mut next = 0u64;
        for item in core::iter::once(receiver).chain(args) {
            self.charge(1)?;
            if self.concat_spreadable(item)? {
                let length = self.array_length_wide(item)?;
                let end = next
                    .checked_add(length)
                    .filter(|n| *n <= 9_007_199_254_740_991)
                    .ok_or(Error::Type {
                        message: "concat exceeds maximum array-like length",
                    })?;
                for index in 0..length {
                    self.charge(1)?;
                    if self.present(item, &wide_key(index))? {
                        let value = self.get(item, &wide_key(index))?;
                        self.define(&result, wide_key(next), Property::data(value))?;
                    }
                    next = next.saturating_add(1);
                }
                next = end;
            } else {
                if next == 9_007_199_254_740_991 {
                    return Err(Error::Type {
                        message: "concat exceeds maximum array-like length",
                    });
                }
                self.define(&result, wide_key(next), Property::data(item.clone()))?;
                next = next.saturating_add(1);
            }
        }
        self.set(
            &result,
            length_key(),
            Value::Number(index_number(next)),
            true,
        )?;
        Ok(result)
    }
    fn concat_spreadable(&mut self, item: &Value) -> Result<bool, Error> {
        if !matches!(item, Value::Object(_) | Value::Function(_)) {
            return Ok(false);
        }
        let symbol = Value::Symbol(self.well_known("isConcatSpreadable")?);
        let value = self.get_key(item, &symbol)?;
        Ok(if matches!(value, Value::Undefined) {
            self.is_array(item)
        } else {
            value.to_boolean()
        })
    }
}
