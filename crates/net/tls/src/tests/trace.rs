// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The handshake of RFC 8448, section 3, "Simple 1-RTT Handshake".
//!
//! Every constant here is transcribed from `docs/rfc/rfc8448.txt`, which
//! this repository keeps verbatim (D-59). The names are the ones the
//! document gives the blocks it prints.

/// The client's ephemeral private key, which the scripted generator of the
/// state machine's trace test will hand out.
#[expect(
    dead_code,
    reason = "the state machine that consumes it is the third part of this step"
)]
pub(crate) const CLIENT_PRIVATE_KEY: &str =
    "49af42ba7f7994852d713ef2784bcbcaa7911de26adc5642cb634540e7ea5005";

/// The client's ephemeral public key.
pub(crate) const CLIENT_PUBLIC_KEY: &str =
    "99381de560e4bd43d23d8e435a7dbafeb3c06e51c13cae4d5413691e529aaf2c";

/// The `ClientHello` of the trace, header included.
pub(crate) const CLIENT_HELLO: &str = "010000c00303cb34ecb1e78163ba1c38c6dacb196a6dffa21a8d9912ec18a2ef\
     6283024dece7000006130113031302010000910000000b000900000673657276\
     6572ff01000100000a00140012001d0017001800190100010101020103010400\
     230000003300260024001d002099381de560e4bd43d23d8e435a7dbafeb3c06e\
     51c13cae4d5413691e529aaf2c002b0003020304000d0020001e040305030603\
     020308040805080604010501060102010402050206020202002d00020101001c\
     00024001";

/// The `ServerHello` of the trace, header included.
pub(crate) const SERVER_HELLO: &str = "020000560303a6af06a4121860dc5e6e60249cd34c95930c8ac5cb1434dac155\
     772ed3e2692800130100002e00330024001d0020c9828876112095fe66762bdb\
     f7c672e156d6cc253b833df1dd69b1b04e751f0f002b00020304";

/// The `EncryptedExtensions` of the trace.
pub(crate) const ENCRYPTED_EXTENSIONS: &str = "080000240022000a00140012001d00170018001901000101010201030104001c\
     0002400100000000";

/// The `Certificate` message of the trace.
pub(crate) const CERTIFICATE: &str = "0b0001b9000001b50001b0308201ac30820115a003020102020102300d06092a\
     864886f70d01010b0500300e310c300a06035504031303727361301e170d3136\
     303733303031323335395a170d3236303733303031323335395a300e310c300a\
     0603550403130372736130819f300d06092a864886f70d010101050003818d00\
     30818902818100b4bb498f8279303d980836399b36c6988c0c68de55e1bdb826\
     d3901a2461eafd2de49a91d015abbc9a95137ace6c1af19eaa6af98c7ced4312\
     0998e187a80ee0ccb0524b1b018c3e0b63264d449a6d38e22a5fda4308467480\
     30530ef0461c8ca9d9efbfae8ea6d1d03e2bd193eff0ab9a8002c47428a6d35a\
     8d88d79f7f1e3f0203010001a31a301830090603551d1304023000300b060355\
     1d0f0404030205a0300d06092a864886f70d01010b05000381810085aad2a0e5\
     b9276b908c65f73a7267170618a54c5f8a7b337d2df7a594365417f2eae8f8a5\
     8c8f8172f9319cf36b7fd6c55b80f21a03015156726096fd335e5e67f2dbf102\
     702e608ccae6bec1fc63a42a99be5c3eb7107c3c54e9b9eb2bd5203b1c3b84e0\
     a8b2f759409ba3eac9d91d402dcc0cc8f8961229ac9187b42b4de10000";

/// The `CertificateVerify` of the trace, which signs with RSA-PSS.
pub(crate) const CERTIFICATE_VERIFY: &str = "0f000084080400805a747c5d88fa9bd2e55ab085a61015b7211f824cd484145a\
     b3ff52f1fda8477b0b7abc90db78e2d33a5c141a078653fa6bef780c5ea248ee\
     aaa785c4f394cab6d30bbe8d4859ee511f602957b15411ac027671459e46445c\
     9ea58c181e818e95b8c3fb0bf3278409d3be152a3da5043e063dda65cdf5aea2\
     0d53dfacd42f74f3";

