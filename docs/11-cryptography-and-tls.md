# 11. Cryptography and TLS

This document specifies the cryptographic primitives, the certificate
handling, and the TLS 1.3 client of AuDHSOS, and the order in which they
are built. It is both design and implementation plan, in the form of
document 10, for a track that is independent of the kernel phases.

## 11.1 Purpose

HTTPS needs a TLS client. Rule R8 forbids external code, so every
primitive is project code: hashes, key derivation, authenticated
encryption, elliptic curves, a deterministic random bit generator, a DER
parser, X.509 path validation, and the TLS 1.3 state machine.

Every crate of this track is pure logic. Nothing here performs I/O, opens
a socket, or touches hardware. The whole track is therefore host-testable
today, before a network stack exists; the network stack later supplies
bytes and nothing else.

## 11.2 Scope

In scope for the first version:

- TLS 1.3 client, RFC 8446, as the only protocol version.
- Cipher suites `TLS_AES_128_GCM_SHA256`, `TLS_AES_256_GCM_SHA384`,
  `TLS_CHACHA20_POLY1305_SHA256`.
- Key exchange `x25519` (D-56).
- Certificate signature verification with `ecdsa_secp256r1_sha256`,
  `ecdsa_secp256r1_sha384`, and `ed25519`.
- Server certificate validation against caller-supplied trust anchors,
  RFC 5280 path rules, RFC 6125 name matching.
- ALPN, server name indication, key update, close notify.

Not in scope for the first version, listed so that the boundary is
explicit: TLS 1.2 and earlier; the server role; session resumption,
pre-shared keys, and 0-RTT; client certificates; RSA signature
verification; the `secp256r1` key exchange; hybrid post-quantum key
exchange; certificate revocation (CRL, OCSP, stapling); name constraints;
renegotiation; compression; `record_size_limit`; DTLS.

RSA verification is the one omission that costs interoperability: many
public chains are RSA to the root. It is later work and needs a bignum
crate with Montgomery multiplication. Everything else on the list is
optional for a working HTTPS request.

The `secp256r1` key exchange left the list with D-56, after the curve was
implemented. A key exchange multiplies a secret scalar; the P-256 of
`crypto-ec` verifies signatures, where every value is public and the code
may branch on it, and that is what makes it short enough to compare
against the formulas. Offering the group would mean a second scalar
multiplication written to the constant-time discipline inside a module
that earns its clarity by not needing one. X25519 is mandatory to
implement in RFC 8446, so offering it alone reaches every conforming
server.

## 11.3 Crate catalog

| Crate | Path | Layer | Depends on |
|-------|------|-------|------------|
| `crypto-ct` | `crates/crypto/ct` | c0 | - |
| `crypto-hash` | `crates/crypto/hash` | c1 | `crypto-ct` |
| `crypto-aead` | `crates/crypto/aead` | c1 | `crypto-ct` |
| `crypto-ec` | `crates/crypto/ec` | c2 | `crypto-ct`, `crypto-hash` |
| `crypto-rng` | `crates/crypto/rng` | c2 | `crypto-ct`, `crypto-aead` |
| `audhsos-der` | `crates/net/der` | c0 | `audhsos-time` (D-46) |
| `audhsos-x509` | `crates/net/x509` | c3 | `audhsos-der`, `crypto-hash`, `crypto-ec` |
| `audhsos-tls` | `crates/net/tls` | c4 | `crypto-ct`, `crypto-hash`, `crypto-aead`, `crypto-ec`, `crypto-rng`, `audhsos-der`, `audhsos-x509` |

All eight are logic crates: `no_std`, `#![forbid(unsafe_code)]`, no
allocation, `Target::Host` in the policy table, coverage gate on. Each
takes `test-support` behind the feature `test-strategies` where it owns
types used in property tests. No crate of this track is depended on by
the kernel; the dependency edges run from userland only, and from the
network track of [document 12](12-parallel-work.md), which uses
`crypto-rng` for initial sequence numbers and transaction ids (D-51).

