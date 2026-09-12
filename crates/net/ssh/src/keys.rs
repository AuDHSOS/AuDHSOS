// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The six keys of RFC 4253, section 7.2.
//!
//! `HASH(K || H || X || session_id)` for `X` from `A` to `F`, extended by
//! hashing `K || H || <what there is so far>` until there is enough. It
//! is not HKDF and cannot be taken from `crypto-hash::hkdf`: there is no
//! extract step, the salt is the exchange hash, and the extension feeds
//! the whole key back rather than a counter and one block.

use crypto_hash::Sha256;

use crate::exchange::{HASH_LEN, hash_mpint};

/// Which of the six a call is for. The letters are the ones section 7.2
/// hashes, and they are what the letter means: two initial IVs, two
/// encryption keys, two integrity keys, client to server first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// `A`: the initial IV for what the client sends.
    IvClientToServer,
    /// `B`: the initial IV for what the server sends.
    IvServerToClient,
    /// `C`: the encryption key for what the client sends.
    EncryptionClientToServer,
    /// `D`: the encryption key for what the server sends.
    EncryptionServerToClient,
    /// `E`: the integrity key for what the client sends.
    IntegrityClientToServer,
    /// `F`: the integrity key for what the server sends.
    IntegrityServerToClient,
}

impl Key {
    /// The single character section 7.2 hashes for this key.
    #[must_use]
    pub const fn letter(self) -> u8 {
        match self {
            Key::IvClientToServer => b'A',
            Key::IvServerToClient => b'B',
            Key::EncryptionClientToServer => b'C',
            Key::EncryptionServerToClient => b'D',
            Key::IntegrityClientToServer => b'E',
            Key::IntegrityServerToClient => b'F',
        }
    }
}

/// Fills `out` with one of the six keys.
///
/// `shared` is `K` as unsigned big-endian bytes, which is hashed as the
/// `mpint` of RFC 4251, section 5. `out` may be longer than the hash, in
/// which case the extension rule of section 7.2 runs: the key so far is
/// hashed after `K` and `H`, and what comes out is appended.
pub fn derive(
    shared: &[u8],
    exchange_hash: &[u8; HASH_LEN],
    session_id: &[u8; HASH_LEN],
    key: Key,
    out: &mut [u8],
) {
    let mut hash = start(shared, exchange_hash);
    hash.update(&[key.letter()]);
    hash.update(session_id);
    let mut block = hash.finish();

    let mut filled = 0usize;
    loop {
        let slot = out.get_mut(filled..).unwrap_or(&mut []);
        let take = slot.len().min(block.len());
        let (head, _) = slot.split_at_mut(take);
        head.copy_from_slice(block.get(..take).unwrap_or(&[]));
        filled = filled.saturating_add(take);
        if filled >= out.len() {
            return;
        }
        // K2 = HASH(K || H || K1), K3 = HASH(K || H || K1 || K2), and so
        // on: what is fed back is the whole key so far.
        let mut next = start(shared, exchange_hash);
        next.update(out.get(..filled).unwrap_or(&[]));
        block = next.finish();
    }
}

/// A hash that has taken `K` and `H`, which every step of section 7.2
/// starts with.
fn start(shared: &[u8], exchange_hash: &[u8; HASH_LEN]) -> Sha256 {
    let mut hash = Sha256::new();
    hash_mpint(&mut hash, shared);
    hash.update(exchange_hash);
    hash
}
