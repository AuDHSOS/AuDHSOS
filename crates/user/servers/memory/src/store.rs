// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Who holds which memory object, which ones are free, and what is zeroed
//! when.
//!
//! The free list holds whole memory objects, sorted by the physical address
//! they begin at. A request is served from the first object that can hold
//! it at the alignment it asks for; what lies before and behind the piece
//! that goes out is split off and stays free. A release puts the object
//! back and joins it to the neighbours it touches, which is what keeps the
//! store from grinding its objects down to single pages (D-88).
//!
//! Zeroing happens twice over the same bytes and both are meant. The pass
//! on return is what keeps what a client wrote out of free memory; the pass
//! on hand-out is what makes the guarantee to the client hold whatever the
//! history of those bytes is (D-12). The free list is therefore zero by
//! construction, and the second pass is the one a reader of the code can
//! check against the promise rather than against an argument.
//!
//! Invariants: the free list is sorted by physical address and no two of
//! its objects overlap; a live object overlaps no free object and no other
//! live object; every object that goes out has been zeroed since it was
//! last written to.

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_abi::{Error, Handle};
use audhsos_collections::ArrayVec;

use crate::pages::Pages;

/// One memory object, with where it lies and how large it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Object {
    /// The capability to the object.
    pub handle: Handle,
    /// The physical address it begins at.
    pub start: u64,
    /// How many bytes it covers.
    pub len: u64,
}

impl Object {
    /// The first physical address above the object.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }
}

/// One object a client holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Held {
    /// The object.
    pub object: Object,
    /// The badge of the client that holds it.
    pub owner: u64,
}

/// The memory the server has and the memory it has given out.
///
/// `FREE` is how many pieces the free memory may be in and `LIVE` how many
/// objects may be out at once. Both are the whole memory cost of the store.
#[derive(Debug)]
pub struct Store<const FREE: usize, const LIVE: usize> {
    free: ArrayVec<Object, FREE>,
    live: ArrayVec<Held, LIVE>,
}

impl<const FREE: usize, const LIVE: usize> Default for Store<FREE, LIVE> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const FREE: usize, const LIVE: usize> Store<FREE, LIVE> {
    /// A store that holds no memory.
    #[must_use]
    pub const fn new() -> Self {
        Store {
            free: ArrayVec::new(),
            live: ArrayVec::new(),
        }
    }

