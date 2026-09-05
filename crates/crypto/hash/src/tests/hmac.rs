// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! HMAC against the vectors of RFC 4231, section 4.

use crypto_ct::Choice;

use crate::hmac::Hmac;
use crate::sha256::Sha256;
use crate::sha512::{Sha384, Sha512};
use crate::tests::hex;

/// One test case of RFC 4231 with the three tags this crate can produce.
struct Case {
    /// The label the RFC gives the case.
    name: &'static str,
    /// The key.
    key: Vec<u8>,
    /// The message.
    message: Vec<u8>,
    /// The expected tag under SHA-256.
    sha256: &'static str,
    /// The expected tag under SHA-384.
    sha384: &'static str,
    /// The expected tag under SHA-512.
    sha512: &'static str,
}

/// The seven cases of RFC 4231, section 4.
fn cases() -> Vec<Case> {
    vec![
    Case {
        name: "one",
        key: vec![0x0b; 20],
        message: b"Hi There".to_vec(),
        sha256: "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        sha384: "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6",
        sha512: "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
    },
    Case {
        name: "two",
        key: b"Jefe".to_vec(),
        message: b"what do ya want for nothing?".to_vec(),
        sha256: "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        sha384: "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649",
        sha512: "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
    },
    Case {
        name: "three",
        key: vec![0xaa; 20],
        message: vec![0xdd; 50],
        sha256: "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
        sha384: "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b2a5ab39dc13814b94e3ab6e101a34f27",
        sha512: "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb",
    },
    Case {
        name: "four",
        key: (1u8..=25).collect::<Vec<u8>>(),
        message: vec![0xcd; 50],
        sha256: "82558a389a443c0ea4cc819899f2083a85f0faa3e578f8077a2e3ff46729665b",
        sha384: "3e8a69b7783c25851933ab6290af6ca77a9981480850009cc5577c6e1f573b4e6801dd23c4a7d679ccf8a386c674cffb",
        sha512: "b0ba465637458c6990e5a8c5f61d4af7e576d97ff94b872de76f8050361ee3dba91ca5c11aa25eb4d679275cc5788063a5f19741120c4f2de2adebeb10a298dd",
    },
    Case {
        name: "five",
        key: vec![0x0c; 20],
        message: b"Test With Truncation".to_vec(),
        sha256: "a3b6167473100ee06e0c796c2955552bfa6f7c0a6a8aef8b93f860aab0cd20c5",
        sha384: "3abf34c3503b2a23a46efc619baef897f4c8e42c934ce55ccbae9740fcbc1af4ca62269e2a37cd88ba926341efe4aeea",
        sha512: "415fad6271580a531d4179bc891d87a650188707922a4fbb36663a1eb16da008711c5b50ddd0fc235084eb9d3364a1454fb2ef67cd1d29fe6773068ea266e96b",
    },
    Case {
        name: "six",
        key: vec![0xaa; 131],
        message: b"Test Using Larger Than Block-Size Key - Hash Key First".to_vec(),
        sha256: "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54",
        sha384: "4ece084485813e9088d2c63a041bc5b44f9ef1012a2b588f3cd11f05033ac4c60c2ef6ab4030fe8296248df163f44952",
        sha512: "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f3526b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598",
    },
    Case {
        name: "seven",
        key: vec![0xaa; 131],
        message: b"This is a test using a larger than block-size key and a larger than block-size data. The key needs to be hashed before being used by the HMAC algorithm.".to_vec(),
        sha256: "9b09ffa71b942fcb27635fbcd5b0e944bfdc63644f0713938a7f51535c3a35e2",
        sha384: "6617178e941f020d351e2f254e8fd32c602420feb0b8fb9adccebb82461e99c5a678cc31e799176d3860e6110c46523e",
        sha512: "e37b6a775dc87dbaa4dfa9f96e5e3ffddebd71f8867289865df5a32d20cdc944b6022cac3c4982b10d5eeb55c3e4de15134676fb6de0446065c97440fa8c6a58",
    },
    ]
}

