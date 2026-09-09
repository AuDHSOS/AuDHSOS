// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Generic in-place fill/copyWithin. Neither operation changes length directly,
//! invokes species nor skips observable setters. Overlapping copies run backwards.
use super::{Error, Execution, Value, indexing::wide_key};

impl Execution<'_> {
    pub(super) fn array_fill(&mut self, receiver: &Value, args: &[Value]) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let start = self.array_clamped_index(args.get(1).unwrap_or(&Value::Undefined), length)?;
        let end = self.array_optional_end(args.get(2), length)?;
        let value = args.first().unwrap_or(&Value::Undefined);
        for index in start..end {
            self.charge(1)?;
            self.set(receiver, wide_key(index), value.clone(), true)?;
        }
        Ok(receiver.clone())
    }
    pub(super) fn array_copy_within(
        &mut self,
        receiver: &Value,
        args: &[Value],
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let mut to = self.array_clamped_index(args.first().unwrap_or(&Value::Undefined), length)?;
        let mut from =
            self.array_clamped_index(args.get(1).unwrap_or(&Value::Undefined), length)?;
        let end = self.array_optional_end(args.get(2), length)?;
        let count = end.saturating_sub(from).min(length.saturating_sub(to));
        let backward = from < to && to < from.saturating_add(count);
        if backward {
            from = from.saturating_add(count).saturating_sub(1);
            to = to.saturating_add(count).saturating_sub(1);
        }
        for _ in 0..count {
            self.charge(1)?;
            let source = wide_key(from);
            let target = wide_key(to);
            if self.present(receiver, &source)? {
                let value = self.get(receiver, &source)?;
                // Set roots receiver/value before an inherited setter or GC.
                self.set(receiver, target, value, true)?;
            } else {
                self.array_delete_or_throw(receiver, &target)?;
            }
            if backward {
                from = from.saturating_sub(1);
                to = to.saturating_sub(1);
            } else {
                from = from.saturating_add(1);
                to = to.saturating_add(1);
            }
        }
        Ok(receiver.clone())
    }
    fn array_optional_end(&mut self, end: Option<&Value>, length: u64) -> Result<u64, Error> {
        match end {
            None | Some(Value::Undefined) => Ok(length),
            Some(end) => self.array_clamped_index(end, length),
        }
    }
}
