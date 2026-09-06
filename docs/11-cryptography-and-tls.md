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
- Certificate signature verification with ECDSA over P-256 and over
  P-384, with SHA-256 or SHA-384, and with `ed25519`. A chain that ends at
  a P-384 root is therefore walked to the end; `GTS Root R4` is one such
  root, and `tools/tls-probe` reaches it.
- Handshake signature verification with `ecdsa_secp256r1_sha256`,
  `ecdsa_secp384r1_sha384`, and `ed25519`. These are the schemes the
  `ClientHello` offers, and the two lists are not the same list: a scheme
  names one curve, while `ecdsa-with-SHA384` in a certificate names none,
  so the `CertificateVerify` is held to a stricter rule than the chain
  below it (D-61).
- Server certificate validation against caller-supplied trust anchors,
  RFC 5280 path rules, RFC 6125 name matching.
- ALPN, server name indication, key update, close notify.

Not in scope for the first version, listed so that the boundary is
explicit: TLS 1.2 and earlier; the server role; session resumption,
pre-shared keys, and 0-RTT; client certificates; the `secp256r1` key
exchange; hybrid post-quantum key
exchange; certificate revocation (CRL, OCSP, stapling); name constraints;
renegotiation; compression; `record_size_limit`; DTLS.

RSA verification was on that list and is now section 11.15, planned and
not yet built. It is the one omission that costs interoperability: many
public chains are RSA to the root, and `www.ietf.org`,
`www.rust-lang.org`, and `www.bbc.co.uk` are three that this client
cannot reach for that reason alone.

An earlier version of this section said the arithmetic was nearly there —
that `crypto-ec::montgomery` is generic over the number of limbs, so a
2048-bit modulus is the same code with thirty-two limbs rather than four
or six, and only the exponentiation and the PKCS #1 encoding were
missing. The first half of that was wrong. The module is generic over the
limb count, but its `Params` carries the modulus and its two conversion
constants as associated *constants*: they are known when the code is
compiled, and an RSA modulus is known when a certificate is read. What
carries over is smaller and further down — the three free functions that
already take their modulus as an argument. D-77 records the consequence,
which is that the arithmetic moves into a crate of its own.

Everything else on the list is optional for a working HTTPS request.

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
| `crypto-bignum` | `crates/crypto/bignum` | c0 | - |
| `crypto-ec` | `crates/crypto/ec` | c2 | `crypto-ct`, `crypto-hash`, `crypto-bignum` |
| `crypto-rng` | `crates/crypto/rng` | c2 | `crypto-ct`, `crypto-aead` |
| `crypto-rsa` | `crates/crypto/rsa` | c2 | `crypto-ct`, `crypto-hash`, `crypto-bignum` |
| `audhsos-der` | `crates/net/der` | c0 | - (`audhsos-time` when it exists, D-46 and 11.14) |
| `audhsos-x509` | `crates/net/x509` | c3 | `audhsos-der`, `crypto-hash`, `crypto-ec`, `crypto-rsa` |
| `audhsos-tls` | `crates/net/tls` | c4 | `crypto-ct`, `crypto-hash`, `crypto-aead`, `crypto-ec`, `crypto-rng`, `crypto-rsa`, `audhsos-der`, `audhsos-x509` |

All ten are logic crates: `no_std`, `#![forbid(unsafe_code)]`, no
allocation, `Target::Host` in the policy table, coverage gate on. Each
takes `test-support` as a dev-dependency. Four carry a feature for the
data their own tests and the tests above them need: `test-signing` on
`crypto-ec` and on `crypto-rsa`, `test-certificates` on `audhsos-x509`,
and `test-doubles` on `crypto-rng`. None is enabled by a product build. No crate of this track is depended on by
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

The reader owns the syntax of a time — the form the profile allows, the
digits, the `Z` suffix, and the two-digit year window — and hands the six
fields to `CivilTime::new`, which owns what a field may hold, the true
length of a month included. A `UTCTime` naming the twenty-ninth of
February of a year that is not leap is therefore refused here, and the
value a caller gets converts to a `UnixTime` without a second parser.

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
the RFC 5280 subset. `now` is a `CivilTime`, whose fields order
lexicographically in the order time runs, so the window comparison is the
derived one; a caller that holds a `UnixTime` converts it first. The
rules are: issuer and subject distinguished names compared as
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

