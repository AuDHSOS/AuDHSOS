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
| Key exchange, curve signatures | `crypto-ec` (X25519, P-256, P-384, Ed25519) |
| RSA signatures | `crypto-rsa` over `crypto-bignum` (PKCS #1 v1.5, PSS) |
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

## The four anchors that ship, and what each reaches

Each was taken from this machine's own trust store, not from the
connection being tested:

```sh
security find-certificate -a -c "ISRG Root X1" -p \
  /System/Library/Keychains/SystemRootCertificates.keychain
```

| Anchor | Key | Reached by |
| --- | --- | --- |
| `gts-root-r1.der` | RSA 4096 | `google.de` — the default |
| `isrg-root-x1.der` | RSA 4096 | `www.rust-lang.org`, `www.ietf.org` |
| `globalsign-root-r46.der` | RSA 4096 | `www.bbc.co.uk` |
| `isrg-root-x2.der` | ECDSA P-384 | `www.ietf.org`, by the all-curve chain |
| `gts-root-r4.der` | ECDSA P-384 | nothing, today; kept for the record |

Their SHA-256 fingerprints:

```text
GTS Root R1          D9:47:43:2A:BD:E7:B7:FA:90:FC:2E:6B:59:10:1B:12:
                     80:E0:E1:C7:E4:E4:0F:A3:C6:88:7F:FF:57:A7:F4:CF
ISRG Root X1         96:BC:EC:06:26:49:76:F3:74:60:77:9A:CF:28:C5:A7:
                     CF:E8:A3:C0:AA:E1:1A:8F:FC:EE:05:C0:BD:DF:08:C6
ISRG Root X2         69:72:9B:8E:15:A8:6E:FC:17:7A:57:AF:B7:17:1D:FC:
                     64:AD:D2:8C:2F:CA:8C:F1:50:7E:34:45:3C:CB:14:70
GlobalSign Root R46  4F:A3:12:6D:8D:3A:11:D1:C4:85:5A:4F:80:7C:BA:D6:
                     CF:91:9D:3A:5A:88:B0:3B:EA:2C:63:72:D9:3C:40:C9
GTS Root R4          34:9D:FA:40:58:C5:E2:63:12:3B:39:8A:E7:95:57:3C:
                     4E:13:13:C8:3F:E6:8F:93:55:6C:D5:E8:03:1B:3C:7D
```

## The three hosts RSA was for

Section 11.15 of document 11 names three chains this client could not
walk, and they are the end of steps R1 to R6:

```sh
cargo run -- www.ietf.org      / --anchor anchors/isrg-root-x1.der
cargo run -- www.rust-lang.org / --anchor anchors/isrg-root-x1.der
cargo run -- www.bbc.co.uk     / --anchor anchors/globalsign-root-r46.der
```

All three complete. What each one exercises is not the same thing:

- `www.rust-lang.org` is RSA the whole way — a 2048-bit leaf under a
  2048-bit intermediate under the 4096-bit `ISRG Root X1`, every link
  `sha256WithRSAEncryption`. Nothing in that chain was reachable before.
- `www.bbc.co.uk` is the same shape under `GlobalSign Root R46`, with a
  `sha384WithRSAEncryption` link in it, so the second of the three
  `DigestInfo` prefixes is on a real path.
- `www.ietf.org` is the mixed case, and the more interesting one. The
  chain it serves is ECDSA to the third certificate: leaf, `YE2`,
  `Root YE`, and then `ISRG Root X2` cross-signed by `ISRG Root X1` with
  RSA. Pinned to `isrg-root-x1.der` the walk ends on that RSA signature;
  pinned to `isrg-root-x2.der` it stops one certificate earlier and never
  needs RSA at all. The same host, two anchors, two algorithms — which is
  what a cross-signed root is for.

The last of those is also where P-384 keeps a live check now that
`google.de` no longer offers one: `ISRG Root X2` is a P-384 root, and
`Root YE` under it is a P-384 intermediate.

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
  CN=WR2                  RSA 2048 key,    signed sha256WithRSAEncryption
    CN=GTS Root R1        RSA 4096 key
```

and every signature in it is checked. It was not this chain when this file
was first written: Google issued `*.google.de` from `WE2` under the P-384
`GTS Root R4`, and the default anchor was that root. The rotation is the
plainest argument for RSA there is — a client that verifies only curves
lost the default host of its own probe without a line of its code
changing.

An anchor is a subject and a key, not a certificate: the root's own
signature is never verified, because whatever a root says about itself is
said by the party the system decided to trust. `TrustAnchor::from_certificate`
reads a certificate no further than the key for that reason, so a
cross-signed copy of a root works here as well as the self-signed one —
`ISRG Root X2` as `www.ietf.org`'s own chain carries it, signed by
`ISRG Root X1`, is the same subject and the same key:

```sh
cargo run -- www.ietf.org / --anchor <the fourth certificate of the chain>
```

The self-signed copies are what ship, because a trust store is a better
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
| `www.bbc.co.uk` under `isrg-root-x1.der` | `the chain reaches nothing this client trusts` |
| the root's subject over another valid key | `the signature does not verify` |
| a root with one byte flipped in its key | `anchor: the public key has the wrong shape` |

The fourth is what says the arithmetic is on the path and discriminating
rather than skipped: a well formed key that did not sign the
intermediate. The fifth is not a usable key at all, and is refused before
a socket is opened, which is the one place a caller can still do something
about the file.

A root outlives the chain that reaches it. `GTS Root R4` runs to 2036 and
nothing this file names reaches it any more. When a run starts failing
with a chain error, look at what the server actually sends:

```sh
echo | openssl s_client -connect google.de:443 -servername google.de -showcerts 2>/dev/null \
  | openssl x509 -noout -subject -issuer
```

and replace the anchor with the root that chain now reaches, taken from a
trust store rather than from the connection being tested.
