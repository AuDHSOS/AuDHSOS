// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ChaCha20-Poly1305` from RFC 8439, section 2.8.
//!
//! The one-time authentication key is the first half of the keystream block
//! at counter zero; the message starts at counter one. The tag covers the
//! associated data, the ciphertext, each padded to a multiple of sixteen,
//! and the two lengths.
//!
//! Invariant: `open` computes the tag over the received ciphertext and
//! compares it before it decrypts anything, so plaintext exists only after
//! authentication has succeeded.

use crypto_ct::{ct_eq, wipe};

use crate::aead::{Aead, Tag};
use crate::chacha20::{self, ChaCha20};
use crate::error::AeadError;
use crate::poly1305::{self, Poly1305};

/// Bytes of key.
pub const KEY_LEN: usize = chacha20::KEY_LEN;
/// Bytes of nonce.
pub const NONCE_LEN: usize = chacha20::NONCE_LEN;

/// The zeros the padding of the authenticated data is taken from.
const ZEROS: [u8; 16] = [0u8; 16];

/// The authenticated cipher of RFC 8439.
#[derive(Clone)]
pub struct ChaCha20Poly1305 {
    /// The stream cipher under the same key.
    cipher: ChaCha20,
}

impl ChaCha20Poly1305 {
    /// A cipher under `key`.
    #[must_use]
    pub fn from_key(key: &[u8; KEY_LEN]) -> ChaCha20Poly1305 {
        ChaCha20Poly1305 {
            cipher: ChaCha20::new(key),
        }
    }

    /// The one-time authentication key for `nonce`: the first thirty-two
    /// bytes of the keystream block at counter zero.
    fn authentication_key(&self, nonce: &[u8; NONCE_LEN]) -> [u8; poly1305::KEY_LEN] {
        let block = self.cipher.block(nonce, 0);
        let mut key = [0u8; poly1305::KEY_LEN];
        for (slot, byte) in key.iter_mut().zip(block) {
            *slot = byte;
        }
        key
    }

    /// The tag over `aad` and `ciphertext`.
    fn mac(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Tag, AeadError> {
        let aad_len = u64::try_from(aad.len()).map_err(|_| AeadError::MessageTooLong)?;
        let text_len = u64::try_from(ciphertext.len()).map_err(|_| AeadError::MessageTooLong)?;

        let mut key = self.authentication_key(nonce);
        let mut mac = Poly1305::new(&key);
        wipe(&mut key);

        mac.update(aad);
        mac.update(padding(aad.len()));
        mac.update(ciphertext);
        mac.update(padding(ciphertext.len()));
        mac.update(&aad_len.to_le_bytes());
        mac.update(&text_len.to_le_bytes());
        Ok(mac.finish())
    }
}

impl Aead for ChaCha20Poly1305 {
    const KEY_LEN: usize = KEY_LEN;
    const NONCE_LEN: usize = NONCE_LEN;

    fn new(key: &[u8]) -> Result<ChaCha20Poly1305, AeadError> {
        let key: &[u8; KEY_LEN] = key.try_into().map_err(|_| AeadError::KeyLength)?;
        Ok(ChaCha20Poly1305::from_key(key))
    }

    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError> {
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| AeadError::NonceLength)?;
        self.cipher.apply_keystream(nonce, 1, in_out)?;
        self.mac(nonce, aad, in_out)
    }

    fn open(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| AeadError::NonceLength)?;
        let expected = self.mac(nonce, aad, in_out)?;
        if !ct_eq(&expected, tag).is_true() {
            wipe(in_out);
            return Err(AeadError::BadTag);
        }
        self.cipher.apply_keystream(nonce, 1, in_out)
    }
}

/// The zeros that pad an authenticated part to a multiple of sixteen.
const fn padding(len: usize) -> &'static [u8] {
    // `len & 15` is below sixteen, so the needed count is below sixteen and
    // the split is inside the array.
    let needed = 16usize.wrapping_sub(len & 15) & 15;
    let (zeros, _) = ZEROS.split_at(needed);
    zeros
}
