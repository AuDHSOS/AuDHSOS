// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Generic splice with ordered property operations and bounded 53-bit indices.
//! Only the species result is allocated; moves preserve holes without snapshots.
use super::{
    Error, Execution, Property, Value,
    indexing::{index_number, length_index, wide_key},
    length_key,
};
use crate::value::integer_or_infinity;

impl Execution<'_> {
    pub(super) fn array_splice(
        &mut self,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let start = self.array_clamped_index(args.first().unwrap_or(&Value::Undefined), length)?;
        let remaining = length.saturating_sub(start);
        let deleted = match args.get(1) {
            Some(count) => length_index(
                integer_or_infinity(self.numeric(count)?).clamp(0.0, index_number(remaining)),
            ),
            None if args.is_empty() => 0,
            None => remaining,
        };
        let items = args.get(2..).unwrap_or(&[]);
        let inserted = u64::try_from(items.len()).map_err(|_| Error::Type {
            message: "splice exceeds maximum array-like length",
        })?;
        let new_length = length
            .saturating_sub(deleted)
            .checked_add(inserted)
            .filter(|n| *n <= 9_007_199_254_740_991)
            .ok_or(Error::Type {
                message: "splice exceeds maximum array-like length",
            })?;
        let result = self.array_species_create(receiver, deleted)?;
        self.native_roots.push(result.clone());
        for k in 0..deleted {
            self.charge(1)?;
            let source = wide_key(start.saturating_add(k));
            if self.present(receiver, &source)? {
                let value = self.get(receiver, &source)?;
                self.define(&result, wide_key(k), Property::data(value))?;
            }
        }
        self.set(
            &result,
            length_key(),
            Value::Number(index_number(deleted)),
            true,
        )?;
        if inserted < deleted {
            for k in start..length.saturating_sub(deleted) {
                self.splice_move(
                    receiver,
                    k.saturating_add(deleted),
                    k.saturating_add(inserted),
                )?;
            }
            for k in (new_length..length).rev() {
                self.charge(1)?;
                self.array_delete_or_throw(receiver, &wide_key(k))?;
            }
        } else if inserted > deleted {
            for k in (start..length.saturating_sub(deleted)).rev() {
                self.splice_move(
                    receiver,
                    k.saturating_add(deleted),
                    k.saturating_add(inserted),
                )?;
            }
        }
        let mut k = start;
        for item in items {
            self.charge(1)?;
            self.set(receiver, wide_key(k), item.clone(), true)?;
            k = k.saturating_add(1);
        }
        self.set(
            receiver,
            length_key(),
            Value::Number(index_number(new_length)),
            true,
        )?;
        Ok(result)
    }
    fn splice_move(&mut self, receiver: &Value, from: u64, to: u64) -> Result<(), Error> {
        self.charge(1)?;
        let source = wide_key(from);
        let target = wide_key(to);
        if self.present(receiver, &source)? {
            let value = self.get(receiver, &source)?;
            self.set(receiver, target, value, true)
        } else {
            self.array_delete_or_throw(receiver, &target)
        }
    }
}
