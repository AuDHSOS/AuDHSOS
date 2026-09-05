// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! HMAC from RFC 2104, over any hash of this crate.
//!
//! Invariants: the key never reaches a formatter; the padded key is
//! overwritten before it goes out of scope; verification compares tags in
//! time that does not depend on where they differ.

use crypto_ct::{Choice, ct_eq, wipe};

use crate::hash::Hash;

/// The byte the inner padding exclusive-ors into the key.
const INNER_PAD: u8 = 0x36;
/// The byte the outer padding exclusive-ors into the key.
const OUTER_PAD: u8 = 0x5C;

/// A keyed message authentication code over the hash `H`.
///
/// The key is secret. It is absorbed into the two hash states in `new` and
/// exists nowhere afterwards; the message is public as far as this crate is
/// concerned, so `update` is an ordinary hash update.
#[derive(Clone)]
pub struct Hmac<H: Hash> {
    /// The state over the inner padding and the message.
    inner: H,
    /// The state over the outer padding, closed in `finish`.
    outer: H,
}

impl<H: Hash> Hmac<H> {
    /// A code under `key`, which may be of any length: a key longer than one
    /// block is replaced by its digest, a shorter one is padded with zeros.
    #[must_use]
    pub fn new(key: &[u8]) -> Hmac<H> {
        let mut pad = H::ZERO_BLOCK;
        if key.len() > pad.as_ref().len() {
            let digest = H::digest(key);
            for (slot, byte) in pad.as_mut().iter_mut().zip(digest.as_ref()) {
                *slot = *byte;
            }
        } else {
            for (slot, byte) in pad.as_mut().iter_mut().zip(key) {
                *slot = *byte;
            }
        }

        let mut inner = H::new();
        for byte in pad.as_mut() {
            *byte ^= INNER_PAD;
        }
        inner.update(pad.as_ref());

        let mut outer = H::new();
        for byte in pad.as_mut() {
            *byte ^= INNER_PAD ^ OUTER_PAD;
        }
        outer.update(pad.as_ref());

        wipe(pad.as_mut());
        Hmac { inner, outer }
    }

    /// Adds `bytes` to the message.
    pub fn update(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }

    /// The tag over the message.
    #[must_use]
    pub fn finish(self) -> H::Output {
        let inner = self.inner.finish();
        let mut outer = self.outer;
        outer.update(inner.as_ref());
        outer.finish()
    }

    /// The tag over one contiguous message.
    #[must_use]
    pub fn tag(key: &[u8], message: &[u8]) -> H::Output {
        let mut state = Hmac::<H>::new(key);
        state.update(message);
        state.finish()
    }

    /// Whether `tag` is the code of `message` under `key`.
    ///
    /// The comparison is constant time, and a tag of the wrong length is
    /// rejected rather than compared against a prefix.
    #[must_use]
    pub fn verify(key: &[u8], message: &[u8], tag: &[u8]) -> Choice {
        let expected = Hmac::<H>::tag(key, message);
        ct_eq(expected.as_ref(), tag)
    }
}
