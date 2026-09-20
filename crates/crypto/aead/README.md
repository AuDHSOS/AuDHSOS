# crypto-aead

Authenticated encryption with associated data, in the three forms TLS 1.3
selects: `ChaCha20-Poly1305` from RFC 8439, and AES-128-GCM and AES-256-GCM
from NIST SP 800-38D.

Every primitive here sees a key, so the rule of the track applies without
exception: no branch and no index on a secret value, and no lookup table
in any function that touches key material. `ChaCha20` and `Poly1305` are
built that way by their designers. AES is not, which is why the block
cipher in this crate is bitsliced and computes its substitution box as
arithmetic in GF(2^8) rather than reading it from a table, and why the
multiplication GCM needs is a masked shift-and-exclusive-or rather than
the usual precomputed products.

Every primitive that holds key material overwrites it when it goes out of
scope: `ChaCha20` its key words, `Aes` its round keys, `GHash` its hash
key and accumulator, `Poly1305` its multiplier and addend. The key
schedule `Aes::new_128` and `Aes::new_256` expand is overwritten before
the constructor returns.

`open` verifies before it decrypts, so unauthenticated plaintext never
exists, and it clears the buffer when verification fails, so a caller
that ignores the result finds nothing usable there.
