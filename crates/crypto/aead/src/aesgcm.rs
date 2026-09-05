// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! AES-GCM from NIST SP 800-38D, in the shape TLS 1.3 uses it: a
//! twelve-byte nonce, a sixteen-byte tag, counter mode over four blocks at
//! a time, and GHASH over the associated data, the ciphertext, and the two
//! lengths.
//!
//! A nonce of any other length would need GHASH to derive the first counter
//! block. TLS never uses one, so this crate refuses it rather than carrying
//! a path nothing exercises.
//!
//! Invariant: `open` computes the tag over the received ciphertext and
//! compares it before it decrypts anything.

use crypto_ct::{ct_eq, wipe};

use crate::aead::{Aead, Tag, padding};
use crate::aes::{Aes, BLOCK_LEN, LANES};
use crate::error::AeadError;
use crate::ghash::GHash;

/// Bytes of nonce.
pub const NONCE_LEN: usize = 12;

/// Bytes the four lanes consume at a time.
const GROUP_LEN: usize = BLOCK_LEN * LANES;

/// The counter value of the block whose encryption masks the tag.
const TAG_COUNTER: u32 = 1;

/// The counter value of the first block of the message.
const FIRST_COUNTER: u32 = 2;

/// What the two key lengths have in common.
#[derive(Clone)]
struct AesGcm {
    /// The block cipher.
    cipher: Aes,
    /// The hash key: the encryption of the zero block.
    hash_key: [u8; BLOCK_LEN],
}

impl AesGcm {
    /// The mode over `cipher`.
    fn new(cipher: Aes) -> AesGcm {
        let mut hash_key = [0u8; BLOCK_LEN];
        cipher.encrypt_block(&mut hash_key);
        AesGcm { cipher, hash_key }
    }

    /// The counter block for `counter`.
    fn counter_block(nonce: &[u8; NONCE_LEN], counter: u32) -> [u8; BLOCK_LEN] {
        let mut block = [0u8; BLOCK_LEN];
        for (slot, byte) in block.iter_mut().zip(nonce) {
            *slot = *byte;
        }
        for (slot, byte) in block.iter_mut().skip(NONCE_LEN).zip(counter.to_be_bytes()) {
            *slot = byte;
        }
        block
    }

    /// Exclusive-ors the keystream from [`FIRST_COUNTER`] onward into
    /// `data`, four blocks at a time.
    fn apply_keystream(&self, nonce: &[u8; NONCE_LEN], data: &mut [u8]) -> Result<(), AeadError> {
        let blocks =
            u64::try_from(data.len().div_ceil(BLOCK_LEN)).map_err(|_| AeadError::MessageTooLong)?;
        if let Some(after_first) = blocks.checked_sub(1) {
            let last = u64::from(FIRST_COUNTER)
                .checked_add(after_first)
                .ok_or(AeadError::MessageTooLong)?;
            if last > u64::from(u32::MAX) {
                return Err(AeadError::MessageTooLong);
            }
        }

        let mut counter = FIRST_COUNTER;
        let (groups, remainder) = data.as_chunks_mut::<GROUP_LEN>();
        for group in groups {
            let stream = self.keystream(nonce, counter);
            for (byte, key) in group.iter_mut().zip(stream) {
                *byte ^= key;
            }
            counter = counter.wrapping_add(4);
        }
        if !remainder.is_empty() {
            let stream = self.keystream(nonce, counter);
            for (byte, key) in remainder.iter_mut().zip(stream) {
                *byte ^= key;
            }
        }
        Ok(())
    }

    /// The keystream of four consecutive counter blocks.
    fn keystream(&self, nonce: &[u8; NONCE_LEN], counter: u32) -> [u8; GROUP_LEN] {
        let mut blocks = [[0u8; BLOCK_LEN]; LANES];
        for (lane, block) in blocks.iter_mut().enumerate() {
            let step = u32::try_from(lane).unwrap_or(0);
            *block = AesGcm::counter_block(nonce, counter.wrapping_add(step));
        }
        self.cipher.encrypt_blocks(&mut blocks);

        let mut stream = [0u8; GROUP_LEN];
        let (chunks, _) = stream.as_chunks_mut::<BLOCK_LEN>();
        for (chunk, block) in chunks.iter_mut().zip(blocks) {
            *chunk = block;
        }
        stream
    }

