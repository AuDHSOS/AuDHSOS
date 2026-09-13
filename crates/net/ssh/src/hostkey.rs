// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Host keys: the `ssh-ed25519` blobs of RFC 8709 and the rule that says
//! which key this client will talk to.
//!
//! SSH has no certificate chain. A client trusts a host key because it
//! read that key from a source it trusts, so this module judges no key of
//! its own: [`Trust`] is a parameter, [`accept`] refuses a key no rule
//! admits, and a client constructed without a rule reaches no host.
//!
//! What the server signs is the exchange hash of [`crate::exchange`], and
//! that signature is the only evidence that the peer holds the private
//! half of the key it sent.

use crypto_ct::ct_eq;
use crypto_ec::ed25519::{self, PUBLIC_LEN, SIGNATURE_LEN};
use crypto_hash::Sha256;

use crate::error::SshError;
use crate::exchange::{HASH_LEN, hash_string};
use crate::kex::HOST_KEY_ED25519;
use crate::wire::{Reader, Writer};

/// The SHA-256 fingerprint of a host key blob, which is what OpenSSH
/// prints after `SHA256:` and what an image carries.
pub const FINGERPRINT_LEN: usize = 32;

/// An `ssh-ed25519` key blob: two length fields, the name, and the key.
pub const BLOB_LEN: usize = 4 + 11 + 4 + PUBLIC_LEN;

/// The signature blob of RFC 8709, section 6, under the same name.
pub const SIGNATURE_BLOB_LEN: usize = 4 + 11 + 4 + SIGNATURE_LEN;

/// A host key a server sent, read out of the blob of RFC 8709, section 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HostKey {
    /// The 32 octets of RFC 8032, section 5.1.5.
    key: [u8; PUBLIC_LEN],
}

impl HostKey {
    /// Reads `K_S`.
    ///
    /// # Errors
    ///
    /// [`SshError::HostKey`] for a blob naming another algorithm, a key
    /// of another length, or bytes after the key; [`SshError::OutOfBounds`]
    /// when the blob ends early.
    pub fn parse(blob: &[u8]) -> Result<HostKey, SshError> {
        let mut reader = Reader::new(blob);
        if reader.read_string()? != HOST_KEY_ED25519.as_bytes() {
            return Err(SshError::HostKey);
        }
        let key: [u8; PUBLIC_LEN] = reader
            .read_string()?
            .try_into()
            .map_err(|_| SshError::HostKey)?;
        if !reader.is_empty() {
            return Err(SshError::HostKey);
        }
        Ok(HostKey { key })
    }

    /// The algorithm this key is under, which is the one of section 14.5.
    #[must_use]
    pub const fn algorithm(&self) -> &'static str {
        HOST_KEY_ED25519
    }

    /// The 32 octets the signature is verified against.
    #[must_use]
    pub const fn public_key(&self) -> &[u8; PUBLIC_LEN] {
        &self.key
    }

    /// Writes the blob back, which is what the exchange hash is taken
    /// over and what a fingerprint is computed from.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `out` holds fewer than
    /// [`BLOB_LEN`] bytes.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SshError> {
        let mut writer = Writer::new(out);
        writer.write_string(HOST_KEY_ED25519.as_bytes())?;
        writer.write_string(&self.key)?;
        Ok(writer.position())
    }

    /// The SHA-256 of the blob.
    ///
    /// A parsed key encodes back to the bytes it was read from — the
    /// blob carries a name and a key of fixed length and nothing else —
    /// so this digest is the digest of what arrived. The two strings go
    /// in under the rule the exchange hash uses, because the two have to
    /// agree about what a `string` is.
    #[must_use]
    pub fn fingerprint(&self) -> [u8; FINGERPRINT_LEN] {
        let mut hash = Sha256::new();
        hash_string(&mut hash, HOST_KEY_ED25519.as_bytes());
        hash_string(&mut hash, &self.key);
        hash.finish()
    }

    /// Checks the signature of RFC 8709, section 6, over the exchange
    /// hash.
    ///
    /// # Errors
    ///
    /// [`SshError::HostKey`] for a signature blob naming another
    /// algorithm, carrying other than 64 octets, or holding bytes after
    /// them; [`SshError::OutOfBounds`] when the blob ends early; and
    /// [`SshError::Signature`] when the signature is not the peer's over
    /// this hash, which RFC 4250, section 4.2.2, answers with
    /// [`crate::msg::disconnect::HOST_KEY_NOT_VERIFIABLE`].
    pub fn verify(&self, hash: &[u8; HASH_LEN], signature: &[u8]) -> Result<(), SshError> {
        let mut reader = Reader::new(signature);
        if reader.read_string()? != HOST_KEY_ED25519.as_bytes() {
            return Err(SshError::HostKey);
        }
        let value: [u8; SIGNATURE_LEN] = reader
            .read_string()?
            .try_into()
            .map_err(|_| SshError::HostKey)?;
        if !reader.is_empty() {
            return Err(SshError::HostKey);
        }
        ed25519::verify(&self.key, hash, &value).map_err(|_| SshError::Signature)
    }
}

/// Which host keys this client will talk to.
///
/// The two sources section 14.10 names are a fingerprint the image
/// carries, which [`Fingerprint`] is, and a file a client reads through
/// the file system server, which is the caller's and not this crate's:
/// no allocation happens here and no path is opened.
pub trait Trust {
    /// Whether this key belongs to the host this client is reaching.
    fn accepts(&self, key: &HostKey) -> bool;
}

/// One fingerprint, which admits one host key and no other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    /// The SHA-256 of the blob this client expects.
    digest: [u8; FINGERPRINT_LEN],
}

impl Fingerprint {
    /// The rule that admits the key whose blob hashes to `digest`.
    #[must_use]
    pub const fn new(digest: [u8; FINGERPRINT_LEN]) -> Fingerprint {
        Fingerprint { digest }
    }

    /// The fingerprint this rule holds.
    #[must_use]
    pub const fn digest(&self) -> &[u8; FINGERPRINT_LEN] {
        &self.digest
    }
}

impl Trust for Fingerprint {
    fn accepts(&self, key: &HostKey) -> bool {
        ct_eq(&self.digest, &key.fingerprint()).is_true()
    }
}

/// The whole of what a client does with `K_S`: read it, ask the rule, and
/// check the signature over `H`.
///
/// The rule is asked first, so a key from a host this client will not
/// talk to costs no signature check.
///
/// # Errors
///
/// [`SshError::HostKey`] for a blob this client does not read,
/// [`SshError::HostKeyRejected`] when the rule refuses the key, and
/// [`SshError::Signature`] when the signature is not the peer's over
/// `hash`.
pub fn accept(
    blob: &[u8],
    hash: &[u8; HASH_LEN],
    signature: &[u8],
    trust: &impl Trust,
) -> Result<HostKey, SshError> {
    let key = HostKey::parse(blob)?;
    if !trust.accepts(&key) {
        return Err(SshError::HostKeyRejected);
    }
    key.verify(hash, signature)?;
    Ok(key)
}
