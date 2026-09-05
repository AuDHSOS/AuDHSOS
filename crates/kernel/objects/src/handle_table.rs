// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handle slots of the machine: one arena shared by every process,
//! with a per-process list threaded through it (D-58).
//!
//! Invariants: a slot is occupied by at most one entry and belongs to
//! exactly one process while it is; a lookup checks the index, the
//! generation, **and** the owner, so one process can never name a slot of
//! another; the list of a process holds exactly the slots that name it as
//! their owner, and its count is the length of that list; a generation
//! that was handed out is never zero.
//!
//! An empty arena is all zeros and its constructor is `const`, for the same
//! reason as the pools (D-66): a link is a one-based index whose zero means
//! "none", the free list starts empty behind a high-water mark, and a
//! generation counts up when a slot is handed out.

use audhsos_abi::{Error, Handle, ObjectType, Rights};

use crate::object::{AnyObjectId, ProcessId};

/// A link to another slot of the arena: zero is no link, and any other
/// value is the index of a slot plus one. The zero makes an unlinked slot
/// part of an all-zero arena.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Link(u32);

impl Link {
    /// No link.
    pub const NONE: Link = Link(0);

    /// A link to `index`, or [`Link::NONE`] for an index that cannot be
    /// stored, which no arena of this system reaches.
    #[must_use]
    pub const fn to(index: u32) -> Self {
        match index.checked_add(1) {
            Some(raw) => Link(raw),
            None => Link::NONE,
        }
    }

    /// The slot this link names, or `None`.
    #[must_use]
    pub const fn index(self) -> Option<u32> {
        match self.0.checked_sub(1) {
            Some(index) => Some(index),
            None => None,
        }
    }

    /// `true` if the link names no slot.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// What one handle names, and what its holder may do with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The object the handle names.
    pub object: AnyObjectId,
    /// What the holder may do with the object.
    pub rights: Rights,
    /// The value the kernel delivers with a message sent through this
    /// capability. It is zero for every handle Phase 5 creates.
    pub badge: u64,
}

impl Entry {
    /// A handle to `object` with `rights` and no badge.
    #[must_use]
    pub const fn new(object: AnyObjectId, rights: Rights) -> Self {
        Entry {
            object,
            rights,
            badge: 0,
        }
    }

    /// The same entry with a badge.
    #[must_use]
    pub const fn with_badge(self, badge: u64) -> Self {
        Entry { badge, ..self }
    }

    /// The type of the object the handle names.
    #[must_use]
    pub const fn object_type(&self) -> ObjectType {
        self.object.object_type()
    }
}

/// An occupied slot: the entry, its owner, and the links that chain the
/// slots of that owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Occupant {
    owner: ProcessId,
    entry: Entry,
    previous: Link,
    next: Link,
}

/// One slot of the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot {
    generation: u32,
    next_free: Link,
    occupant: Option<Occupant>,
}

impl Slot {
    /// A slot that was never used. Every field is zero.
    const FREE: Slot = Slot {
        generation: 0,
        next_free: Link::NONE,
        occupant: None,
    };
}

/// What a process holds: the head of its chain through the arena, how many
/// slots it holds, and how many its creator granted it. This is a ceiling
/// against the shared arena, not memory set aside for the process (D-58).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HandleList {
    head: Link,
    count: u32,
    capacity: u32,
}

impl HandleList {
    /// A list that holds nothing and may grow to `capacity` handles.
    #[must_use]
    pub const fn with_capacity(capacity: u32) -> Self {
        HandleList {
            head: Link::NONE,
            count: 0,
            capacity,
        }
    }

    /// How many handles the process holds.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.count
    }

    /// How many handles the process may hold.
    #[must_use]
    pub const fn capacity(self) -> u32 {
        self.capacity
    }

    /// `true` if the process holds no handle.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// How many handles the process may still take.
    #[must_use]
    pub const fn free_slots(self) -> u32 {
        self.capacity.saturating_sub(self.count)
    }
}

/// The handle slots of the whole machine.
#[derive(Debug)]
pub struct HandleArena<const N: usize> {
    slots: [Slot; N],
    free_head: Link,
    free_tail: Link,
    high_water: u32,
    live: u32,
}

impl<const N: usize> Default for HandleArena<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> HandleArena<N> {
    /// An empty arena. The value is all zeros and the constructor is
    /// `const`, so a `static` arena costs the image nothing but `.bss`.
    #[must_use]
    pub const fn new() -> Self {
        HandleArena {
            slots: [const { Slot::FREE }; N],
            free_head: Link::NONE,
            free_tail: Link::NONE,
            high_water: 0,
            live: 0,
        }
    }