/// The server's `Finished`.
pub(crate) const SERVER_FINISHED: &str = "140000209b9b141d906337fbd2cbdce71df4deda4ab42c309572cb7fffee5454\
     b78f0718";

/// The client's `Finished`.
pub(crate) const CLIENT_FINISHED: &str = "14000020a8ec436d677634ae525ac1fcebe11a039ec17694fac6e98527b642f2\
     edd5ce61";

/// The ticket the server sends afterwards, which this client ignores.
pub(crate) const NEW_SESSION_TICKET: &str = "040000c90000001efad6aac502000000b22c035d829359ee5ff7af4ec9000000\
     00262a6494dc486d2c8a34cb33fa90bf1b0070ad3c498883c9367c09a2be785a\
     bc55cd226097a3a982117283f82a03a143efd3ff5dd36d64e861be7fd61d2827\
     db279cce145077d454a3664d4e6da4d29ee03725a6a4dafcd0fc67d2aea70529\
     513e3da2677fa5906c5b3f7d8f92f228bda40dda721470f9fbf297b5aea61764\
     6fac5c03272e970727c621a79141ef5f7de6505e5bfbc388e93343694093934a\
     e4d3570008002a000400000400";

/// The `ClientHello` this client writes for the parameters the test fixes, built from RFC 8446 by an implementation outside this repository.
///
/// The six RSA schemes of D-82 were added to it by arithmetic on the
/// value that implementation produced, not by taking what this client now
/// emits: the `signature_algorithms` list grows from three code points to
/// nine, so its list length goes from six to eighteen, the extension from
/// eight to twenty, the extension block from `0x5d` to `0x69`, and the
/// message from `0xac` to `0xb8` — twelve bytes in four places, and the
/// six code points appended in the order the writer emits them.
pub(crate) const OUR_CLIENT_HELLO: &str = "010000b80303000102030405060708090a0b0c0d0e0f10111213141516171819\
     1a1b1c1d1e1f20404142434445464748494a4b4c4d4e4f505152535455565758\
     595a5b5c5d5e5f0006130113021303010000690000000b000900000673657276\
     6572000a00040002001d000d0014001204030503080708040805080604010501\
     0601001000050003026832002b0003020304003300260024001d002099381de5\
     60e4bd43d23d8e435a7dbafeb3c06e51c13cae4d5413691e529aaf2c";

/// The value the two key shares agree on.
pub(crate) const SHARED_SECRET: &str =
    "8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d";

/// The secret the schedule starts from.
pub(crate) const EARLY_SECRET: &str =
    "33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a";

/// The secret of the handshake stage.
pub(crate) const HANDSHAKE_SECRET: &str =
    "1dc826e93606aa6fdc0aadc12f741b01046aa6b99f691ed221a9f0ca043fbeac";

/// The client's handshake traffic secret.
pub(crate) const CLIENT_HANDSHAKE_SECRET: &str =
    "b3eddb126e067f35a780b3abf45e2d8f3b1a950738f52e9600746a0e27a55a21";

/// The server's handshake traffic secret.
pub(crate) const SERVER_HANDSHAKE_SECRET: &str =
    "b67b7d690cc16c4e75e54213cb2d37b4e9c912bcded9105d42befd59d391ad38";

/// The secret of the last stage.
pub(crate) const MASTER_SECRET: &str =
    "18df06843d13a08bf2a449844c5f8a478001bc4d4c627984d5a41da8d0402919";

/// The client's application traffic secret.
pub(crate) const CLIENT_APPLICATION_SECRET: &str =
    "9e40646ce79a7f9dc05af8889bce6552875afa0b06df0087f792ebb7c17504a5";

