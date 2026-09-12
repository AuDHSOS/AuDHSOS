// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `chacha20-poly1305@openssh.com`, the one cipher this client offers
//! (D-134).
//!
//! Two `ChaCha20` instances under one 512-bit key: one encrypts nothing but
//! the four-byte packet length, the other encrypts the packet and keys
//! the Poly1305 over both. The separation is what keeps the length
//! confidential without making it a decryption oracle for the payload.
//!
//! The two halves are named the other way round in the two documents that
//! describe this cipher: `PROTOCOL.chacha20poly1305` calls the length key
//! `K_1` and the packet key `K_2`, the draft calls them `K_2` and `K_1`.
//! The bytes are the same in both and the worked example of appendix A
//! settles them, so this module names them by what they encrypt and uses
//! neither label.

use crypto_aead::chacha20::{ChaCha20, KEY_LEN, NONCE_LEN};
use crypto_aead::poly1305::Poly1305;

use crate::error::SshError;

/// The key material the key exchange must produce for this cipher: two
/// `ChaCha20` keys.
pub const KEY_BYTES: usize = KEY_LEN * 2;

/// The Poly1305 tag every packet carries.
pub const TAG_BYTES: usize = 16;

/// The length field this cipher encrypts on its own.
pub const LENGTH_BYTES: usize = 4;

/// What the packet is padded to. The cipher is a stream cipher, so this
/// is the eight of RFC 4253, section 6, and not a block of its own.
pub const BLOCK: usize = 8;

/// One direction of one connection.
pub struct ChaChaPoly {
    /// Keyed by the second half of the key material; encrypts the length.
    length: ChaCha20,
    /// Keyed by the first half; encrypts the packet and keys the tag.
    packet: ChaCha20,
}

impl ChaChaPoly {
    /// The two instances of a 64-byte key.
    #[must_use]
    pub fn new(key: &[u8; KEY_BYTES]) -> ChaChaPoly {
        let (halves, _) = key.as_chunks::<KEY_LEN>();
        let first = halves.first().copied().unwrap_or([0u8; KEY_LEN]);
        let second = halves.get(1).copied().unwrap_or([0u8; KEY_LEN]);
        ChaChaPoly {
            length: ChaCha20::new(&second),
            packet: ChaCha20::new(&first),
        }
    }

    /// Encrypts a framed packet in place and writes its tag.
    ///
    /// `frame` is the length field and everything after it; the tag is
    /// over the ciphertext of both, which is what a receiver can check
    /// before it decrypts anything.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `frame` is shorter than a length
    /// field, and [`SshError::PayloadLength`] for a frame so long that
    /// the keystream would run past its counter — which no packet of
    /// RFC 4253, section 6.1, comes near, and which is refused here
    /// rather than left in the clear.
    pub fn seal(
        &self,
        sequence: u32,
        frame: &mut [u8],
        tag: &mut [u8; TAG_BYTES],
    ) -> Result<(), SshError> {
        let nonce = nonce(sequence);
        let key = self.tag_key(&nonce);
        self.crypt(&nonce, frame)?;
        tag.copy_from_slice(&Poly1305::tag_of(&key, frame));
        Ok(())
    }

    /// The packet length from the four bytes that carry it, without
    /// touching them: the tag is over the ciphertext, so nothing may be
    /// decrypted in place before it has been checked.
    #[must_use]
    pub fn length(&self, sequence: u32, encrypted: &[u8; LENGTH_BYTES]) -> u32 {
        let nonce = nonce(sequence);
        let mut bytes = *encrypted;
        // Four bytes at counter zero: one block, so the keystream cannot
        // run past its counter and there is no answer to carry. What can
        // fail is in `crypt`, which refuses instead of discarding.
        let _ = self.length.apply_keystream(&nonce, 0, &mut bytes);
        u32::from_be_bytes(bytes)
    }

    /// Checks the tag and then decrypts `frame` in place.
    ///
    /// # Errors
    ///
    /// [`SshError::Tag`] when the tag does not check, in which case
    /// nothing is decrypted; [`SshError::OutOfBounds`] when `frame` is
    /// shorter than a length field; [`SshError::PayloadLength`] for a
    /// frame the keystream cannot reach across.
    pub fn open(&self, sequence: u32, frame: &mut [u8], tag: &[u8]) -> Result<(), SshError> {
        let nonce = nonce(sequence);
        let key = self.tag_key(&nonce);
        if !Poly1305::verify(&key, frame, tag).is_true() {
            return Err(SshError::Tag);
        }
        self.crypt(&nonce, frame)
    }

    /// Exclusive-ors both keystreams over a frame: the length field
    /// under its own key at counter zero, the rest under the packet key
    /// from counter one. Encrypting and decrypting are the same
    /// operation, so sealing and opening share this.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `frame` holds no length field, and
    /// [`SshError::PayloadLength`] for a frame so long that a keystream
    /// would run past its counter. No packet of RFC 4253, section 6.1,
    /// comes near that; what matters is that such a frame is refused
    /// rather than left in the clear with a tag over it.
    fn crypt(&self, nonce: &[u8; NONCE_LEN], frame: &mut [u8]) -> Result<(), SshError> {
        let available = frame.len();
        let (length, rest) =
            frame
                .split_at_mut_checked(LENGTH_BYTES)
                .ok_or(SshError::OutOfBounds {
                    needed: LENGTH_BYTES,
                    available,
                })?;
        self.length
            .apply_keystream(nonce, 0, length)
            .and_then(|()| self.packet.apply_keystream(nonce, 1, rest))
            .map_err(|_| SshError::PayloadLength(available))
    }

    /// The Poly1305 key of one packet: the first half of the packet
    /// cipher's block at counter zero, which is the block the packet
    /// itself does not use.
    fn tag_key(&self, nonce: &[u8; NONCE_LEN]) -> [u8; KEY_LEN] {
        let block = self.packet.block(nonce, 0);
        let mut key = [0u8; KEY_LEN];
        for (slot, byte) in key.iter_mut().zip(block.iter().take(KEY_LEN)) {
            *slot = *byte;
        }
        key
    }
}

/// The nonce of one packet.
///
/// The cipher takes the sequence number as a `uint64` under the SSH wire
/// encoding, which is a 64-bit nonce; this `ChaCha20` takes the 96-bit
/// nonce of RFC 8439, whose first word is the upper half of that
/// document's 64-bit block counter. The counter stays far below 2^32, so
/// that word is zero, and the sequence number sits in the last eight
/// bytes exactly where the 64-bit form puts it.
fn nonce(sequence: u32) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    let bytes = sequence.to_be_bytes();
    for (slot, byte) in nonce.iter_mut().skip(8).zip(bytes) {
        *slot = byte;
    }
    nonce
}
