// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! EMSA-PSS, in the `rsae` shape and no other.
//!
//! RFC 8017, section 9.1.2 is the verification and appendix B.2.1 is the
//! mask generation function it masks with. Three of that section's
//! parameters are not parameters here. The mask is generated with the
//! same hash that made the digest, the salt is as long as that hash's
//! output, and the trailer is `0xBC` — which is what RFC 8446 fixes for
//! `rsa_pss_rsae_sha256` and its two siblings, the only PSS schemes this
//! client offers (D-81).
//!
//! The salt length is therefore not read out of the encoding. A verifier
//! that recovered it from the position of the `0x01` separator would
//! accept a signature made with a salt of any length, which is a wider
//! rule than the one the schemes state; this one knows what it is looking
//! for and looks there.

use crypto_ct::ct_eq;

use crate::error::RsaError;
use crate::hash::HashId;
use crate::window::{head, head_mut, tail, tail_mut};

/// The trailer field of RFC 8017, section 9.1.2, step 4.
pub const TRAILER: u8 = 0xBC;

/// The eight zero bytes `M'` begins with, RFC 8017, section 9.1.2,
/// step 12.
const PADDING1: [u8; 8] = [0; 8];

/// The widest encoded message this crate handles, which is the widest
/// modulus.
const MAX_EM: usize = crypto_bignum::MAX_BYTES;

/// MGF1 of RFC 8017, appendix B.2.1: `out` filled with the hash of the
/// seed and a counter, block after block.
pub fn mgf1(hash: HashId, seed: &[u8], out: &mut [u8]) {
    let block_len = hash.output_len();
    let mut counter = 0u32;
    let mut written = 0usize;
    while written < out.len() {
        let block = hash.digest_parts(&[seed, &counter.to_be_bytes()]);
        let target = tail_mut(out, written);
        for (slot, byte) in target.iter_mut().zip(head(&block, block_len)) {
            *slot = *byte;
        }
        written = written.saturating_add(block_len);
        counter = counter.saturating_add(1);
    }
}

/// EMSA-PSS-VERIFY of RFC 8017, section 9.1.2, with the salt as long as
/// the hash output.
///
/// # Errors
///
/// [`RsaError::BadSignature`] for each of the five ways the section calls
/// the encoding inconsistent, and for a key too small to carry one.
pub fn verify(
    hash: HashId,
    em_bits: usize,
    message: &[u8],
    encoded: &[u8],
) -> Result<(), RsaError> {
    let hash_len = hash.output_len();
    let salt_len = hash_len;
    let em_len = encoded.len();
    if em_len < hash_len.saturating_add(salt_len).saturating_add(2) {
        return Err(RsaError::BadSignature);
    }
    if encoded.last().copied().unwrap_or(0) != TRAILER {
        return Err(RsaError::BadSignature);
    }

    let db_len = em_len.saturating_sub(hash_len).saturating_sub(1);
    let masked = head(encoded, db_len);
    let seed = head(tail(encoded, db_len), hash_len);
    let spare = spare_mask(em_len, em_bits);
    if masked.first().copied().unwrap_or(0) & spare != 0 {
        return Err(RsaError::BadSignature);
    }

    let mut db = [0u8; MAX_EM];
    mgf1(hash, seed, head_mut(&mut db, db_len));
    for (slot, byte) in db.iter_mut().zip(masked) {
        *slot ^= *byte;
    }
    for slot in db.iter_mut().take(1) {
        *slot &= !spare;
    }

    let unmasked = head(&db, db_len);
    let zeros = db_len.saturating_sub(salt_len).saturating_sub(1);
    if head(unmasked, zeros).iter().any(|byte| *byte != 0) {
        return Err(RsaError::BadSignature);
    }
    if tail(unmasked, zeros).first().copied().unwrap_or(0) != 0x01 {
        return Err(RsaError::BadSignature);
    }
    let salt = tail(unmasked, zeros.saturating_add(1));

    let digest = hash.digest(message);
    let recomputed = hash.digest_parts(&[&PADDING1, head(&digest, hash_len), salt]);
    if ct_eq(head(&recomputed, hash_len), seed).is_true() {
        return Ok(());
    }
    Err(RsaError::BadSignature)
}

/// EMSA-PSS-ENCODE of RFC 8017, section 9.1.1, with the salt supplied.
///
/// This is not part of the product surface, and the salt is an argument
/// because a test that wants a salt of the wrong length has to be able to
/// ask for one.
///
/// # Errors
///
/// [`RsaError::BadSignature`] when the encoded message cannot hold the
/// hash, the salt, and the two fixed bytes.
#[cfg(feature = "test-signing")]
pub fn encode(
    hash: HashId,
    em_bits: usize,
    message: &[u8],
    salt: &[u8],
    out: &mut [u8],
) -> Result<(), RsaError> {
    let hash_len = hash.output_len();
    let em_len = out.len();
    if em_len < hash_len.saturating_add(salt.len()).saturating_add(2) {
        return Err(RsaError::BadSignature);
    }
    let db_len = em_len.saturating_sub(hash_len).saturating_sub(1);
    let zeros = db_len.saturating_sub(salt.len()).saturating_sub(1);

    let digest = hash.digest(message);
    let seed = hash.digest_parts(&[&PADDING1, head(&digest, hash_len), salt]);

    let mut db = [0u8; MAX_EM];
    for slot in tail_mut(head_mut(&mut db, db_len), zeros)
        .iter_mut()
        .take(1)
    {
        *slot = 0x01;
    }
    let salt_slot = tail_mut(head_mut(&mut db, db_len), zeros.saturating_add(1));
    for (slot, byte) in salt_slot.iter_mut().zip(salt) {
        *slot = *byte;
    }

    let mut mask = [0u8; MAX_EM];
    mgf1(hash, head(&seed, hash_len), head_mut(&mut mask, db_len));
    for (slot, byte) in db.iter_mut().zip(mask) {
        *slot ^= byte;
    }
    for slot in db.iter_mut().take(1) {
        *slot &= !spare_mask(em_len, em_bits);
    }

    let source = head(&db, db_len)
        .iter()
        .chain(head(&seed, hash_len))
        .chain(&[TRAILER]);
    for (slot, byte) in out.iter_mut().zip(source) {
        *slot = *byte;
    }
    Ok(())
}

/// The mask of the bits at the top of the encoding that `emBits` says
/// must be zero, which is `8 * emLen - emBits` of them.
fn spare_mask(em_len: usize, em_bits: usize) -> u8 {
    let spare = em_len.saturating_mul(8).saturating_sub(em_bits);
    let shift = u32::try_from(spare).unwrap_or(8);
    !0xffu8.checked_shr(shift).unwrap_or(0)
}
