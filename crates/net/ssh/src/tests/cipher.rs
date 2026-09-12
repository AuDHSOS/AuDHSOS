// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `chacha20-poly1305@openssh.com` against appendix A of the draft
//! `docs/openssh/draft-ietf-sshm-chacha20-poly1305-04.txt`, which is the
//! only published vector this track has for a whole packet.

use crate::cipher::{ChaChaPoly, KEY_BYTES, TAG_BYTES};
use crate::error::SshError;
use crate::tests::unhex;

/// Figure 5: the 64 bytes of key material the key exchange produced.
const KEY: &str = "8bbff6855fc102338c373e73aac0c914\
                   f076a905b2444a32eecaffeae22becc5\
                   e9b7a7a5825a8249346ec1c28301cf39\
                   4543fc7569887d76e168f37562ac0740";

/// Figure 4: the packet before encryption, length field and all.
const PLAIN: &str = "00000048065e00000000000000384c6f\
                     72656d20697073756d20646f6c6f7220\
                     73697420616d65742c20636f6e736563\
                     7465747572206164697069736963696e\
                     6720656c69744e43e804dc6c";

/// Figure 18: what goes on the wire, the tag included.
const SEALED: &str = "2c3ecce4a5bc05895bf07a7ba956b6c6\
                      8829ac7c83b780b7000ecde745afc705\
                      bbc378ce03a280236b87b53bed583966\
                      2302b164b6286a48cd1e097138e3cb90\
                      9b8b2b829dd18d2a35ff82d995349e85\
                      5bf02c298ef775f2d1a7e8b8";

/// The sequence number the example's packet was sent under.
const SEQUENCE: u32 = 7;

/// The key, as the two instances take it.
fn key() -> [u8; KEY_BYTES] {
    let bytes = unhex(KEY);
    let mut key = [0u8; KEY_BYTES];
    for (slot, byte) in key.iter_mut().zip(bytes) {
        *slot = byte;
    }
    key
}

#[test]
fn the_worked_example_of_the_draft_is_what_this_cipher_produces() {
    let cipher = ChaChaPoly::new(&key());
    let mut frame = unhex(PLAIN);
    let mut tag = [0u8; TAG_BYTES];
    assert_eq!(cipher.seal(SEQUENCE, &mut frame, &mut tag), Ok(()));

    let mut sealed = frame.clone();
    sealed.extend_from_slice(&tag);
    assert_eq!(sealed, unhex(SEALED));
}

#[test]
fn the_length_field_is_read_without_the_rest_of_the_packet() {
    // Figure 8: the encrypted length field, and the 0x48 under it.
    let cipher = ChaChaPoly::new(&key());
    let sealed = unhex(SEALED);
    let head: [u8; 4] = sealed.first_chunk::<4>().copied().unwrap_or([0, 0, 0, 0]);
    assert_eq!(head, unhex("2c3ecce4").as_slice());
    assert_eq!(cipher.length(SEQUENCE, &head), 0x48);
}

#[test]
fn what_the_example_sealed_opens_as_the_packet_it_was() {
    let cipher = ChaChaPoly::new(&key());
    let sealed = unhex(SEALED);
    let (frame, tag) = sealed.split_at(sealed.len().saturating_sub(TAG_BYTES));
    let mut frame = frame.to_vec();
    assert_eq!(cipher.open(SEQUENCE, &mut frame, tag), Ok(()));
    assert_eq!(frame, unhex(PLAIN));
}

#[test]
fn a_tag_that_does_not_check_decrypts_nothing() {
    let cipher = ChaChaPoly::new(&key());
    let sealed = unhex(SEALED);
    let (body, tag) = sealed.split_at(sealed.len().saturating_sub(TAG_BYTES));
    let mut tag = tag.to_vec();
    if let Some(byte) = tag.first_mut() {
        *byte ^= 0x01;
    }
    let mut frame = body.to_vec();
    assert_eq!(cipher.open(SEQUENCE, &mut frame, &tag), Err(SshError::Tag));
    assert_eq!(frame, body, "nothing was decrypted");
}

#[test]
fn another_sequence_number_is_another_packet_altogether() {
    // The nonce is the sequence number, so the same bytes under the next
    // number neither seal to the same thing nor open.
    let cipher = ChaChaPoly::new(&key());
    let mut frame = unhex(PLAIN);
    let mut tag = [0u8; TAG_BYTES];
    assert_eq!(cipher.seal(SEQUENCE + 1, &mut frame, &mut tag), Ok(()));
    let mut sealed = frame.clone();
    sealed.extend_from_slice(&tag);
    assert_ne!(sealed, unhex(SEALED));

    let expected = unhex(SEALED);
    let (body, tag) = expected.split_at(expected.len().saturating_sub(TAG_BYTES));
    let mut frame = body.to_vec();
    assert_eq!(
        cipher.open(SEQUENCE + 1, &mut frame, tag),
        Err(SshError::Tag)
    );
}

#[test]
fn a_frame_without_a_length_field_is_refused_at_both_ends() {
    let cipher = ChaChaPoly::new(&key());
    let mut tag = [0u8; TAG_BYTES];
    let mut short = [0u8; 3];
    assert_eq!(
        cipher.seal(SEQUENCE, &mut short, &mut tag),
        Err(SshError::OutOfBounds {
            needed: 4,
            available: 3
        })
    );
    // Opening one has to pass the tag first, so the tag of those three
    // bytes is what gets it as far as the length field.
    let mut frame = [0u8; 3];
    let mut sealed = [0u8; 3];
    let good = ChaChaPoly::new(&key());
    let _ = good.seal(SEQUENCE, &mut sealed, &mut tag);
    let key = crypto_aead::poly1305::Poly1305::tag_of(&tag_key(), &frame);
    assert_eq!(
        cipher.open(SEQUENCE, &mut frame, &key),
        Err(SshError::OutOfBounds {
            needed: 4,
            available: 3
        })
    );
}

/// The Poly1305 key of the example's packet, which figure 15 prints as a
/// matrix and this recomputes the same way the cipher does.
fn tag_key() -> [u8; 32] {
    let bytes = key();
    let (halves, _) = bytes.as_chunks::<32>();
    let first = halves.first().copied().unwrap_or([0u8; 32]);
    let cipher = crypto_aead::chacha20::ChaCha20::new(&first);
    let mut nonce = [0u8; 12];
    nonce[11] = 7;
    let block = cipher.block(&nonce, 0);
    let mut key = [0u8; 32];
    for (slot, byte) in key.iter_mut().zip(block) {
        *slot = byte;
    }
    key
}