## 11.4 `crypto-ct`: constant-time discipline

The rule for the whole track: no branch and no index on a secret value.
Public values are lengths, protocol constants, certificate contents, and
everything an observer of the wire already knows. Secret values are keys,
traffic secrets, shared secrets, and plaintext.

```rust
pub struct Choice(u8);                       // 0 or 1, no Ord, no Deref
pub fn ct_eq(a: &[u8], b: &[u8]) -> Choice;  // length is public, contents are not
pub fn ct_select_u8(c: Choice, a: u8, b: u8) -> u8;
pub fn ct_select_u32(c: Choice, a: u32, b: u32) -> u32;
pub fn ct_select_u64(c: Choice, a: u64, b: u64) -> u64;
pub fn ct_swap<const N: usize>(c: Choice, a: &mut [u8; N], b: &mut [u8; N]);
pub fn ct_copy<const N: usize>(c: Choice, destination: &mut [u8; N], source: &[u8; N]);
pub fn wipe(bytes: &mut [u8]);
pub struct Secret<const N: usize>([u8; N]);  // Debug prints the length only
```

`Choice` has exactly one way out, `is_true`, and its documentation says
where that exit is allowed: at the point where the program acts on the
comparison, such as accepting or rejecting a record whose tag failed to
verify, which an observer learns anyway. A type with no conversion at all
would only push the same decision somewhere less visible.

`ct_swap` and `ct_copy` take arrays rather than slices, so a length
mismatch is a compile error instead of a silent operation on the common
prefix. `wipe` is the erase of `Secret<N>` for buffers whose length the
type system does not carry, such as the padded key inside HMAC.

`Secret<N>` has no `PartialEq`; comparison is `ct_eq`. Its `Drop`
overwrites the bytes and passes the buffer through
`core::hint::black_box`. Without `unsafe` there is no `write_volatile`,
so this is best effort, not a guarantee, and the module documentation
says so. Keys therefore live as short as possible and never leave the
connection state.

Where a lint conflicts with the discipline, the discipline wins:
`indexing_slicing` forces every access through `get` and `as_chunks`,
which is exactly what is wanted, and `arithmetic_side_effects` forces
`wrapping_add` and friends, which is what the primitives specify anyway.

## 11.5 `crypto-hash`

```rust
pub trait Hash: Clone + Sized {
    const BLOCK_LEN: usize; const OUTPUT_LEN: usize; const ZERO_BLOCK: Self::Block;
    type Output: AsRef<[u8]> + Copy;
    type Block: AsRef<[u8]> + AsMut<[u8]> + Copy;
    fn new() -> Self; fn update(&mut self, bytes: &[u8]); fn finish(self) -> Self::Output;
    fn digest(bytes: &[u8]) -> Self::Output;                     // provided
}
```

Two members of the trait exist for HMAC. A padded key is exactly one
block, and no generic function can name `[u8; H::BLOCK_LEN]`, so the
block is an associated type and the trait carries a zero block to start
from. `Output` cannot require `Default`, because arrays longer than
thirty-two bytes do not implement it and SHA-384 and SHA-512 are longer.

- `sha256.rs`: FIPS 180-4 SHA-256 over 64-byte blocks, 64-bit byte
  counter, one-shot `digest`.
- `sha512.rs`: the SHA-512 compression function; `Sha384` is the same
  core with the SHA-384 initial values and a 48-byte output.
- `hmac.rs`: `Hmac<H: Hash>` per RFC 2104, key shortening and padding.
- `hkdf.rs`: `extract(salt, ikm) -> Prk<H>`, `expand(prk, info, out)`
  per RFC 5869; `out.len() > 255 * OUTPUT_LEN` is `Error::OutputTooLong`.

Tests: catalog 6.6.31.

## 11.6 `crypto-aead`

