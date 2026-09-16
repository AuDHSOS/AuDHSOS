# Reference documents: ISO/IEC

The normative form of the OpenType specification, kept verbatim so that a
clause can be read against its source without a network, and so that the
source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. This is a
document. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, D-124 what decides whether a document is kept under it, and
D-153 this directory.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `c074461e_ISO_IEC_14496-22_2019.pdf` | ISO/IEC 14496-22:2019(E), *Information technology — Coding of audio-visual objects — Part 22: Open Font Format*, fourth edition, January 2019, 640 pages | 2026-09-16 from `https://standards.iso.org/ittf/PubliclyAvailableStandards/c074461_ISO_IEC_14496-22_2019.zip` | 4823452 | `32b4f1f184bfb24ffbdc01cb19eada57c0d2ff63d3f2bc94b2627bc983882098` |

## How the file was obtained

Not by a script, and this directory has none. ISO/IEC 14496-22 is on
ISO's Publicly Available Standards list and costs nothing, but the
address above answers a request with an HTML page titled *Licence
Agreement for Publicly Available Standards*, whose only control is a
button named `ok` with the value `I accept`; the catalogue page at
`https://www.iso.org/standard/74461.html` answers `403` with a bot check.
A script that accepts a licence agreement on every run accepts it without
anybody reading it, so the acceptance is a person's and the file was
downloaded by hand.

This is the second document in this tree that a shell on the development
machine cannot fetch; [`docs/uefi/`](../uefi/README.md) records the
first, for a different gate. The checksum is what makes that harmless: it
says which bytes were read, whoever fetched them.

## Which edition, and what it governs

The fourth edition, 2019-01, which is the current one. ISO/IEC 14496-22
is the normative form of OpenType;
[`docs/microsoft/`](../microsoft/README.md) holds what Microsoft serves,
OpenType 1.9.1, which is what every implementation is written against and
which by Microsoft's own statement incorporates revisions of a
preliminary working draft of the fifth edition. So the two are the same
specification and not the same text, and each has a job:

| Question | Read |
|----------|------|
| What is in a table, and what is a field called | `docs/microsoft/`, which is the text the code follows and is newer |
| What a clause number means, and what the standard requires of a conforming implementation | this document, which has the clause numbering, the scope, the normative references, the terms and definitions, and the conformance clause |
| Which of the two governs where they differ | this document |

A citation names whichever of the two was read, as D-40 requires: a page
and a heading for `docs/microsoft/`, a clause number for this file. A
clause number quoted without reading this document is a citation from
memory, which the project's rule forbids.

## Terms

This document is not covered by this repository's licence. It prints
ISO/IEC's notice, © ISO/IEC 2019, and the restriction, which carries an
exception the other restricted documents of this tree do not have:

> All rights reserved. Unless otherwise specified, or required in the
> context of its implementation, no part of this publication may be
> reproduced or utilized otherwise in any form or by any means,
> electronic or mechanical, including photocopying, or posting on the
> internet or an intranet, without prior written permission.

This project is an implementation of the Open Font Format, and the copy
is here so that the implementation can be written and checked against the
text, which is the case the exception names. The exception is read as
narrower than what this repository does, not wider: a public repository
posts the file on the internet, and the exception does not say how far it
reaches. So the copy stands where the ITU documents in
[`docs/itu/`](../itu/README.md) and the UEFI documents in
[`docs/uefi/`](../uefi/README.md) stand, under the second case of D-124
and for the same reason — the copy is what D-59 is for, a clause readable
without a network at wording that cannot change under what cites it. What
follows is stated rather than assumed: the file is unmodified so that its
notice travels inside it, nothing is republished from this repository,
and the copy goes if ISO objects.

## Why it is here

Nothing in the workspace reads a font file yet. The standard is kept
ahead of that work, and it is what settles a disagreement between the
code and `docs/microsoft/`, which is the one question that copy cannot
answer about itself.
