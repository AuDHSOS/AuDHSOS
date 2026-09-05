// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SHA-512 and SHA-384 against the vectors of FIPS 180-4 and its
//! appendices.

use test_support::generators::bytes;
use test_support::property::check;

use crate::hash::Hash;
use crate::sha512::{Sha384, Sha512};
use crate::tests::hex;

#[test]
fn the_fips_vectors_hash_as_documented() {
    assert_eq!(
        hex(&Sha512::digest(b"")),
        "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
    );
    assert_eq!(
        hex(&Sha512::digest(b"abc")),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
    assert_eq!(
        hex(&Sha512::digest(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "204a8fc6dda82f0a0ced7beb8e08a41657c16ef468b228a8279be331a703c33596fd15c13b1b07f9aa1d3bea57789ca031ad85c7a71dd70354ec631238ca3445"
    );
    assert_eq!(
        hex(&Sha512::digest(
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        )),
        "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"
    );
    assert_eq!(
        hex(&Sha384::digest(b"")),
        "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
    );
    assert_eq!(
        hex(&Sha384::digest(b"abc")),
        "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
    );
    assert_eq!(
        hex(&Sha384::digest(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "3391fdddfc8dc7393707a65b1b4709397cf8b1d162af05abfe8f450de5f36bc6b0455a8520bc4e6f5fe95b1fe3c8452b"
    );
    assert_eq!(
        hex(&Sha384::digest(
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        )),
        "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"
    );
}

#[test]
fn the_million_character_message_hashes_as_documented() {
    let mut long = Sha512::new();
    let mut short = Sha384::new();
    for _ in 0..1_000 {
        long.update(&[b'a'; 1_000]);
        short.update(&[b'a'; 1_000]);
    }
    assert_eq!(
        hex(&long.finish()),
        "e718483d0ce769644e2e42c7bc15b4638e1f98b13b2044285632a803afa973ebde0ff244877ea60a4cb0432ce577c31beb009c5c2c49aa2e4eadb217ad8cc09b"
    );
    assert_eq!(
        hex(&short.finish()),
        "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985"
    );
}

#[test]
fn the_padding_boundaries_hash_correctly() {
    let long: [(usize, &str); 9] = [
        (
            111,
            "fa9121c7b32b9e01733d034cfc78cbf67f926c7ed83e82200ef86818196921760b4beff48404df811b953828274461673c68d04e297b0eb7b2b4d60fc6b566a2",
        ),
        (
            112,
            "c01d080efd492776a1c43bd23dd99d0a2e626d481e16782e75d54c2503b5dc32bd05f0f1ba33e568b88fd2d970929b719ecbb152f58f130a407c8830604b70ca",
        ),
        (
            113,
            "55ddd8ac210a6e18ba1ee055af84c966e0dbff091c43580ae1be703bdb85da31acf6948cf5bd90c55a20e5450f22fb89bd8d0085e39f85a86cc46abbca75e24d",
        ),
        (
            127,
            "828613968b501dc00a97e08c73b118aa8876c26b8aac93df128502ab360f91bab50a51e088769a5c1eff4782ace147dce3642554199876374291f5d921629502",
        ),
        (
            128,
            "b73d1929aa615934e61a871596b3f3b33359f42b8175602e89f7e06e5f658a243667807ed300314b95cacdd579f3e33abdfbe351909519a846d465c59582f321",
        ),
        (
            129,
            "4f681e0bd53cda4b5a2041cc8a06f2eabde44fb16c951fbd5b87702f07aeab611565b19c47fde30587177ebb852e3971bbd8d3fd30da18d71037dfbd98420429",
        ),
        (
            239,
            "52c853cb8d907f3d4d6b889beb027985d7c273486d75f8baf26f80d24e90c74c6c3de3e22131582380a7d14d43f2941a31385439cd6ddc469f628015e50bf286",
        ),
        (
            240,
            "4c296d90c61052a62ffb1dd196f1b7b09373b1f93e71836baebf89690546b7595684dbe9467a8e484fa0d1094272b4344a7c24f5fee8daedeb0bf549c985ab5f",
        ),
        (
            256,
            "6a9169eb662f136d87374070e8828b3e615a7eca32a89446e9225b02832709be095e635c824a2bb70213ba2ea0ababac0809827843992c851903b7ac0c136699",
        ),
    ];
    for (length, expected) in long {
        let message = vec![b'a'; length];
        assert_eq!(hex(&Sha512::digest(&message)), expected, "length {length}");
    }
    let short: [(usize, &str); 9] = [
        (
            111,
            "3c37955051cb5c3026f94d551d5b5e2ac38d572ae4e07172085fed81f8466b8f90dc23a8ffcdea0b8d8e58e8fdacc80a",
        ),
        (
            112,
            "187d4e07cb306103c69967bf544d0dfbe9042577599c73c330abc0cb64c61236d5ed565ee19119d8c31779a38f791fcd",
        ),
        (
            113,
            "1d6bed01626682961b50da078a6b1da707c1da0c8a0a3226f159235bd45ed724a0622fa6f39fd70007a6c72a5cda43ae",
        ),
        (
            127,
            "9bd06b1763c2cf7aef40e795dc65bc96d59c41b537f3ad72ebdefd485476b5717c1aeb37c327fe9c1831b12b9efd08ae",
        ),
        (
            128,
            "edb12730a366098b3b2beac75a3bef1b0969b15c48e2163c23d96994f8d1bef760c7e27f3c464d3829f56c0d53808b0b",
        ),
        (
            129,
            "39b6f5a7b0e781dbc419f72e49b30eaac10f2c98c4403bc610da31067fd1b48f324138c8615d2b496d08d73d5e865326",
        ),
        (
            239,
            "e247c35f4bc1aa38026f8880c8c97305545d00d3f859e00c57d1c1f0a176b3c6b749c4eb081f08bd0fba500969cd056a",
        ),
        (
            240,
            "4d86957beab348a29180f02d02564ac1d32f5b4c217ece2b038f7c184f0cafc8c8e438eb82aa03796170e0a7ce8c0675",
        ),
        (
            256,
            "ee89d91a5f594f72052c561e5c2458280439eaa77cc1352e27893931c6d9ce5d869fb8a024358c460adc1af9f4fe5b4a",
        ),
    ];
    for (length, expected) in short {
        let message = vec![b'a'; length];
        assert_eq!(hex(&Sha384::digest(&message)), expected, "length {length}");
    }
}

#[test]
fn the_short_digest_is_the_prefix_of_the_long_one_under_its_own_state() {
    let short = Sha384::digest(b"abc");
    let long = Sha512::digest(b"abc");
    assert_eq!(short.len(), 48);
    assert_eq!(long.len(), 64);
    assert_ne!(&short[..], &long[..48]);
}

#[test]
fn the_trait_agrees_with_the_inherent_functions() {
    assert_eq!(<Sha512 as Hash>::digest(b"abc"), Sha512::digest(b"abc"));
    assert_eq!(<Sha384 as Hash>::digest(b"abc"), Sha384::digest(b"abc"));
    let mut long = <Sha512 as Hash>::new();
    Hash::update(&mut long, b"abc");
    assert_eq!(Hash::finish(long), Sha512::digest(b"abc"));
    let mut short = <Sha384 as Hash>::new();
    Hash::update(&mut short, b"abc");
    assert_eq!(Hash::finish(short), Sha384::digest(b"abc"));
    assert_eq!(Sha512::default().finish(), Sha512::digest(b""));
    assert_eq!(Sha384::default().finish(), Sha384::digest(b""));
    assert_eq!(<Sha512 as Hash>::BLOCK_LEN, 128);
    assert_eq!(<Sha512 as Hash>::OUTPUT_LEN, 64);
    assert_eq!(<Sha384 as Hash>::OUTPUT_LEN, 48);
    assert_eq!(<Sha384 as Hash>::ZERO_BLOCK, [0u8; 128]);
}

#[test]
fn a_clone_continues_the_same_message() {
    let mut state = Sha512::new();
    state.update(b"abcdef");
    let clone = state.clone();
    state.update(b"ghi");
    assert_eq!(clone.finish(), Sha512::digest(b"abcdef"));
    assert_eq!(state.finish(), Sha512::digest(b"abcdefghi"));
    let mut short = Sha384::new();
    short.update(b"abcdef");
    let clone = short.clone();
    short.update(b"ghi");
    assert_eq!(clone.finish(), Sha384::digest(b"abcdef"));
    assert_eq!(short.finish(), Sha384::digest(b"abcdefghi"));
}

#[test]
fn property_any_split_of_a_message_gives_the_same_digest() {
    check("sha512_chunked", &bytes(0..=400), |message| {
        let expected = Sha512::digest(message);
        let expected_short = Sha384::digest(message);
        for split in 0..=message.len() {
            let (head, tail) = message.split_at(split);
            let mut long = Sha512::new();
            long.update(head);
            long.update(tail);
            let mut short = Sha384::new();
            short.update(head);
            short.update(tail);
            if long.finish() != expected || short.finish() != expected_short {
                return Err(format!("split at {split} differs"));
            }
        }
        Ok(())
    });
}
