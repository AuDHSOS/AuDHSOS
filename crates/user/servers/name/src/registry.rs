// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The registry: what name stands for what endpoint, and who put it there.
//!
//! The kernel has no global namespace, and a process can name only what it
//! holds a handle to. Two programs the root task started separately have no
//! way of reaching each other, and this is where one of them leaves a
//! capability to itself for the other to find.
//!
//! Ownership is by badge. The badge is the only thing about a sender the
//! kernel guarantees — it is written into the capability when the root task
//! hands it out, and a client cannot forge one — so a name belongs to the
//! badge that registered it, and to nothing a message says about itself.
//!
//! Invariants: a name appears at most once; every entry has a non-zero
//! owner, because a message that reaches the server through a capability
//! without a badge names no client and cannot own anything; a lookup never
//! changes the registry.

use audhsos_abi::{Error, Handle};
use audhsos_collections::ArrayVec;
use user_proto::Name;

/// How many names the registry holds.
pub const CAPACITY: usize = 64;

/// One name and what it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registered {
    /// The name.
    pub name: Name,
    /// The endpoint it stands for.
    pub endpoint: Handle,
    /// The badge of the client that registered it.
    pub owner: u64,
}

/// The registry of the name server.
#[derive(Debug)]
pub struct Registry {
    entries: ArrayVec<Registered, CAPACITY>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Registry {
            entries: ArrayVec::new(),
        }
    }

    /// How many names are registered.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when nothing is registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Everything that is registered, in the order it was registered in.
    pub fn entries(&self) -> impl Iterator<Item = Registered> + '_ {
        self.entries.iter().copied()
    }

    /// Puts `endpoint` under `name` for the client `owner`.
    ///
    /// A client may replace a name it registered itself, which is what a
    /// server that restarts does. A name another client holds is refused.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for an owner of zero, which is what a
    /// message that arrived through a capability without a badge has;
    /// [`Error::AlreadyExists`] for a name another client holds;
    /// [`Error::PoolExhausted`] when the registry is full.
    pub fn register(&mut self, owner: u64, name: Name, endpoint: Handle) -> Result<(), Error> {
        if owner == 0 {
            return Err(Error::InvalidArgument);
        }
        match self.position(&name) {
            Some(index) => self.replace(index, owner, endpoint),
            None => self
                .entries
                .push(Registered {
                    name,
                    endpoint,
                    owner,
                })
                .map_err(|_| Error::PoolExhausted),
        }
    }

    /// The endpoint `name` stands for.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when nothing is registered under the name.
    pub fn lookup(&self, name: &Name) -> Result<Handle, Error> {
        self.entries
            .iter()
            .find(|entry| entry.name == *name)
            .map(|entry| entry.endpoint)
            .ok_or(Error::NotFound)
    }

    /// Takes `name` out of the registry, for the client that owns it.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when nothing is registered under the name;
    /// [`Error::AccessDenied`] when another client owns it.
    pub fn forget(&mut self, owner: u64, name: &Name) -> Result<Handle, Error> {
        let index = self.position(name).ok_or(Error::NotFound)?;
        let entry = self.entries.get(index).copied().ok_or(Error::NotFound)?;
        if entry.owner != owner {
            return Err(Error::AccessDenied);
        }
        let _removed = self.entries.remove(index);
        Ok(entry.endpoint)
    }

    /// Takes out everything `owner` registered and answers with how many
    /// names that was.
    ///
    /// This is what the root task's report of a dead client leads to: the
    /// names a process left behind would otherwise stand for an endpoint
    /// nobody answers on.
    pub fn forget_client(&mut self, owner: u64) -> usize {
        let mut gone = 0usize;
        // Walked from the back, so that removing an entry does not move one
        // that has not been looked at yet.
        let mut index = self.entries.len();
        while index > 0 {
            index = index.wrapping_sub(1);
            let held = self
                .entries
                .get(index)
                .is_some_and(|entry| entry.owner == owner);
            if held {
                let _removed = self.entries.remove(index);
                gone = gone.wrapping_add(1);
            }
        }
        gone
    }

    /// Overwrites the entry at `index`, for the client that owns it.
    fn replace(&mut self, index: usize, owner: u64, endpoint: Handle) -> Result<(), Error> {
        let entry = self.entries.get_mut(index).ok_or(Error::NotFound)?;
        if entry.owner != owner {
            return Err(Error::AlreadyExists);
        }
        entry.endpoint = endpoint;
        Ok(())
    }

    /// Where `name` stands in the registry.
    fn position(&self, name: &Name) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name == *name)
    }
}
