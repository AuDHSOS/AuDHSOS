// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A fixed-capacity pool of kernel objects with generation-checked ids.
//!
//! Invariants: every slot is either free, and then either linked into the
//! free list exactly once or above the high-water mark, or occupied with at
//! least one reference; a generation that was handed out is never zero and
//! grows by one on every allocation, so that an id of a released object is
//! rejected once the slot is reused; released slots are handed out again in
//! the order in which they were released, before slots that were never
//! used.
//!
//! An empty pool is all zeros and its constructor is `const` (D-66). That
//! is why the generation counts up in [`Pool::allocate`] rather than
//! starting at one, and why the free list starts empty with a high-water
//! mark rather than linked through every slot: a pool that is all zeros
//! reaches the `.bss` of the kernel image without being built on a stack
//! first, and the kernel's pools are far larger than the boot stack.

use core::fmt;
use core::marker::PhantomData;

/// The name of one object in a [`Pool`]. The type parameter keeps ids of
/// different pools apart; it carries no value.
pub struct ObjectId<T> {
    index: u32,
    generation: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> ObjectId<T> {
    /// The id with the given slot index and generation.
    #[must_use]
    pub const fn new(index: u32, generation: u32) -> Self {
        ObjectId {
            index,
            generation,
            _marker: PhantomData,
        }
    }

    /// The slot index.
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// The generation the slot had when the id was handed out.
    #[must_use]
    pub const fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> Clone for ObjectId<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ObjectId<T> {}

impl<T> PartialEq for ObjectId<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for ObjectId<T> {}

impl<T> core::hash::Hash for ObjectId<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for ObjectId<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({}, gen {})", self.index, self.generation)
    }
}

/// Why a pool operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoolError {
    /// Every slot is occupied.
    Exhausted,
    /// The id names no live object: the index is out of range, the slot is
    /// free, or the generation belongs to an earlier occupant.
    StaleId,
    /// The reference count is at its maximum.
    RefOverflow,
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolError::Exhausted => f.write_str("the pool is full"),
            PoolError::StaleId => f.write_str("the id names no live object"),
            PoolError::RefOverflow => f.write_str("the reference count is at its maximum"),
        }
    }
}

impl From<PoolError> for audhsos_abi::Error {
    fn from(error: PoolError) -> Self {
        match error {
            PoolError::Exhausted => audhsos_abi::Error::PoolExhausted,
            PoolError::StaleId => audhsos_abi::Error::InvalidHandle,
            PoolError::RefOverflow => audhsos_abi::Error::QuotaExceeded,
        }
    }
}

/// What an occupied slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Occupant<T> {
    refs: u32,
    value: T,
}

/// One slot of a pool. A slot is free exactly when it has no occupant;
/// `next_free` links the free slots in the order in which they were
/// released. [`Slot::FREE`] is all zeros, which is what puts a fresh pool
/// into the `.bss`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Slot<T> {
    generation: u32,
    next_free: Option<u32>,
    occupant: Option<Occupant<T>>,
}

impl<T> Slot<T> {
    /// A slot that was never used: no occupant, no link, and a generation
    /// of zero, which [`Pool::allocate`] raises before it hands one out.
    /// Every field is zero, so an array of these is `.bss`.
    const FREE: Slot<T> = Slot {
        generation: 0,
        next_free: None,
        occupant: None,
    };
}

/// `N` slots for values of type `T`.
#[derive(Debug)]
pub struct Pool<T, const N: usize> {
    slots: [Slot<T>; N],
    free_head: Option<u32>,
    free_tail: Option<u32>,
    high_water: u32,
    live: u32,
}

impl<T, const N: usize> Default for Pool<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Pool<T, N> {
    /// An empty pool. Every slot is free: none has been used, so the free
    /// list is empty and the high-water mark stands at zero. The value is
    /// all zeros and the constructor is `const`, so a `static` pool costs
    /// the image nothing but `.bss` (D-66).
    #[must_use]
    pub const fn new() -> Self {
        Pool {
            slots: [const { Slot::FREE }; N],
            free_head: None,
            free_tail: None,
            high_water: 0,
            live: 0,
        }
    }

