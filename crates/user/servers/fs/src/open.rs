// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a client holds open, and how the server finds it again.
//!
//! A file handle is a number this server chose, not a kernel object,
//! because a file is not a kernel object. The number is an index into the
//! client's own table plus [`FIRST_FILE`], so the two roots — the volume
//! the system writes and the volume it booted from — keep the numbers
//! below it and no handle of one client names anything of another.
//!
//! The client is found by the badge on the endpoint the message came
//! through, which is the only thing about a sender a server can trust. The
//! display server finds a surface the same way.
//!
//! Invariants: a handle answers a slot of the client it was given to and
//! of no other; the two roots occupy no slot; a client that has closed
//! everything occupies no table.
//!
//! Known limit: a client that ends without closing keeps its table, and
//! nothing here learns that it ended. Risk 7 of
//! `docs/15-the-disk-on-the-machine.md` says what that costs and what it
//! would take to lift.

use fs_fat::{Dir, Entries, File, Location, Name};
use user_proto::file::FIRST_FILE;

/// How many clients the server keeps a table for.
pub const MAX_CLIENTS: usize = 16;

/// How many entries one client may hold open at once.
pub const MAX_OPEN: usize = 16;

/// Which volume a handle belongs to.
///
/// A handle names a file of one volume and of no other, so the volume is
/// part of what the handle stands for and not something a caller says
/// again at every message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    /// The volume the system writes, whose root is `user_proto::file::ROOT`.
    Written,
    /// The volume the machine booted from, whose root is
    /// `user_proto::file::BOOT`.
    Booted,
}

/// One thing a client holds open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A file, with the directory and the name that describe it, so that
    /// a stat reads the entry the file came from.
    File {
        /// Which volume it is on.
        volume: Which,
        /// The shared file state.
        shared: usize,
        /// The directory it lies in.
        parent: Dir,
        /// Its name in that directory.
        name: Name,
    },
    /// A directory, with the walk over its entries and how many of them
    /// have been handed out.
    Dir {
        /// Which volume it is on.
        volume: Which,
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

    /// Which volume this handle is on.
    #[must_use]
    pub const fn volume(&self) -> Which {
        match self {
            Opened::Dir { volume, .. } | Opened::File { volume, .. } => *volume,
        }
    }
}

/// The table of one client.
#[derive(Clone, Copy, Debug)]
struct Client {
    badge: u64,
    slots: [Option<Opened>; MAX_OPEN],
    /// Where the client's walk over each root directory stands, and how
    /// many entries of it the client has been given: one per volume, in
    /// the order the handles have them.
    ///
    /// A root occupies no slot, so a client reads it without opening
    /// anything; without this the walk would start again at every message
    /// and a listing would cost one pass of the directory per entry.
    roots: [Option<(Entries, u64)>; ROOTS],
}

impl Client {
    /// A client that holds nothing open.
    const fn new(badge: u64) -> Self {
        Client {
            badge,
            slots: [None; MAX_OPEN],
            roots: [None; ROOTS],
        }
    }

    /// `true` when the client holds nothing open.
    fn is_idle(&self) -> bool {
        self.slots.iter().all(Option::is_none) && self.roots.iter().all(Option::is_none)
    }
}

/// The open files of every client.
#[derive(Clone, Copy, Debug)]
pub struct Clients {
    tables: [Option<Client>; MAX_CLIENTS],
    files: [Option<SharedFile>; MAX_FILES],
}

const MAX_FILES: usize = MAX_CLIENTS * MAX_OPEN;