```rust
pub const TAG_LEN: usize = 16;
pub type Tag = [u8; TAG_LEN];

pub trait Aead: Sized { const KEY_LEN: usize; const NONCE_LEN: usize;
    fn new(key: &[u8]) -> Result<Self, AeadError>;
    fn seal(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8]) -> Result<Tag, AeadError>;
    fn open(&self, nonce: &[u8], aad: &[u8], in_out: &mut [u8], tag: &Tag)
        -> Result<(), AeadError>; }
```

The tag length is a constant of the crate rather than of the trait: all
three suites of TLS 1.3 authenticate with sixteen bytes, and a `Tag` that
is one type makes the record layer simpler than a `Tag` that is one type
per suite. `open` returns nothing on success, because the plaintext is as
long as the buffer it decrypted in place.

`open` computes the tag over the received ciphertext and compares it with
`ct_eq` before it decrypts anything, so unauthenticated plaintext never
exists; when the comparison fails it clears the buffer and returns
`AeadError::BadTag`, so a caller that ignores the result finds nothing
usable there.

- `chacha20.rs`: RFC 8439 block function and keystream; a 32-bit counter
  overflow is an error, not a wrap.
- `poly1305.rs`: the 130-bit accumulator in three limbs of 44, 44, and 42
  bits over `u64`, so that a limb product fits in 128 bits; the final
  reduction picks between the accumulator and the accumulator minus the
  modulus with a mask, not a branch. The key is clamped on its bytes,
  where RFC 8439 writes the rule down, rather than through masks folded
  into the limb split where nobody can check it.
- `chachapoly.rs`: the RFC 8439 AEAD construction.
- `aes.rs`: AES-128 and AES-256, bitsliced over `u64` words as eight bit
  planes of four blocks. The substitution box is the multiplicative
  inverse in GF(2^8), computed as the 254th power with three
  multiplications and seven squarings, followed by the affine
  transformation of FIPS 197; a published boolean circuit would be
  faster, but this form follows from the definition and can be checked
  against it. The key schedule runs on bytes and borrows the same sliced
  substitution for its four-byte words. Only encryption exists: GCM never
  decrypts a block, so there is no inverse substitution box and no
  inverse mixing step. There is no lookup table anywhere in the file, so
  no memory access depends on a key or on plaintext.
- `ghash.rs`: multiplication in GF(2^128) as 128 masked shift-and-xor
  steps, no tables.
- `aesgcm.rs`: counter mode plus GHASH, `Aes128Gcm` and `Aes256Gcm`. The
  nonce is twelve bytes and nothing else: any other length would need
  GHASH to derive the first counter block, TLS never uses one, and a path
  nothing exercises is a path nobody checks.

ChaCha20-Poly1305 is implemented first and is the suite the tests use
throughout; AES-GCM follows. Tests: catalog 6.6.32.

## 11.7 `crypto-ec`

- `fe25519.rs`: field arithmetic modulo 2^255 - 19 in five 51-bit limbs
  over `u64` with `u128` intermediates; add, sub, mul, square, invert by
  exponentiation, `pow22523`, canonical encoding and decoding. Constant
  time throughout.
- `x25519.rs`: the Montgomery ladder with conditional swaps per RFC 7748,
  scalar clamping, and the all-zero output check that RFC 8446 requires
  the peer to abort on.
- `ed25519.rs`: Edwards points in extended coordinates, `verify(public,
  message, signature)` per RFC 8032 with the canonicality checks: `S < L`,
  canonical point encodings, no small-order public keys. Verification
  handles public data only.
- `p256/`: the field modulo p = 2^256 - 2^224 + 2^192 + 2^96 - 1, the
  scalar field modulo n, Jacobian point arithmetic, and
  `PublicKey::verify(digest, r, s)` with the range checks on `r` and `s`
  and an on-curve check of the public point. Both moduli are served by one
  Montgomery multiplication parameterized by the modulus rather than by a
  Solinas reduction for the field and a second implementation for the
  order: the order has no shape a Solinas step can use, and one
  multiplication that both use is one to check.

  The key arrives as the uncompressed point encoding of SEC 1 and nothing
  else; the signature arrives as two integers, because the DER that
  carries them belongs to the certificate parser. Only the leftmost bytes
  of a digest are used, as many as the order is wide, which is what makes
  SHA-384 usable with this curve. A point carries no equality: Jacobian
  coordinates are not unique, so a comparison would answer a question the
  caller did not ask.

