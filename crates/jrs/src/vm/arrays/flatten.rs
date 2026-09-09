// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
//! Iterative `FlattenIntoArray`, with per-source length snapshots and stack roots.
//! Nesting storage is bounded cumulatively across reentrant flat/flatMap calls.
use super::{
    Error, Execution, Property, Value,
    indexing::{index_number, wide_key},
};
use alloc::vec::Vec;

struct Frame {
    source: Value,
    length: u64,
    next: u64,
    depth: f64,
    mapping: bool,
}
impl Execution<'_> {
    pub(super) fn array_flatten(
        &mut self,
        receiver: &Value,
        args: &[Value],
        mapping: bool,
    ) -> Result<Value, Error> {
        let length = self.array_length_wide(receiver)?;
        let first = args.first().unwrap_or(&Value::Undefined);
        let depth = if mapping {
            if !matches!(first, Value::Function(_)) {
                return Err(Error::Type {
                    message: "flatMap mapper is not callable",
                });
            }
            1.0
        } else if matches!(first, Value::Undefined) {
            1.0
        } else {
            crate::value::integer_or_infinity(self.numeric(first)?).max(0.0)
        };
        let target = self.array_species_create(receiver, 0)?;
        self.native_roots.push(target.clone());
        let roots = self.native_roots.len();
        let frames = self.flatten_frames;
        let result = self.flatten_into(
            &target,
            Frame {
                source: receiver.clone(),
                length,
                next: 0,
                depth,
                mapping,
            },
            first,
            args.get(1).unwrap_or(&Value::Undefined),
        );
        self.flatten_frames = frames;
        self.native_roots.truncate(roots);
        result?;
        Ok(target)
    }
    fn flatten_push(&mut self, frames: &mut Vec<Frame>, frame: Frame) -> Result<(), Error> {
        if self.flatten_frames >= self.limits.nesting {
            return Err(Error::Limit {
                resource: "array flatten frames",
            });
        }
        self.flatten_frames = self.flatten_frames.saturating_add(1);
        self.native_roots.push(frame.source.clone());
        frames.push(frame);
        Ok(())
    }
    fn flatten_into(
        &mut self,
        target: &Value,
        initial: Frame,
        mapper: &Value,
        this: &Value,
    ) -> Result<(), Error> {
        let mut frames = Vec::new();
        self.flatten_push(&mut frames, initial)?;
        let mut output = 0u64;
        while let Some(frame) = frames.last_mut() {
            self.charge(1)?;
            if frame.next >= frame.length {
                frames.pop();
                self.native_roots.pop();
                self.flatten_frames = self.flatten_frames.saturating_sub(1);
                continue;
            }
            let index = frame.next;
            frame.next = frame.next.saturating_add(1);
            let source = frame.source.clone();
            let depth = frame.depth;
            let mapping = frame.mapping;
            let key = wide_key(index);
            if !self.present(&source, &key)? {
                continue;
            }
            let roots = self.native_roots.len();
            let step = (|| {
                let mut element = self.get(&source, &key)?;
                self.native_roots.push(element.clone());
                if mapping {
                    element = self.call_sync(
                        mapper.clone(),
                        this.clone(),
                        &[element, Value::Number(index_number(index)), source],
                    )?;
                    self.native_roots.push(element.clone());
                }
                if depth > 0.0 && self.is_array(&element) {
                    let length = self.array_length_wide(&element)?;
                    return Ok(Some(Frame {
                        source: element,
                        length,
                        next: 0,
                        depth: depth - 1.0,
                        mapping: false,
                    }));
                }
                if output >= 9_007_199_254_740_991 {
                    return Err(Error::Type {
                        message: "flattened array exceeds maximum length",
                    });
                }
                self.define(target, wide_key(output), Property::data(element))?;
                output = output.saturating_add(1);
                Ok(None)
            })();
            self.native_roots.truncate(roots);
            if let Some(child) = step? {
                self.flatten_push(&mut frames, child)?;
            }
        }
        Ok(())
    }
}
