# Reference documents

The standards this system implements, kept verbatim so that a vector can
be checked against its source without a network, and so that the source
cannot change under a test that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-57 records the
arrangement.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `rfc8448.txt` | RFC 8448, *Example Handshake Traces for TLS 1.3*, M. Thomson, January 2019 | 2026-09-05 from `https://www.rfc-editor.org/rfc/rfc8448.txt` | 159343 | `6564d1376d1ec744fc7a9993da15ebc1b9be361908b166091f47ef605c537fba` |

The checksum is here so that a reader can tell the file has not been
edited. It is 3811 lines of the text as the RFC Editor publishes it,
byte for byte, including the page breaks.

## Terms

These documents are not covered by this repository's licence. RFC 8448
carries:

> Copyright (c) 2019 IETF Trust and the persons identified as the
> document authors. All rights reserved.

It is subject to BCP 78 and the IETF Trust's Legal Provisions relating to
IETF Documents, which permit reproduction in full. Code components
extracted from an RFC carry the Simplified BSD Licence; this project
extracts test vectors, which it transcribes into Rust source with the
document and section named at each table, as decision D-40 requires.

## Why RFC 8448 in particular

It is the only published trace of a complete TLS 1.3 handshake: every
secret, every message, and every record of one connection, with the
private keys that produced them. A client that reproduces it byte for
byte has been checked against an implementation that was not this one,
which no amount of testing a client against its own idea of a server can
do.
