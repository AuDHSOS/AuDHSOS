// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! HKDF from RFC 5869: extract a pseudorandom key from input key material,
//! then expand it into as much output as the caller asks for.
//!
//! Invariant: `expand` writes every byte of its output buffer or returns an
//! error and writes none, so a caller cannot mistake a partial derivation
//! for a complete one.

use crate::error::HashError;
use crate::hash::Hash;
use crate::hmac::Hmac;

/// The most output blocks the construction allows; the counter byte of the
/// expansion is what bounds it.
const MAX_BLOCKS: usize = 255;

/// The pseudorandom key that [`extract`] produces and [`expand`] consumes.
///
/// It is key material. It carries no `Debug`, and the protocol keeps it only
/// as long as the schedule step that needs it.
#[derive(Clone)]
pub struct Prk<H: Hash> {
    /// The output of the extraction step.
    bytes: H::Output,
}

/// The derived key is a fixed-size array, so a pseudorandom key copies even
/// though the hash state it came from does not. The key schedule of the
/// protocol passes one step's key into the next, and a move would make that
/// read as a transfer of ownership that it is not.
impl<H: Hash> Copy for Prk<H> {}

impl<H: Hash> Prk<H> {
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

    let mut previous: Option<H::Output> = None;
    let mut counter: u8 = 1;
    for chunk in out.chunks_mut(H::OUTPUT_LEN) {
        let mut mac = Hmac::<H>::new(prk.as_bytes());
        if let Some(block) = previous {
            mac.update(block.as_ref());
        }
        mac.update(info);
        mac.update(&[counter]);
        let block = mac.finish();
        for (slot, byte) in chunk.iter_mut().zip(block.as_ref()) {
            *slot = *byte;
        }
        previous = Some(block);
        counter = counter.wrapping_add(1);
    }
    Ok(())
}
