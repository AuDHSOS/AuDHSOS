// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The key schedule of RFC 8446, section 7.1.
//!
//! The schedule is a chain: a secret is extracted, secrets are derived
//! from it for the traffic of one epoch, and what is left is carried into
//! the next extraction. Nothing here decides when to advance; the state
//! machine does that, and this module only says what each step computes.
//!
//! Invariants: every derivation is one `HKDF-Expand-Label` and every
//! secret is one hash long; a label is the seven bytes `tls13 ` and what
//! follows; the transcript hash a step takes is the one the standard names
//! for that step, and the caller passes it rather than this module
//! guessing.

use crypto_hash::{Prk, Sha256, Sha384, expand, extract};

use crate::error::TlsError;
use crate::secret::Secret;
use crate::suite::{CipherSuite, IV_LEN, MAX_HASH, MAX_KEY};

/// The prefix every label carries.
const PREFIX: &[u8] = b"tls13 ";

/// The longest label this crate uses, without the prefix.
const MAX_LABEL: usize = 16;

/// The key and the nonce of one direction of one epoch.
#[derive(Clone)]
pub struct TrafficKeys {
    /// The key.
    key: Secret,
    /// The nonce the sequence number is exclusive-ored into.
    iv: [u8; IV_LEN],
    /// The suite, which says which cipher the key belongs to.
    suite: CipherSuite,
}

impl TrafficKeys {
    /// The key.
    #[must_use]
    pub fn key(&self) -> &[u8] {
        self.key.as_bytes()
    }

    /// The nonce base.
    #[must_use]
    pub const fn iv(&self) -> &[u8; IV_LEN] {
        &self.iv
    }

    /// The suite these keys belong to.
    #[must_use]
    pub const fn suite(&self) -> CipherSuite {
        self.suite
    }

    /// The nonce of record `sequence`: the base with the number
    /// exclusive-ored into its right end, as RFC 8446 section 5.3
    /// prescribes.
    #[must_use]
    pub fn nonce(&self, sequence: u64) -> [u8; IV_LEN] {
        let mut nonce = self.iv;
        let counter = sequence.to_be_bytes();
        for (slot, byte) in nonce.iter_mut().skip(IV_LEN.saturating_sub(8)).zip(counter) {
            *slot ^= byte;
        }
        nonce
    }
}

/// Expands `secret` into `out` under `label` and `context`.
///
/// # Errors
///
/// [`TlsError::BadDerivation`] when the output is longer than the
/// construction allows, or the label longer than this crate writes.
pub fn expand_label(
    suite: CipherSuite,
    secret: &[u8],
    label: &[u8],
    context: &[u8],
    out: &mut [u8],
) -> Result<(), TlsError> {
    // struct { uint16 length; opaque label<7..255>; opaque context<0..255>; }
    let mut info = [0u8; 2 + 1 + PREFIX.len() + MAX_LABEL + 1 + MAX_HASH];
    let total = PREFIX.len().wrapping_add(label.len());
    if label.len() > MAX_LABEL || context.len() > MAX_HASH {
        return Err(TlsError::BadDerivation);
    }

    let mut used = 0usize;
    let length = u16::try_from(out.len()).map_err(|_| TlsError::BadDerivation)?;
    used = push(&mut info, used, &length.to_be_bytes())?;
    used = push(
        &mut info,
        used,
        &[u8::try_from(total).map_err(|_| TlsError::BadDerivation)?],
    )?;
    used = push(&mut info, used, PREFIX)?;
    used = push(&mut info, used, label)?;
    used = push(
        &mut info,
        used,
        &[u8::try_from(context.len()).map_err(|_| TlsError::BadDerivation)?],
    )?;
    used = push(&mut info, used, context)?;
    let info = info.get(..used).ok_or(TlsError::BadDerivation)?;

    if suite == CipherSuite::Aes256GcmSha384 {
        let prk = Prk::<Sha384>::from_output(fixed48(secret));
        expand::<Sha384>(&prk, info, out).map_err(|_| TlsError::BadDerivation)
    } else {
        let prk = Prk::<Sha256>::from_output(fixed32(secret));
        expand::<Sha256>(&prk, info, out).map_err(|_| TlsError::BadDerivation)
    }
}

/// Derives a secret from `secret` under `label`, over a transcript hash.
///
/// # Errors
///
/// See [`expand_label`].
pub fn derive_secret(
    suite: CipherSuite,
    secret: &[u8],
    label: &[u8],
    transcript: &[u8],
) -> Result<Secret, TlsError> {
    let mut derived = Secret::zero(suite.hash_len());
    expand_label(suite, secret, label, transcript, derived.as_bytes_mut())?;
    Ok(derived)
}

/// Extracts a secret from `salt` and `material`.
#[must_use]
pub fn extract_secret(suite: CipherSuite, salt: &[u8], material: &[u8]) -> Secret {
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            Secret::from_slice(extract::<Sha384>(salt, material).as_bytes())
        }
        _ => Secret::from_slice(extract::<Sha256>(salt, material).as_bytes()),
    }
}

/// The hash of an empty message under the suite's hash.
#[must_use]
pub fn empty_hash(suite: CipherSuite) -> Secret {
    match suite {
        CipherSuite::Aes256GcmSha384 => Secret::from_slice(Sha384::digest(&[]).as_ref()),
        _ => Secret::from_slice(Sha256::digest(&[]).as_ref()),
    }
}