Signature *creation* is not part of the product surface. It exists only
behind the feature `test-signing`: deterministic Ed25519 signing and
deterministic ECDSA per RFC 6979, used by the test certificate builder in
11.9 so that all test data is project-generated and reproducible. The
feature is off in every non-test build and is rejected by the layering
check outside test and xtask contexts.

Tests: catalog 6.6.33.

## 11.8 `crypto-rng`

```rust
pub trait Entropy { fn fill(&mut self, out: &mut [u8]) -> Result<(), EntropyError>; }
pub trait Rng { fn fill(&mut self, out: &mut [u8]) -> Result<(), RngError>; }
pub struct ChaChaRng { /* 32-byte key, 64-bit counter, byte budget */ }
```

`ChaChaRng` is a `ChaCha20`-based deterministic random bit generator. The
nonce is the sequence number of the request, the output is the stream from
block one onwards, and block zero of the same stream becomes the next key,
so a state captured after a request says nothing about the bytes that
request produced. A reseed mixes the fresh material into the key with an
exclusive-or rather than replacing it, so a source that turns out to be
predictable cannot take the state over, and it happens after a fixed byte
budget or on demand. A request that would cross the budget when the source
fails produces nothing at all.

Behind `test-doubles` — the name the workspace already uses for doubles —
the crate ships `ScriptedRng`, which replays a byte string and reports
exhaustion instead of repeating, and the two entropy sources the tests of
the reseed path need. The RFC 8448 transcript test of the protocol depends
on the scripted generator: it is what makes the client's private key and
its random the ones the document wrote down.

The platform entropy source does not exist yet. It arrives with the
network stack as `RDSEED` in `kernel-hal-x86_64` behind a `random_bytes`
system call, one further `asm!` site against the budget. Until then no
product code constructs a `ChaChaRng`; the crate ships the algorithm and
the traits.

Tests: catalog 6.6.34.

## 11.9 `audhsos-der` and `audhsos-x509`

`audhsos-der` is a strict, zero-copy DER reader: definite lengths only,
minimal length encoding, minimal integer encoding, no indefinite forms,
nesting depth bounded by `MAX_DEPTH = 16`, trailing bytes rejected by
`finish`. Object identifiers are compared as byte constants, never parsed
into arcs. `UTCTime` and `GeneralizedTime` become the `UnixTime` of
`audhsos-time` ([document 12](12-parallel-work.md), D-46) with full
range validation, including the leap-year rules and the two-digit year
window of RFC 5280; the calendar arithmetic itself lives in
`audhsos-time`, so certificate validity and network timers share one
type.

`audhsos-x509` parses a certificate into borrowed slices:

```rust
pub struct Certificate<'a> { pub tbs: &'a [u8], pub serial: &'a [u8],
    pub issuer: &'a [u8], pub subject: &'a [u8],
    pub not_before: UnixTime, pub not_after: UnixTime,
    pub spki: SubjectPublicKey<'a>, pub signature: Signature<'a>, /* extensions */ }
pub enum SubjectPublicKey<'a> { EcdsaP256(&'a [u8]), Ed25519(&'a [u8]) }
```

Extensions read: basic constraints, key usage, extended key usage,
subject alternative name, authority and subject key identifier. Any
unrecognized extension marked critical rejects the certificate.

`verify_chain(end_entity, intermediates, anchors, name, now)` implements
the RFC 5280 subset: issuer and subject distinguished names compared as
DER bytes, one signature verification per link, the validity window
against `now`, `cA` and the path length constraint on every intermediate,
`keyCertSign` on every CA, `serverAuth` on the leaf, chain length at most
eight, and RFC 6125 name matching against `dNSName` entries only, with a
wildcard allowed in the leftmost label and nowhere else. There is no
common-name fallback and no revocation check; the absence of revocation
is a documented gap, not an oversight.

