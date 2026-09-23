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
//! What a client registers is not what the registry stores. The kernel
//! copies rights and badge onto the handle it installs in the receiver, so
//! the handle a server sends carries the `RECV` and `BADGE` of the server's
//! own endpoint; [`Registry::accept`] stores a handle narrowed to
//! [`HANDED_OUT`] instead, and a lookup hands out nothing above it.
//!
//! Invariants: a name appears at most once; every entry has a non-zero
//! owner, because a message that reaches the server through a capability
//! without a badge names no client and cannot own anything; a lookup never
//! changes the registry.

use audhsos_abi::{Error, Handle, Rights};
use audhsos_collections::ArrayVec;
use user_proto::Name;

/// How many names the registry holds.
pub const CAPACITY: usize = 64;

/// Maximum names one client may hold in the server binary; eight owners can
/// each use their full quota.
pub const PER_OWNER_LIMIT: usize = 8;

/// The rights a registered endpoint is stored with, and so the most a
/// lookup can hand out: a client sends to the server it found and passes
/// the handle on.
///
/// `RECV` would let a client take the requests other clients sent to that
/// server, and `BADGE` would let it mint the badge the server trusts.
pub const HANDED_OUT: Rights = Rights::SEND.union(Rights::TRANSFER);

/// The handle calls [`Registry::accept`] makes. The registry itself makes
/// no system call; the program that serves the registry implements this on
/// its gate.
pub trait Handles {
    /// A second handle to the same object with `rights`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered; [`Error::AccessDenied`] for a handle
    /// without `DUPLICATE` or for rights above those it carries.
    fn duplicate(&mut self, handle: Handle, rights: Rights) -> Result<Handle, Error>;

    /// Gives `handle` up. A handle the registry no longer stores is a slot
    /// of the server's table that stays taken until it is closed.
    fn close(&mut self, handle: Handle);
}

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
    per_owner_limit: usize,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new(PER_OWNER_LIMIT)
    }
}

impl Registry {
    /// An empty registry.
    #[must_use]
    pub const fn new(per_owner_limit: usize) -> Self {
        Registry {
            entries: ArrayVec::new(),
            per_owner_limit,
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

    /// Takes the endpoint a client sent: stores a handle narrowed to
    /// [`HANDED_OUT`] under `name` and gives the sent handle up, so that
    /// the registry holds nothing a lookup may not hand out.
    ///
    /// The handle a replaced entry held is given up too.
    ///
    /// # Errors
    ///
    /// The errors of [`Registry::register`], and whatever the duplication
    /// answered. The sent handle is given up in every case.
    pub fn accept<H: Handles>(
        &mut self,
        handles: &mut H,
        owner: u64,
        name: Name,
        endpoint: Handle,
    ) -> Result<(), Error> {
        let narrowed = match handles.duplicate(endpoint, HANDED_OUT) {
            Ok(narrowed) => narrowed,
            Err(error) => {
                handles.close(endpoint);
                return Err(error);
            }
        };
        let outcome = self.register(owner, name, narrowed);
        handles.close(endpoint);
        match outcome {
            Ok(replaced) => {
                if let Some(old) = replaced {
                    handles.close(old);
                }
                Ok(())
            }
            Err((rejected, error)) => {
                handles.close(rejected);
                Err(error)
            }
        }
    }

    /// Puts `endpoint` under `name` for the client `owner` and answers with
    /// the handle the entry held before, if it held one. The caller closes
    /// that handle.
    ///
    /// A client may replace a name it registered itself, which is what a
    /// server that restarts does. A name another client holds is refused.
    ///
    /// Callers that take handles out of messages go through
    /// [`Registry::accept`]: this one stores what it is given.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for an owner of zero, which is what a
    /// message that arrived through a capability without a badge has;
    /// [`Error::AlreadyExists`] for a name another client holds;
    /// [`Error::PoolExhausted`] when the registry or the owner's quota is full.
    /// An error returns `endpoint` to the caller for closing.
    pub fn register(
        &mut self,
        owner: u64,
        name: Name,
        endpoint: Handle,
    ) -> Result<Option<Handle>, (Handle, Error)> {
        if owner == 0 {
            return Err((endpoint, Error::InvalidArgument));
        }
        if let Some(index) = self.position(&name) {
            return self
                .replace(index, owner, endpoint)
                .map(Some)
                .map_err(|error| (endpoint, error));
        }
        if self
            .entries
            .iter()
            .filter(|entry| entry.owner == owner)
            .count()
            >= self.per_owner_limit
        {
            return Err((endpoint, Error::PoolExhausted));
        }
        self.entries
            .push(Registered {
                name,
                endpoint,
                owner,
            })
            .map(|()| None)
            .map_err(|_| (endpoint, Error::PoolExhausted))
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

    /// Takes out everything `owner` registered, gives the handle of every
    /// entry it took out up, and answers with how many names that was.
    ///
    /// This is what the root task's report of a dead client leads to: the
    /// names a process left behind would otherwise stand for an endpoint
    /// nobody answers on, and each of them holds a slot of the server's
    /// table.
    pub fn forget_client<H: Handles>(&mut self, handles: &mut H, owner: u64) -> usize {
        let mut gone = 0usize;
        // Walked from the back, so that removing an entry does not move one
        // that has not been looked at yet.
        let mut index = self.entries.len();
        while index > 0 {
            index = index.wrapping_sub(1);
            let held = self
                .entries
                .get(index)
                .filter(|entry| entry.owner == owner)
                .map(|entry| entry.endpoint);
            if let Some(endpoint) = held {
                let _removed = self.entries.remove(index);
                handles.close(endpoint);
                gone = gone.wrapping_add(1);
            }
        }
        gone
    }

    /// Overwrites the entry at `index`, for the client that owns it, and
    /// answers with the handle it held.
    fn replace(&mut self, index: usize, owner: u64, endpoint: Handle) -> Result<Handle, Error> {
        let entry = self.entries.get_mut(index).ok_or(Error::NotFound)?;
        if entry.owner != owner {
            return Err(Error::AlreadyExists);
        }
        let held = entry.endpoint;
        entry.endpoint = endpoint;
        Ok(held)
    }

    /// Where `name` stands in the registry.
    fn position(&self, name: &Name) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name == *name)
    }
}
