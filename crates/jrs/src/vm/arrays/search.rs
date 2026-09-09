// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Relative indexing and predicate searches over full 53-bit array-like lengths.
//! Predicate bodies run in VM frames: the fetched value remains rooted across
//! callbacks, and iteration shares normal fuel, frame and exception accounting.
use super::{
    Builtin, Error, Execution, Value,
    indexing::{index_number, length_index, wide_key},
};
use crate::value::integer_or_infinity;

impl Execution<'_> {
    pub(super) fn array_at(&mut self, receiver: &Value, index: &Value) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        // Index conversion occurs even for empty receivers, after reading length.
        let relative = integer_or_infinity(self.numeric(index)?);
        let length = index_number(length);
        let absolute = if relative < 0.0 {
            length + relative
        } else {
            relative
        };
        if absolute < 0.0 || absolute >= length {
            return Ok(Value::Undefined);
        }
        self.get(receiver, &wide_key(length_index(absolute)))
    }
}

pub(super) const fn find_body(builtin: Builtin) -> Option<&'static str> {
    Some(match builtin {
        // Get is unconditional: holes are visited. Capture the value once before
        // invoking the predicate, which may replace/delete the element or run GC.
        Builtin::ArrayFind => {
            "for(let k=0;k<length;k++){let value=receiver[k];if(invoke(callback,thisArg,value,k,receiver))return value;}return undefined;"
        }
        Builtin::ArrayFindIndex => {
            "for(let k=0;k<length;k++){let value=receiver[k];if(invoke(callback,thisArg,value,k,receiver))return k;}return -1;"
        }
        Builtin::ArrayFindLast => {
            "for(let k=length-1;k>=0;k--){let value=receiver[k];if(invoke(callback,thisArg,value,k,receiver))return value;}return undefined;"
        }
        Builtin::ArrayFindLastIndex => {
            "for(let k=length-1;k>=0;k--){let value=receiver[k];if(invoke(callback,thisArg,value,k,receiver))return k;}return -1;"
        }
        _ => return None,
    })
}
