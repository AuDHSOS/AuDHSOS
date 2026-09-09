// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! For-in iteration snapshots keys one prototype at a time and rechecks own
//! descriptors before yielding. Non-enumerable keys still shadow prototypes.

use super::Execution;
use crate::{Error, Value};
use alloc::{collections::BTreeSet, rc::Rc, vec::Vec};

pub(super) struct Enumeration {
    pub(super) object: Value,
    keys: Option<Vec<Rc<[u16]>>>,
    visited: BTreeSet<Rc<[u16]>>,
    index: usize,
    prototypes: usize,
    pub(super) iterator: Option<crate::iterator::Record>,
}

impl Execution<'_> {
    pub(super) fn enumeration_init(
        &mut self,
        id: usize,
        mut object: Value,
        values: bool,
    ) -> Result<(), Error> {
        let iterator = if values {
            Some(self.get_iterator(&object)?)
        } else {
            None
        };
        if values {
            object = Value::Undefined;
        }
        if !values && !matches!(object, Value::Null | Value::Undefined) {
            object = self.box_value(&object)?;
        }
        let frame = self.frames.last_mut().ok_or(Error::InvalidBytecode)?;
        if frame.enumerations.len() <= id {
            frame
                .enumerations
                .resize_with(id.saturating_add(1), || None);
        }
        *frame
            .enumerations
            .get_mut(id)
            .ok_or(Error::InvalidBytecode)? = Some(Enumeration {
            object,
            keys: None,
            visited: BTreeSet::new(),
            index: 0,
            prototypes: 0,
            iterator,
        });
        Ok(())
    }
    pub(super) fn enumeration_next(&mut self, id: usize) -> Result<Option<Value>, Error> {
        self.enumeration_step_value(id, true)
    }
    pub(super) fn enumeration_step_value(
        &mut self,
        id: usize,
        read_value: bool,
    ) -> Result<Option<Value>, Error> {
        let mut state = self
            .frames
            .last_mut()
            .and_then(|f| f.enumerations.get_mut(id))
            .and_then(Option::take)
            .ok_or(Error::InvalidBytecode)?;
        let roots = self.native_roots.len();
        self.native_roots.push(state.object.clone());
        let result = if let Some(record) = &mut state.iterator {
            self.iterator_step_value(record, read_value)
        } else {
            self.enumeration_step(&mut state)
                .map(|v| v.map(Value::String))
        };
        self.native_roots.truncate(roots);
        *self
            .frames
            .last_mut()
            .and_then(|f| f.enumerations.get_mut(id))
            .ok_or(Error::InvalidBytecode)? = Some(state);
        result
    }

    fn enumeration_step(&mut self, state: &mut Enumeration) -> Result<Option<Rc<[u16]>>, Error> {
        loop {
            self.charge(1)?;
            if matches!(
                state.object,
                Value::Null | Value::Undefined | Value::Number(_) | Value::Boolean(_)
            ) {
                return Ok(None);
            }
            if state.keys.is_none() {
                let keys = self.own_keys(&state.object)?;
                state.keys = Some(keys);
                state.index = 0;
            }
            while let Some(key) = state
                .keys
                .as_ref()
                .and_then(|keys| keys.get(state.index))
                .cloned()
            {
                self.charge(1)?;
                state.index = state.index.saturating_add(1);
                if state.visited.contains(&key) {
                    continue;
                }
                if let Some(property) = self.own(&state.object, &key)? {
                    if state.visited.len() >= self.limits.properties {
                        return Err(Error::Limit {
                            resource: "enumeration keys",
                        });
                    }
                    state.visited.insert(key.clone());
                    if property.enumerable {
                        return Ok(Some(key));
                    }
                }
            }
            if matches!(state.object, Value::String(_)) {
                return Ok(None);
            }
            state.object = self.prototype_of(&state.object)?;
            state.keys = None;
            state.prototypes = state.prototypes.saturating_add(1);
            if state.prototypes > self.limits.heap_entries {
                return Err(Error::Limit {
                    resource: "prototype walk",
                });
            }
        }
    }
}
