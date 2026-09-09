// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Shared 53-bit array-like length/index conversion for generic algorithms.
use super::{Error, Execution, Value, length_key};
use crate::value::integer_or_infinity;
use alloc::rc::Rc;

impl Execution<'_> {
    pub(in crate::vm) fn array_length_wide(&mut self, receiver: &Value) -> Result<u64, Error> {
        let length = self.get(receiver, &length_key())?;
        let n = integer_or_infinity(self.numeric(&length)?).clamp(0.0, 9_007_199_254_740_991.0);
        Ok(length_index(n))
    }
    pub(super) fn array_clamped_index(&mut self, value: &Value, length: u64) -> Result<u64, Error> {
        let n = integer_or_infinity(self.numeric(value)?);
        let size = index_number(length);
        Ok(if n < 0.0 {
            if n <= -size {
                0
            } else {
                length.saturating_sub(length_index(-n))
            }
        } else if n >= size {
            length
        } else {
            length_index(n)
        })
    }
    pub(super) fn array_delete_or_throw(
        &mut self,
        receiver: &Value,
        key: &[u16],
    ) -> Result<(), Error> {
        if self.object_mut(receiver)?.delete(key) {
            Ok(())
        } else {
            Err(Error::Type {
                message: "cannot delete array-like property",
            })
        }
    }
}
pub(super) fn wide_key(index: u64) -> Rc<[u16]> {
    Value::string(&alloc::format!("{index}")).units()
}
#[expect(
    clippy::cast_precision_loss,
    clippy::as_conversions,
    reason = "callers supply array-like indices bounded by 2^53-1"
)]
pub(in crate::vm) const fn index_number(index: u64) -> f64 {
    index as f64
}
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    reason = "callers supply nonnegative integral values clamped to 2^53-1"
)]
pub(super) const fn length_index(n: f64) -> u64 {
    n as u64
}