/// The server's application traffic secret.
pub(crate) const SERVER_APPLICATION_SECRET: &str =
    "a11af9f05531f856ad47116b45a950328204b4f44bfb6b3a4b4f1f3fcb631643";

/// The key the server's handshake records are under.
pub(crate) const SERVER_HANDSHAKE_KEY: &str = "3fce516009c21727d0f2e4e86ee403bc";

/// The nonce base of the same.
pub(crate) const SERVER_HANDSHAKE_IV: &str = "5d313eb2671276ee13000b30";

/// The key the server's `Finished` is authenticated with.
pub(crate) const SERVER_FINISHED_KEY: &str =
    "008d3b66f816ea559f96b537e885c31fc068bf492c652f01f288a1d8cdc19fc8";

/// The key the client's `Finished` is authenticated with.
pub(crate) const CLIENT_FINISHED_KEY: &str =
    "b80ad01015fb2f0bd65ff7d4da5d6bf83f84821d1f87fdc7d3c75b5a7b42d9c4";

/// The key the client's application records are under.
pub(crate) const CLIENT_APPLICATION_KEY: &str = "17422dda596ed5d9acd890e3c63f5051";

/// The nonce base of the same.
pub(crate) const CLIENT_APPLICATION_IV: &str = "5b78923dee08579033e523d9";

/// The record carrying the server's whole encrypted flight.
pub(crate) const SERVER_FLIGHT_RECORD: &str = "17030302a2d1ff334a56f5bff6594a07cc87b580233f500f45e489e7f33af35e\
     df7869fcf40aa40aa2b8ea73f848a7ca07612ef9f945cb960b4068905123ea78\
     b111b429ba9191cd05d2a389280f526134aadc7fc78c4b729df828b5ecf7b13b\
     d9aefb0e57f271585b8ea9bb355c7c79020716cfb9b1183ef3ab20e37d57a6b9\
     d7477609aee6e122a4cf51427325250c7d0e509289444c9b3a648f1d71035d2e\
     d65b0e3cdd0cbae8bf2d0b227812cbb360987255cc744110c453baa4fcd61092\
     8d809810e4b7ed1a8fd991f06aa6248204797e36a6a73b70a2559c09ead68694\
     5ba246ab66e5edd8044b4c6de3fcf2a89441ac66272fd8fb330ef8190579b368\
     4596c960bd596eea520a56a8d650f563aad27409960dca63d3e688611ea5e22f\
     4415cf9538d51a200c27034272968a264ed6540c84838d89f72c24461aad6d26\
     f59ecaba9acbbb317b66d902f4f292a36ac1b639c637ce343117b65962224531\
     7b49eeda0c6258f100d7d961ffb138647e92ea330faeea6dfa31c7a84dc3bd7e\
     1b7a6c7178af36879018e3f252107f243d243dc7339d5684c8b0378bf30244da\
     8c87c843f5e56eb4c5e8280a2b48052cf93b16499a66db7cca71e4599426f7d4\
     61e66f99882bd89fc50800becca62d6c74116dbd2972fda1fa80f85df881edbe\
     5a37668936b335583b599186dc5c6918a396fa48a181d6b6fa4f9d62d513afbb\
     992f2b992f67f8afe67f76913fa388cb5630c8ca01e0c65d11c66a1e2ac4c859\
     77b7c7a6999bbf10dc35ae69f5515614636c0b9b68c19ed2e31c0b3b66763038\
     ebba42f3b38edc0399f3a9f23faa63978c317fc9fa66a73f60f0504de93b5b84\
     5e275592c12335ee340bbc4fddd502784016e4b3be7ef04dda49f4b440a30cb5\
     d2af939828fd4ae3794e44f94df5a631ede42c1719bfdabf0253fe5175be898e\
     750edc53370d2b";

/// The record carrying the client's `Finished`.
pub(crate) const CLIENT_FINISHED_RECORD: &str = "170303003575ec4dc238cce60b298044a71e219c56cc77b0517fe9b93c7a4bfc\
     44d87f38f80338ac98fc46deb384bd1caeacab6867d726c40546";
