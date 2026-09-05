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
pub(crate) const OUR_CLIENT_HELLO: &str = "010000ac0303000102030405060708090a0b0c0d0e0f10111213141516171819\
     1a1b1c1d1e1f20404142434445464748494a4b4c4d4e4f505152535455565758\
     595a5b5c5d5e5f00061301130213030100005d0000000b000900000673657276\
     6572000a00040002001d000d0008000604030503080700100005000302683200\
     2b0003020304003300260024001d002099381de560e4bd43d23d8e435a7dbafe\
     b3c06e51c13cae4d5413691e529aaf2c";