    /// The number of slots.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        u32::try_from(N).unwrap_or(u32::MAX)
    }

    /// The number of occupied slots.
    #[must_use]
    pub const fn live(&self) -> u32 {
        self.live
    }

    /// `true` if no slot is occupied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn slot(&self, index: u32) -> Option<&Slot<T>> {
        self.slots.get(usize::try_from(index).ok()?)
    }

    fn slot_mut(&mut self, index: u32) -> Option<&mut Slot<T>> {
        self.slots.get_mut(usize::try_from(index).ok()?)
    }

    /// The occupant `id` names.
    fn occupant(&self, id: ObjectId<T>) -> Result<&Occupant<T>, PoolError> {
        let slot = self.slot(id.index).ok_or(PoolError::StaleId)?;
        if slot.generation != id.generation {
            return Err(PoolError::StaleId);
        }
        slot.occupant.as_ref().ok_or(PoolError::StaleId)
    }

    /// The occupant `id` names, for modification.
    fn occupant_mut(&mut self, id: ObjectId<T>) -> Result<&mut Occupant<T>, PoolError> {
        let slot = self.slot_mut(id.index).ok_or(PoolError::StaleId)?;
        if slot.generation != id.generation {
            return Err(PoolError::StaleId);
        }
        slot.occupant.as_mut().ok_or(PoolError::StaleId)
    }

    /// Stores `value` in a free slot and returns its id with one reference.
    ///
    /// # Errors
    ///
    /// [`PoolError::Exhausted`] if every slot is occupied.
    pub fn allocate(&mut self, value: T) -> Result<ObjectId<T>, PoolError> {
        let index = self.take_free_slot()?;
        let slot = self.slot_mut(index).ok_or(PoolError::Exhausted)?;
        // A generation of zero is never handed out, so that an id built
        // from a slot that was never used names nothing.
        let generation = match slot.generation.wrapping_add(1) {
            0 => 1,
            generation => generation,
        };
        slot.generation = generation;
        slot.next_free = None;
        slot.occupant = Some(Occupant { refs: 1, value });
        self.live = self.live.saturating_add(1);
        Ok(ObjectId::new(index, generation))
    }

    /// The index of a slot to occupy: the head of the free list, which
    /// holds the slots that were released, or the next slot that was never
    /// used.
    fn take_free_slot(&mut self) -> Result<u32, PoolError> {
        if let Some(index) = self.free_head {
            let next_free = self.slot(index).and_then(|slot| slot.next_free);
            self.free_head = next_free;
            if next_free.is_none() {
                self.free_tail = None;
            }
            return Ok(index);
        }
        if usize::try_from(self.high_water).is_ok_and(|used| used < N) {
            let index = self.high_water;
            self.high_water = self.high_water.saturating_add(1);
            return Ok(index);
        }
        Err(PoolError::Exhausted)
    }

    /// The value `id` names.
    ///
    /// # Errors
    ///
    /// [`PoolError::StaleId`] if `id` names no live object.
    pub fn get(&self, id: ObjectId<T>) -> Result<&T, PoolError> {
        self.occupant(id).map(|occupant| &occupant.value)
    }

    /// The value `id` names, for modification.
    ///
    /// # Errors
    ///
    /// [`PoolError::StaleId`] if `id` names no live object.
    pub fn get_mut(&mut self, id: ObjectId<T>) -> Result<&mut T, PoolError> {
        self.occupant_mut(id).map(|occupant| &mut occupant.value)
    }

    /// The number of references to the object `id` names.
    ///
    /// # Errors
    ///
    /// [`PoolError::StaleId`] if `id` names no live object.
    pub fn references(&self, id: ObjectId<T>) -> Result<u32, PoolError> {
        self.occupant(id).map(|occupant| occupant.refs)
    }

    /// Adds one reference to the object `id` names.
    ///
    /// # Errors
    ///
    /// [`PoolError::StaleId`] if `id` names no live object;
    /// [`PoolError::RefOverflow`] if the count is at its maximum.
    pub fn retain(&mut self, id: ObjectId<T>) -> Result<(), PoolError> {
        let occupant = self.occupant_mut(id)?;
        occupant.refs = occupant.refs.checked_add(1).ok_or(PoolError::RefOverflow)?;
        Ok(())
    }

    /// Drops one reference to the object `id` names and says whether that
    /// was the last one. The slot is then free again and goes to the end of
    /// the free list; its generation rises when it is handed out next,
    /// which is what makes the old id stale.
    ///
    /// The object itself does not come back out. A pool holds what nothing
    /// else does, so there is nobody for it to go to, and an object of this
    /// kernel is large enough that handing one back costs more of a kernel
    /// stack than any caller was ever going to get out of it (D-73).
    ///
    /// # Errors
    ///
    /// [`PoolError::StaleId`] if `id` names no live object.
    pub fn release(&mut self, id: ObjectId<T>) -> Result<bool, PoolError> {
        let occupant = self.occupant_mut(id)?;
        if occupant.refs > 1 {
            occupant.refs = occupant.refs.saturating_sub(1);
            return Ok(false);
        }
        let slot = self.slot_mut(id.index).ok_or(PoolError::StaleId)?;
        let freed = slot.occupant.take().is_some();
        slot.next_free = None;
        self.live = self.live.saturating_sub(1);
        match self.free_tail {
            Some(tail) => self.link_free(tail, id.index),
            None => self.free_head = Some(id.index),
        }
        self.free_tail = Some(id.index);
        Ok(freed)
    }

    /// Does something to the object `id` names, if the pool holds it, and
    /// says whether it did.
    ///
    /// Every caller that changes an object it has already looked up goes
    /// through this: the lookup cannot fail there, and writing it out at
    /// each of them would be a branch per caller that nothing can reach.
    pub fn with(&mut self, id: ObjectId<T>, body: impl FnOnce(&mut T)) -> bool {
        match self.get_mut(id) {
            Ok(value) => {
                body(value);
                true
            }
            Err(_) => false,
        }
    }

    /// Every live object with its id, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (ObjectId<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            let occupant = slot.occupant.as_ref()?;
            let index = u32::try_from(index).ok()?;
            Some((ObjectId::new(index, slot.generation), &occupant.value))
        })
    }

    /// The id of every live object, in slot order.
    pub fn ids(&self) -> impl Iterator<Item = ObjectId<T>> + '_ {
        self.iter().map(|(id, _)| id)
    }

    /// Appends the slot `index` behind the slot `tail` of the free list.
    /// A `tail` outside the pool leaves the list unchanged.
    pub(crate) fn link_free(&mut self, tail: u32, index: u32) {
        if let Some(slot) = self.slot_mut(tail) {
            slot.next_free = Some(index);
        }
    }

    /// The generation a slot carries, for the generation wrap-around test.
    #[cfg(test)]
    pub(crate) fn generation_of(&self, index: u32) -> Option<u32> {
        self.slot(index).map(|slot| slot.generation)
    }

    /// Sets the reference count of an occupied slot, so that the overflow
    /// can be reached without retaining a slot `u32::MAX` times.
    #[cfg(test)]
    pub(crate) fn set_references(&mut self, index: u32, references: u32) -> bool {
        match self.slot_mut(index).and_then(|slot| slot.occupant.as_mut()) {
            Some(occupant) => {
                occupant.refs = references;
                true
            }
            None => false,
        }
    }

    /// Sets the generation of a free slot, so that the wrap-around can be
    /// reached without releasing a slot `u32::MAX` times.
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
