# Reference documents: the Web Consortium

The standards of the Web Consortium this project reads, kept verbatim so
that a clause can be read against its source without a network, and so
that the source cannot change under anything that cites it. Three
documents: the image format, and the two the font work is written
against.

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
| `css-fonts-4.html` | *CSS Fonts Module Level 4*, W3C Working Draft of 13 September 2026 | 2026-09-16 from `https://www.w3.org/TR/2026/WD-css-fonts-4-20260913/` | 1388137 | `4d72b9122458281dc34a830bc952c4495ec09e8c0466ea9722834f32568b93e8` |
| `woff2.html` | *WOFF File Format 2.0*, W3C Recommendation of 8 August 2024 | 2026-09-16 from `https://www.w3.org/TR/2024/REC-WOFF2-20240808/` | 125108 | `65dfdde1efb57bf123526b9c15a4025ecc78158426a28fefdb396ecf4fc26dc8` |

The checksums are here so that a reader can tell a file has not been
edited. Each is the page as the server delivered it, byte for byte, of
9805, 13934 and 1869 lines; each was fetched twice and the two fetches
agreed. Unlike the specification under `docs/ecma/`, these are kept
whole: nothing is cut from any of them.

The two font documents were taken from a dated address rather than from
`/TR/css-fonts-4/` and `/TR/WOFF2/`, because a dated address is fixed and
the undated one moves to the next draft; both served the same bytes on
the day of the fetch. PNG predates that rule here and keeps the undated
address it was taken from, which for a Recommendation of a finished
edition names the same document.

CSS Fonts Module Level 4 names 73 figure files by a relative path, and
they are below `images/`. They are listed one per line with its own
checksum by `fetch.sh`, and not in the table above, because 73 rows of
`fiddlesticks-italics.png` would bury the three documents without telling
a reader anything. What pins them is one digest over that listing:

```sh
LC_ALL=C find images -type f | LC_ALL=C sort | xargs shasum -a 256 |
	shasum -a 256
```

answers
`0223f7c0db9571c9bacd9c670bb8c90d8696d17d78efc66edbbcbc3ba7263848`.

What is not kept is the presentation. Each document names a stylesheet by
a relative path — `style.css` for CSS Fonts Module Level 4, `conform.css`
for WOFF 2.0 — and the Consortium's logo and its `fixup.js` by an address
on the Consortium's own server, and none of the four is here. A browser
lays the files out with default styling and shows nothing where the logo
is. The text, the tables, the figures and the anchors are all there,
which is what reading a clause or searching for one needs. The files
themselves are unmodified, so the references stay as the server wrote
them.

## How the files were fetched

`sh fetch.sh`, which writes every document of the table above and every
figure file, and prints `<sha256>  <path>` for each. Comparing that
output against the table is how a reader checks that this directory is
what it says it is.

## Why the two font documents are here

**WOFF 2.0** is the wrapper the web puts a font in, and the reason it is
a document of its own rather than a note in the OpenType specification is
its compression: a WOFF 2.0 file is not a font file with a header on it,
it is the tables of the font transformed and then Brotli-compressed, with
the `glyf` and `loca` tables given a transform of their own that a
decoder has to undo before there is a font to read. The format also fixes
what a decoder must reject, which is what keeps a font file from being an
arbitrary byte stream with a font's name on it.

**CSS Fonts Module Level 4** is the other half of the same question: not
what a font file contains but which face a run of text is drawn with. It
states the font matching algorithm — family, then style, then weight,
then stretch, in that order and with the fallbacks each step takes — the
`@font-face` rule and its descriptors, the generic families, and the
properties that reach the variation axes and the OpenType features of a
face. A renderer that has parsed a font file still has to decide which
one to use, and this is the document that answers that.

RFC 8081 belongs with these two and is in
[`docs/rfc/`](../rfc/README.md): it defines the `font/*` media types,
`font/woff2` among them, which is what says a WOFF 2.0 file is a font
rather than an application-specific blob.

## Why PNG is here

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

These documents are not covered by this repository's licence. Each
carries the notice of the World Wide Web Consortium — © 1996-2025 for
PNG, © 2026 for CSS Fonts Module Level 4, © 2024 for WOFF 2.0 — and each
states that the Consortium's liability, trademark and permissive document
licence rules apply. That licence allows the document to be copied and
redistributed provided the notice and the link to the licence travel with
it. The files here are unmodified, and the figure files belong to the
document that names them and stand with it. This is the first case of
D-124 and there is nothing further to state.

Software written from a specification is a separate matter, and the
Consortium's software licence of 2023 is the one the documents point at
for it.

TrueType is a registered trademark of Apple, Inc., and OpenType a
registered trademark of Microsoft Corporation; the WOFF 2.0 document
prints both.
