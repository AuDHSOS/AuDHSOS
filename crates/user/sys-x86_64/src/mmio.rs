// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Volatile access to a mapped device window, so that everything above this
//! crate stays safe.
//!
//! A device register is not memory that behaves: a compiler may fold,
//! reorder, or drop accesses through a plain slice, and `server-display`
//! reaches its framebuffer as one because a framebuffer tolerates that. A
//! register does not, so each accessor here is one `read_volatile` or one
//! `write_volatile` (13.7).
//!
//! Invariant: every access is checked against the length the region was
//! made with, so an offset outside it answers `None` and writes nothing.
//! That check is what makes the `unsafe` local: the precondition of each
//! access is the bound, and the bound is tested on the host (D-113).

/// A mapped device window.
///
/// The lifetime is the mapping's: a window outlives no unmap, because the
/// value borrows the bytes the mapping made reachable.
#[derive(Debug)]
pub struct Mmio<'a> {
    base: *mut u8,
    len: usize,
    mapping: core::marker::PhantomData<&'a mut [u8]>,
}

#[expect(
    clippy::needless_pass_by_ref_mut,
    reason = "`address_mut` reads only itself, and its exclusive borrow is what makes the write through the pointer it answers sound"
)]
impl<'a> Mmio<'a> {
    /// The window of `len` bytes at `base`.
    ///
    /// # Safety
    ///
    /// `base` must be the start of a mapping of at least `len` bytes that
    /// stays mapped, readable and writable, for the life of this value, and
    /// nothing else may reach those bytes while it lives.
    #[must_use]
    pub const unsafe fn new(base: *mut u8, len: usize) -> Mmio<'a> {
        Mmio {
            base,
            len,
            mapping: core::marker::PhantomData,
        }
    }

    /// The window over the bytes of `mapping`, which is the safe way in
    /// when the caller already holds the mapping as a slice.
    #[must_use]
    pub const fn of(mapping: &'a mut [u8]) -> Mmio<'a> {
        // SAFETY: a slice the caller holds is a mapping of its own length
        // that stays mapped while the borrow lives, and the borrow is
        // exclusive.
        unsafe { Mmio::new(mapping.as_mut_ptr(), mapping.len()) }
    }

    /// How many bytes the window covers.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` when the window covers no byte.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The byte at `offset`.
    #[must_use]
    pub fn read_u8(&self, offset: usize) -> Option<u8> {
        let at = self.address::<u8>(offset)?;
        // SAFETY: `address` answered, so the whole value lies inside the
        // mapping the caller promised, and the pointer is aligned for a
        // byte.
        Some(unsafe { at.read_volatile() })
    }

    /// The little-endian `u16` at `offset`.
    #[must_use]
    pub fn read_u16(&self, offset: usize) -> Option<u16> {
        let at = self.address::<u16>(offset)?;
        // SAFETY: `address` answered, so the whole value lies inside the
        // mapping and the offset is a multiple of its width.
        Some(unsafe { at.read_volatile() })
    }

    /// The little-endian `u32` at `offset`.
    #[must_use]
    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        let at = self.address::<u32>(offset)?;
        // SAFETY: `address` answered, so the whole value lies inside the
        // mapping and the offset is a multiple of its width.
        Some(unsafe { at.read_volatile() })
    }

    /// The little-endian `u64` at `offset`.
    #[must_use]
    pub fn read_u64(&self, offset: usize) -> Option<u64> {
        let at = self.address::<u64>(offset)?;
        // SAFETY: `address` answered, so the whole value lies inside the
        // mapping and the offset is a multiple of its width.
        Some(unsafe { at.read_volatile() })
    }

    /// Writes the byte at `offset`; `false` when it lies outside.
    pub fn write_u8(&mut self, offset: usize, value: u8) -> bool {
        let Some(at) = self.address_mut::<u8>(offset) else {
            return false;
        };
        // SAFETY: `address_mut` answered, so the whole value lies inside the
        // mapping, and this value is the only one that reaches it.
        unsafe { at.write_volatile(value) };
        true
    }

    /// Writes the `u16` at `offset`; `false` when it lies outside.
    pub fn write_u16(&mut self, offset: usize, value: u16) -> bool {
        let Some(at) = self.address_mut::<u16>(offset) else {
            return false;
        };
        // SAFETY: `address_mut` answered, so the whole value lies inside the
        // mapping, the offset is a multiple of its width, and this value is
        // the only one that reaches it.
        unsafe { at.write_volatile(value) };
        true
    }

    /// Writes the `u32` at `offset`; `false` when it lies outside.
    pub fn write_u32(&mut self, offset: usize, value: u32) -> bool {
        let Some(at) = self.address_mut::<u32>(offset) else {
            return false;
        };
        // SAFETY: `address_mut` answered, so the whole value lies inside the
        // mapping, the offset is a multiple of its width, and this value is
        // the only one that reaches it.
        unsafe { at.write_volatile(value) };
        true
    }

    /// Writes the `u64` at `offset`; `false` when it lies outside.
    pub fn write_u64(&mut self, offset: usize, value: u64) -> bool {
        let Some(at) = self.address_mut::<u64>(offset) else {
            return false;
        };
        // SAFETY: `address_mut` answered, so the whole value lies inside the
        // mapping, the offset is a multiple of its width, and this value is
        // the only one that reaches it.
        unsafe { at.write_volatile(value) };
        true
    }

    /// A window over the `len` bytes at `offset` of this one, or `None`
    /// when they do not lie whole inside it. A driver takes the structure
    /// it was told the offset of this way and can then reach no further.
    #[must_use]
    pub fn window(&mut self, offset: usize, len: usize) -> Option<Mmio<'_>> {
        let end = offset.checked_add(len)?;
        if end > self.len {
            return None;
        }
        let base = self.base.wrapping_add(offset);
        // SAFETY: the range lies inside this window's own mapping, which
        // the borrow of `self` keeps alive and exclusive for the life of
        // the value answered.
        Some(unsafe { Mmio::new(base, len) })
    }

    /// The pointer to a `T` at `offset`, if the value lies whole inside the
    /// window and the offset is a multiple of its width.
    fn address<T>(&self, offset: usize) -> Option<*const T> {
        self.checked::<T>(offset).map(|at| at.cast_const().cast())
    }

    /// The same pointer, for a write. It takes `&mut self` because what it
    /// hands out is written through, and that write is sound only while
    /// nothing else reaches the region.
    fn address_mut<T>(&mut self, offset: usize) -> Option<*mut T> {
        self.checked::<T>(offset).map(<*mut u8>::cast)
    }

    /// The byte pointer at `offset`, checked against the length and the
    /// width of `T`.
    fn checked<T>(&self, offset: usize) -> Option<*mut u8> {
        let width = size_of::<T>();
        let end = offset.checked_add(width)?;
        if end > self.len {
            return None;
        }
        if offset.checked_rem(width)? != 0 {
            return None;
        }
        Some(self.base.wrapping_add(offset))
    }
}
