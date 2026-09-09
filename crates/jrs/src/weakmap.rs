// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Identity-keyed weak associations. Entries are ephemerons, never strong
//! key edges. Tree lookup is O(log n) without hashing or external dependencies.

use crate::{
    Value,
    bytecode::Builtin,
    heap::Handle,
    value::{Callable, FunctionValue},
};
use alloc::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    // Resolving functions share a latch handle, but have different identities.
    Heap(Handle, bool),
    Native(Builtin),
}
impl Key {
    pub(crate) const fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Symbol(symbol) if !symbol.registered => Some(Self::Heap(symbol.handle, false)),
            Value::Object(object) => Some(Self::Heap(object.handle, false)),
            Value::Function(FunctionValue(Callable::Native(builtin))) => {
                Some(Self::Native(*builtin))
            }
            Value::Function(FunctionValue(Callable::Resolver { handle, reject, .. })) => {
                Some(Self::Heap(*handle, *reject))
            }
            Value::Function(FunctionValue(
                Callable::Script { handle, .. }
                | Callable::Bound { handle, .. }
                | Callable::Host { handle, .. },
            )) => Some(Self::Heap(*handle, false)),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct WeakMap {
    pub(crate) entries: BTreeMap<Key, Value>,
}
