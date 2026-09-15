// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Agent: one execution thread's heap and its Realms (ECMA-262 9.7).
//!
//! An Agent owns exactly one object heap. Every Realm created in it allocates
//! from that heap, so values are transferable between Realms of one Agent and
//! never between Agents.

use super::{
    heap::{GenerationalHeap, HeapError},
    realm::Realm,
};

/// One Agent: an object heap and the Realm currently executing in it.
pub struct Agent {
    /// The Agent's object heap.
    pub heap: GenerationalHeap,
    /// The Agent's current Realm.
    pub realm: Realm,
}

impl Agent {
    /// Creates an Agent with a fresh heap and one Realm of intrinsics.
    ///
    /// # Errors
    ///
    /// Returns a [`HeapError`] when the intrinsic object graph cannot be built.
    pub fn new() -> Result<Self, HeapError> {
        let mut heap = GenerationalHeap::new();
        let realm = Realm::new(&mut heap)?;
        Ok(Self { heap, realm })
    }
}
