# Reference documents: the Web Consortium

The standards of the Web Consortium this project reads, kept verbatim so
that a clause can be read against its source without a network, and so
that the source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement for `docs/rfc/`, and this directory is the same arrangement
for a standard that is neither an RFC nor an Ecma one.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `png-3.html` | *Portable Network Graphics (PNG) Specification (Third Edition)*, W3C Recommendation of 24 June 2025 | 2026-09-07 from `https://www.w3.org/TR/png-3/` | 656574 | `a3ac82bb9eb8664b93961a2a9ff6def3d70e3a9f63d92721143c1f391d04edec` |

The checksum is here so that a reader can tell the file has not been
edited. It is the page as the server delivered it, byte for byte, 9805
lines, and it was fetched twice and the two fetches agreed. Unlike the
specification under `docs/ecma/`, this one is kept whole: it carries its
own text and one image, the Consortium's logo, which is a file on their
server and not part of the document.

## Why it is here

For the second half of a compressor. `audhsos-deflate` writes what a PDF
calls `FlateDecode`, which is the format of RFC 1951 in the wrapper of
RFC 1950 — and that same pair is what a PNG carries in its image chunks.
What PNG adds is the part this document states and those two do not: the
filters, which replace the bytes of a row of pixels by their differences
from a neighbour — the one to the left, the one above, the average of the
two, or the one the Paeth predictor chooses — so that what reaches the
compressor is far more repetitive than the picture was. A decoder undoes
it row by row.

The same predictors turn up in PDF, where a Flate stream may declare one
in its decode parameters. So the document is here for the format this
project may one day read or write, and for the one it already writes.

## Terms

This document is not covered by this repository's licence. It carries the
notice of the World Wide Web Consortium, © 1996-2025, and is published
under the Consortium's permissive document licence, which allows the
document to be copied and redistributed provided the notice and the link
to the licence travel with it. The file here is unmodified. Software
written from the specification is a separate matter, and the Consortium's
software licence of 2023 is the one the document points at for it.
