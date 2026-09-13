// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a client holds open, and how the server finds it again.
//!
//! A file handle is a number this server chose, not a kernel object,
//! because a file is not a kernel object. The number is an index into the
//! client's own table plus one, so [`user_proto::file::ROOT`] stays the
//! root directory and no handle of one client names anything of another.
//!
//! The client is found by the badge on the endpoint the message came
//! through, which is the only thing about a sender a server can trust. The
//! display server finds a surface the same way.
//!
//! Invariants: a handle answers a slot of the client it was given to and
//! of no other; the root directory is handle [`ROOT`] for every client and
//! occupies no slot; a client that has closed everything occupies no
//! table.
//!
//! Known limit: a client that ends without closing keeps its table, and
//! nothing here learns that it ended. Risk 7 of
//! `docs/15-the-disk-on-the-machine.md` says what that costs and what it
//! would take to lift.

use fs_fat::{Dir, Entries, File, Name};
use user_proto::file::ROOT;

/// How many clients the server keeps a table for.
pub const MAX_CLIENTS: usize = 16;

/// How many entries one client may hold open at once.
pub const MAX_OPEN: usize = 16;

/// One thing a client holds open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A file, with the directory and the name that describe it, so that
    /// a stat reads the entry the file came from.
    File {
        /// The file as `fs-fat` tracks it, cursor and all.
        file: File,
        /// The directory it lies in.
        parent: Dir,
        /// Its name in that directory.
        name: Name,
    },
    /// A directory, with the walk over its entries and how many of them
    /// have been handed out.
    Dir {
        /// The directory.
        dir: Dir,
        /// Where the walk stands.
        walk: Entries,
        /// How many entries have been answered, which is the cursor the
        /// client is given back.
        handed: u64,
    },
}

impl Opened {
    /// The directory this handle stands for, or `None` for a file.
    #[must_use]
    pub const fn as_dir(&self) -> Option<Dir> {
        match self {
            Opened::Dir { dir, .. } => Some(*dir),
            Opened::File { .. } => None,
        }
    }
}

/// The table of one client.
#[derive(Clone, Copy, Debug)]
struct Client {
    badge: u64,
    slots: [Option<Opened>; MAX_OPEN],
    /// Where the client's walk over the root directory stands, and how
    /// many entries of it the client has been given.
    ///
    /// The root occupies no slot, so a client reads it without opening
    /// anything; without this the walk would start again at every message
    /// and a listing would cost one pass of the directory per entry.
    root: Option<(Entries, u64)>,
}

impl Client {
    /// A client that holds nothing open.
    const fn new(badge: u64) -> Self {
        Client {
            badge,
            slots: [None; MAX_OPEN],
            root: None,
        }
    }

    /// `true` when the client holds nothing open.
    fn is_idle(&self) -> bool {
        self.slots.iter().all(Option::is_none) && self.root.is_none()
    }
}

/// The open files of every client.
#[derive(Clone, Copy, Debug)]
pub struct Clients {
    tables: [Option<Client>; MAX_CLIENTS],
}

impl Default for Clients {
    fn default() -> Self {
        Self::new()
    }
}

impl Clients {
    /// A server no client has opened anything at.
    #[must_use]
    pub const fn new() -> Self {
        Clients {
            tables: [None; MAX_CLIENTS],
        }
    }

    /// The table of `badge`, made if the client has none.
    ///
    /// Answers `None` when [`MAX_CLIENTS`] tables are in use and this
    /// badge is not one of them.
    fn table(&mut self, badge: u64) -> Option<&mut Client> {
        let mut free = None;
        for (index, slot) in self.tables.iter().enumerate() {
            match slot {
                Some(client) if client.badge == badge => {
                    return self.tables.get_mut(index)?.as_mut();
                }
                Some(_) => {}
                None => free = free.or(Some(index)),
            }
        }
        let slot = self.tables.get_mut(free?)?;
        *slot = Some(Client::new(badge));
        slot.as_mut()
    }

    /// The table of `badge`, without making one.
    fn existing(&mut self, badge: u64) -> Option<&mut Client> {
        self.tables
            .iter_mut()
            .flatten()
            .find(|client| client.badge == badge)
    }

    /// Puts `opened` into a free slot of `badge` and answers its handle.
    ///
    /// Answers `None` when the client holds [`MAX_OPEN`] already, or when
    /// the server keeps [`MAX_CLIENTS`] tables and this is a new client.
    pub fn insert(&mut self, badge: u64, opened: Opened) -> Option<u32> {
        let client = self.table(badge)?;
        let index = client.slots.iter().position(Option::is_none)?;
        let slot = client.slots.get_mut(index)?;
        *slot = Some(opened);
        handle_of(index)
    }

    /// Where `badge` stands in its walk over the root directory, made by
    /// `fresh` when the client has none.
    ///
    /// Answers `None` when the server keeps [`MAX_CLIENTS`] tables and
    /// this is a new client.
    pub fn root_walk(
        &mut self,
        badge: u64,
        fresh: impl FnOnce() -> Entries,
    ) -> Option<&mut (Entries, u64)> {
        let client = self.table(badge)?;
        Some(client.root.get_or_insert_with(|| (fresh(), 0)))
    }

    /// What `handle` of `badge` stands for.
    #[must_use]
    pub fn get(&mut self, badge: u64, handle: u32) -> Option<&mut Opened> {
        let index = index_of(handle)?;
        self.existing(badge)?.slots.get_mut(index)?.as_mut()
    }

    /// Takes `handle` of `badge` out of the table.
    ///
    /// Answers `false` for a handle the client does not hold. A client
    /// that closes its last file keeps no table, so the sixteen tables go
    /// to whoever has something open.
    pub fn remove(&mut self, badge: u64, handle: u32) -> bool {
        let Some(index) = index_of(handle) else {
            return false;
        };
        let Some(client) = self.existing(badge) else {
            return false;
        };
        let taken = client.slots.get_mut(index).and_then(Option::take).is_some();
        if taken && client.is_idle() {
            self.forget(badge);
        }
        taken
    }

    /// Drops the table of `badge`, whatever stands in it. The server calls
    /// it when a client is gone.
    pub fn forget(&mut self, badge: u64) {
        for slot in &mut self.tables {
            if slot.is_some_and(|client| client.badge == badge) {
                *slot = None;
            }
        }
    }

    /// How many clients hold something open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tables.iter().flatten().count()
    }

    /// `true` when no client holds anything open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The handle of the slot at `index`.
///
/// One above the index, because zero is [`ROOT`], which is every client's
/// root directory and occupies no slot.
fn handle_of(index: usize) -> Option<u32> {
    u32::try_from(index).ok()?.checked_add(1)
}

/// The slot `handle` names, or `None` for [`ROOT`] and for a handle no
/// table is that long for.
fn index_of(handle: u32) -> Option<usize> {
    if handle == ROOT {
        return None;
    }
    let index = usize::try_from(handle.checked_sub(1)?).ok()?;
    (index < MAX_OPEN).then_some(index)
}
