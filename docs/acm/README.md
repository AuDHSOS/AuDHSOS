# Reference documents: ACM

One paper, because one tool of this repository implements what it
specifies. `norec` is the technique of this document and nothing else, and
a technique cited from memory is a technique nobody can check the
implementation against.

Nothing here is compiled, linked, or read at run time. This is a document.
Rule R8 of the safety policy is about dependencies and is untouched:
`Cargo.lock` still lists only workspace members.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `norec.pdf` | Manuel Rigger and Zhendong Su, *Detecting Optimization Bugs in Database Engines via Non-Optimizing Reference Engine Construction*, ESEC/FSE 2020, 13 pages, `doi:10.1145/3368089.3409710` | 2026-09-13 from `https://www.manuelrigger.at/preprints/NoREC.pdf` | 1194062 | `7d8de699381d0beb36aa0098cb6cd27ef02fd828bed4b5a54faee5fffb37d385` |

It was fetched three times and the three fetches agreed byte for byte.

## Which version this is

The authors' accepted version, served by the first author, and not the
version in the ACM Digital Library. The two are the same paper; the file
here was typeset from the camera-ready source, its reference format still
carries the placeholder DOI the authors had when they built it, and its
section numbering is the one the citations in this repository use.

The published version was not obtainable. On 2026-09-13 the Digital Library
answered an automated request for `https://dl.acm.org/doi/pdf/10.1145/
3368089.3409710` with `403 Forbidden`, which is the same kind of gate
`docs/uefi/README.md` records for the UEFI Forum. A reader with a browser
can fetch it there; a check that runs without one cannot.

## Terms

Copyright in the published version is ACM's. Nothing in this repository's
licence extends to this file, and no licence was granted for the copy: this
is the second case of D-124, a document served freely by its publisher — the
author, here — and not licensed for redistribution. What follows from that
is stated rather than assumed. The file is unmodified, so whatever notice
it carries travels inside it. The copy stays inside this repository and is
not redistributed under this repository's licence. And it goes if an author
or ACM objects, with the citations that point at it.

## What is cited from it

`norec` implements section 3 and is written against these four parts:

- 3.1, the two queries. `SELECT * FROM t0 WHERE φ` against
  `SELECT (φ IS TRUE) FROM t0`, and why the second cannot be optimized the
  way the first can.
- 3.2, the translation. The `FROM` clause, its joins and their `ON`
  conditions are copied into the second query unchanged; only the `WHERE`
  condition moves.
- 3.3, counting. The first query is counted either by its rows or by
  `COUNT(*)`, and the two alternate; the second is always
  `SELECT SUM(count) FROM (...)`, which reads `TRUE` as one and both
  `FALSE` and `NULL` as zero.
- 3.4, the limits. No subquery, because its result may legitimately differ
  between the two forms; no function of the clock or of a random source,
  for the same reason; and no `DISTINCT`, aggregate or window function,
  which compute over several rows and do not survive the translation. The
  paper also states what the technique cannot find: an optimization that
  removes an error, because SQL does not say whether `AND` and `OR`
  short-circuit.

## Who cites it

[`crates/tools/norec`](../../crates/tools/norec/README.md), its module
documentation, and [6.6.74](../06-testing-strategy.md) of the testing
strategy.
