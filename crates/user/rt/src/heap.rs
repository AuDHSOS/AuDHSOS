// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The heap of a user program: offsets in a region of memory the program
//! owns, and nothing else.
//!
//! The allocator never sees a pointer. It is handed the length of an arena
//! and answers with offsets into it, so the whole of it is arithmetic that
//! runs on the host under test. Whoever owns the memory turns an offset
//! into an address; that step is one addition, and it lives in the adapter
//! crate that owns the mapping.
//!
//! One free list, first fit, and coalescing on release — not size classes.
//! Size classes are faster and cannot do what
//! [6.6.12](../../../../docs/06-testing-strategy.md#6612-userland-allocator-user-rt)
//! asks of this allocator: two neighbours that are released have to become
//! one block large enough for their sum, and a class list hands back two
//! blocks of the class they came from however they lie in memory.
//!
//! Invariants: every live block lies inside the arena and is disjoint from
//! every other live block and from every free extent; the free extents are
//! sorted, disjoint, and separated by at least one live byte, so releasing
//! everything leaves exactly one extent covering the whole arena; an
//! operation that returns an error has changed nothing.

use audhsos_collections::ArrayVec;

/// A half-open range of the arena that nothing holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Extent {
    /// First byte of the range.
    pub start: u64,
    /// One past the last byte.
    pub end: u64,
}

impl Extent {
    /// How many bytes the extent covers.
    #[must_use]
    pub const fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// `true` for a range of no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

/// A block the allocator handed out and nobody has given back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Block {
    /// Where the block starts in the arena.
    pub offset: u64,
    /// How many bytes were asked for.
    pub len: u64,
}

/// Why an allocation or a release did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AllocError {
    /// A length of zero. This allocator refuses it rather than handing out
    /// a unique offset for it: a block of no bytes has nothing a caller may
    /// read or write, and refusing it keeps the block table free of entries
    /// that describe nothing.
    ZeroLength,
    /// An alignment that is zero or not a power of two.
    Alignment(u64),
    /// The length and the alignment together do not fit into a `u64`.
    Overflow,
    /// No free extent holds the request.
    OutOfMemory,
    /// The block table is full. Memory may still be free; what ran out is
    /// the bookkeeping.
    TooManyBlocks,
    /// The free list is full, so the request cannot be recorded. Memory may
    /// still be free; what ran out is the number of pieces it is in.
    TooFragmented,
    /// The offset names no block this allocator handed out.
    NotAllocated(u64),
    /// An arena that would become smaller than it is.
    Shrink {
        /// The length the arena has.
        current: u64,
        /// The length that was asked for.
        requested: u64,
    },
}

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AllocError::ZeroLength => f.write_str("a block of zero bytes"),
            AllocError::Alignment(align) => write!(f, "{align} is no power of two"),
            AllocError::Overflow => f.write_str("the length and the alignment do not fit a word"),
            AllocError::OutOfMemory => f.write_str("no free extent holds the request"),
            AllocError::TooManyBlocks => f.write_str("the block table is full"),
            AllocError::TooFragmented => f.write_str("the free list is full"),
            AllocError::NotAllocated(offset) => write!(f, "{offset:#x} names no live block"),
            AllocError::Shrink { current, requested } => {
                write!(f, "an arena of {current} bytes cannot become {requested}")
            }
        }
    }
}

/// The allocator over one arena.
///
/// `BLOCKS` is how many live blocks it can remember and `EXTENTS` how many
/// pieces the free space may be in. Both are the caller's choice, because
/// the tables are the whole memory cost of the allocator and a program that
/// allocates twice does not want the tables of one that allocates ten
/// thousand times.
#[derive(Debug)]
pub struct Allocator<const BLOCKS: usize, const EXTENTS: usize> {
    arena_len: u64,
    free: ArrayVec<Extent, EXTENTS>,
    live: ArrayVec<Block, BLOCKS>,
}