/// The traffic keys a secret produces.
///
/// # Errors
///
/// See [`expand_label`].
pub fn traffic_keys(suite: CipherSuite, secret: &[u8]) -> Result<TrafficKeys, TlsError> {
    let mut key = Secret::zero(suite.key_len());
    expand_label(suite, secret, b"key", &[], key.as_bytes_mut())?;
    let mut iv = [0u8; IV_LEN];
    expand_label(suite, secret, b"iv", &[], &mut iv)?;
    Ok(TrafficKeys { key, iv, suite })
}

/// The key a `Finished` message is authenticated with.
///
/// # Errors
///
/// See [`expand_label`].
pub fn finished_key(suite: CipherSuite, secret: &[u8]) -> Result<Secret, TlsError> {
    let mut key = Secret::zero(suite.hash_len());
    expand_label(suite, secret, b"finished", &[], key.as_bytes_mut())?;
    Ok(key)
}

/// The secret that follows `secret` after a key update.
///
/// # Errors
///
/// See [`expand_label`].
pub fn next_traffic_secret(suite: CipherSuite, secret: &[u8]) -> Result<Secret, TlsError> {
    let mut next = Secret::zero(suite.hash_len());
    expand_label(suite, secret, b"traffic upd", &[], next.as_bytes_mut())?;
    Ok(next)
}

/// The stage of the schedule a connection has reached.
///
/// The three extractions are the whole of it. What each stage derives is
/// the caller's business, because only the caller knows which transcript
/// belongs to which step.
#[derive(Clone, Debug)]
pub struct Schedule {
    /// The suite, which decides every length.
    suite: CipherSuite,
    /// The secret the current stage extracted.
    current: Secret,
}

impl Schedule {
    /// The early stage, with no pre-shared key, which is where a client
    /// that does not resume begins.
    #[must_use]
    pub fn new(suite: CipherSuite) -> Schedule {
        let zeros = Secret::zero(suite.hash_len());
        Schedule {
            suite,
            current: extract_secret(suite, &[], zeros.as_bytes()),
        }
    }

    /// The suite.
    #[must_use]
    pub const fn suite(&self) -> CipherSuite {
        self.suite
    }

    /// The secret of the current stage.
    #[must_use]
    pub fn secret(&self) -> &[u8] {
        self.current.as_bytes()
    }

    /// Advances into the next stage with `material` as the input keying
    /// material: the shared value for the handshake stage, and zeros for
    /// the master stage.
    ///
    /// # Errors
    ///
    /// See [`expand_label`].
    pub fn advance(&mut self, material: &[u8]) -> Result<(), TlsError> {
        let empty = empty_hash(self.suite);
        let derived = derive_secret(self.suite, self.secret(), b"derived", empty.as_bytes())?;
        self.current = extract_secret(self.suite, derived.as_bytes(), material);
        Ok(())
    }

    /// Advances into the master stage, whose material is zeros.
    ///
    /// # Errors
    ///
    /// See [`expand_label`].
    pub fn advance_to_master(&mut self) -> Result<(), TlsError> {
        let zeros = Secret::zero(self.suite.hash_len());
        let material = zeros.as_bytes().to_owned_array();
        self.advance(material.as_slice())
    }

    /// A secret of this stage, over `transcript`.
    ///
    /// # Errors
    ///
    /// See [`expand_label`].
    pub fn derive(&self, label: &[u8], transcript: &[u8]) -> Result<Secret, TlsError> {
        derive_secret(self.suite, self.secret(), label, transcript)
    }
}

/// The bytes of a secret as an owned array, so that a caller can pass them
/// while the secret they came from is still borrowed.
trait ToOwnedArray {
    /// The bytes, copied.
    fn to_owned_array(&self) -> OwnedBytes;
}

impl ToOwnedArray for [u8] {
    fn to_owned_array(&self) -> OwnedBytes {
        let mut owned = OwnedBytes {
            bytes: [0u8; MAX_HASH],
            len: self.len().min(MAX_HASH),
        };
        for (slot, byte) in owned.bytes.iter_mut().zip(self) {
            *slot = *byte;
        }
        owned
    }
}

/// A copy of a secret's bytes.
struct OwnedBytes {
    /// The bytes.
    bytes: [u8; MAX_HASH],
    /// How many of them count.
    len: usize,
}

impl OwnedBytes {
    /// The bytes.
    fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }
}

/// Appends `bytes` at `used` and returns the new position.
fn push(buffer: &mut [u8], used: usize, bytes: &[u8]) -> Result<usize, TlsError> {
    let end = used
        .checked_add(bytes.len())
        .ok_or(TlsError::BadDerivation)?;
    let slice = buffer.get_mut(used..end).ok_or(TlsError::BadDerivation)?;
    for (slot, byte) in slice.iter_mut().zip(bytes) {
        *slot = *byte;
    }
    Ok(end)
}

/// A secret as the thirty-two byte output SHA-256 produces.
fn fixed32(secret: &[u8]) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (slot, byte) in bytes.iter_mut().zip(secret) {
        *slot = *byte;
    }
    bytes
}

/// A secret as the forty-eight byte output SHA-384 produces.
fn fixed48(secret: &[u8]) -> [u8; 48] {
    let mut bytes = [0u8; 48];
    for (slot, byte) in bytes.iter_mut().zip(secret) {
        *slot = *byte;
    }
    bytes
}

/// The maximum key length, which the record layer asserts against.
const _: () = assert!(MAX_KEY == 32);
