// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! HKDF from RFC 5869: extract a pseudorandom key from input key material,
//! then expand it into as much output as the caller asks for.
//!
//! Invariants: `expand` writes every byte of its output buffer or returns
//! an error and writes none, so a caller cannot mistake a partial
//! derivation for a complete one; a pseudorandom key and the previous
//! output block are overwritten before they go out of scope.

use crypto_ct::wipe;

use crate::error::HashError;
use crate::hash::Hash;
use crate::hmac::Hmac;

/// The most output blocks the construction allows; the counter byte of the
/// expansion is what bounds it.
const MAX_BLOCKS: usize = 255;

/// The pseudorandom key that [`extract`] produces and [`expand`] consumes.
///
/// It is key material. It carries no `Debug`, it is not `Copy`, so that a
/// hand-over to the next schedule step is a move or an explicit `clone`
/// rather than a silent second copy, and it overwrites its bytes when it
/// goes out of scope.
#[derive(Clone)]
pub struct Prk<H: Hash> {
    /// The output of the extraction step.
    bytes: H::Output,
}

impl<H: Hash> Prk<H> {
    /// Overwrites the bytes with zeros. `Drop` calls this; a cleared key
    /// expands to output under an all-zero key and is for dropping only.
    pub(crate) fn clear(&mut self) {
        wipe(self.bytes.as_mut());
    }

    /// Takes a pseudorandom key that was derived elsewhere, which is what a
    /// key schedule does when it feeds one step into the next.
    #[must_use]
    pub const fn from_output(bytes: H::Output) -> Prk<H> {
        Prk { bytes }
    }

    /// The bytes, for the step that consumes them.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl<H: Hash> Drop for Prk<H> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// The extraction step: concentrates the entropy of `ikm` under `salt`.
///
/// An empty salt is the same as a salt of hash-length zeros, because HMAC
/// pads both to the same block, which is what RFC 5869 prescribes.
#[must_use]
pub fn extract<H: Hash>(salt: &[u8], ikm: &[u8]) -> Prk<H> {
    Prk {
        bytes: Hmac::<H>::tag(salt, ikm),
    }
}

/// The expansion step: fills `out` with output bound to `info`.
///
/// # Errors
///
/// [`HashError::OutputTooLong`] when `out` is longer than 255 hash lengths,
/// which is the bound of the construction. `out` is untouched in that case.
pub fn expand<H: Hash>(prk: &Prk<H>, info: &[u8], out: &mut [u8]) -> Result<(), HashError> {
    let limit = MAX_BLOCKS.saturating_mul(H::OUTPUT_LEN);
    if out.len() > limit {
        return Err(HashError::OutputTooLong);
    }

    // The key schedule of the code is the same for every block, so it is
    // built once and cloned: `Hmac::new` absorbs two padded blocks, which a
    // 255-block expansion would otherwise pay 510 times.
    let keyed = Hmac::<H>::new(prk.as_bytes());
    let mut previous: Option<H::Output> = None;
    let mut counter: u8 = 1;
    for chunk in out.chunks_mut(H::OUTPUT_LEN) {
        let mut mac = keyed.clone();
        if let Some(block) = previous {
            mac.update(block.as_ref());
        }
        mac.update(info);
        mac.update(&[counter]);
        let mut block = mac.finish();
        for (slot, byte) in chunk.iter_mut().zip(block.as_ref()) {
            *slot = *byte;
        }
        if let Some(stale) = previous.as_mut() {
            wipe(stale.as_mut());
        }
        previous = Some(block);
        wipe(block.as_mut());
        counter = counter.wrapping_add(1);
    }
    if let Some(last) = previous.as_mut() {
        wipe(last.as_mut());
    }
    Ok(())
}
