// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ECMA-262 `SortIndexedProperties`: collect first, stable bounded merge passes,
//! then write back and delete holes. Comparators may throw, mutate and collect.

use super::{Error, Execution, Property, Value, indexing::wide_key};
use alloc::vec::Vec;
use core::cmp::Ordering;

impl Execution<'_> {
    pub(super) fn array_sort(
        &mut self,
        receiver: &Value,
        comparator: &Value,
        copying: bool,
    ) -> Result<Value, Error> {
        if !matches!(comparator, Value::Undefined | Value::Function(_)) {
            return Err(Error::Type {
                message: "sort comparator must be callable or undefined",
            });
        }
        let length = self.array_length_wide(receiver)?;
        // toSorted always uses ArrayCreate, not a public constructor or species.
        // Its Array length RangeError precedes all indexed reads/comparisons.
        let target = if copying {
            let array = self.array_create_wide(length)?;
            self.native_roots.push(array.clone());
            array
        } else {
            receiver.clone()
        };
        let items = self.sorted_properties(receiver, length, comparator, copying)?;
        for (index, item) in items.iter().enumerate() {
            self.charge(1)?;
            let index = u64::try_from(index).map_err(|_| Error::InvalidBytecode)?;
            if copying {
                self.define(&target, wide_key(index), Property::data(item.clone()))?;
            } else {
                self.set(&target, wide_key(index), item.clone(), true)?;
            }
        }
        if !copying {
            let count = u64::try_from(items.len()).map_err(|_| Error::InvalidBytecode)?;
            for index in count..length {
                self.charge(1)?;
                self.array_delete_or_throw(receiver, &wide_key(index))?;
            }
        }
        Ok(target)
    }

    fn sorted_properties(
        &mut self,
        receiver: &Value,
        length: u64,
        comparator: &Value,
        read_holes: bool,
    ) -> Result<Vec<Value>, Error> {
        let mut items = Vec::new();
        for index in 0..length {
            self.charge(1)?;
            let key = wide_key(index);
            if read_holes || self.present(receiver, &key)? {
                if items.len() >= self.limits.properties {
                    return Err(Error::Limit {
                        resource: "sort items",
                    });
                }
                let item = self.get(receiver, &key)?;
                // Sorting callbacks can delete every property of the receiver.
                self.native_roots.push(item.clone());
                items.push(item);
            }
        }
        self.sort_items(&mut items, comparator)?;
        Ok(items)
    }

    fn sort_items(&mut self, items: &mut Vec<Value>, comparator: &Value) -> Result<(), Error> {
        let mut scratch = Vec::with_capacity(items.len());
        let mut width = 1usize;
        while width < items.len() {
            scratch.clear();
            for start in (0..items.len()).step_by(width.saturating_mul(2)) {
                let mid = start.saturating_add(width).min(items.len());
                let end = mid.saturating_add(width).min(items.len());
                let mut left = start;
                let mut right = mid;
                while left < mid || right < end {
                    self.charge(1)?;
                    let take_left = right == end
                        || (left < mid
                            && self.compare_elements(
                                items.get(left).ok_or(Error::InvalidBytecode)?,
                                items.get(right).ok_or(Error::InvalidBytecode)?,
                                comparator,
                            )? != Ordering::Greater);
                    let index = if take_left { &mut left } else { &mut right };
                    scratch.push(items.get(*index).ok_or(Error::InvalidBytecode)?.clone());
                    *index = index.saturating_add(1);
                }
            }
            core::mem::swap(items, &mut scratch);
            width = width.saturating_mul(2);
        }
        Ok(())
    }

    fn compare_elements(
        &mut self,
        left: &Value,
        right: &Value,
        comparator: &Value,
    ) -> Result<Ordering, Error> {
        if matches!(left, Value::Undefined) {
            return Ok(if matches!(right, Value::Undefined) {
                Ordering::Equal
            } else {
                Ordering::Greater
            });
        }
        if matches!(right, Value::Undefined) {
            return Ok(Ordering::Less);
        }
        if !matches!(comparator, Value::Undefined) {
            let value = self.call_sync(
                comparator.clone(),
                Value::Undefined,
                &[left.clone(), right.clone()],
            )?;
            return Ok(self
                .numeric(&value)?
                .partial_cmp(&0.0)
                .unwrap_or(Ordering::Equal));
        }
        let left = self.string_units(left)?;
        let right = self.string_units(right)?;
        self.charge(u64::try_from(left.len().min(right.len())).unwrap_or(u64::MAX))?;
        Ok(left.cmp(&right))
    }
}
