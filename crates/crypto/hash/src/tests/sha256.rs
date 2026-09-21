// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! SHA-256 against the vectors of FIPS 180-4 and its appendices.

use test_support::generators::bytes;
use test_support::property::check;

use crate::hash::Hash;
use crate::sha256::{BLOCK_LEN, LENGTH_LEN, MAX_MESSAGE_LEN, Sha256, zeros_after};
use crate::tests::hex;

#[test]
fn the_fips_vectors_hash_as_documented() {
    assert_eq!(
        hex(&Sha256::digest(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&Sha256::digest(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&Sha256::digest(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        hex(&Sha256::digest(
            b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
        )),
        "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
    );
}

#[test]
fn the_million_character_message_hashes_as_documented() {
    let mut state = Sha256::new();
    for _ in 0..1_000 {
        state.update(&[b'a'; 1_000]);
    }
    assert_eq!(
        hex(&state.finish()),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn the_padding_boundaries_hash_correctly() {
    let cases: [(usize, &str); 9] = [
        (
            55,
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
        ),
        (
            56,
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
        ),
        (
            57,
            "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6",
        ),
        (
            63,
            "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
        ),
        (
            64,
            "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
        ),
        (
            65,
            "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
        ),
        (
            119,
            "31eba51c313a5c08226adf18d4a359cfdfd8d2e816b13f4af952f7ea6584dcfb",
        ),
        (
            120,
            "2f3d335432c70b580af0e8e1b3674a7c020d683aa5f73aaaedfdc55af904c21c",
        ),
        (
            128,
            "6836cf13bac400e9105071cd6af47084dfacad4e5e302c94bfed24e013afb73e",
        ),
    ];
    for (length, expected) in cases {
        let message = vec![b'a'; length];
        assert_eq!(hex(&Sha256::digest(&message)), expected, "length {length}");
    }
}

#[test]
fn the_trait_agrees_with_the_inherent_functions() {
    assert_eq!(<Sha256 as Hash>::digest(b"abc"), Sha256::digest(b"abc"));
    let mut state = <Sha256 as Hash>::new();
    Hash::update(&mut state, b"abc");
    assert_eq!(Hash::finish(state), Sha256::digest(b"abc"));
    assert_eq!(Sha256::default().finish(), Sha256::digest(b""));
    assert_eq!(<Sha256 as Hash>::BLOCK_LEN, 64);
    assert_eq!(<Sha256 as Hash>::OUTPUT_LEN, 32);
    assert_eq!(<Sha256 as Hash>::ZERO_BLOCK, [0u8; 64]);
}

#[test]
fn a_clone_continues_the_same_message() {
    let mut state = Sha256::new();
    state.update(b"abcdef");
    let clone = state.clone();
    state.update(b"ghi");
    assert_eq!(clone.finish(), Sha256::digest(b"abcdef"));
    assert_eq!(state.finish(), Sha256::digest(b"abcdefghi"));
}

#[test]
fn property_any_split_of_a_message_gives_the_same_digest() {
    check("sha256_chunked", &bytes(0..=200), |message| {
        let expected = Sha256::digest(message);
        for split in 0..=message.len() {
            let (head, tail) = message.split_at(split);
            let mut state = Sha256::new();
            state.update(head);
            state.update(tail);
            if state.finish() != expected {
                return Err(format!("split at {split} differs"));
            }
        }
        Ok(())
    });
}

#[test]
fn property_byte_at_a_time_matches_the_one_shot_digest() {
    check("sha256_byte_at_a_time", &bytes(0..=140), |message| {
        let mut state = Sha256::new();
        for byte in message {
            state.update(&[*byte]);
        }
        if state.finish() == Sha256::digest(message) {
            Ok(())
        } else {
            Err("byte-wise hashing differs".to_owned())
        }
    });
}

/// The counter clamps at the end of the domain FIPS 180-4, section 1,
/// gives SHA-256. It used to wrap, which put a message of 2^61 bytes at
/// the bit length of a short one and so under that message's digest.
#[test]
fn the_byte_counter_clamps_at_the_end_of_the_domain() {
    let mut state = Sha256::new();
    state.set_counted(MAX_MESSAGE_LEN.saturating_sub(64));
    state.update(&[0u8; 64]);
    assert_eq!(state.counted(), MAX_MESSAGE_LEN);
    state.update(&[0u8; 64]);
    assert_eq!(state.counted(), MAX_MESSAGE_LEN);
    assert_eq!(
        MAX_MESSAGE_LEN.checked_mul(8),
        Some(u64::MAX.wrapping_sub(7)),
        "the clamped counter still fits the bit length the padding encodes"
    );
}

#[test]
fn clearing_a_state_drops_the_chaining_words_and_the_partial_block() {
    let mut first = Sha256::new();
    first.update(b"one message");
    let mut second = Sha256::new();
    second.update(b"a different message, longer than the first");
    first.clear();
    second.clear();
    assert_eq!(
        hex(&first.finish()),
        hex(&second.finish()),
        "nothing of either message is left"
    );
}

#[test]
fn a_cleared_state_is_not_a_fresh_state() {
    let mut cleared = Sha256::new();
    cleared.clear();
    assert_ne!(
        hex(&cleared.finish()),
        hex(&Sha256::digest(b"")),
        "the initial words are overwritten, not restored"
    );
}

/// Regression for issue #49: the padding used to absorb one zero byte per
/// call and now absorbs one slice whose length this function gives. Every
/// buffer state has to reach the start of the length field, and no state
/// may take a run of a whole block or more.
#[test]
fn the_padding_run_reaches_the_start_of_the_length_field() {
    let target = BLOCK_LEN - LENGTH_LEN;
    for buffered in 0..BLOCK_LEN {
        let run = zeros_after(buffered);
        assert!(run < BLOCK_LEN, "buffered {buffered} takes a whole block");
        assert_eq!((buffered + run) % BLOCK_LEN, target, "buffered {buffered}");
    }
}

/// The digest of `length` repetitions of `a`, for every length from zero
/// to three blocks, taken from an independent implementation.
const DIGESTS: [&str; 201] = [
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb",
    "961b6dd3ede3cb8ecbaacbd68de040cd78eb2ed5889130cceb4c49268ea4d506",
    "9834876dcfb05cb167a5c24953eba58c4ac89b1adf57f28f2f9d09af107ee8f0",
    "61be55a8e2f6b4e172338bddf184d6dbee29c98853e0a0485ecee7f27b9af0b4",
    "ed968e840d10d2d313a870bc131a4e2c311d7ad09bdf32b3418147221f51a6e2",
    "ed02457b5c41d964dbd2f2a609d63fe1bb7528dbe55e1abf5b52c249cd735797",
    "e46240714b5db3a23eee60479a623efba4d633d27fe4f03c904b9e219a7fbe60",
    "1f3ce40415a2081fa3eee75fc39fff8e56c22270d1a978a7249b592dcebd20b4",
    "f2aca93b80cae681221f0445fa4e2cae8a1f9f8fa1e1741d9639caad222f537d",
    "bf2cb58a68f684d95a3b78ef8f661c9a4e5b09e82cc8f9cc88cce90528caeb27",
    "28cb017dfc99073aa1b47c1b30f413e3ce774c4991eb4158de50f9dbb36d8043",
    "f24abc34b13fade76e805799f71187da6cd90b9cac373ae65ed57f143bd664e5",
    "a689d786e81340e45511dec6c7ab2d978434e5db123362450fe10cfac70d19d0",
    "82cab7df0abfb9d95dca4e5937ce2968c798c726fea48c016bf9763221efda13",
    "ef2df0b539c6c23de0f4cbe42648c301ae0e22e887340a4599fb4ef4e2678e48",
    "0c0beacef8877bbf2416eb00f2b5dc96354e26dd1df5517320459b1236860f8c",
    "b860666ee2966dd8f903be44ee605c6e1366f926d9f17a8f49937d11624eb99d",
    "c926defaaa3d13eda2fc63a553bb7fb7326bece6e7cb67ca5296e4727d89bab4",
    "a0b4aaab8a966e2193ba172d68162c4656860197f256b5f45f0203397ff3f99c",
    "42492da06234ad0ac76f5d5debdb6d1ae027cffbe746a1c13b89bb8bc0139137",
    "7df8e299c834de198e264c3e374bc58ecd9382252a705c183beb02f275571e3b",
    "ec7c494df6d2a7ea36668d656e6b8979e33641bfea378c15038af3964db057a3",
    "897d3e95b65f26676081f8b9f3a98b6ee4424566303e8d4e7c7522ebae219eab",
    "09f61f8d9cd65e6a0c258087c485b6293541364e42bd97b2d7936580c8aa3c54",
    "2f521e2a7d0bd812cbc035f4ed6806eb8d851793b04ba147e8f66b72f5d1f20f",
    "9976d549a25115dab4e36d0c1fb8f31cb07da87dd83275977360eb7dc09e88de",
    "cc0616e61cbd6e8e5e34e9fb2d320f37de915820206f5696c31f1fbd24aa16de",
    "9c547cb8115a44883b9f70ba68f75117cd55359c92611875e386f8af98c172ab",
    "6913c9c7fd42fe23df8b6bcd4dbaf1c17748948d97f2980b432319c39eddcf6c",
    "3a54fc0cbc0b0ef48b6507b7788096235d10292dd3ae24e22f5aa062d4f9864a",
    "61c60b487d1a921e0bcc9bf853dda0fb159b30bf57b2e2d2c753b00be15b5a09",
    "3ba3f5f43b92602683c19aee62a20342b084dd5971ddd33808d81a328879a547",
    "852785c805c77e71a22340a54e9d95933ed49121e7d2bf3c2d358854bc1359ea",
    "a27c896c4859204843166af66f0e902b9c3b3ed6d2fd13d435abc020065c526f",
    "629362afc62c74497caed2272e30f8125ecd0965f8d8d7cfc4e260f7f8dd319d",
    "22c1d24bcd03e9aee9832efccd6da613fc702793178e5f12c945c7b67ddda933",
    "21ec055b38ce759cd4d0f477e9bdec2c5b8199945db4439bae334a964df6246c",
    "365a9c3e2c2af0a56e47a9dac51c2c5381bf8f41273bad3175e0e619126ad087",
    "b4d5e56e929ba4cda349e9274e3603d0be246b82016bca20f363963c5f2d6845",
    "e33cdf9c7f7120b98e8c78408953e07f2ecd183006b5606df349b4c212acf43e",
    "c0f8bd4dbc2b0c03107c1c37913f2a7501f521467f45dd0fef6958e9a4692719",
    "7a538607fdaab9296995929f451565bbb8142e1844117322aafd2b3d76b01aff",
    "66d34fba71f8f450f7e45598853e53bfc23bbd129027cbb131a2f4ffd7878cd0",
    "16849877c6c21ef0bfa68e4f6747300ddb171b170b9f00e189edc4c2fc4db93e",
    "52789e3423b72beeb898456a4f49662e46b0cbb960784c5ef4b1399d327e7c27",
    "6643110c5628fff59edf76d82d5bf573bf800f16a4d65dfb1e5d6f1a46296d0b",
    "11eaed932c6c6fddfc2efc394e609facf4abe814fc6180d03b14fce13a07d0e5",
    "97daac0ee9998dfcad6c9c0970da5ca411c86233a944c25b47566f6a7bc1ddd5",
    "8f9bec6a62dd28ebd36d1227745592de6658b36974a3bb98a4c582f683ea6c42",
    "160b4e433e384e05e537dc59b467f7cb2403f0214db15c5db58862a3f1156d2e",
    "bfc5fe0e360152ca98c50fab4ed7e3078c17debc2917740d5000913b686ca129",
    "6c1b3dc7a706b9dc81352a6716b9c666c608d8626272c64b914ab05572fc6e84",
    "abe346a7259fc90b4c27185419628e5e6af6466b1ae9b5446cac4bfc26cf05c4",
    "a3f01b6939256127582ac8ae9fb47a382a244680806a3f613a118851c1ca1d47",
    "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
    "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
    "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6",
    "d5c039b748aa64665782974ec3dc3025c042edf54dcdc2b5de31385b094cb678",
    "111bb261277afd65f0744b247cd3e47d386d71563d0ed995517807d5ebd4fba3",
    "11ee391211c6256460b6ed375957fadd8061cafbb31daf967db875aebd5aaad4",
    "35d5fc17cfbbadd00f5e710ada39f194c5ad7c766ad67072245f1fad45f0f530",
    "f506898cc7c2e092f9eb9fadae7ba50383f5b46a2a4fe5597dbb553a78981268",
    "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
    "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
    "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
    "ac137fce49837c7c2945f6160d3c0e679e6f40070850420a22bc10e0692cbdc7",
    "6116c09f89718e829f69b7afb1d9d60c6973935d96b33213e4d28689fe108ee7",
    "574c61e5f2aad36ec528ecf4c6de44793f9dab06b38d1c5a41f8a9d64b4ed53b",
    "ef5ed46eed2002191724db57b334457889c9ad4f3e7027cdd7dd41340db75d11",
    "6bd5e5034855a11241f0dee8fc72850ffd9955b28347a86428b5fa19119f6ad0",
    "eefa4cfbea79400c2f4239e1f702e02ebece761f78b6a35c9d2c167a79f9570c",
    "d66304b6180365e47c858f6c84d3da065caf4b3350c9f45277a1af82e3dbb055",
    "0e058e3f7d0439f9054d59c735587ae99655f6473a234ce494d82b5586f7eac6",
    "f638359d0db860cf30cb5e6744986938c70153043efe8a348354b275d87ece74",
    "8af881bc88895bd9d8cea975a7d06dc0275d9db9d57f138216936b65e8b06489",
    "f8b8aba652e5b3cde6bc74bcb7bff15289a222cfdb7759a9809dc08574911f60",
    "bc19b9d355411a4e3d3426f5d39bfc9a856ca5f042bc3a468572080086ad1384",
    "d3477171d6f80a02b6415538fdd582144910c9b8482b982a1b62b441f21dc819",
    "daa76b4d16679957833226a8844506098a37bac94979f1cbfc6c4ac3b09b9b60",
    "0f45e858fbc4176cdf4e411f88281edefc390ae5afe7df0f44cd9297f0a64580",
    "8c48280d57fb88f161adf34d9f597d93ca32b7edfcd23b2afe64c3789b3f7855",
    "f5854359819cdcf44e17e76eb800ce37d7733ebb9193756195731f21964d31a1",
    "e14eff3310babb7d63933e8139833b5f2f0a3ec80a0bd01edede97f3c1edd52b",
    "f5475022feb69870295b9c1e78c5a4919374061d5345167815801879f931ebb0",
    "842ef530b6dd8c51176fd95d9945df3a756506aa8c4a35fa68bb5e1fa14aca64",
    "cdde4c26febdf9d2f3b34a1dea95bbd2280bf2207f64a1b404608e31a3c4b197",
    "cfb924c52d9c3df5a853c41b57ff8941f56e75e0636350ba4b3590fb14e6c42d",
    "97538d8661e8d75c8fc59f7a759b1baeb3cbf6ca282b90016fa76cc05a36e6d8",
    "5e4b545fa5e2fc35422bc4244ef188745ab80b532436cf8a6a1c44551a235bd8",
    "6b2ec79170f50ea57b886dc81a2cf78721c651a002c8365a524019a7ed5a8a40",
    "9b9fe7f0a48c2b9aeb70fa0828c10780a1597e18f671eb284e0fb2e11c9a7ba8",
    "bb882439013bfb093c58bbe94a21da6a04a510da9a6c9697bb4ae8dd67418a2b",
    "96e78b5a83de995f8fa002fca4f182d8bfe8ca955976fbb28d39dcbd94be1493",
    "85094a38651a070207736483ada17d98298a80125c005c74d58997bf5d2ac690",
    "e1c84483d1d8887780218845bcbf6cee4b28373a2bbd1695e7d3e2d24af63097",
    "ee4caa5518a866f33e174d6e71ba3961a86ca00a7486b132e5a9f01bfaa1d794",
    "2a2c60fe96afac9151d4bd932fdb45be2c7e175e02b0598426cac15c38511cb4",
    "970d31b428de8fea74b16484d8a8adb2c9e1bd974d7c621fd04332bc3499f117",
    "132844681d57d5ae76dfd35a07e4fc10fdc0ce9f0882a2b0c9db0e84116d5bad",
    "2816597888e4a0d3a36b82b83316ab32680eb8f00f8cd3b904d681246d285a0e",
    "9d0793397991b57a99a07c6e6b4a92bab68dbf605345cd0b87f385a448a726bc",
    "f2183711cffcdd20b99b71fda77455f60b957cfd22479d2a85b87d585a5b8305",
    "e1d56dc5b4a1a4f47e17a6e429ddc6adba71256ba4815df3e97ad4af0dfb4db1",
    "41e5c920c12114588b8e5ccdf26d891a6967fefeb77e1b1a2f4b7fb5925216c6",
    "63154a4c82b99c7b09c010d97138ecea5ba2c7e176e8b46857ce4378b6f25874",
    "5d5c8fb6b61493248e1843ef8fb9875ff28b92075e6eb87e153caf4698bdacc3",
    "3e24531cdaa595ab56f976b96c1a1df8009eabec300a5a0261c0e44f47a43b89",
    "0cef0da81c8b79cfb13f95c379a5b6392662e3623dde5bcc5bf33a0111356c0e",
    "389f29f2dfa9409ee9176ec6565d8a96839516b16973461f19f7b9d8c73a8225",
    "91c87492c42ba6d401099e0d7c909304e316510dfe4b6e1a801bcc48963e61cb",
    "6374f73208854473827f6f6a3f43b1f53eaa3b82c21c1a6d69a2110b2a79baad",
    "f54353008a2553262ecdc4a34749563ba0950e8b0fc8652780b0a614b99683c1",
    "ba02731ae695aae5cd49b49d84330b63995733eb22102aca755f0179b1e0e20f",
    "6cb750b2181606511f4a7e4786b53692f51a9cdc4d3172fdcfb9267aeca7e4e4",
    "64410e651b346524cfe56e68c237ea76c0377921697027eb794a067501fb2910",
    "9caaa833c93273f927b4145f86a41791ea6e39a31d118a6aae3dbcdaac5f84ac",
    "e4f4e17a2d08bea9f614eee277c9be1de3c6c4b90a90525f7e0493e53c5c4d70",
    "879dc4f05b19ebc3b037f4683633df1332b054cf52fa372d323c421cb893a2aa",
    "31eba51c313a5c08226adf18d4a359cfdfd8d2e816b13f4af952f7ea6584dcfb",
    "2f3d335432c70b580af0e8e1b3674a7c020d683aa5f73aaaedfdc55af904c21c",
    "e9615320128cc7a3d6078e9af05603188e5ccbf0d07d8b735d3df5e8e0c1281f",
    "cf6c368c04c239cc1688a5a3b39f94a55ce5b7631e027fc0138f0548189c62fa",
    "6675ba780648c8506cb002c86621f9d33f8093e12541c690a8d3261c47c92bcc",
    "9c1ed60a6337af7d1659339ef95dccb7c91a55b9eaab349341a7f97198eee519",
    "a8e1a0f35c15c01a458bcb345528b751556c14850f6f9bdcc9933b8003d29b43",
    "36bcf9292589fe6ea3e82fefe3aab1b8ca8b8347ea5a14b23e470ecb3ad7c57b",
    "c57e9278af78fa3cab38667bef4ce29d783787a2f731d4e12200270f0c32320a",
    "6836cf13bac400e9105071cd6af47084dfacad4e5e302c94bfed24e013afb73e",
    "c12cb024a2e5551cca0e08fce8f1c5e314555cc3fef6329ee994a3db752166ae",
    "1e3c4f4750c8c29bbfa9ced317788176b156d342e57f7777f62fd7221a44312f",
    "3c29725085c0e486282716b0eaa714c7b1acfb3e5bc29fe15a4302b026f12194",
    "949c0f0c316b4805f1c6ca8f8e51ad06626b99461dc8d2119267ada76ddcf234",
    "d8daa29582b1421a30ee082493b6c82f11e200549ac169dc37785b785ebf4f0c",
    "363c626b2be88aff47b76af3c99032789a20ab851cbe04359da9f04d95371b41",
    "dfa58dfd72f3c7080d0249a7758fd3636872f63fa24b18473ed36f031e248347",
    "6f0e44b9ce4ea61d52a3479c10f60ef916937f799f11964b7f1c7771063905c4",
    "b6dc2da678c065ebdce374ebe1842728277203ee1a9a29832f058cf013d5ad85",
    "d2c5db392a1850a905f5ae9af3a6e3ecfd1bbb0ef06d5b618d278221bb7a6d28",
    "d49e337c5dd5458227c36eee64ffaeab8fd8cc43395999aabdf38b40bb489547",
    "c094ed2f614ab7a02e7557f8eba6b03b457ce7beacf1d8031088f97a1770e5e6",
    "6e816ec42ddb7baf8dc674b626a858bbffaf76b6f43c4b7ac94c82c9588c963b",
    "4b0b00b33140acea6c9634a11e3b751cd2f14e60848ac2b16b5248c5b19f2fa6",
    "2bd3f6efab5e871a28d37632e97cd83dfafe9822fedefa0bf54b53d486cfeb38",
    "b71df3b2ce77a2e2d78f7a7b91bf8868b61be0e8a12d300b1300b140d7fada13",
    "a1b186afc61494910f0a90621baefc4a5c0f4daeab84a344b511aa26a43814ca",
    "915bda2b208601a5fd51e6ad63dae50669038227884d065648a7e2677f87b1cf",
    "7388356d5b2955e6c64e465154e861b613de1eaa5fcec3e0814bc824013e78df",
    "7a36b0422fdcb13d670cf2d8c71358458627bd181e83436cb9567279a8c9e22a",
    "0ba3de9dc2b2ad1719d333e6bb1b13a70dfdf6ff030cd10d3cf57d8028a62f4f",
    "7595af82ae2fa59cd9bf3b4405d31c69b98de71fed5945fd777d8ab3b393a85f",
    "5416d58d0995ead40b1732bff8dee597da5996baae2e2373ce716987b677ee72",
    "e03013400c0bddd83e6d7d14ce28c3ecd8afd02a789651f54d7f4273dc0528eb",
    "1e6113e164f841cb83a5a1cc6805e1e76d36709a4b6d6381a7b8f61a69a63859",
    "d872285d3a590992d9c5e400597cb1dc3d03f0b026587884517a044693fc7376",
    "3018a75299cc5a9b310462ab40b97d47fd7c89ff61cd6fed75220520b109f73c",
    "b13e94fa92730fa8b0a844f439685ccbc4add792fb7c6465558cb9c1d4e42681",
    "55c43b2461cc47af2d0d69ee0fc5750ea5be00f376626dde7920f4d089753957",
    "21e5c186d692c621c3429f831438582db0b5d4b237fc099bc4199da336d7ff9e",
    "043350130caae98e0ee9e5a6a264912556191acc6962c777d55722269684b5b4",
    "bf18b43b61652b5d73f41ebf3d72e5e43aebf5076f497dde31ea3de9de4998ef",
    "1668eb4fad53ba7b37b233bcb819f4a0c38ee2a6d03e204134edcc882bb56ae8",
    "5ea73424311beb8d22b6fec2b13174b8fdb50dd6d1c8947dfe3a39cbedbb32a7",
    "eebf59a76255d00465005349dd5749d41ca0b5325caa659f5ca9fe4273b63e6a",
    "f32d572a3f9484bc969cec31a4e38b1acda8af1e4df3275df29aea93a165bbed",
    "e44758b5dba37d21fe5dfa314b0631cd608ad4ae4bc86b704d23b605b063d2c1",
    "f5639e2ff5ea92905d47c9b29bc3fd98142f53027fd781871b9c021bfb011840",
    "3feac2b8b234acd891b28600a61e09aa88c8d4132ef0cdce44ac4cbb9881ed8a",
    "17090cb45eb2f79637f79037312e7fe6d0a904287c33df804e74e19dc1da5f50",
    "42a24fed59929765b5d831e6b664a73106722606bfcbf23c70352f601d285a32",
    "5f2f638e593ea23068aa79fc42b32adeb207c59479dc3e2dd75e0c17bb7b0ef4",
    "47b41530cff7b48105959438c733534fb7d095d6f2688dff6d23d4642cd79669",
    "ef78bc8cd62a8dcc8f962d3faacce28cfaab53bc5675f29741f22d063f2f5f3a",
    "78c5cc71e6199f2029245f90b21182b1aeb3b5d2be95fb1a123fc7feb0a1ce76",
    "d69e5307d949d2edc25a747a2c0f9d9deaf341c84e1ba0fb48017220bd7a90a2",
    "011b49dc68918e993625cc2f7427f873be6c0331604b30c5a1b61bd18a91b90b",
    "4e85880044257f9b69a1368115d2f2d9940cc6d767f6dd68cecdf38c87ea3570",
    "1e00840b96b85f8847e4cbc9a965c4bac9d8d63c9f5b0418972e68a072b3efc1",
    "caad4b2fbb830fa54002a1675f1599d9b092cd398ad6fbb816bfe8f948cdd620",
    "f3136971913c3872c4b536b6d89908f7b6371b0653213f142c757a01722828c7",
    "d61a600a7c6f7dc2c2bf90f56f061df65b7ecc95c37ec4dd85776254ba8578d2",
    "9ce4e58a8e1314bf14ca8a52ac20fa1b1e27a78c40a24b5a287422f992b29b9d",
    "40bf63d544cc5cc5f62396793afd6e9b51114c5a39a9ea608b425c53a0da16cd",
    "a88d44a2940a3a2fc363304926d263bf271afb562bab5640cb0e81f5e84320a3",
    "8b1b7967cef5b2f2036b25c6a393e37bd774a7cbde7be79bb443d26bf0a90c86",
    "10449d158a5fcb9070fb43abdb35699387985dbdad24d1f7e81287d5c6127189",
    "c869356d3a3e6b1784e385ec19e49236100972e57447057f6a1b562c9a9010ca",
    "ee1af252d0c7b22336f29f8461a27fb70a3850e709e769b50402d1a8bda1ee03",
    "51662577d64ded54ab5f90ea923b5df4a01d5ccab80d5a3514c972a263d11d1c",
    "8f95c978db871e3ff5db22d5e3c7c27212d6310fd5768192dfc423ac44200a71",
    "09b87c3c7e387732895ac44d51e37540e8a11da7563c719cd81d348d92c96b47",
    "311834ddbafe20677dddbf9f4beb2b5b0b9e6ee97dddb70fce4e8f62c1b7518d",
    "7cee24628d290c16183532716cc5a8a889bc951b4b0a1507c32b8e29cee01052",
    "33f93a9879ef18f9779150b2dbace6f8cc17b29e1af6be4e1048fc647489f1c2",
    "aa73b13a51ab2ca9733957448439927115ed8afba1e2def0792eb50eb68d1655",
    "1f82c1acc9ec4bcc988950d392ec05bf1afe5a42642f84dc8d406ab57f9e1285",
    "96b864ee9e10a31f8425084ed115df8951bb8d1303cb13f43c0a600eb03643b0",
    "3a92099a8fae59371d3e7bfd8b968e0690c0ad98a581235cc62739abdd173aba",
    "441337206ef605eec6200e7fb6259982ba0f2e84b4b28e107e140eba36d81484",
    "60048478ae47edd7ef18f1235afd254a72ffaf32c4bc5726e8d250c3be51e3cb",
    "c2a908d98f5df987ade41b5fce213067efbcc21ef2240212a41e54b5e7c28ae5",
];

/// Regression for issue #49: every message length from zero to three
/// blocks, against digests taken from an independent implementation, so
/// that a padding run off by one block shows up wherever it sits.
#[test]
fn every_length_up_to_three_blocks_hashes_as_documented() {
    for (length, expected) in DIGESTS.iter().enumerate() {
        let message = vec![b'a'; length];
        assert_eq!(hex(&Sha256::digest(&message)), *expected, "length {length}");
    }
}