    /// How many bytes are free.
    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.free
            .iter()
            .fold(0u64, |total, object| total.saturating_add(object.len))
    }

    /// How many bytes are out.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.live
            .iter()
            .fold(0u64, |total, held| total.saturating_add(held.object.len))
    }

    /// How many pieces the free memory is in.
    #[must_use]
    pub const fn free_objects(&self) -> usize {
        self.free.len()
    }

    /// How many objects are out.
    #[must_use]
    pub const fn live_objects(&self) -> usize {
        self.live.len()
    }

    /// The free objects, lowest address first.
    pub fn free(&self) -> impl Iterator<Item = Object> + '_ {
        self.free.iter().copied()
    }

    /// The objects that are out.
    pub fn held(&self) -> impl Iterator<Item = Held> + '_ {
        self.live.iter().copied()
    }

    /// Takes memory over: zeroes it and puts it in the free list.
    ///
    /// This is what the root task hands the server at the start, and what
    /// D-12 means by zeroing when memory is returned — the root task is
    /// returning it.
    ///
    /// # Errors
    ///
    /// [`Error::PoolExhausted`] when the free list has no room, in which
    /// case the object is zeroed and dropped rather than held half; the
    /// errors of the zeroing pass and of a join.
    pub fn adopt(&mut self, pages: &mut impl Pages, object: Object) -> Result<(), Error> {
        wipe(pages, object)?;
        self.put_free(pages, object)
    }

    /// Hands out `len` bytes aligned to `align`, to the client `owner`.
    ///
    /// The length is rounded up to whole pages, because a mapping is made
    /// of pages and a client that was given part of one could read what its
    /// neighbour wrote into the rest.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidArgument`] for a length of zero, for an alignment
    /// that is zero or no power of two, or for an owner of zero;
    /// [`Error::OutOfKernelMemory`] when no free object holds the request;
    /// [`Error::PoolExhausted`] when a table is full; the errors of the
    /// split and of the zeroing pass.
    pub fn allocate(
        &mut self,
        pages: &mut impl Pages,
        owner: u64,
        len: u64,
        align: u64,
    ) -> Result<Object, Error> {
        if owner == 0 || len == 0 || align == 0 || !align.is_power_of_two() {
            return Err(Error::InvalidArgument);
        }
        let wanted = len
            .checked_next_multiple_of(PAGE_SIZE)
            .ok_or(Error::InvalidArgument)?;
        if self.live.is_full() {
            return Err(Error::PoolExhausted);
        }
        let (index, start, object) = self
            .first_fit(wanted, align)
            .ok_or(Error::OutOfKernelMemory)?;
        // Two splits at most, and each of them needs a slot the free list
        // may not have. Both are refused before anything is cut.
        let leading = start.wrapping_sub(object.start);
        let trailing = object.end().wrapping_sub(start.wrapping_add(wanted));
        let extra = usize::from(leading > 0).wrapping_add(usize::from(trailing > 0));
        if self.free.len().wrapping_add(extra) > FREE.wrapping_add(1) {
            return Err(Error::PoolExhausted);
        }

        let _taken = self.free.remove(index);
        let mut piece = object;
        if leading > 0 {
            let upper = pages.split(piece.handle, leading)?;
            let below = Object {
                handle: piece.handle,
                start: piece.start,
                len: leading,
            };
            piece = Object {
                handle: upper,
                start,
                len: piece.len.wrapping_sub(leading),
            };
            self.insert_free(below)?;
        }
        if trailing > 0 {
            let upper = pages.split(piece.handle, wanted)?;
            let above = Object {
                handle: upper,
                start: start.wrapping_add(wanted),
                len: trailing,
            };
            piece.len = wanted;
            self.insert_free(above)?;
        }

        wipe(pages, piece)?;
        self.live
            .push(Held {
                object: piece,
                owner,
            })
            .map_err(|_| Error::PoolExhausted)?;
        Ok(piece)
    }

    /// Takes an object back from the client that holds it.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a handle this store did not hand out, a
    /// second release of the same one included;
    /// [`Error::AccessDenied`] when another client holds it; the errors of
    /// the zeroing pass and of a join. Nothing is zeroed in the two cases
    /// that are refused.
    pub fn release(
        &mut self,
        pages: &mut impl Pages,
        owner: u64,
        handle: Handle,
    ) -> Result<Object, Error> {
        let index = self.position(handle).ok_or(Error::NotFound)?;
        let held = *self.live.get(index).ok_or(Error::NotFound)?;
        if held.owner != owner {
            return Err(Error::AccessDenied);
        }
        let _returned = self.live.remove(index);
        wipe(pages, held.object)?;
        self.put_free(pages, held.object)?;
        Ok(held.object)
    }

    /// Takes back everything `owner` holds and answers with how many
    /// objects that was.
    ///
    /// This is what the root task's report of a dead client leads to. The
    /// memory of a process that is gone is memory nobody will ever return.
    ///
    /// # Errors
    ///
    /// The errors of the zeroing pass and of a join. Whatever was taken
    /// back before the error stays taken back.
    pub fn forget_client(&mut self, pages: &mut impl Pages, owner: u64) -> Result<usize, Error> {
        let mut taken = 0usize;
        while let Some(index) = self.live.iter().position(|held| held.owner == owner) {
            let held = *self.live.get(index).ok_or(Error::NotFound)?;
            let _returned = self.live.remove(index);
            wipe(pages, held.object)?;
            self.put_free(pages, held.object)?;
            taken = taken.wrapping_add(1);
        }
        Ok(taken)
    }

    /// Puts `object` in the free list and joins it to the neighbours it
    /// touches.
    fn put_free(&mut self, pages: &mut impl Pages, object: Object) -> Result<(), Error> {
        self.insert_free(object)?;
        self.join_around(pages, object.start)
    }

    /// Puts `object` in the free list, sorted by address.
    fn insert_free(&mut self, object: Object) -> Result<(), Error> {
        let at = self
            .free
            .iter()
            .position(|free| free.start > object.start)
            .unwrap_or(self.free.len());
        self.free
            .insert(at, object)
            .map_err(|_| Error::PoolExhausted)
    }

    /// Joins the free object that begins at `start` to the one below it and
    /// to the one above it, where they touch.
    fn join_around(&mut self, pages: &mut impl Pages, start: u64) -> Result<(), Error> {
        let Some(at) = self.free.iter().position(|free| free.start == start) else {
            return Ok(());
        };
        // Upwards first: joining downwards would move this object's index.
        self.join_at(pages, at)?;
        match at.checked_sub(1) {
            Some(below) => self.join_at(pages, below),
            None => Ok(()),
        }
    }

    /// The free object at `index` and the one above it, when the two lie
    /// end to end.
    fn touching(&self, index: usize) -> Option<(Object, Object)> {
        let lower = *self.free.get(index)?;
        let upper = *self.free.get(index.wrapping_add(1))?;
        (lower.end() == upper.start).then_some((lower, upper))
    }

    /// Joins the free object above `index` into the one at `index`, when
    /// the two touch.
    fn join_at(&mut self, pages: &mut impl Pages, index: usize) -> Result<(), Error> {
        let Some((lower, upper)) = self.touching(index) else {
            return Ok(());
        };
        pages.merge(lower.handle, upper.handle)?;
        let above = index.wrapping_add(1);
        let _joined = self.free.remove(above);
        // Two objects came out of the list and one goes back in, so the
        // slot is there.
        let _removed = self.free.remove(index);
        let _placed = self.free.insert(
            index,
            Object {
                handle: lower.handle,
                start: lower.start,
                len: lower.len.saturating_add(upper.len),
            },
        );
        Ok(())
    }

    /// The first free object that holds `len` bytes aligned to `align`,
    /// with the address the piece would begin at.
    fn first_fit(&self, len: u64, align: u64) -> Option<(usize, u64, Object)> {
        self.free.iter().enumerate().find_map(|(index, object)| {
            let start = object.start.checked_next_multiple_of(align)?;
            let end = start.checked_add(len)?;
            (end <= object.end()).then_some((index, start, *object))
        })
    }

    /// Where the object `handle` names stands among the live ones.
    fn position(&self, handle: Handle) -> Option<usize> {
        self.live
            .iter()
            .position(|held| held.object.handle == handle)
    }
}

/// Maps `object`, fills it with zeros, and takes it out again.
///
/// One pass over the whole object, so that what a recording double sees is
/// one zeroing of exactly the range the object covers.
fn wipe(pages: &mut impl Pages, object: Object) -> Result<(), Error> {
    let address = pages.map(object.handle, object.len)?;
    let zeroed = pages.zero(address, object.len);
    // The object comes out of the address space whether the fill worked or
    // not: a window left behind would be memory this server maps and does
    // not know it maps.
    let unmapped = pages.unmap(address, object.len);
    zeroed.and(unmapped)
}
