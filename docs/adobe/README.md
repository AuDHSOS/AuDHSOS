# Reference documents: Adobe

The three technical notes the OpenType specification defers to, kept
verbatim so that a structure or a charstring operator can be read against
its source without a network, and so that the source cannot change under
anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, D-124 what decides whether a document is kept under it, and
D-157 this directory.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `5176.CFF.pdf` | Adobe Technical Note #5176, *The Compact Font Format Specification*, version 1.0, 4 December 2003, 62 pages | 2026-09-16 from `https://adobe-type-tools.github.io/font-tech-notes/pdfs/5176.CFF.pdf` | 1062494 | `60e0696c4fac326de597722af37bcdbbd688e0337a548709a287405ff3b3996a` |
| `5177.Type2.pdf` | Adobe Technical Note #5177, *The Type 2 Charstring Format*, 16 March 2000, 38 pages | 2026-09-16 from `https://adobe-type-tools.github.io/font-tech-notes/pdfs/5177.Type2.pdf` | 199112 | `74e77442e63e38f03f2be5a41d2dafb3076260d2b6cd9b9d14f3990d6e17102b` |
| `5902.AdobePSNameGeneration.pdf` | Adobe Technical Note #5902, *Generating PostScript Names for Fonts Using OpenType Font Variations*, version 1.0, 14 September 2016, 5 pages | 2026-09-16 from `https://adobe-type-tools.github.io/font-tech-notes/pdfs/5902.AdobePSNameGeneration.pdf` | 91872 | `5bf6a671653921d21b5d0a174cc21de159628b81dabe027ec96758a7d40f6aa8` |

The checksums are here so that a reader can tell a file has not been
edited since. Each file is byte for byte what the server delivered; every
one was fetched twice, by `sh fetch.sh`, which prints `<sha256>  <path>`
for each, and the two fetches agreed.

## What is in which document

**#5176** is the Compact Font Format: the INDEX, the DICT, the FontSet
and the charset, string and subroutine structures a `CFF ` table is made
of. The OpenType pages name those structures and do not define them, so
this is the document that says what the bytes are.

**#5177** is the Type 2 charstring: the operators an outline is drawn
with, the operand stack they read, the hint operators, and the width that
is encoded in the first operand rather than in a field of its own. A CFF
outline is a Type 2 charstring, so a renderer that has parsed the table
still needs this to draw a glyph.

**#5902** is one rule: how to generate a PostScript name for an instance
of a variable font whose `fvar` table does not carry one. It is five
pages and is here because the OpenType index page names it beside the
other two.

## Why it is here

[`docs/microsoft/`](../microsoft/README.md) holds the OpenType
specification, and its CFF pages defer to these notes for what they name
without defining. A specification kept without the documents it defers to
is a specification with a hole in it at exactly the point where a CFF
outline is read.

`text-core` interprets CFF/CFF2 dictionaries, indices, and Type 2 charstrings
in document 17, T5–T6. These notes define the operators used by the reader
and its host tests under D-59.

## Terms

These documents are not covered by this repository's licence, and reading
their terms takes two steps, because the two answers differ.

Each PDF prints its own notice of its time — © 1996–1998, 2000, 2003 by
Adobe Systems Incorporated for #5176 — and states that no part of the
publication may be reproduced without the prior written consent of the
publisher. Taken alone that would put these copies where the ITU
documents in [`docs/itu/`](../itu/README.md) stand.

It does not stand alone. Adobe serves the collection these files come
from at `https://adobe-type-tools.github.io/font-tech-notes/`, whose
repository carries `LICENSE.md`, the Creative Commons
Attribution-NoDerivatives 4.0 International Public License, and whose
README says to refer to it for copying and redistribution. That is a
later grant by the same rights holder, and it permits redistribution of
the work as a whole, unmodified, with attribution. The files here are
unmodified, each carries its own notice inside it, and this README names
Adobe as the author and the address the copies came from, which is the
attribution the licence asks for. *NoDerivatives* is no constraint on
this repository, which keeps documents and modifies none.

So this is the first case of D-124, documents that may be redistributed,
and there is nothing further to state.

PostScript and OpenType are trademarks of Adobe Systems Incorporated and
Microsoft Corporation respectively.

Software written from these notes is a separate matter. Nothing is
transcribed from them yet; when something is, the note and the section
are named at the point of transcription, as D-40 requires.
