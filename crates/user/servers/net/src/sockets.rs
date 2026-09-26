// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The sockets of every client: which slot a number names, whose it is,
//! and what it stands for.
//!
//! A number carries the generation of the slot it names, as a handle of
//! `net-stack` carries the generation of its own. The slot counts its uses
//! and a number made under an older count is refused, so a client holding
//! the number of a socket it closed reads nothing of the socket that took
//! the slot (D-116).
//!
//! Invariants: a slot is either free or belongs to exactly one client; a
//! number of the wrong client is refused as if the slot were free; and a
//! slot that has been handed out stays with the client that had it until
//! that client is gone, because the rings of a slot are memory the client
//! maps and no call of this server takes a mapping back.

use net_stack::Handle;

/// How many sockets this server hands out at once, over every client.
/// One memory object of two rings stands behind each.
pub const MAX_SOCKETS: usize = 4;

/// What a socket of a client stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A datagram socket.
    Datagram,
    /// A connection, opened by this client or taken by its listener.
    Connection,
    /// A connection waiting for a peer to open it. It has no rings until
    /// an accept turns it into one.
    Listener,
    /// A connection the client closed while the peer had not acknowledged
    /// its `FIN`. No number names it; the server gives it up once the
    /// acknowledgment arrives.
    Draining,
}

/// One socket.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// Whose it is, which is the badge of the endpoint it was asked for
    /// through.
    pub badge: u64,
    /// What it stands for.
    pub kind: Kind,
    /// What the stack calls it.
    pub handle: Handle,
    /// How often this slot has been handed out, which the number carries.
    pub generation: u32,
    /// Whether the client has been told that the peer sent no more.
    pub peer_closed: bool,
}

/// The table of sockets.
#[derive(Debug, Default)]
pub struct Sockets {
    /// One slot per socket this server hands out.
    slots: [Option<Entry>; MAX_SOCKETS],
    /// How often each slot has been handed out.
    generations: [u32; MAX_SOCKETS],
    /// Which client last held each slot, kept after the slot is closed.
    owners: [Option<u64>; MAX_SOCKETS],
}

/// How far up the generation sits in a socket number.
const GENERATION_SHIFT: u32 = 16;

/// The part of a number that is the slot, one above its index so that
/// zero names no socket.
const SLOT_MASK: u32 = 0xFFFF;

impl Sockets {
    /// A table with nothing open.
    #[must_use]
    pub const fn new() -> Sockets {
        Sockets {
            slots: [None; MAX_SOCKETS],
            generations: [0; MAX_SOCKETS],
            owners: [None; MAX_SOCKETS],
        }
    }

    /// Opens a socket for `badge` and answers its number and the slot it
    /// took, or `None` when every slot is taken.
    pub fn open(&mut self, badge: u64, kind: Kind, handle: Handle) -> Option<(u32, usize)> {
        let index = self.free_for(badge)?;
        let generation = self.generations.get(index).copied().unwrap_or(0);
        if let Some(owner) = self.owners.get_mut(index) {
            *owner = Some(badge);
        }
        *self.slots.get_mut(index)? = Some(Entry {
            badge,
            kind,
            handle,
            generation,
            peer_closed: false,
        });
        Some((number_of(index, generation), index))
    }

    /// The first free slot `badge` may have: one nobody has held, or one
    /// it held itself.
    ///
    /// The rings of a slot are one memory object, and a client that was
    /// given it maps it for as long as it runs — the server cannot take
    /// that mapping back. A slot therefore stays with the client that
    /// first had it until [`release`](Sockets::release) says that client
    /// is gone, and a client that finds none is refused rather than
    /// handed the rings another client can still read.
    fn free_for(&self, badge: u64) -> Option<usize> {
        self.slots.iter().enumerate().position(|(index, slot)| {
            slot.is_none()
                && self
                    .owners
                    .get(index)
                    .copied()
                    .flatten()
                    .is_none_or(|owner| owner == badge)
        })
    }

    /// Lets go of every slot a client that is gone last held, so that
    /// another client may have them.
    pub fn release(&mut self, badge: u64) {
        for (index, owner) in self.owners.iter_mut().enumerate() {
            if *owner == Some(badge) && self.slots.get(index).is_none_or(Option::is_none) {
                *owner = None;
            }
        }
    }

    /// Whether a slot is held for `badge`, open or not.
    #[must_use]
    pub fn holds(&self, badge: u64) -> bool {
        self.owners.contains(&Some(badge))
    }

    /// The slot `number` names, for the client `badge` names.
    #[must_use]
    pub fn slot(&self, badge: u64, number: u32) -> Option<usize> {
        let (index, generation) = parts(number)?;
        let entry = self.slots.get(index)?.as_ref()?;
        (entry.badge == badge && entry.generation == generation && entry.kind != Kind::Draining)
            .then_some(index)
    }

    /// The socket `number` names, for the client `badge` names.
    #[must_use]
    pub fn get(&self, badge: u64, number: u32) -> Option<&Entry> {
        self.slots.get(self.slot(badge, number)?)?.as_ref()
    }

    /// The same, for changing it.
    pub fn get_mut(&mut self, badge: u64, number: u32) -> Option<&mut Entry> {
        let index = self.slot(badge, number)?;
        self.slots.get_mut(index)?.as_mut()
    }

    /// The socket in slot `index`.
    #[must_use]
    pub fn at(&self, index: usize) -> Option<&Entry> {
        self.slots.get(index)?.as_ref()
    }

    /// The same, for changing it.
    pub fn at_mut(&mut self, index: usize) -> Option<&mut Entry> {
        self.slots.get_mut(index)?.as_mut()
    }

    /// Closes the socket in slot `index` and answers what it was.
    ///
    /// The generation of the slot moves on, so the number that named it
    /// names nothing from here.
    pub fn close(&mut self, index: usize) -> Option<Entry> {
        let entry = self.slots.get_mut(index)?.take()?;
        if let Some(generation) = self.generations.get_mut(index) {
            // The count stays inside the sixteen bits a number carries, so
            // that the count a slot holds and the count a number holds are
            // the same value after the sixty-five thousandth use.
            *generation = generation.wrapping_add(1) & SLOT_MASK;
        }
        Some(entry)
    }

    /// Every slot of one client, for the moment that client is gone.
    pub fn of(&self, badge: u64) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(index, slot)| {
                slot.as_ref()
                    .and_then(|entry| (entry.badge == badge).then_some(index))
            })
    }

    /// Every slot that holds a socket.
    pub fn open_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|_| index))
    }

    /// How many sockets are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    /// `true` when nothing is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The number of the socket in slot `index` at `generation`.
fn number_of(index: usize, generation: u32) -> u32 {
    let slot = u32::try_from(index).unwrap_or(0).wrapping_add(1) & SLOT_MASK;
    (generation << GENERATION_SHIFT) | slot
}

/// The slot and the generation a number carries, or `None` for a number
/// that names no slot.
fn parts(number: u32) -> Option<(usize, u32)> {
    let slot = number & SLOT_MASK;
    if slot == 0 {
        return None;
    }
    let index = usize::try_from(slot.wrapping_sub(1)).ok()?;
    (index < MAX_SOCKETS).then_some((index, number >> GENERATION_SHIFT))
}
