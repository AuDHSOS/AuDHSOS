// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R7: rasterized coverage kept against the key of D-184.
//!
//! Invariants: the bytes of two live entries never overlap, because a block is
//! placed only where no live entry covers it; a hit returns the coverage its
//! key was stored with, because a candidate is confirmed by the whole key and
//! then by the variation coordinates themselves, which the block carries.

use text_core::Fixed;

use crate::error::RasterError;

/// What identifies one rasterized glyph (D-184).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheKey {
    /// The immutable font-set snapshot (D-162).
    pub generation: u64,
    /// The ordered face index in the role chain.
    pub face: u32,
    /// The face-local glyph.
    pub glyph: u16,
    /// The run scale the glyph was drawn at.
    pub size: Fixed,
    /// Which of the four horizontal positions the origin took (D-174).
    pub subpixel: u8,
}

/// One rasterized glyph: its coverage and where that coverage sits against the
/// quantized origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph<'a> {
    /// Columns of coverage.
    pub width: u32,
    /// Rows of coverage.
    pub height: u32,
    /// Pixels from the origin to the first column.
    pub left: i32,
    /// Pixels from the origin to the first row.
    pub top: i32,
    /// `width * height` coverage bytes, one row after another.
    pub coverage: &'a [u8],
}

/// One record of the caller's entry table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    key: CacheKey,
    digest: u64,
    axes: usize,
    offset: usize,
    length: usize,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
}

/// Coverage stored in bytes the caller owns, evicted first in, first out.
///
/// The memory bound is exactly two numbers the caller states: the length of
/// the ring and the number of entry records. Nothing else grows.
#[derive(Debug)]
pub struct GlyphCache<'a> {
    ring: &'a mut [u8],
    entries: &'a mut [Entry],
    head: usize,
    first: usize,
    count: usize,
    generation: u64,
}

/// Bytes one variation coordinate occupies in a ring block.
const COORDINATE: usize = 8;

impl<'a> GlyphCache<'a> {
    /// A cache over the caller's ring and entry table.
    pub const fn new(ring: &'a mut [u8], entries: &'a mut [Entry]) -> Self {
        Self {
            ring,
            entries,
            head: 0,
            first: 0,
            count: 0,
            generation: 0,
        }
    }