Trust anchors are `TrustAnchor { subject: &[u8], spki: &[u8] }` supplied
by the caller. The repository contains none. Which roots a system trusts
is an operator decision; the xtask converts a PEM bundle the operator
supplies into a DER table that goes into the boot image, using the PEM
decoder of `audhsos-encoding` ([document 12](12-parallel-work.md),
D-47).

Certificates for tests are built by `x509::builder` behind the feature
`test-certificates`, which writes DER and signs with the `test-signing`
primitives of 11.7. Chains, expired certificates, wrong names, broken
signatures, and bad constraints are constructed, not vendored.

Tests: catalog 6.6.35 and 6.6.36. Fuzz targets `der` and `x509`.

## 11.10 `audhsos-tls`

Sans-I/O: the connection consumes transport bytes and produces transport
bytes. It never allocates, never blocks, and knows nothing about sockets.

```rust
pub struct Buffers<'a> { pub incoming: &'a mut [u8], pub outgoing: &'a mut [u8],
                         pub handshake: &'a mut [u8] }
pub struct ClientConfig<'a> { pub anchors: TrustAnchors<'a>, pub server_name: DnsName<'a>,
    pub alpn: &'a [&'a [u8]], pub suites: &'a [CipherSuite], pub groups: &'a [NamedGroup] }

impl<'a> Connection<'a> {
    pub fn new(config: &'a ClientConfig<'a>, rng: &'a mut dyn Rng,
               now: UnixTime, buffers: Buffers<'a>) -> Result<Self, Error>;
    pub fn read_tls(&mut self, input: &[u8]) -> Result<usize, Error>;
    pub fn write_tls(&mut self, output: &mut [u8]) -> Result<usize, Error>;
    pub fn poll(&mut self) -> Result<Event, Error>;
    pub fn send(&mut self, plaintext: &[u8]) -> Result<usize, Error>;
    pub fn recv(&mut self, out: &mut [u8]) -> Result<usize, Error>;
    pub fn close(&mut self) -> Result<(), Error>;
}
pub enum Event { WantsRead, WantsWrite, Handshaked, Data(usize), PeerClosed }
```

Buffer minimums are constants: `MIN_INCOMING = 16_645` (a maximum
ciphertext record plus its header), `MIN_OUTGOING = 16_645`,
`MIN_HANDSHAKE = 16_384` for reassembling handshake messages that span
records. A larger message is `Error::HandshakeTooLarge`, which is a
policy, not a protocol limit, and is documented as such.

Modules:

- `record.rs`: header parsing and writing, the legacy version fields,
  the 2^14 plaintext and 2^14+256 ciphertext limits, the inner content
  type and padding removal, per-direction sequence numbers whose
  exhaustion is an error, and the `change_cipher_spec` records that
  middlebox compatibility mode permits and that are dropped.
- `keys.rs`: the RFC 8446 §7.1 key schedule. `hkdf_expand_label`,
  `derive_secret`, early, handshake, and master secrets, traffic keys and
  IVs, the per-record nonce as IV xor sequence number, and `key_update`.
- `transcript.rs`: the running handshake hash, the buffering of
  `ClientHello` until the suite fixes the hash, and the `message_hash`
  substitution after a `HelloRetryRequest`.
- `messages/`: encoding and decoding of `ClientHello`, `ServerHello`,
  `EncryptedExtensions`, `Certificate`, `CertificateVerify`, `Finished`,
  `NewSessionTicket` (parsed, then ignored), `KeyUpdate`, and the
  extensions `supported_versions`, `supported_groups`, `key_share`,
  `signature_algorithms`, `server_name`, `application_layer_protocol_negotiation`.
