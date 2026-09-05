# tls-probe

A real call, over the crates this repository builds.

The trace tests replay RFC 8448 and the machine tests drive a server
written in the same file. Neither can say what a foreign server does with
what this client sends. This program can: it opens a socket, runs the
handshake against a host on the internet, and speaks HTTP/1.1 inside the
result.

Everything above the socket is this repository's code.

| Layer | Crate |
| --- | --- |
| Randomness | `crypto-rng` (`ChaCha20` DRBG) over `/dev/urandom` |
| Key exchange, signatures | `crypto-ec` (X25519, P-256, Ed25519) |
| Key schedule | `crypto-hash` (SHA-256/384, HMAC, HKDF) |
| Record protection | `crypto-aead` (AES-GCM, ChaCha20-Poly1305) |
| Certificates | `audhsos-der`, `audhsos-x509` |
| Validity windows | `audhsos-time` |
| Protocol | `audhsos-tls` |
| PEM anchors | `audhsos-encoding` |

The host contributes three things and no more: a TCP socket, a wall clock,
and a device full of random bytes. `src/wire.rs` is the whole of the I/O —
`read_tls`, `poll`, `write_tls`, and a `TcpStream`.

## Running it

It is a workspace of its own, like `fuzz/`, because it is the only crate in
the tree that links `std`; the checks and the coverage of the kernel
workspace must not see it.

```sh
cd tools/tls-probe
cargo run                                   # google.de, port 443, path /
cargo run -- www.google.de /
cargo run -- example.test:8443 / --anchor my-ca.pem
```

The anchor may be DER or PEM.

## What a run proves

A successful run says: the server agreed on TLS 1.3 with X25519 and one of
the three suites, the key schedule on both sides produced the same keys —
each side's `Finished` is a hash of the transcript under them — the chain
verifies from the leaf to the pinned anchor, the leaf carries the name
that was asked for and is inside its validity window, the server's
`CertificateVerify` verifies under the leaf's key, and the record layer
carried a real protocol both ways.

The chain `google.de` presents is

```
CN=*.google.de            ECDSA P-256 key, signed ecdsa-with-SHA256
  CN=WE2                  ECDSA P-256 key, signed ecdsa-with-SHA384
    CN=GTS Root R4        ECDSA P-384 key
```

and every signature in it is now checked, the P-384 one included. The
anchor is `anchors/gts-root-r4.der`: the self-signed `GTS Root R4`, taken
from this machine's own trust store rather than downloaded, SHA-256
fingerprint

```
34:9D:FA:40:58:C5:E2:63:12:3B:39:8A:E7:95:57:3C:4E:13:13:C8:3F:E6:8F:93:55:6C:D5:E8:03:1B:3C:7D
```

An anchor is a subject and a key, not a certificate: the root's own
signature is never verified, because whatever a root says about itself is
said by the party the system decided to trust. `TrustAnchor::from_certificate`
reads a certificate no further than the key for that reason, so the root
as Google's own chain carries it works here too, although that copy is
cross-signed by GlobalSign with RSA and this system cannot verify RSA:

```sh
cargo run -- google.de / --anchor <the third certificate of the chain>
```

The self-signed copy is what ships, because a trust store is a better
provenance for an anchor than the connection being tested.

What a run still does not prove is that the chain reaches a root **the
world** trusts. One anchor is pinned here; a root program — the set of
anchors a system ships with — is a separate piece of work, and this
repository has none yet.

The checks are real, and the probe is easy to point at the negative cases:

| What is handed to it | What it answers |
| --- | --- |
| the genuine `GTS Root R4` | the handshake completes |
| the leaf, as its own anchor | `the chain reaches nothing this client trusts` |
| `www.ietf.org`, which chains to an ISRG root | `the chain reaches nothing this client trusts` |
| `R4`'s subject over another valid P-384 key | `the signature does not verify` |
| `R4` with one byte flipped in its key | `anchor: the public key has the wrong shape` |

The fourth is what says the P-384 arithmetic is on the path and
discriminating rather than skipped: a well formed P-384 key that did not
sign the intermediate. The fifth is not a point of the curve at all, and
is refused before a socket is opened, which is the one place a caller can
still do something about the file.

Note which hosts do reach this anchor. `cloudflare.com` and
`rfc-editor.org` verify against it too, under `WE1` rather than `WE2`:
Google Trust Services issues for a good deal more than Google. A host that
does not, such as `www.ietf.org`, is the negative case.

`GTS Root R4` runs to 2036, but Google may change what it issues from
before then. When a run starts failing with a chain error, look at what
the server actually sends:

```sh
echo | openssl s_client -connect google.de:443 -servername google.de -showcerts 2>/dev/null \
  | openssl x509 -noout -subject -issuer
```

and replace the anchor with the root that chain now reaches, taken from a
trust store rather than from the connection being tested.
