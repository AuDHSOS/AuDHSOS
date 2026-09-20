# crypto-hash

The hash functions the TLS 1.3 client needs and the two constructions
built on them: SHA-256, SHA-384, and SHA-512 from FIPS 180-4, HMAC from
RFC 2104, and HKDF from RFC 5869.

Everything is written against the `Hash` trait, so HMAC and HKDF exist
once rather than once per digest, and the key schedule of the protocol
can be generic over the hash its cipher suite selects.

None of the code here branches or indexes on message content. A hash has
no secret input of its own, but HMAC does, and the message schedule and
the compression rounds are shared, so the property is established where
the data enters rather than argued about later.

Key material is overwritten where it is held: `Sha256` and the SHA-512
core zero their chaining words and partial block on drop, `Hmac` zeros
the padded key and the digest of an over-long key, and `Prk` zeros its
bytes on drop. `Prk` is not `Copy`, so a hand-over to the next key
schedule step is a move or an explicit `clone` rather than a silent
second copy.

The round constants and the initial values are literal tables, the
digits FIPS 180-4, sections 4.2.2, 4.2.3 and 5.3.3 to 5.3.5, print.
`tests::constants` derives all 168 of them from their definition — the
fractional parts of the square and cube roots of the first primes — in
256-bit integer arithmetic and compares, so a mistyped digit fails a
test and not a handshake.

Message lengths are counted in bytes in a 64-bit counter, which
saturates. FIPS 180-4, section 1, defines SHA-256 for messages below
2^61 bytes, and `Sha256::update` clamps its counter there; it defines
SHA-512 far past what a 64-bit byte counter reaches, so there the
counter is its own bound. A message at or past either bound pads with a
length no shorter message pads with, rather than with a wrapped one, and
what comes out is no SHA-256 or SHA-512 digest. No caller in this system
produces one.
