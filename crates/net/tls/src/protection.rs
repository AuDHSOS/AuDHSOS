// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Protecting and unprotecting records.
//!
//! An encrypted record says it carries application data whatever it
//! carries; the real content type is the last non-zero byte inside the
//! encryption, after the padding. That is what hides which records are
//! handshake, which are alerts, and which are data.
//!
//! Invariants: the sequence number counts records of one epoch and never
//! wraps, because a repeated nonce under one key is the one failure this
//! construction cannot survive; the additional data of the cipher is the
//! record header exactly as it goes on the wire; and a record that does
//! not open leaves no plaintext behind.

use crypto_aead::{Aead, Aes128Gcm, Aes256Gcm, ChaCha20Poly1305, TAG_LEN};

use crate::error::TlsError;
use crate::keys::TrafficKeys;
use crate::record::{ContentType, HEADER_LEN, MAX_CIPHERTEXT, write_header};
use crate::suite::CipherSuite;

/// The cipher a suite names.
enum Cipher {
    /// AES-128-GCM.
    Aes128(Aes128Gcm),
    /// AES-256-GCM.
    Aes256(Aes256Gcm),
    /// `ChaCha20-Poly1305`.
    ChaCha(ChaCha20Poly1305),
}

impl Cipher {
    /// The cipher for `suite` under `key`.
    fn new(suite: CipherSuite, key: &[u8]) -> Result<Cipher, TlsError> {
        match suite {
            CipherSuite::Aes128GcmSha256 => Aes128Gcm::new(key)
                .map(Cipher::Aes128)
                .map_err(|_| TlsError::BadDerivation),
            CipherSuite::Aes256GcmSha384 => Aes256Gcm::new(key)
                .map(Cipher::Aes256)
                .map_err(|_| TlsError::BadDerivation),
            CipherSuite::ChaCha20Poly1305Sha256 => ChaCha20Poly1305::new(key)
                .map(Cipher::ChaCha)
                .map_err(|_| TlsError::BadDerivation),
        }
    }

    /// Encrypts in place and returns the tag.
    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<[u8; TAG_LEN], TlsError> {
        match self {
            Cipher::Aes128(cipher) => cipher.seal(nonce, aad, in_out),
            Cipher::Aes256(cipher) => cipher.seal(nonce, aad, in_out),
            Cipher::ChaCha(cipher) => cipher.seal(nonce, aad, in_out),
        }
        .map_err(|_| TlsError::BadRecord)
    }

    /// Verifies and decrypts in place.
    fn open(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
        tag: &[u8; TAG_LEN],
    ) -> Result<(), TlsError> {
        match self {
            Cipher::Aes128(cipher) => cipher.open(nonce, aad, in_out, tag),
            Cipher::Aes256(cipher) => cipher.open(nonce, aad, in_out, tag),
            Cipher::ChaCha(cipher) => cipher.open(nonce, aad, in_out, tag),
        }
        .map_err(|_| TlsError::BadRecord)
    }
}

/// One direction of one key epoch.
pub struct RecordProtection {
    /// The keys.
    keys: TrafficKeys,
    /// The cipher under them.
    cipher: Cipher,
    /// How many records this epoch has carried.
    sequence: u64,
}

impl RecordProtection {
    /// The protection `keys` provide.
    ///
    /// # Errors
    ///
    /// [`TlsError::BadDerivation`] when the key does not fit the cipher,
    /// which cannot happen for keys the schedule produced.
    pub fn new(keys: TrafficKeys) -> Result<RecordProtection, TlsError> {
        let cipher = Cipher::new(keys.suite(), keys.key())?;
        Ok(RecordProtection {
            keys,
            cipher,
            sequence: 0,
        })
    }

    /// How many records have been protected or unprotected.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Writes a protected record carrying `plaintext` of type `inner` into
    /// `out`, and returns its length.
    ///
    /// # Errors
    ///
    /// [`TlsError::RecordOverflow`] for a plaintext beyond the limit,
    /// [`TlsError::BufferTooSmall`] when the record does not fit, and
    /// [`TlsError::SequenceExhausted`] when the epoch is used up.
    pub fn seal(
        &mut self,
        inner: ContentType,
        plaintext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, TlsError> {
        let inner_len = plaintext.len().wrapping_add(1);
        let fragment = inner_len.wrapping_add(TAG_LEN);
        if plaintext.len() > crate::record::MAX_PLAINTEXT || fragment > MAX_CIPHERTEXT {
            return Err(TlsError::RecordOverflow);
        }
        let total = HEADER_LEN.wrapping_add(fragment);
        if out.len() < total {
            return Err(TlsError::BufferTooSmall);
        }

        write_header(ContentType::ApplicationData, fragment, out)?;
        let (header, rest) = out.split_at_mut(HEADER_LEN);
        let body = rest.get_mut(..inner_len).ok_or(TlsError::BufferTooSmall)?;
        for (slot, byte) in body.iter_mut().zip(plaintext) {
            *slot = *byte;
        }
        if let Some(slot) = body.get_mut(plaintext.len()) {
            *slot = inner.to_byte();
        }

        let nonce = self.keys.nonce(self.sequence);
        let tag = self.cipher.seal(&nonce, header, body)?;
        let room = rest
            .get_mut(inner_len..fragment)
            .ok_or(TlsError::BufferTooSmall)?;
        for (slot, byte) in room.iter_mut().zip(tag) {
            *slot = byte;
        }

        self.advance()?;
        Ok(total)
    }

    /// Unprotects a record in place and returns what it really carried.
    ///
    /// # Errors
    ///
    /// [`TlsError::BadRecord`] when the record does not open, when it is
    /// shorter than a tag, or when its plaintext is all padding;
    /// [`TlsError::UnknownContentType`] when the type inside is not one of
    /// the four; [`TlsError::SequenceExhausted`] when the epoch is used up.
    pub fn open<'a>(
        &mut self,
        header: &[u8],
        body: &'a mut [u8],
    ) -> Result<(ContentType, &'a [u8]), TlsError> {
        let split = body.len().checked_sub(TAG_LEN).ok_or(TlsError::BadRecord)?;
        let (content, tail) = body.split_at_mut(split);
        let tag: &[u8; TAG_LEN] = tail.first_chunk::<TAG_LEN>().ok_or(TlsError::BadRecord)?;

        let nonce = self.keys.nonce(self.sequence);
        self.cipher.open(&nonce, header, content, tag)?;
        self.advance()?;

        // The type is the last byte that is not padding.
        let mut end = content.len();
        while end > 0 && content.get(end.wrapping_sub(1)) == Some(&0) {
            end = end.wrapping_sub(1);
        }
        let position = end.checked_sub(1).ok_or(TlsError::BadRecord)?;
        let byte = content.get(position).copied().ok_or(TlsError::BadRecord)?;
        let inner = ContentType::from_byte(byte)?;
        let plaintext = content.get(..position).ok_or(TlsError::BadRecord)?;
        Ok((inner, plaintext))
    }

    /// Counts one record, refusing to wrap.
    fn advance(&mut self) -> Result<(), TlsError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(TlsError::SequenceExhausted)?;
        Ok(())
    }
}