impl<const BLOCKS: usize, const EXTENTS: usize> Default for Allocator<BLOCKS, EXTENTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const BLOCKS: usize, const EXTENTS: usize> Allocator<BLOCKS, EXTENTS> {
    /// An allocator over an arena of no bytes.
    ///
    /// Every allocator starts this way, because the memory a program's heap
    /// lives in is mapped after the program has started; [`grow`](Self::grow)
    /// is what hands it over.
    #[must_use]
    pub const fn new() -> Self {
        Allocator {
            arena_len: 0,
            free: ArrayVec::new(),
            live: ArrayVec::new(),
        }
    }

    /// How many bytes the arena covers.
    #[must_use]
    pub const fn arena_len(&self) -> u64 {
        self.arena_len
    }

    /// How many bytes are free, over all extents.
    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.free
            .iter()
            .fold(0u64, |total, extent| total.saturating_add(extent.len()))
    }

    /// How many bytes the live blocks ask for. It is not the difference to
    /// [`free_bytes`](Self::free_bytes): the padding an alignment needed
    /// belongs to neither.
    #[must_use]
    pub fn live_bytes(&self) -> u64 {
        self.live
            .iter()
            .fold(0u64, |total, block| total.saturating_add(block.len))
    }

    /// How many live blocks there are.
    #[must_use]
    pub const fn live_blocks(&self) -> usize {
        self.live.len()
    }

    /// How many pieces the free space is in.
    #[must_use]
    pub const fn free_extents(&self) -> usize {
        self.free.len()
    }

    /// The free extents, lowest first.
    pub fn extents(&self) -> impl Iterator<Item = Extent> + '_ {
        self.free.iter().copied()
    }

    /// The live blocks, lowest offset first.
    pub fn blocks(&self) -> impl Iterator<Item = Block> + '_ {
        self.live.iter().copied()
    }

    /// Extends the arena to `new_len`.
    ///
    /// The new bytes lie at the end and join the last extent when it
    /// reaches them.
    ///
    /// # Errors
    ///
    /// [`AllocError::Shrink`] for a length below the current one;
    /// [`AllocError::TooFragmented`] when the new bytes need an extent of
    /// their own and the free list is full. In both cases the arena is
    /// unchanged.
    pub fn grow(&mut self, new_len: u64) -> Result<(), AllocError> {
        if new_len < self.arena_len {
            return Err(AllocError::Shrink {
                current: self.arena_len,
                requested: new_len,
            });
        }
        if new_len == self.arena_len {
            return Ok(());
        }
        let added = Extent {
            start: self.arena_len,
            end: new_len,
        };
        let last = self.free.len().checked_sub(1);
        match last.and_then(|index| self.free.get_mut(index)) {
            Some(extent) if extent.end == added.start => extent.end = added.end,
            _ => self
                .free
                .push(added)
                .map_err(|_| AllocError::TooFragmented)?,
        }
        self.arena_len = new_len;
        Ok(())
    }

    /// Hands out `len` bytes aligned to `align` and answers with the offset
    /// of the first one.
    ///
    /// The search is first fit over the free extents, lowest first.
    ///
    /// # Errors
    ///
    /// [`AllocError::ZeroLength`] for a length of zero;
    /// [`AllocError::Alignment`] for an alignment that is zero or no power
    /// of two; [`AllocError::Overflow`] when the length and the alignment
    /// do not fit a word together; [`AllocError::OutOfMemory`] when no
    /// extent holds the request; [`AllocError::TooManyBlocks`] and
    /// [`AllocError::TooFragmented`] when a table is full. Nothing is
    /// changed in any of those cases.
    pub fn allocate(&mut self, len: u64, align: u64) -> Result<u64, AllocError> {
        if len == 0 {
            return Err(AllocError::ZeroLength);
        }
        if align == 0 || !align.is_power_of_two() {
            return Err(AllocError::Alignment(align));
        }
        // The widest a request can be, which is what an aligned block of
        // `len` costs in its worst placement. A request that cannot be
        // expressed at all is refused here rather than by every extent in
        // turn.
        let _widest = len
            .checked_add(align.wrapping_sub(1))
            .ok_or(AllocError::Overflow)?;

        let (index, offset, extent) = self.first_fit(len, align).ok_or(AllocError::OutOfMemory)?;
        let end = offset.wrapping_add(len);
        let before = Extent {
            start: extent.start,
            end: offset,
        };
        let after = Extent {
            start: end,
            end: extent.end,
        };
        // A block taken out of the middle of an extent leaves two where one
        // was, so that case needs a slot the list may not have.
        if !before.is_empty() && !after.is_empty() && self.free.is_full() {
            return Err(AllocError::TooFragmented);
        }
        if self.live.is_full() {
            return Err(AllocError::TooManyBlocks);
        }

        // From here nothing can fail: the extent leaves the list before its
        // remains go back in, so at most one slot is needed beyond what was
        // there, and the two checks above bought it.
        let _taken = self.free.remove(index);
        if !after.is_empty() {
            let _placed = self.free.insert(index, after);
        }
        if !before.is_empty() {
            let _placed = self.free.insert(index, before);
        }
        let at = self.live_position(offset);
        let _recorded = self.live.insert(at, Block { offset, len });
        Ok(offset)
    }

    /// Gives the block at `offset` back and answers with its length.
    ///
    /// The freed range joins the extents on either side of it when they
    /// touch it, so an arena that has had everything returned is one extent
    /// again whatever order the returns came in.
    ///
    /// # Errors
    ///
    /// [`AllocError::NotAllocated`] for an offset this allocator did not
    /// hand out or has already taken back; [`AllocError::TooFragmented`]
    /// when the freed range touches neither neighbour and the free list is
    /// full. Nothing is changed in either case.
    pub fn release(&mut self, offset: u64) -> Result<u64, AllocError> {
        let (at, block) = self
            .find_live(offset)
            .ok_or(AllocError::NotAllocated(offset))?;
        let end = block.offset.wrapping_add(block.len);
        let position = self.free_position(block.offset);
        let previous = position
            .checked_sub(1)
            .and_then(|index| self.free.get(index))
            .copied();
        let next = self.free.get(position).copied();
        let joins_before = previous.is_some_and(|extent| extent.end == block.offset);
        let joins_after = next.is_some_and(|extent| extent.start == end);
        // A range that touches neither neighbour becomes an extent of its
        // own, and the list may have no slot for it.
        if !joins_before && !joins_after && self.free.is_full() {
            return Err(AllocError::TooFragmented);
        }

        // The freed range and the neighbours it touches become one extent.
        // At most two leave the list and exactly one goes back in, so from
        // here nothing can fail.
        let start = previous
            .filter(|_| joins_before)
            .map_or(block.offset, |extent| extent.start);
        let finish = next
            .filter(|_| joins_after)
            .map_or(end, |extent| extent.end);
        if joins_after {
            let _merged = self.free.remove(position);
        }
        let mut place = position;
        if joins_before {
            place = position.wrapping_sub(1);
            let _merged = self.free.remove(place);
        }
        let _placed = self.free.insert(place, Extent { start, end: finish });
        let _returned = self.live.remove(at);
        Ok(block.len)
    }

    /// The first extent that holds `len` bytes aligned to `align`, with the
    /// offset the block would start at.
    fn first_fit(&self, len: u64, align: u64) -> Option<(usize, u64, Extent)> {
        self.free.iter().enumerate().find_map(|(index, extent)| {
            let offset = extent.start.checked_next_multiple_of(align)?;
            let end = offset.checked_add(len)?;
            (end <= extent.end).then_some((index, offset, *extent))
        })
    }

    /// Where a live block with `offset` belongs in the live table, which is
    /// sorted by offset.
    fn live_position(&self, offset: u64) -> usize {
        self.live
            .iter()
            .position(|block| block.offset > offset)
            .unwrap_or(self.live.len())
    }

    /// Where an extent starting at `start` belongs in the free list.
    fn free_position(&self, start: u64) -> usize {
        self.free
            .iter()
            .position(|extent| extent.start > start)
            .unwrap_or(self.free.len())
    }

    /// The live block that starts at `offset`, with its place in the table.
    fn find_live(&self, offset: u64) -> Option<(usize, Block)> {
        self.live
            .iter()
            .enumerate()
            .find_map(|(index, block)| (block.offset == offset).then_some((index, *block)))
    }
}