#[derive(Clone, Copy, Debug)]
struct SharedFile {
    volume: Which,
    location: Location,
    file: File,
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
            files: [None; MAX_FILES],
        }
    }

    /// Whether `badge` can hold one more handle.
    #[must_use]
    pub fn has_room(&self, badge: u64) -> bool {
        match self
            .tables
            .iter()
            .flatten()
            .find(|client| client.badge == badge)
        {
            Some(client) => client.slots.iter().any(Option::is_none),
            None => self.tables.iter().any(Option::is_none),
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
        if !self.has_room(badge) {
            return None;
        }
        let client = self.table(badge)?;
        let index = client.slots.iter().position(Option::is_none)?;
        let slot = client.slots.get_mut(index)?;
        *slot = Some(opened);
        handle_of(index)
    }

    /// Adds a handle to the shared state of a file on one volume.
    pub fn insert_file(
        &mut self,
        badge: u64,
        volume: Which,
        file: File,
        parent: Dir,
        name: Name,
    ) -> Option<u32> {
        if !self.has_room(badge) {
            return None;
        }
        let location = file.location();
        let existing = self.files.iter().position(|slot| {
            slot.is_some_and(|held| held.volume == volume && held.location == location)
        });
        let shared = existing.or_else(|| self.files.iter().position(Option::is_none))?;
        if existing.is_none() {
            *self.files.get_mut(shared)? = Some(SharedFile {
                volume,
                location,
                file,
            });
        }
        let handle = self.insert(
            badge,
            Opened::File {
                volume,
                shared,
                parent,
                name,
            },
        );
        if handle.is_none()
            && existing.is_none()
            && let Some(slot) = self.files.get_mut(shared)
        {
            *slot = None;
        }
        handle
    }

    /// The mutable file state named by a shared slot.
    pub fn file_mut(&mut self, shared: usize) -> Option<&mut File> {
        self.files
            .get_mut(shared)?
            .as_mut()
            .map(|held| &mut held.file)
    }

    /// Whether any client has this named file open.
    #[must_use]
    pub fn has_open_file(&self, volume: Which, parent: Dir, name: Name) -> bool {
        self.tables.iter().flatten().any(|client| {
            client.slots.iter().flatten().any(|opened| {
                matches!(opened, Opened::File { volume: held_volume, parent: held_parent, name: held_name, .. }
                    if *held_volume == volume && *held_parent == parent && *held_name == name)
            })
        })
    }

    /// Where `badge` stands in its walk over the root directory of volume
    /// `root`, made by `fresh` when the client has none.
    ///
    /// Answers `None` when the server keeps [`MAX_CLIENTS`] tables and
    /// this is a new client.
    pub fn root_walk(
        &mut self,
        badge: u64,
        root: usize,
        fresh: impl FnOnce() -> Entries,
    ) -> Option<&mut (Entries, u64)> {
        let client = self.table(badge)?;
        let slot = client.roots.get_mut(root)?;
        Some(slot.get_or_insert_with(|| (fresh(), 0)))
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
        let taken = client.slots.get_mut(index).and_then(Option::take);
        let idle = client.is_idle();
        if let Some(Opened::File { shared, .. }) = taken {
            self.release_file(shared);
        }
        if taken.is_some() && idle {
            self.forget(badge);
        }
        taken.is_some()
    }

    /// Ends a walk over a root and drops an idle client table.
    pub fn clear_root(&mut self, badge: u64, root: usize) -> bool {
        let Some(client) = self.existing(badge) else {
            return false;
        };
        let taken = client.roots.get_mut(root).and_then(Option::take).is_some();
        if taken && client.is_idle() {
            self.forget(badge);
        }
        taken
    }

    fn release_file(&mut self, shared: usize) {
        let held = self.tables.iter().flatten().any(|client| {
            client.slots.iter().flatten().any(
                |opened| matches!(opened, Opened::File { shared: index, .. } if *index == shared),
            )
        });
        if !held && let Some(slot) = self.files.get_mut(shared) {
            *slot = None;
        }
    }

    /// Drops the table of `badge`, whatever stands in it. The server calls
    /// it when a client is gone.
    pub fn forget(&mut self, badge: u64) {
        for slot in &mut self.tables {
            if slot.is_some_and(|client| client.badge == badge) {
                *slot = None;
            }
        }
        for shared in 0..MAX_FILES {
            if self.files.get(shared).is_some_and(Option::is_some) {
                self.release_file(shared);
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

/// How many volumes a client holds a root of.
pub const ROOTS: usize = 2;

/// The handle of the slot at `index`, which is [`FIRST_FILE`] above it:
/// the numbers below belong to the roots, which occupy no slot.
fn handle_of(index: usize) -> Option<u32> {
    u32::try_from(index).ok()?.checked_add(FIRST_FILE)
}

/// The slot `handle` names, or `None` for a root and for a handle no
/// table is that long for.
fn index_of(handle: u32) -> Option<usize> {
    let index = usize::try_from(handle.checked_sub(FIRST_FILE)?).ok()?;
    (index < MAX_OPEN).then_some(index)
}