    /// The tag over `aad` and `ciphertext`.
    fn tag(
        &self,
        nonce: &[u8; NONCE_LEN],
        aad: &[u8],
        ciphertext: &[u8],
    ) -> Result<Tag, AeadError> {
        let aad_bits = bit_length(aad.len())?;
        let text_bits = bit_length(ciphertext.len())?;

        let mut hash = GHash::new(&self.hash_key);
        hash.update(aad);
        hash.update(padding(aad.len()));
        hash.update(ciphertext);
        hash.update(padding(ciphertext.len()));
        let mut lengths = [0u8; BLOCK_LEN];
        for (slot, byte) in lengths.iter_mut().zip(
            aad_bits
                .to_be_bytes()
                .into_iter()
                .chain(text_bits.to_be_bytes()),
        ) {
            *slot = byte;
        }
        hash.update(&lengths);
        let hashed = hash.finish();

        let mut mask = AesGcm::counter_block(nonce, TAG_COUNTER);
        self.cipher.encrypt_block(&mut mask);
        let mut tag = [0u8; BLOCK_LEN];
        for (slot, (masked, hashed)) in tag.iter_mut().zip(mask.iter().zip(hashed)) {
            *slot = masked ^ hashed;
        }
        Ok(tag)
    }

    /// Encrypts in place and returns the tag.
    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError> {
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| AeadError::NonceLength)?;
        self.apply_keystream(nonce, in_out)?;
        self.tag(nonce, aad, in_out)
    }

    /// Verifies and then decrypts in place.
    fn open(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        let nonce: &[u8; NONCE_LEN] = nonce.try_into().map_err(|_| AeadError::NonceLength)?;
        let expected = self.tag(nonce, aad, in_out)?;
        if !ct_eq(&expected, tag).is_true() {
            wipe(in_out);
            return Err(AeadError::BadTag);
        }
        self.apply_keystream(nonce, in_out)
    }
}

/// The length in bits, as the trailing block of the hash carries it.
fn bit_length(bytes: usize) -> Result<u64, AeadError> {
    u64::try_from(bytes)
        .ok()
        .and_then(|value| value.checked_mul(8))
        .ok_or(AeadError::MessageTooLong)
}

/// AES-128-GCM.
#[derive(Clone)]
pub struct Aes128Gcm(AesGcm);

impl Aes128Gcm {
    /// The cipher under `key`.
    #[must_use]
    pub fn from_key(key: &[u8; 16]) -> Aes128Gcm {
        Aes128Gcm(AesGcm::new(Aes::new_128(key)))
    }
}

impl Aead for Aes128Gcm {
    const KEY_LEN: usize = 16;
    const NONCE_LEN: usize = NONCE_LEN;

    fn new(key: &[u8]) -> Result<Aes128Gcm, AeadError> {
        let key: &[u8; 16] = key.try_into().map_err(|_| AeadError::KeyLength)?;
        Ok(Aes128Gcm::from_key(key))
    }

    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError> {
        self.0.seal(nonce, aad, in_out)
    }

    fn open(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        self.0.open(nonce, aad, in_out, tag)
    }
}

/// AES-256-GCM.
#[derive(Clone)]
pub struct Aes256Gcm(AesGcm);

impl Aes256Gcm {
    /// The cipher under `key`.
    #[must_use]
    pub fn from_key(key: &[u8; 32]) -> Aes256Gcm {
        Aes256Gcm(AesGcm::new(Aes::new_256(key)))
    }
}

impl Aead for Aes256Gcm {
    const KEY_LEN: usize = 32;
    const NONCE_LEN: usize = NONCE_LEN;

    fn new(key: &[u8]) -> Result<Aes256Gcm, AeadError> {
        let key: &[u8; 32] = key.try_into().map_err(|_| AeadError::KeyLength)?;
        Ok(Aes256Gcm::from_key(key))
    }

    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError> {
        self.0.seal(nonce, aad, in_out)
    }

    fn open(
        &self,
        nonce: &[u8],
        aad: &[u8],
        in_out: &mut [u8],
        tag: &Tag,
    ) -> Result<(), AeadError> {
        self.0.open(nonce, aad, in_out, tag)
    }
}
