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
the data enters rather than argued about later. The round constants and
the initial values are generated from their definition in FIPS 180-4,
the fractional parts of the square and cube roots of the first primes,
rather than copied from a table.

Message lengths are counted in bytes in a 64-bit counter. A message
longer than 2^61 bytes would overflow the bit length the padding
encodes; no caller in this system produces one.
