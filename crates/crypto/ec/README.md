# crypto-ec

The elliptic curve operations a TLS 1.3 client needs: X25519 for the key
exchange, and Ed25519 and ECDSA over P-256 and P-384 for checking the
signatures on certificates and on the handshake.

The two ECDSA curves share everything but their constants. Both are short
Weierstrass curves whose `a` is minus three, so one set of Jacobian
formulas covers them; neither group order has a shape a Solinas reduction
can use, so one Montgomery multiplication covers all four moduli. What is
written twice is what actually differs: the prime, the order, the curve
constant, and the base point. `montgomery` and `jacobian` hold the
arithmetic, `p256` and `p384` hold the numbers and the ECDSA on top of
them.

P-384 is not decoration. A certificate chain that ends at a P-384 root
cannot be walked to the end without it, and the roots that sign the
public web include such roots — Google Trust Services issues from a P-256
intermediate under `GTS Root R4`, whose key is P-384.

The split that shapes the crate is between secret and public inputs.
X25519 multiplies a secret scalar, so its ladder is constant time: masked
conditional swaps, no branch and no index on the scalar. Signature
verification touches nothing secret — a public key, a message, and a
signature are all on the wire — so it uses ordinary double-and-add, which
is simpler and easier to check.

ECDSA signing is not part of the product surface. A client that presents
no certificate never signs, and leaving it out removes the nonce
generation ECDSA is notorious for. It exists only behind the feature
`test-signing`, deterministic per RFC 6979, so that the certificates in
the test suites are project-generated rather than vendored.

Ed25519 signing is product surface, because a Secure Shell client
authenticates with a key of its own (D-135). Two functions see a secret
and are written for it: `Point::mul_secret` and `Scalar::mul_secret`
double and add at every position and keep the sum behind a mask, where
`Point::mul` and `Scalar::mul` add only where a bit is set and are for the
public scalars a verifier reduces. `subtract_order`, which ends every
scalar addition, takes its difference behind a mask in a fixed two rounds
for the same reason. Nothing enforces the boundary: a caller that hands a
secret to the public multiplication is wrong, and the name is all that
says so.

## Where the numbers come from

The P-256 constants are the ones NIST publishes for that curve. The P-384
constants are RFC 5903, section 3.2, which `docs/rfc/rfc5903.txt` holds
verbatim; RFC 5114, section 2.7, states the same values independently and
agrees with it.

The vectors are of the same kind. P-256 is checked against RFC 6979,
appendix A.2.5; P-384 against appendix A.2.6 of the same document, and
against RFC 5903, appendix 8.2, whose two key pairs and shared point are
three multiplications the test did not compute.
