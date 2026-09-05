// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The container key material lives in.
//!
//! Invariants: the bytes never reach a formatter, equality is the
//! constant-time comparison, and dropping the value overwrites the buffer.
//! The last one is best effort and the module documentation says why.

use core::fmt;
use core::hint::black_box;

use crate::choice::Choice;
use crate::compare::ct_eq;

/// `N` bytes of key material.
///
/// The type exists for three reasons. It keeps secrets out of `Debug`
/// output, which is how key material usually escapes. It makes `==`
/// unavailable, so that comparing two secrets goes through [`Secret::ct_eq`]
/// rather than through a comparison that stops at the first differing byte.
/// And it overwrites its buffer when it goes out of scope.
///
/// The overwrite is not a guarantee. A reliable erase needs a volatile
/// write, which is `unsafe` and therefore absent from this crate; what
/// happens instead is a write of zeros followed by
/// [`core::hint::black_box`], which no compiler in use today optimizes
/// away. Treat it as a hygiene measure, and keep secrets short-lived.
#[derive(Clone)]
pub struct Secret<const N: usize> {
    bytes: [u8; N],
}

impl<const N: usize> Secret<N> {
    /// A secret of `N` zero bytes, to be filled by the caller.
    #[must_use]
    pub const fn zero() -> Secret<N> {
        Secret { bytes: [0u8; N] }
    }

    /// Takes ownership of `bytes`.
    #[must_use]
    pub const fn new(bytes: [u8; N]) -> Secret<N> {
        Secret { bytes }
    }

    /// The bytes, for the primitive that consumes them.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; N] {
        &self.bytes
    }

    /// The bytes, for the primitive that fills them.
    #[must_use]
    pub const fn as_bytes_mut(&mut self) -> &mut [u8; N] {
        &mut self.bytes
    }

    /// Whether two secrets are equal, in time that does not depend on where
    /// they differ.
    #[must_use]
    pub fn ct_eq(&self, other: &Secret<N>) -> Choice {
        ct_eq(&self.bytes, &other.bytes)
    }

    /// Overwrites the buffer with zeros. `Drop` calls this; a caller that
    /// wants the buffer cleared earlier can call it too.
    pub fn clear(&mut self) {
        self.bytes.fill(0);
        let _ = black_box(&self.bytes);
    }
}

/// Overwrites `bytes` with zeros.
///
/// The same best-effort erase [`Secret`] performs, for buffers that cannot
/// be a `Secret` because their length is not known to the type system, such
/// as the padded key inside HMAC.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    let _ = black_box(&bytes);
}

impl<const N: usize> Drop for Secret<N> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<const N: usize> fmt::Debug for Secret<N> {
    /// Prints the length and no byte of the content.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret<{N}>")
    }
}

impl<const N: usize> Default for Secret<N> {
    fn default() -> Secret<N> {
        Secret::zero()
    }
}