    /// How many glyphs are stored.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.count
    }

    /// Whether no glyph is stored.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Drop every entry. This is what a changed generation costs.
    pub const fn clear(&mut self) {
        self.head = 0;
        self.first = 0;
        self.count = 0;
    }

    /// The coverage stored for this key and instance, if any.
    #[must_use]
    pub fn get(&self, key: CacheKey, coordinates: &[Fixed]) -> Option<Glyph<'_>> {
        let digest = digest(key, coordinates);
        let index = self.find(digest, key, coordinates)?;
        let entry = self.entries.get(index)?;
        let start = entry
            .offset
            .checked_add(entry.axes.checked_mul(COORDINATE)?)?;
        Some(Glyph {
            width: entry.width,
            height: entry.height,
            left: entry.left,
            top: entry.top,
            coverage: self
                .ring
                .get(start..entry.offset.checked_add(entry.length)?)?,
        })
    }

    /// Store one glyph's coverage.
    ///
    /// A glyph whose block is larger than the ring, or than one entry record
    /// can be spared for, is not stored and the caller draws it every time.
    /// # Errors
    /// Returns `Overflow` when a length leaves `usize` and `BufferTooSmall`
    /// when the coverage is shorter than its dimensions state.
    pub fn insert(
        &mut self,
        key: CacheKey,
        coordinates: &[Fixed],
        glyph: Glyph<'_>,
    ) -> Result<(), RasterError> {
        let pixels = usize::try_from(glyph.width)
            .map_err(|_| RasterError::Overflow)?
            .checked_mul(usize::try_from(glyph.height).map_err(|_| RasterError::Overflow)?)
            .ok_or(RasterError::Overflow)?;
        if glyph.coverage.len() < pixels {
            return Err(RasterError::BufferTooSmall);
        }
        if key.generation != self.generation {
            self.clear();
            self.generation = key.generation;
        }
        let axes = coordinates.len();
        let length = axes
            .checked_mul(COORDINATE)
            .and_then(|head| head.checked_add(pixels))
            .ok_or(RasterError::Overflow)?;
        if length > self.ring.len() || self.entries.is_empty() {
            return Ok(());
        }
        let Some(offset) = self.allocate(length) else {
            return Ok(());
        };
        let mut at = offset;
        for coordinate in coordinates {
            let end = at.checked_add(COORDINATE).ok_or(RasterError::Overflow)?;
            self.ring
                .get_mut(at..end)
                .ok_or(RasterError::Overflow)?
                .copy_from_slice(&coordinate.bits().to_le_bytes());
            at = end;
        }
        let end = at.checked_add(pixels).ok_or(RasterError::Overflow)?;
        self.ring
            .get_mut(at..end)
            .ok_or(RasterError::Overflow)?
            .copy_from_slice(glyph.coverage.get(..pixels).ok_or(RasterError::Overflow)?);
        self.push(Entry {
            key,
            digest: digest(key, coordinates),
            axes,
            offset,
            length,
            width: glyph.width,
            height: glyph.height,
            left: glyph.left,
            top: glyph.top,
        })
    }

    /// The index of a live entry with this digest, key and instance.
    fn find(&self, digest: u64, key: CacheKey, coordinates: &[Fixed]) -> Option<usize> {
        for step in 0..self.count {
            let index = self.slot(step)?;
            let entry = self.entries.get(index)?;
            if entry.digest != digest || entry.key != key || entry.axes != coordinates.len() {
                continue;
            }
            if self.coordinates_agree(entry, coordinates)? {
                return Some(index);
            }
        }
        None
    }

    /// Whether the coordinates the block carries are the ones asked for, which
    /// is what keeps a digest collision from returning another instance.
    fn coordinates_agree(&self, entry: &Entry, coordinates: &[Fixed]) -> Option<bool> {
        for (axis, coordinate) in coordinates.iter().enumerate() {
            let at = entry.offset.checked_add(axis.checked_mul(COORDINATE)?)?;
            let stored = self.ring.get(at..at.checked_add(COORDINATE)?)?;
            if stored != coordinate.bits().to_le_bytes() {
                return Some(false);
            }
        }
        Some(true)
    }

    /// The entry table index of the `step`-th oldest live entry.
    fn slot(&self, step: usize) -> Option<usize> {
        let len = self.entries.len();
        if len == 0 {
            return None;
        }
        self.first.checked_add(step)?.checked_rem(len)
    }

    /// Reserve `length` bytes, evicting the oldest entries until they fit.
    fn allocate(&mut self, length: usize) -> Option<usize> {
        loop {
            let start = match self.head.checked_add(length) {
                Some(end) if end <= self.ring.len() => self.head,
                _ => 0,
            };
            if self.free(start, length)? {
                self.head = start.checked_add(length)?;
                return Some(start);
            }
            if !self.evict() {
                return None;
            }
        }
    }

    /// Whether no live entry covers `[start, start + length)`.
    fn free(&self, start: usize, length: usize) -> Option<bool> {
        let end = start.checked_add(length)?;
        for step in 0..self.count {
            let entry = self.entries.get(self.slot(step)?)?;
            let entry_end = entry.offset.checked_add(entry.length)?;
            if entry_end > start && entry.offset < end {
                return Some(false);
            }
        }
        Some(true)
    }

    /// Drop the oldest entry. Returns whether one was there to drop.
    fn evict(&mut self) -> bool {
        if self.count == 0 {
            return false;
        }
        let len = self.entries.len();
        self.first = self.first.wrapping_add(1).checked_rem(len).unwrap_or(0);
        self.count = self.count.saturating_sub(1);
        true
    }

    /// Append one entry, evicting the oldest when the table is full.
    fn push(&mut self, entry: Entry) -> Result<(), RasterError> {
        if self.count == self.entries.len() {
            self.evict();
        }
        let index = self.slot(self.count).ok_or(RasterError::Overflow)?;
        *self.entries.get_mut(index).ok_or(RasterError::Overflow)? = entry;
        self.count = self.count.checked_add(1).ok_or(RasterError::Overflow)?;
        Ok(())
    }
}

/// FNV-1a-64 over the key and the variation coordinates, which narrows a
/// lookup to one comparison per live entry.
fn digest(key: CacheKey, coordinates: &[Fixed]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    eat(&key.generation.to_le_bytes());
    eat(&key.face.to_le_bytes());
    eat(&key.glyph.to_le_bytes());
    eat(&key.size.bits().to_le_bytes());
    eat(&[key.subpixel]);
    for coordinate in coordinates {
        eat(&coordinate.bits().to_le_bytes());
    }
    hash
}