    /// How many slots the arena has.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        u32::try_from(N).unwrap_or(u32::MAX)
    }

    /// How many slots are occupied.
    #[must_use]
    pub const fn live(&self) -> u32 {
        self.live
    }

    /// `true` if no slot is occupied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn slot(&self, index: u32) -> Option<&Slot> {
        self.slots.get(usize::try_from(index).ok()?)
    }

    fn slot_mut(&mut self, index: u32) -> Option<&mut Slot> {
        self.slots.get_mut(usize::try_from(index).ok()?)
    }

    /// The index of a slot to occupy: the head of the free list, which
    /// holds the slots that were closed, or the next slot that was never
    /// used.
    fn take_free_slot(&mut self) -> Option<u32> {
        if let Some(index) = self.free_head.index() {
            let next_free = self.slot(index).map_or(Link::NONE, |slot| slot.next_free);
            self.free_head = next_free;
            if next_free.is_none() {
                self.free_tail = Link::NONE;
            }
            return Some(index);
        }
        if usize::try_from(self.high_water).is_ok_and(|used| used < N) {
            let index = self.high_water;
            self.high_water = self.high_water.saturating_add(1);
            return Some(index);
        }
        None
    }

    /// Puts a closed slot at the end of the free list, so that slots come
    /// back in the order in which they were closed.
    fn put_free_slot(&mut self, index: u32) {
        if let Some(slot) = self.slot_mut(index) {
            slot.next_free = Link::NONE;
        }
        match self.free_tail.index() {
            Some(tail) => {
                if let Some(slot) = self.slot_mut(tail) {
                    slot.next_free = Link::to(index);
                }
            }
            None => self.free_head = Link::to(index),
        }
        self.free_tail = Link::to(index);
    }

    /// Installs `entry` as a handle of `process` and returns the handle.
    ///
    /// # Errors
    ///
    /// [`Error::QuotaExceeded`] when the process already holds as many
    /// handles as its creator granted it; [`Error::OutOfHandles`] when the
    /// arena has no slot left.
    pub fn insert(
        &mut self,
        process: ProcessId,
        list: &mut HandleList,
        entry: Entry,
    ) -> Result<Handle, Error> {
        if list.count >= list.capacity {
            return Err(Error::QuotaExceeded);
        }
        let index = self.take_free_slot().ok_or(Error::OutOfHandles)?;
        let head = list.head;
        let generation = {
            let slot = self.slot_mut(index).ok_or(Error::OutOfHandles)?;
            // A generation of zero is never handed out, so a handle built
            // from a slot that was never used names nothing.
            let generation = match slot.generation.wrapping_add(1) {
                0 => 1,
                generation => generation,
            };
            slot.generation = generation;
            slot.next_free = Link::NONE;
            slot.occupant = Some(Occupant {
                owner: process,
                entry,
                previous: Link::NONE,
                next: head,
            });
            generation
        };
        if let Some(old_head) = head.index()
            && let Some(occupant) = self.occupant_mut_at(old_head)
        {
            occupant.previous = Link::to(index);
        }
        list.head = Link::to(index);
        list.count = list.count.saturating_add(1);
        self.live = self.live.saturating_add(1);
        Handle::new(index, generation).ok_or(Error::OutOfHandles)
    }

    fn occupant_at(&self, index: u32) -> Option<&Occupant> {
        self.slot(index)?.occupant.as_ref()
    }

    fn occupant_mut_at(&mut self, index: u32) -> Option<&mut Occupant> {
        self.slot_mut(index)?.occupant.as_mut()
    }

    /// The slot `handle` names, if it is live and belongs to `process`.
    fn resolve(&self, process: ProcessId, handle: Handle) -> Result<u32, Error> {
        let index = handle.index();
        let slot = self.slot(index).ok_or(Error::InvalidHandle)?;
        if slot.generation != handle.generation() {
            return Err(Error::InvalidHandle);
        }
        let occupant = slot.occupant.as_ref().ok_or(Error::InvalidHandle)?;
        if occupant.owner != process {
            return Err(Error::InvalidHandle);
        }
        Ok(index)
    }

    /// What `handle` names for `process`.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] when the index is out of range, the
    /// generation belongs to an earlier entry, the slot is free, or the
    /// slot belongs to another process.
    pub fn lookup(&self, process: ProcessId, handle: Handle) -> Result<&Entry, Error> {
        let index = self.resolve(process, handle)?;
        self.occupant_at(index)
            .map(|occupant| &occupant.entry)
            .ok_or(Error::InvalidHandle)
    }

    /// What `handle` names for `process`, for modification.
    ///
    /// # Errors
    ///
    /// As [`HandleArena::lookup`].
    pub fn lookup_mut(&mut self, process: ProcessId, handle: Handle) -> Result<&mut Entry, Error> {
        let index = self.resolve(process, handle)?;
        self.occupant_mut_at(index)
            .map(|occupant| &mut occupant.entry)
            .ok_or(Error::InvalidHandle)
    }

    /// Installs a second handle of `process` to the same object, with
    /// `rights`, which must be a subset of what the original carries.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] as [`HandleArena::lookup`];
    /// [`Error::AccessDenied`] when the original lacks
    /// [`Rights::DUPLICATE`] or `rights` is not a subset of it;
    /// [`Error::QuotaExceeded`] or [`Error::OutOfHandles`] as
    /// [`HandleArena::insert`]. The original is unchanged in every case.
    pub fn duplicate(
        &mut self,
        process: ProcessId,
        list: &mut HandleList,
        handle: Handle,
        rights: Rights,
    ) -> Result<Handle, Error> {
        let original = *self.lookup(process, handle)?;
        if !original.rights.contains(Rights::DUPLICATE) {
            return Err(Error::AccessDenied);
        }
        if !rights.is_subset_of(original.rights) {
            return Err(Error::AccessDenied);
        }
        let entry = Entry {
            object: original.object,
            rights,
            badge: original.badge,
        };
        self.insert(process, list, entry)
    }

    /// Removes `handle` from `process` and returns what it named.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidHandle`] as [`HandleArena::lookup`]; a second close
    /// of the same handle therefore fails.
    pub fn close(
        &mut self,
        process: ProcessId,
        list: &mut HandleList,
        handle: Handle,
    ) -> Result<Entry, Error> {
        let index = self.resolve(process, handle)?;
        self.detach(index, list).ok_or(Error::InvalidHandle)
    }

    /// Unchains the slot at `index` from its owner's list and frees it.
    fn detach(&mut self, index: u32, list: &mut HandleList) -> Option<Entry> {
        let occupant = self.slot_mut(index)?.occupant.take()?;
        if let Some(previous) = occupant.previous.index()
            && let Some(before) = self.occupant_mut_at(previous)
        {
            before.next = occupant.next;
        }
        if let Some(next) = occupant.next.index()
            && let Some(after) = self.occupant_mut_at(next)
        {
            after.previous = occupant.previous;
        }
        if occupant.previous.is_none() {
            list.head = occupant.next;
        }
        list.count = list.count.saturating_sub(1);
        self.live = self.live.saturating_sub(1);
        self.put_free_slot(index);
        Some(occupant.entry)
    }

    /// Closes every handle of `process`, walking its chain rather than the
    /// arena, and returns how many it closed.
    pub fn close_all(&mut self, list: &mut HandleList) -> u32 {
        let mut closed: u32 = 0;
        while let Some(index) = list.head.index() {
            if self.detach(index, list).is_none() {
                // The head names no occupant, which the invariants rule
                // out; drop the list rather than looping.
                list.head = Link::NONE;
                break;
            }
            closed = closed.saturating_add(1);
        }
        closed
    }

    /// The handles of `process`, most recently installed first.
    #[must_use]
    pub const fn handles_of(&self, list: HandleList) -> Handles<'_, N> {
        Handles {
            arena: self,
            next: list.head,
        }
    }

    /// The generation a slot carries, so that the wrap-around can be
    /// observed.
    #[cfg(test)]
    pub(crate) fn generation_of(&self, index: u32) -> Option<u32> {
        self.slot(index).map(|slot| slot.generation)
    }

    /// Sets the generation of a free slot, so that the wrap-around can be
    /// reached without closing a handle `u32::MAX` times.
    #[cfg(test)]
    pub(crate) fn set_generation(&mut self, index: u32, generation: u32) -> bool {
        match self.slot_mut(index) {
            Some(slot) if slot.occupant.is_none() => {
                slot.generation = generation;
                true
            }
            _ => false,
        }
    }
}

/// The handles of one process, as [`HandleArena::handles_of`] walks them.
#[derive(Debug)]
pub struct Handles<'a, const N: usize> {
    arena: &'a HandleArena<N>,
    next: Link,
}

impl<const N: usize> Iterator for Handles<'_, N> {
    type Item = (Handle, Entry);

    fn next(&mut self) -> Option<Self::Item> {
        let index = self.next.index()?;
        let slot = self.arena.slot(index)?;
        let occupant = slot.occupant.as_ref()?;
        self.next = occupant.next;
        let handle = Handle::new(index, slot.generation)?;
        Some((handle, occupant.entry))
    }
}
