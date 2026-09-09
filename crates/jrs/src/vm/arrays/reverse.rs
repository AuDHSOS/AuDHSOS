// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Reverse and backward strict search on generic array-like objects. Indices
//! cover `ToLength`'s full 53-bit range; observable property operations stay ordered.
use super::{
    Error, Execution, Value,
    indexing::{index_number, length_index, wide_key},
};
use crate::value::integer_or_infinity;

impl Execution<'_> {
    pub(super) fn array_reverse(&mut self, receiver: &Value) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        for lower in 0..length / 2 {
            self.charge(1)?;
            let upper = length.saturating_sub(lower).saturating_sub(1);
            let low = wide_key(lower);
            let high = wide_key(upper);
            let roots = self.native_roots.len();
            let result = (|| {
                let lower_exists = self.present(receiver, &low)?;
                let lower_value = if lower_exists {
                    self.get(receiver, &low)?
                } else {
                    Value::Undefined
                };
                self.native_roots.push(lower_value.clone());
                // The lower getter may have deleted the upper property or changed its prototype.
                let upper_exists = self.present(receiver, &high)?;
                let upper_value = if upper_exists {
                    self.get(receiver, &high)?
                } else {
                    Value::Undefined
                };
                self.native_roots.push(upper_value.clone());
                match (lower_exists, upper_exists) {
                    (true, true) => {
                        self.set(receiver, low, upper_value, true)?;
                        self.set(receiver, high, lower_value, true)?;
                    }
                    (false, true) => {
                        self.set(receiver, low, upper_value, true)?;
                        self.array_delete_or_throw(receiver, &high)?;
                    }
                    (true, false) => {
                        self.array_delete_or_throw(receiver, &low)?;
                        self.set(receiver, high, lower_value, true)?;
                    }
                    (false, false) => {}
                }
                Ok(())
            })();
            self.native_roots.truncate(roots);
            result?;
        }
        Ok(receiver.clone())
    }
    pub(super) fn array_last_index_of(
        &mut self,
        receiver: &Value,
        value: &Value,
        from: Option<&Value>,
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        if length == 0 {
            return Ok(Value::Number(-1.0));
        }
        let last = length.saturating_sub(1);
        let start = if let Some(from) = from {
            let n = integer_or_infinity(self.numeric(from)?);
            if n < 0.0 {
                if n < -index_number(length) {
                    return Ok(Value::Number(-1.0));
                }
                length.saturating_sub(length_index(-n))
            } else if n >= index_number(last) {
                last
            } else {
                length_index(n)
            }
        } else {
            last
        };
        let mut index = start;
        loop {
            self.charge(1)?;
            let key = wide_key(index);
            if self.present(receiver, &key)? && self.get(receiver, &key)?.strictly_equals(value) {
                return Ok(Value::Number(index_number(index)));
            }
            if index == 0 {
                break;
            }
            index = index.saturating_sub(1);
        }
        Ok(Value::Number(-1.0))
    }
}
