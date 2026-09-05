# Reference documents

The standards this system implements, kept verbatim so that a vector can
be checked against its source without a network, and so that the source
cannot change under a test that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `rfc5480.txt` | RFC 5480, *Elliptic Curve Cryptography Subject Public Key Information*, S. Turner, D. Brown, K. Yiu, R. Housley, T. Polk, March 2009 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5480.txt` | 36209 | `593bf29fd0da2ff8b903c3ebf1c9d189a770039159e2ba46a0c3b91355037f26` |
| `rfc5758.txt` | RFC 5758, *Internet X.509 Public Key Infrastructure: Additional Algorithms and Identifiers for DSA and ECDSA*, Q. Dang, S. Santesson, K. Moriarty, D. Brown, T. Polk, January 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5758.txt` | 15834 | `4d02628ff0875a1960d34be584a68f88528b96242bdc5a05a40a29ef01cf1532` |
| `rfc5903.txt` | RFC 5903, *Elliptic Curve Groups modulo a Prime (ECP Groups) for IKE and IKEv2*, D. Fu, J. Solinas, June 2010 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc5903.txt` | 29175 | `939fab548a6e6bb49a5b3c4dd24a3c5df54a46645447b2d6f4df4fd88ff2d69f` |
| `rfc6979.txt` | RFC 6979, *Deterministic Usage of the Digital Signature Algorithm (DSA) and Elliptic Curve Digital Signature Algorithm (ECDSA)*, T. Pornin, August 2013 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc6979.txt` | 140386 | `456e8f17558fdbd206f968b96fc6f1b4a71ea331ab30ad17f711ab3adaa7d701` |
| `rfc8448.txt` | RFC 8448, *Example Handshake Traces for TLS 1.3*, M. Thomson, January 2019 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8448.txt` | 159343 | `6564d1376d1ec744fc7a9993da15ebc1b9be361908b166091f47ef605c537fba` |

The checksums are here so that a reader can tell a file has not been
edited. Each is the text as the RFC Editor publishes it, byte for byte,
including the page breaks: 1123, 451, 899, 4427, and 3811 lines
respectively. Every one was fetched twice and the two fetches agreed.

## Terms

These documents are not covered by this repository's licence. Each
carries a notice of the form

> Copyright (c) YEAR IETF Trust and the persons identified as the
> document authors. All rights reserved.

with the year 2009 for RFC 5480, 2010 for RFC 5758 and RFC 5903, 2013 for
RFC 6979, and 2019 for RFC 8448. They are subject to BCP 78 and the IETF
Trust's Legal Provisions relating to IETF Documents, which permit
reproduction in full. Code components extracted from an RFC carry the
Simplified BSD Licence; this project extracts test vectors, which it
transcribes into Rust source with the document and section named at each
table, as decision D-40 requires.

## Why RFC 8448 in particular

It is the only published trace of a complete TLS 1.3 handshake: every
secret, every message, and every record of one connection, with the
private keys that produced them. A client that reproduces it byte for
byte has been checked against an implementation that was not this one,
which no amount of testing a client against its own idea of a server can
do.

## The four documents of P-384

These four are what P-384 took, and no more. They were gathered because
`crypto-ec` had P-256 and Ed25519 and no P-384, which is where a real
chain stopped: Google Trust Services issues from a P-256 intermediate
under a P-384 root, so `SubjectPublicKey::parse` reached `GTS Root R4`,
saw a curve that was not `prime256v1`, and answered
`UnsupportedAlgorithm`. Every anchor above such an intermediate was out of
reach. The curve is implemented now, and these are the documents its
constants and its vectors come from.

**RFC 5903, section 3.2** — the numbers. The prime
`p = 2^384 - 2^128 - 2^96 + 2^32 - 1`, the curve `y^2 = x^3 - 3x + b`
with `b` in full, the generator `g = (gx, gy)`, the group order, and the
seed the curve was verifiably generated from. Section 5, *Alignment with
Other Standards*, is the table that says the 384-bit random ECP group,
NIST P-384, and SECG `secp384r1` are one curve under three names. RFC 6090 points here for the parameter set,
which is why this is the copy that is kept.

**RFC 5480** — how the key is written down. Section 2.1.1.1 gives
`secp384r1` the identifier `1.3.132.0.34`; section 2.2 gives the
uncompressed point encoding, `0x04` followed by `x` and `y`, which for
P-384 is 97 bytes where P-256 takes 65; section 4 pairs a key of 384 bits
with SHA-384. The ASN.1 module also carries
`ECDSA-Sig-Value ::= SEQUENCE { r INTEGER, s INTEGER }`, the shape a
signature arrives in. This is the document `SubjectPublicKey::parse`
implements, and the one place it has to change.

**RFC 5758, section 3.2** — `ecdsa-with-SHA384` is
`1.2.840.10045.4.3.3`, and its `AlgorithmIdentifier` omits the parameters
field rather than encoding a null. This half was already done when the
others were not: `oid::ECDSA_WITH_SHA384` and
`SignatureAlgorithm::EcdsaSha384` existed and were exercised. The document
is kept because the identifier and the key it names belong to one change,
and a reader of the other three should not have to go looking for it.

**RFC 6979, appendix A.2.6** — the vectors. A P-384 key pair given as the
private scalar `x` and the public point `Ux, Uy`, the group order `q`,
and ten signatures: `k`, `r`, and `s` for SHA-1, SHA-224, SHA-256,
SHA-384, and SHA-512, over each of the messages `"sample"` and `"test"`.
It is the same table one curve up from the one the P-256 code is already
checked against: `crates/crypto/ec/src/tests/p256.rs` transcribes
appendix A.2.5, and `the_rfc_6979_vectors_sign_and_verify_as_documented`
is the test that reads it.

Note what the appendix does not give: it states the order `q` but not
`p`, `b`, or the generator, so a verifier tested against it is tested
against RFC 5903 as well. The two are one vector set.

## What P-384 does not need, and why it is not here

These were read and left out. They are named so that the next reader does
not repeat the search.

- **RFC 5114, section 2.7** states the same P-384 constants as RFC 5903,
  with `a` written out in hexadecimal instead of as `-3`. Comparing the
  two catches a transcription slip, but it is not a second source: both
  are transcriptions of FIPS 186 and SEC 2. Read it there if the check is
  wanted; it earns no copy.
- **RFC 3279** defines `ECDSA-Sig-Value`, but RFC 5480 updates it and
  carries the same definition in its own module.
- **RFC 6090** gives elliptic curve algorithms without naming P-384, and
  points at RFC 5903 for the parameters.
- **RFC 8422** is ECC for TLS 1.2 and earlier. This client speaks 1.3
  only.
- **RFC 9500** carries an encoded P-384 test key. This repository builds
  the certificates it tests with, in `audhsos-x509`'s builder.
- **RFC 8446** assigns `secp384r1` the group `0x0018` and
  `ecdsa_secp384r1_sha384` the scheme `0x0503`. P-384 here is for reading
  a chain, not for the key exchange, which decision D-56 settles on
  `x25519` alone. Whether the TLS 1.3 specification itself belongs in this
  directory is a separate question from this one.
