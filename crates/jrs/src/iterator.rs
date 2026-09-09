// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Heap iterator state and cached user-protocol records.

use crate::Value;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Keys,
    Values,
    Entries,
    String,
}

#[derive(Clone)]
pub(crate) struct State {
    pub(crate) source: Value,
    pub(crate) index: u64,
    pub(crate) kind: Kind,
}

pub(crate) struct Record {
    pub(crate) object: Value,
    pub(crate) next: Value,
    pub(crate) done: bool,
}
