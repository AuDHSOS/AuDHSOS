# crypto-ec

The elliptic curve operations a TLS 1.3 client needs: X25519 for the key
exchange, and Ed25519 and ECDSA over P-256 for checking the signatures on
certificates and on the handshake.

The split that shapes the crate is between secret and public inputs.
X25519 multiplies a secret scalar, so its ladder is constant time: masked
conditional swaps, no branch and no index on the scalar. Signature
verification touches nothing secret — a public key, a message, and a
signature are all on the wire — so it uses ordinary double-and-add, which
is simpler and easier to check.

Signing is not part of the product surface. A client that presents no
certificate never signs, and leaving signing out removes the nonce
generation that ECDSA is notorious for. It exists only behind the feature
`test-signing`, deterministic in both algorithms, so that the certificates
in the test suites are project-generated rather than vendored.