- `client.rs`: the state machine `Start`, `WaitServerHello`,
  `WaitEncryptedExtensions`, `WaitCertificate`, `WaitCertificateVerify`,
  `WaitFinished`, `Connected`, `Closed`, `Poisoned`. It checks the
  downgrade sentinel in `ServerHello.random`, refuses a negotiated
  version other than 1.3, verifies `CertificateVerify` over the context
  string `"TLS 1.3, server CertificateVerify"`, and validates the chain
  before sending its own `Finished`.
- `alert.rs`: every error maps to exactly one alert description; the
  mapping table is tested exhaustively. A fatal alert poisons the
  connection, which then refuses every further call.

Tests: catalog 6.6.37 and 6.6.38. Fuzz targets `tls_record` and
`tls_handshake`.

## 11.11 Testing

Test vectors are transcribed from RFCs and NIST publications into Rust
source files. They are specification data, not third-party code; each
table names its source document and section in a comment. Certificates,
keys, and chains are generated by project code (11.7, 11.9).

The central integration test is the "Simple 1-RTT Handshake" trace of
RFC 8448: with `ScriptedRng` supplying the client's private key and
random, every derived secret, every handshake message, and every record
on the wire must match the document byte for byte. That trace uses an
RSA certificate, so it runs with certificate verification stubbed out; it
tests the transcript, the key schedule, record protection, and
`Finished`. Certificate validation is tested separately against
project-generated ECDSA and Ed25519 chains. Adding RSA verification later
would close this seam.

Beyond that: the vector tests of each primitive, property tests for
round trips and for parsers that must not panic, model tests for path
validation, negative tests for every rejection rule in 11.9 and 11.10,
and four fuzz targets. Coverage thresholds apply to all eight crates.

Every crate whose code touches secrets carries a constant-time review
section in its crate documentation: which functions see secret input, and
the argument why each branch and each index in them is on public data.
The section is part of the review checklist, in the form of the unsafe
checklist in 4.9.

## 11.12 Order of work

| Step | Content | Size | Status |
|------|---------|------|--------|
| T1 | `crypto-ct`, `crypto-hash` | S | implemented |
| T2 | `crypto-aead`: ChaCha20-Poly1305, then bitsliced AES-GCM | L | implemented |
| T3 | `crypto-ec`: `fe25519` and X25519, then Ed25519, then P-256 | L | implemented |
| T4 | `crypto-rng` | S | implemented |
| T5 | `audhsos-der` | M | |
| T6 | `audhsos-x509` with the test certificate builder | L | |
| T7 | `audhsos-tls` | XL | |
| T8 | Integration, jointly with step D9 of [document 12](12-parallel-work.md): transport over `net-tcp`, the entropy system call, and the HTTP client of `net-http` | M | |

T1 to T7 touch nothing outside their own crates and the policy table, so
they can be built between kernel phases without disturbing them. T5 and
T6 additionally need steps E1 and E2 of document 12, which are small and
are scheduled before them. T8 depends on the network stack and on the
kernel and is not scheduled.

## 11.13 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| Self-written cryptography has flaws that tests do not find | a connection that looks encrypted but is not | vector tests from the standards, the RFC 8448 trace, negative tests for every rejection rule, fuzzing, the constant-time review section per crate, verification-only asymmetric surface |
| Constant-time properties are lost to compiler optimization | timing side channels | no tables, no secret-dependent control flow at the source level; `black_box` where the source must not be folded away; the discipline is documented per function |
| No RSA verification | many real chains cannot be validated | stated as a known limit; the bignum crate is scoped as later work and the interfaces leave room for a third signature algorithm |
| No revocation checking | a revoked certificate is accepted | stated as a known limit; short-lived anchors and operator-chosen trust stores are the only mitigation in this version |
| The track grows past its estimate | kernel phases slip | the track is independent; work on it happens between phases, never instead of one |
| Zeroization is best effort without `unsafe` | key material may remain in freed memory | keys live in `Secret<N>` with the shortest possible lifetime; the limit is documented rather than hidden |
