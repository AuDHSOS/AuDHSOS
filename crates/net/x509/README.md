# audhsos-x509

Certificates: what one holds, whether its signature is genuine, whether a
chain of them reaches a trust anchor, and whether the name at its end is
the name that was asked for.

The parser borrows. A certificate is a set of slices into the bytes it
came from, so the body that was signed is the exact body that is hashed,
not a re-encoding of it that might differ in a byte. Nothing is copied and
nothing is allocated.

The profile is narrow on purpose. Signatures are ECDSA over P-256 with
SHA-256 or SHA-384, and Ed25519. Names are matched against `dNSName`
entries of the subject alternative name and nowhere else: the common name
is not a fallback, because a certificate that says one thing in its
extension and another in its subject is a certificate two systems read
differently. Any extension marked critical that this crate does not
understand rejects the certificate, which is what critical means.

What is absent is absent deliberately: no revocation checking, no name
constraints, no policy processing. Document 11, section 11.9, says why,
and section 11.14 lists what the track waited on and what filled each
row.

## Anchors

`anchors` is the table an image carries: the tool that writes the image
calls `write`, the program that reads the file calls `Anchors::parse` and
`Anchors::read_into`, and one certificate becomes one `TrustAnchor`
through `TrustAnchor::from_certificate`. The format is a magic, a
version, a count, and that many length-prefixed certificates (D-148), and
it refuses a record of no bytes and a byte behind the last record, so a
file that was cut short and one that was appended to are seen rather than
read.

## Test certificates

The feature `test-certificates` adds a builder that writes certificates
and signs them with the deterministic signing of `crypto-ec`. Every chain
these tests use is built by it: expired ones, wrong names, broken
signatures, missing constraints. No test certificate is vendored. The
five certificates the repository does carry are the public roots of
`anchors/`, which hold no key material at all beyond the public keys
their authorities publish.
