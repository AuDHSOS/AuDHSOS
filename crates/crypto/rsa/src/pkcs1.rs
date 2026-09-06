// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! EMSA-PKCS1-v1_5, built and never parsed.
//!
//! RFC 8017, section 8.2.2 gives verification two shapes. Steps three and
//! four build the expected encoded message and compare it with the one
//! the signature recovers; the note after them offers a decoder that
//! reads the digest out of what arrived instead, and weighs the two by
//! storage against code size. This is the first shape, for a reason the
//! note does not give: a decoder is where a signature with slack in its
//! padding is accepted, and a construction has no slack to accept
//! (D-80).
//!
//! Two consequences follow and are stated rather than discovered. A
//! `DigestInfo` under BER but not DER — an indefinite length on the
//! `SEQUENCE`, say — is refused here, and PKCS #1 v1.5 accepts it: RFC
//! 2313, section 10.2.3 makes the BER decode its verification operation,
//! so the two documents describe two operations rather than one operation
//! with a tolerance. And padding is compared, not scanned, so a block
//! whose `0xff` run is short by one byte fails for the same reason a
//! wrong digest does.

use crate::error::RsaError;
use crate::hash::HashId;
use crate::window::{head, head_mut, tail_mut};

/// The DER encoding of the `DigestInfo` of SHA-256, without the digest.
///
/// RFC 8017, section 9.2, note 1 writes all nine of these out byte for
/// byte; three of them are here, and the digest is appended to them.
const SHA256_PREFIX: &[u8] = &[
    0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05,
    0x00, 0x04, 0x20,
];

/// The DER encoding of the `DigestInfo` of SHA-384, without the digest.
/// RFC 8017, section 9.2, note 1.
const SHA384_PREFIX: &[u8] = &[
    0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05,
    0x00, 0x04, 0x30,
];

/// The DER encoding of the `DigestInfo` of SHA-512, without the digest.
/// RFC 8017, section 9.2, note 1.
const SHA512_PREFIX: &[u8] = &[
    0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05,
    0x00, 0x04, 0x40,
];

/// The `DigestInfo` prefix of a hash.
#[must_use]
pub const fn prefix_of(hash: HashId) -> &'static [u8] {
    match hash {
        HashId::Sha256 => SHA256_PREFIX,
        HashId::Sha384 => SHA384_PREFIX,
        HashId::Sha512 => SHA512_PREFIX,
    }
}

/// Writes `EM = 0x00 || 0x01 || PS || 0x00 || T` over the whole of `out`,
/// where `T` is the `DigestInfo` of the message under `hash` and `PS` is
/// the `0xff` bytes that fill the rest.
///
/// # Errors
///
/// [`RsaError::BadSignature`] when the key is too small for the encoding,
/// which RFC 8017 states as `emLen` being less than `tLen + 11`: fewer
/// than eight bytes of padding would be left, and a signature under such
/// a key is refused rather than made to fit.
pub fn encode(hash: HashId, message: &[u8], out: &mut [u8]) -> Result<(), RsaError> {
    let prefix = prefix_of(hash);
    let digest_len = hash.output_len();
    let t_len = prefix.len().saturating_add(digest_len);
    if out.len() < t_len.saturating_add(11) {
        return Err(RsaError::BadSignature);
    }
    let separator = out.len().saturating_sub(t_len).saturating_sub(1);
    let digest = hash.digest(message);

    out.fill(0xff);
    for (slot, byte) in head_mut(out, 2).iter_mut().zip([0x00u8, 0x01]) {
        *slot = byte;
    }
    for slot in tail_mut(out, separator).iter_mut().take(1) {
        *slot = 0x00;
    }
    let info = tail_mut(out, separator.saturating_add(1));
    let source = prefix.iter().chain(head(&digest, digest_len));
    for (slot, byte) in info.iter_mut().zip(source) {
        *slot = *byte;
    }
    Ok(())
}