#[test]
fn the_rfc_vectors_authenticate_as_documented() {
    for case in cases() {
        assert_eq!(
            hex(Hmac::<Sha256>::tag(&case.key, &case.message).as_ref()),
            case.sha256,
            "case {} under SHA-256",
            case.name
        );
        assert_eq!(
            hex(Hmac::<Sha384>::tag(&case.key, &case.message).as_ref()),
            case.sha384,
            "case {} under SHA-384",
            case.name
        );
        assert_eq!(
            hex(Hmac::<Sha512>::tag(&case.key, &case.message).as_ref()),
            case.sha512,
            "case {} under SHA-512",
            case.name
        );
    }
}

#[test]
fn a_message_fed_in_pieces_gives_the_same_tag() {
    let key = [0x0bu8; 20];
    let message = b"Hi There";
    for split in 0..=message.len() {
        let (head, tail) = message.split_at(split);
        let mut mac = Hmac::<Sha256>::new(&key);
        mac.update(head);
        mac.update(tail);
        assert_eq!(
            mac.finish(),
            Hmac::<Sha256>::tag(&key, message),
            "split at {split}"
        );
    }
}

#[test]
fn a_key_of_exactly_one_block_takes_the_padding_path() {
    let short_block = [0x5au8; 64];
    let long_block = [0x5au8; 128];
    assert_eq!(
        hex(Hmac::<Sha256>::tag(&short_block, b"message").as_ref()),
        "67437bb07e3af377548cc5d9cc6cf9a45a0611033cd59e800e8a27fd4ba418fa"
    );
    assert_eq!(
        hex(Hmac::<Sha512>::tag(&long_block, b"message").as_ref()),
        "40c291da6c741e1966f8e8d6c74dc24cb36841ca35958e44b214c8ac82cd4fb207138eb2cfb43b24a229a53ff8beb7bed2a316d789baa1c99a52d88e088ea560"
    );
}

#[test]
fn a_key_one_byte_longer_than_a_block_is_hashed_first() {
    let key = [0x5au8; 65];
    assert_eq!(
        hex(Hmac::<Sha256>::tag(&key, b"message").as_ref()),
        "51a150878fcd81ffca19e1efd75865087e60a5f4ab44c204e5012b8d3467fac2"
    );
}

#[test]
fn an_empty_key_and_an_empty_message_are_accepted() {
    assert_eq!(
        hex(Hmac::<Sha256>::tag(b"", b"").as_ref()),
        "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad"
    );
}

#[test]
fn verification_accepts_the_right_tag_and_rejects_every_other() {
    let key = [0x0bu8; 20];
    let message = b"Hi There";
    let tag = Hmac::<Sha256>::tag(&key, message);
    assert_eq!(Hmac::<Sha256>::verify(&key, message, &tag), Choice::YES);

    for position in 0..tag.len() {
        let mut damaged = tag;
        if let Some(byte) = damaged.get_mut(position) {
            *byte ^= 0x01;
        }
        assert_eq!(
            Hmac::<Sha256>::verify(&key, message, &damaged),
            Choice::NO,
            "flipped byte {position}"
        );
    }

    assert_eq!(
        Hmac::<Sha256>::verify(&key, b"Hi there", &tag),
        Choice::NO,
        "a changed message must not verify"
    );
    assert_eq!(
        Hmac::<Sha256>::verify(&[0x0cu8; 20], message, &tag),
        Choice::NO,
        "a changed key must not verify"
    );
}

#[test]
fn a_tag_of_the_wrong_length_is_rejected_rather_than_truncated() {
    let key = [0x0bu8; 20];
    let tag = Hmac::<Sha256>::tag(&key, b"Hi There");
    assert_eq!(
        Hmac::<Sha256>::verify(&key, b"Hi There", &tag[..16]),
        Choice::NO
    );
    assert_eq!(Hmac::<Sha256>::verify(&key, b"Hi There", &[]), Choice::NO);
}

#[test]
fn a_clone_of_a_code_continues_the_same_message() {
    let mut mac = Hmac::<Sha256>::new(b"key");
    mac.update(b"abc");
    let clone = mac.clone();
    mac.update(b"def");
    assert_eq!(clone.finish(), Hmac::<Sha256>::tag(b"key", b"abc"));
    assert_eq!(mac.finish(), Hmac::<Sha256>::tag(b"key", b"abcdef"));
}
