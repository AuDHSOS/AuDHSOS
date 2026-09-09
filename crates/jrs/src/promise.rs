// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Promise records contain heap identities, never owning pointers to each other.

use crate::Value;
use alloc::vec::Vec;

pub(crate) enum State {
    Pending,
    Fulfilled(Value),
    Rejected(Value),
}

#[derive(Clone)]
pub(crate) struct Reaction {
    pub(crate) fulfilled: Value,
    pub(crate) rejected: Value,
    pub(crate) target: Value,
    pub(crate) resolve: Value,
    pub(crate) reject: Value,
    pub(crate) resume: Option<crate::heap::Handle>,
}

pub(crate) struct Capability {
    pub(crate) promise: Value,
    pub(crate) resolve: Value,
    pub(crate) reject: Value,
}

pub(crate) struct Promise {
    pub(crate) state: State,
    pub(crate) reactions: Vec<Reaction>,
    pub(crate) handled: bool,
}

impl Promise {
    pub(crate) const fn new() -> Self {
        Self {
            state: State::Pending,
            reactions: Vec::new(),
            handled: false,
        }
    }
}

pub(crate) enum Job {
    Microtask(Value),
    Reaction {
        reaction: Reaction,
        value: Value,
        reject: bool,
    },
    Thenable {
        target: Value,
        object: Value,
        then: Value,
    },
}