`TrustAnchor::from_certificate` is the conversion of one such certificate.
It reads the body no further than the key and never looks at the
certificate's own signature, which is the whole point rather than a
shortcut: a self-signature says nothing about the operator's decision, and
a root reaches a system often enough as a cross-signed certificate whose
signature this system has no arithmetic for. What it does check is that
the key is one this system can verify with, so that an unusable anchor is
refused while the operator still holds the file (D-62).

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
pub struct ClientConfig<'a> { pub server_name: &'a str, pub anchors: TrustAnchors<'a>,
    pub alpn: &'a [&'a [u8]], pub suites: &'a [CipherSuite], pub now: CivilTime }

impl<'a> Connection<'a> {
    pub fn new(config: &'a ClientConfig<'a>, rng: &mut dyn Rng, buffers: Buffers<'a>)
        -> Result<Self, TlsError>;
    pub fn read_tls(&mut self, input: &[u8]) -> Result<usize, TlsError>;
    pub fn write_tls(&mut self, output: &mut [u8]) -> Result<usize, TlsError>;
    pub fn poll(&mut self) -> Result<Event, TlsError>;
    pub fn send(&mut self, plaintext: &[u8]) -> Result<usize, TlsError>;
    pub fn recv(&mut self, out: &mut [u8]) -> Result<usize, TlsError>;
    pub fn close(&mut self) -> Result<(), TlsError>;
}
pub enum Event { WantsRead, WantsWrite, Handshaked, PeerClosed }
```

Four things about that shape are decisions rather than accidents.

There is no list of groups: D-56 leaves one, so there is nothing to
choose. The moment is a field of the configuration rather than a
parameter, because it is checked against the anchors that sit beside it,
and the two belong together. The generator is borrowed for the call and
not stored: it is asked three times at the start, for a private value, a
random, and a session identifier, and never again.

And `Event` carries no data. `poll` drives the handshake; `recv` drives
everything after it. Only decrypting a record says whether it carries
application data or a post-handshake message, and a record decrypts once,
so the two phases cannot share one entry point without a buffer for
plaintext that nobody asked for yet.

Buffer minimums are constants: `MIN_INCOMING = 16_645` (a maximum
ciphertext record plus its header), `MIN_OUTGOING = 16_645`, and
`MIN_HANDSHAKE = 16_384` for reassembling handshake messages that span
records. A message larger than that is `TlsError::BufferTooSmall`, which
is a policy of this client, not a limit of the protocol. `recv` takes a
buffer that can hold a whole record's plaintext, for the same reason.

Modules:

- `record.rs`: header parsing and writing, the legacy version fields,
  the 2^14 plaintext and 2^14+256 ciphertext limits, the inner content
  type and padding removal, per-direction sequence numbers that are spent
  before they are used and whose exhaustion ends the epoch rather than
  wrapping, and the `change_cipher_spec` record that middlebox
  compatibility mode permits: it is dropped while the window is open, and
  refused after the server's `Finished` or when it carries anything but
  the single byte one.
- `keys.rs`: the RFC 8446 §7.1 key schedule. `hkdf_expand_label`,
  `derive_secret`, early, handshake, and master secrets, traffic keys and
  IVs, the per-record nonce as IV xor sequence number, and `key_update`.
- `transcript.rs`: the running handshake hash and the `message_hash`
  substitution after a `HelloRetryRequest`. The suite is not known when
  the first message is sent, so the transcript keeps both hashes and hands
  out the one that is asked for. The second hash costs a few kilobytes of
  hashing per handshake and removes a buffer, a length limit, and the
  failure that comes with them; the substitution is applied to each hash
  with its own length, so both stay usable.
- `protection.rs`: sealing and opening a record under one epoch's keys,
  with the sequence number that must not wrap.
- `codec.rs`: the shapes the wire uses — numbers of one, two and three
  bytes, and vectors with their length in front of them.
- `handshake.rs`: writing `ClientHello`, `Finished` and `KeyUpdate`, and
  reading `ServerHello`, `EncryptedExtensions`, `Certificate`,
  `CertificateVerify`, `Finished` and `KeyUpdate`, with the extensions
  `supported_versions`, `supported_groups`, `key_share`,
  `signature_algorithms`, `server_name`, and
  `application_layer_protocol_negotiation`. A `NewSessionTicket` is
  recognised by its type and skipped by its length: this client resumes
  nothing, and parsing a ticket into fields nobody reads would be surface
  without purpose.
- `alert.rs`: the alert a peer is told about each error, in one function
  that is tested exhaustively, so that the reason a connection failed and
  the reason the peer is given cannot drift apart.
- `secret.rs`, `suite.rs`, `config.rs`: a secret that clears itself, what
  a suite decides, and what a caller decides.
- `client.rs`: the state machine `WaitServerHello`,
  `WaitEncryptedExtensions`, `WaitCertificate`, `WaitCertificateVerify`,
  `WaitFinished`, `Connected`, `Closed`. It checks the downgrade sentinel
  in `ServerHello.random`, refuses a negotiated version other than 1.3,
  verifies `CertificateVerify` over the context string
  `"TLS 1.3, server CertificateVerify"`, and validates the chain before
  sending its own `Finished`. An error is remembered, so every later call
  returns the same one rather than a new one.

  A `HelloRetryRequest` is refused rather than answered. Since D-56 this
  client offers one group, so a retry can only ask for the group it
  already sent or for one it did not offer, and RFC 8446 forbids both. The
  plan said the retry completes; with one group there is nothing for it to
  complete.
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
RFC 8448, which is kept in this repository at `docs/rfc/rfc8448.txt`
(D-59) so that the transcription can be checked against its source
without a network. Every secret the document prints, the keys of both
directions, the flight the server sends as one encrypted record, the four
messages inside it, and both `Finished` codes are reproduced from the
pieces this crate is built of.

The replay does not run the state machine, and the reason is worth
writing down. The `ClientHello` goes into the transcript that every later
secret depends on, so reproducing the trace byte for byte would mean this
client writing another implementation's hello, extension for extension —
carrying `renegotiation_info`, `session_ticket`, `psk_key_exchange_modes`
and `record_size_limit` for no reason but to match a document. That is the
wrong trade. The machine is driven end to end against a server built in
the tests instead, and the two checks cover different halves: the replay
that the arithmetic matches the world, the machine test that the state
machine does what the arithmetic allows.

The trace cannot yet check a signature: it signs with RSA-PSS, which this
client does not verify, so the replay runs with certificate verification
stubbed out and tests the transcript, the key schedule, record protection,
and `Finished`. Certificate validation is tested separately against
project-generated ECDSA and Ed25519 chains.

Step R5 of 11.15 closes that seam, and closes it further than was
expected when this paragraph was first written. RFC 8448 section 2 prints
the whole private key the traces sign with, so the trace is not only a
handshake to reproduce but a signature vector: the `CertificateVerify` of
the simple 1-RTT handshake is a real `rsa_pss_rsae_sha256` signature under
a key this repository holds. It verifies through `crypto-rsa` directly.
It does not verify through the client, and not because anything is
missing: the modulus is 1024 bits, which D-79 puts below what a
certificate may carry. The chain around it stays stubbed, the signature
inside it stops being.

Beyond that: the vector tests of each primitive, property tests for
round trips and for parsers that must not panic, model tests for path
validation, negative tests for every rejection rule in 11.9 and 11.10,
and the four fuzz targets `der`, `x509`, `tls_record`, and
`tls_handshake`. Coverage thresholds apply to all eight crates.

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
| T3 | `crypto-ec`: `fe25519` and X25519, then Ed25519, then P-256, then P-384 | L | implemented |
| T4 | `crypto-rng` | S | implemented |
| T5 | `audhsos-der` | M | implemented but for the time conversion (11.14) |
| T6 | `audhsos-x509` with the test certificate builder | L | implemented |
| T7 | `audhsos-tls` | XL | implemented |
| T8 | Integration, jointly with step D9 of [document 12](12-parallel-work.md): transport over `net-tcp`, the entropy system call, and the HTTP client of `net-http` | M | |
| R1 | `crypto-bignum`: the limb core out of `crypto-ec`, a runtime `Modulus`, and exponentiation (11.15) | M-L | implemented |
| R2 | `crypto-rsa`: the key with its bounds, and PKCS #1 v1.5 | M | |
| R3 | `crypto-rsa`: MGF1 and EMSA-PSS-VERIFY | M | |
| R4 | `audhsos-x509`: identifiers, parameters, the key, the pairs, and the builder | L | |
| R5 | `audhsos-tls`: code points, the hello, `MAX_SPKI`, the `CertificateVerify` rule, the trace | M | |
| R6 | The fuzz target, `tools/tls-probe` against three hosts, and the documents | S-M | |

T1 to T7 touch nothing outside their own crates and the policy table, so
they can be built between kernel phases without disturbing them. T5 and
T6 additionally need steps E1 and E2 of document 12, which are small and
are scheduled before them. T8 depends on the network stack and on the
kernel and is not scheduled.

R1 to R6 have the same property and none of T8's: they are logic in logic
crates, they wait on nothing outside this track, and each one leaves the
workspace green on its own. R1 is the only one that touches a crate that
is finished — it moves the limb arithmetic out of `crypto-ec`, whose P-256
and P-384 suites are what say the move changed nothing. R6 is the only one
that needs the network, and only for `tools/tls-probe`, which is a
separate workspace and no part of the checks.

## 11.13 Risks

| Risk | Effect | Mitigation |
|------|--------|------------|
| Self-written cryptography has flaws that tests do not find | a connection that looks encrypted but is not | vector tests from the standards, the RFC 8448 trace, negative tests for every rejection rule, fuzzing, the constant-time review section per crate, verification-only asymmetric surface |
| Constant-time properties are lost to compiler optimization | timing side channels | no tables, no secret-dependent control flow at the source level; `black_box` where the source must not be folded away; the discipline is documented per function |
| No RSA verification yet | many real chains cannot be validated | scoped as steps R1 to R6 (11.15) against documents this repository now holds; until they are done it is a stated limit, and the interfaces already leave room for the algorithm |
| Wide arithmetic is written for one purpose and used for another | a bignum that is safe for public values used where values are secret | `crypto-bignum` serves verification only: no secret ever reaches it, its module documentation says so in the form `montgomery.rs` already uses, and nothing in the track signs outside `test-signing` |
| No revocation checking | a revoked certificate is accepted | stated as a known limit; short-lived anchors and operator-chosen trust stores are the only mitigation in this version |
| The track grows past its estimate | kernel phases slip | the track is independent; work on it happens between phases, never instead of one |
| Zeroization is best effort without `unsafe` | key material may remain in freed memory | keys live in `Secret<N>` with the shortest possible lifetime; the limit is documented rather than hidden |

## 11.14 What this track is waiting on

Everything in this section is specified elsewhere and not yet built. It is
listed here because a seam that lives only in a commit message is a seam
nobody finds again.

| What is missing | Where it is felt | Who owns it |
|-----------------|------------------|-------------|
| A clock | `audhsos-time` arrived, so `audhsos-der` yields a `CivilTime` that the calendar validated and `verify_chain` compares one as its `now`. What no crate of this project has is a source for that value, because none of them reads a clock | the platform timer of Phase 4 and the system call that carries it out |
| ~~A PEM decoder~~ | `audhsos-encoding` arrived with strict Base64, hex, and PEM. The trust-anchor conversion of D-42 has its decoder; what is still unwritten is the conversion itself, which is xtask work and not this track's | D-47, [document 12](12-parallel-work.md) |
| A source of entropy | `crypto-rng` ships the generator and the `Entropy` trait; no product code can construct a generator without a source | `RDSEED` in the HAL behind a `random_bytes` system call, D-43 |
| A transport | step T8: the client is sans-I/O and needs bytes moved for it | `net-tcp`, D-49, [document 12](12-parallel-work.md) |

None of these blocks the steps that remain. T6 and T7 were built against
the fields of a certificate time and needed no change when the calendar
arrived: the type moved to `audhsos-time` and gained its checks, and the
signatures that carry it stayed as they were.

The fuzz harness of D-54 arrived, and with it the four targets this track
owes: `der`, `x509`, `tls_record`, and `tls_handshake` live under `fuzz/`
with a corpus each, and `cargo xtask fuzz --regression` replays them as a
step of `check`. Fuzzing proper needs a clang that carries the libFuzzer
runtime; on a machine without one the same sources still build the replay
program, which is what the regression step uses.

## 11.15 RSA verification

Planned, not built. This section is the design and the order of work for
the one omission of 11.2 that costs interoperability, in the form the
rest of this document uses. The decisions it rests on are D-77 to D-83.

### 11.15.1 What the reference documents settle

Five documents are kept under `docs/rfc/` for this track, and their
README says what each is for. Three rules out of them shape the code and
are stated here so that no reader has to rediscover them.

RFC 8017 section 8.2.2 gives two shapes for verifying a PKCS #1 v1.5
signature and this project takes the one that constructs (D-80). RFC 4055
section 5 requires the parameters of `sha256WithRSAEncryption` and its two
siblings to be NULL and requires an implementation to accept them absent
as well, which no other algorithm in this crate allows. And RFC 8446
section 4.2.3 gives the `rsa_pkcs1_*` code points one meaning in the
`ClientHello` and forbids them in the `CertificateVerify` (D-82).

The fifth document is what the first of those three costs. RFC 2313 is
PKCS #1 version 1.5 itself, and its verification is not the construction
of RFC 8017 with a looser encoding — it is the other operation, written
out as its own sections: 10.2.3 BER-decodes the recovered data into a
`DigestInfo` and separates it into a digest and an algorithm identifier,
and 10.2.4 compares that digest against a fresh one. D-80 declines that
operation, so a signature whose `DigestInfo` is BER but not DER is refused
here and valid there. That is a deliberate incompatibility with a
published specification, not a tolerance this client happens not to have,
and it is stated as one. RFC 8017's own note calls the case unlikely in
practice, and the certificates on the public web are DER.

### 11.15.2 `crypto-bignum`

The limb arithmetic, moved out of `crypto-ec` and given a modulus it does
not know until it runs (D-77).

```rust
pub struct Modulus { limbs: [u64; MAX_LIMBS], used: usize, n0inv: u64, r2: [u64; MAX_LIMBS] }
pub const MAX_LIMBS: usize = 64;                       // 4096 bits

impl Modulus {
    pub fn new(big_endian: &[u8]) -> Result<Modulus, BignumError>;
    pub fn bits(&self) -> usize;
    pub fn pow(&self, base: &[u8], exponent: u64, out: &mut [u8]) -> Result<(), BignumError>;
    pub fn pow_wide(&self, base: &[u8], exponent: &[u8], out: &mut [u8])
        -> Result<(), BignumError>;
}
```

The exponentiation is written down twice because it is asked for twice.
`pow` is the verification direction, where the exponent is three or
sixty-five thousand five hundred and thirty-seven and a `u64` is more
than enough. `pow_wide` takes the exponent as bytes, and it exists for
the one sentence of 11.15.3 that says signing is one call into the
exponentiation the verification already has: a private exponent is as
wide as the modulus and does not fit in a `u64`. The narrow one forwards
to the wide one, so there is one algorithm and two doors to it. `bits`
reports the bit length of the modulus, which is what an encoding rule
that speaks of `modBits` needs.

`new` derives what `Params` used to carry as constants. `n0inv` is
`-m^-1` modulo `2^64` by Hensel doubling, five steps from the low limb of
an odd modulus, which is where the requirement that the modulus be odd
comes from. `R2` is `2^(128*N)` modulo the modulus by that many modular
doublings: at four thousand ninety-six bits it is eight thousand one
hundred and ninety-two doublings of sixty-four limbs, once per key.

`pow` is left-to-right square-and-multiply, and it is not constant time.
The argument is the one `montgomery.rs` already makes for the curves and
it is stronger here: a modulus, an exponent, and a signature are all on
the wire, and nothing secret ever enters this crate. The module
documentation says so, in the form of the constant-time review sections
of 11.11 — with the opposite conclusion, and the same obligation to state
it.

The invariant that pays for the single width of D-78: limbs at or above
`used` are zero, in every value the crate holds. The loops run over
`used`, so a 2048-bit key costs a 2048-bit multiplication and a
4096-bit stack frame.

Two lints shape the code more than the algorithm does.
`arithmetic_side_effects` forces `wrapping_add` and friends, which is what
the method specifies anyway. `indexing_slicing` forces every access
through `get` and the iterators, which was free while the width was a
const generic and is not free now: `zip` over two arrays becomes `zip`
over two `get(..used)`. That is most of why R1 is not a small step.

### 11.15.3 `crypto-rsa`

```rust
pub struct PublicKey { modulus: Modulus, exponent: u64, size: usize }

impl PublicKey {
    pub fn new(modulus: &[u8], exponent: &[u8]) -> Result<PublicKey, RsaError>;
    pub fn verify_pkcs1(&self, hash: HashId, message: &[u8], signature: &[u8])
        -> Result<(), RsaError>;
    pub fn verify_pss(&self, hash: HashId, message: &[u8], signature: &[u8])
        -> Result<(), RsaError>;
}
```

`new` takes the shape rules of D-79 that are the primitive's own: the
modulus odd with its top bit set and no wider than `MAX_LIMBS`, the
exponent odd and at least three. It does not take the lower bound on the
size. That rule belongs to whoever judges a certificate, and putting it
here would make the RFC 8448 vector unusable by the crate that should
check it.

`verify_pkcs1` builds `EM = 0x00 || 0x01 || PS || 0x00 || T` with `PS` at
least eight bytes of `0xff`, and compares with `ct_eq`. The three
`DigestInfo` prefixes are byte constants, each with the note of RFC 8017
section 9.2 named beside it, in the form `oid.rs` already uses for object
identifiers. The comparison is over public bytes; `ct_eq` is used because
it is what the workspace has for comparing bytes and because a comparison
with no early exit is the right habit in this crate whether or not this
call needs it.

`verify_pss` is EMSA-PSS-VERIFY with `emBits = modBits - 1`: the mask
from MGF1 over the chosen hash, the leftmost bits checked against that
width, the `0x01` separator, the trailer `0xBC`, and `H'` recomputed over
eight zero bytes, the message hash, and the recovered salt. The salt is
as long as the hash output and is not read from the encoding, because the
three schemes this client offers fix it (D-81).

Signing exists behind `test-signing` and is one call into `pow` with a
wide exponent. No key is generated (D-83).

### 11.15.4 What changes above the primitive

`audhsos-x509` takes most of the work. `oid.rs` gains `rsaEncryption`, the
three `sha*WithRSAEncryption` arcs, and for PSS `id-RSASSA-PSS`, `id-mgf1`
and the three hash identifiers. `SignatureAlgorithm::parse` gains a rule
per algorithm where it now has one rule for all of them: today it calls
`finish` on the identifier's fields and so demands the parameters be
absent, which is right for ECDSA and Ed25519 and wrong for the three RSA
identifiers, where NULL and absent are both correct. `SubjectPublicKey`
gains a variant holding the modulus and the exponent as borrowed slices,
parsed from the inner `RSAPublicKey` of RFC 3279 section 2.3.1; the DER
reader needs nothing new for it, since `read_integer` already strips the
leading zero of a positive integer and refuses a negative one.
`check_usable` applies the size bound of D-79, which is what makes an
out-of-range anchor a refusal rather than a path that reaches nothing.

The builder behind `test-certificates` is the part that is easy to
underestimate. It gains RSA `TestKey` variants over the fixed pairs of
D-83, and two constants stop fitting: `MAX_CERTIFICATE` is 1024 where a
4096-bit leaf under a 4096-bit issuer is about 1450 bytes, and
`TestKey::public_key` returns a 97-byte array where an RSA key body is up
to 526.

`audhsos-tls` is a smaller change with one number in it. `MAX_SPKI` is
128 today, computed for P-384 at 120; an RSA-4096 subject public key
information is 550 bytes — fifteen for the algorithm identifier, 526 for
the inner sequence of two integers, five for the bit string around it,
four for the outer sequence — so the constant becomes 550 and
`Connection` grows by that much. The alternative, borrowing the leaf's
`spki_bytes` instead of copying them, would tie a borrow across two
handshake messages to save the bytes, and is not worth it.

Six code points join `signature_algorithms` in the `ClientHello`, and
`take_certificate_verify` gains the pairs it accepts and, more
importantly, the three it refuses (D-82). The 160-byte buffer that holds
the signed content does not change: the transcript hash is the cipher
suite's, which is SHA-256 or SHA-384 and never SHA-512, however the
signature is hashed.

### 11.15.5 Tests

A section of the catalog for `crypto-bignum` and `crypto-rsa`, and
additions to 6.6.33, 6.6.35, 6.6.36, 6.6.37, and 6.6.38.

For the arithmetic: a schoolbook reference in the test module and property
tests against it over moduli of all four widths, in the form
`crates/crypto/ec/src/tests/reference.rs` already uses for the curves.

For PKCS #1: the negative tests are the point. A padding shorter than
eight bytes of `0xff`, a `DigestInfo` moved inside the block, bytes
appended after the digest, a missing `0x00` separator, and a forgery
against a small exponent. These are what the construction of D-80 buys,
and a test suite that only checks that valid signatures verify would not
notice if the code stopped buying it.

For PSS: a wrong trailer, a leftmost bit that should be zero, a salt of
the wrong length, a separator that is not `0x01`, and a hash that does
not match.

For the whole: the `CertificateVerify` of the RFC 8448 simple 1-RTT
handshake, verified through `crypto-rsa` under the key of section 2 of
that document, and a fuzz target `rsa` under `fuzz/` with a corpus,
registered in the policy table beside the other four. Coverage holds at
ninety percent of lines and eighty-five of branches for both new crates.

The end of it is `tools/tls-probe` against `www.ietf.org`,
`www.rust-lang.org`, and `www.bbc.co.uk`. That is the only check that says
the chains this omission cost are reachable, and it is the one the whole
step is for.
