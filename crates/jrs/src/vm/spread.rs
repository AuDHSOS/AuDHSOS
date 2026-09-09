// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Iterator expansion into the operand stack or fresh literal array. Argument
//! counters are bytecode-private stack values, so await and exceptions preserve
//! or discard them with the surrounding operands. No mutable JS array is used
//! as an argument-list buffer and no public iterator/push hooks are bypassed.

use super::Execution;
use crate::{
    Error, Value,
    object::{Property, length_key},
};

impl Execution<'_> {
    pub(super) fn expanded_count(&mut self) -> Result<usize, Error> {
        let value = self.pop()?;
        let Value::Number(n) = value else {
            return Err(Error::InvalidBytecode);
        };
        let count = usize::try_from(value.to_uint32()).map_err(|_| Error::InvalidBytecode)?;
        if !n.is_finite()
            || n < 0.0
            || count > self.limits.stack
            || !value.strictly_equals(&count_value(count)?)
        {
            return Err(Error::InvalidBytecode);
        }
        Ok(count)
    }
    pub(super) fn append_arguments(&mut self, spread: bool) -> Result<(), Error> {
        let value = self.pop()?;
        let mut count = self.expanded_count()?;
        if spread {
            let mut iterator = self.get_iterator(&value)?;
            while let Some(item) = self.iterator_step(&mut iterator)? {
                self.append_argument(item)?;
                count = count.saturating_add(1);
            }
        } else {
            self.append_argument(value)?;
            count = count.saturating_add(1);
        }
        self.push(count_value(count)?)
    }
    fn append_argument(&mut self, value: Value) -> Result<(), Error> {
        // Reserve the private counter as well as the element. Existing suspended
        // operands count against the same limit; exhaustion is not catchable.
        if self
            .stack
            .len()
            .saturating_add(self.suspended_slots)
            .saturating_add(1)
            >= self.limits.stack
        {
            return Err(Error::Limit {
                resource: "spread arguments",
            });
        }
        self.push(value)
    }
    pub(super) fn append_array(&mut self, spread: bool) -> Result<(), Error> {
        let value = self.pop()?;
        let array = self.stack.last().cloned().ok_or(Error::InvalidBytecode)?;
        if spread {
            let mut iterator = self.get_iterator(&value)?;
            while let Some(item) = self.iterator_step(&mut iterator)? {
                self.append_element(&array, item)?;
            }
        } else {
            self.append_element(&array, value)?;
        }
        Ok(())
    }
    fn append_element(&mut self, array: &Value, value: Value) -> Result<(), Error> {
        let index = self
            .object_ref(array)?
            .properties
            .get(length_key().as_ref())
            .ok_or(Error::InvalidBytecode)?
            .value
            .to_uint32();
        if index == u32::MAX {
            return Err(Error::Limit {
                resource: "array spread length",
            });
        }
        self.define(
            array,
            Value::string(&alloc::format!("{index}")).units(),
            Property::data(value),
        )
    }
    pub(super) fn array_hole(&mut self) -> Result<(), Error> {
        let array = self.stack.last().cloned().ok_or(Error::InvalidBytecode)?;
        let property = self
            .object_mut(&array)?
            .properties
            .get_mut(length_key().as_ref())
            .ok_or(Error::InvalidBytecode)?;
        let length = property
            .value
            .to_uint32()
            .checked_add(1)
            .ok_or(Error::Limit {
                resource: "array literal length",
            })?;
        property.value = Value::Number(f64::from(length));
        Ok(())
    }
}
fn count_value(count: usize) -> Result<Value, Error> {
    Ok(Value::Number(f64::from(u32::try_from(count).map_err(
        |_| Error::Limit {
            resource: "argument count",
        },
    )?)))
}
