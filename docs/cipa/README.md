# Reference documents: the Camera & Imaging Products Association

Exif, the metadata format a camera writes into a JPEG file, kept verbatim
so that a tag can be read against its source without a network, and so
that the source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, and D-124 records why this document is kept under it despite
its terms. The rest of the JPEG picture is in
[`docs/itu/`](../itu/README.md).

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `dc-008-2023.pdf` | CIPA DC-008-Translation-2023, *Exchangeable image file format for digital still cameras: Exif Version 3.0*, established May 2023, 255 pages — the English translation of CIPA DC-008-2023 | 2026-09-09 from `https://www.cipa.jp/std/documents/download_e.html?DC-008-Translation-2023-E` | 4476137 | `563e0ccb3b3700b5e35f104a0207ef6d577dbad10b1f1dc91116a80682bda39c` |

The checksum is here so that a reader can tell the file has not been
edited since. It is byte for byte what the server delivered; it was
fetched twice and the two fetches agreed.

The address in the table is a page carrying a disclaimer that has to be
accepted before the download starts. The download itself is a `POST` to
`https://www.cipa.jp/std/documents/dll.cgi` with the single field
`dlltarget=DC-008-Translation-2023-E`, which is what that page's form
submits once the box is ticked. No account is involved.

The document states that it is a translation and that in case of doubt
the original, CIPA DC-008-2023 in Japanese, is the final authority. The
original is not kept here. CIPA and JEITA formulated the standard
jointly, so the same text is met under the JEITA name.

## What is in it

A JPEG file on a disk is usually not the file
[`docs/itu/`](../itu/README.md) describes. T.871 puts an `APP0` segment
after `SOI`; a camera writes `APP1`, and its contents are this document:
a TIFF Rev. 6.0 structure — byte-order mark, image file header, and a
chain of image file directories — inside a JPEG marker segment.

Clause 4.7.2 is the internal structure of `APP1`. Clauses 4.6.5 to 4.6.8
are the tag tables: the TIFF ones the format inherits, and the Exif, GPS
and Interoperability directories it adds. Version 3.0 admitted UTF-8 into
tags that had been ASCII, and opened `APP11` to the box-structured data
of the JPEG Systems standard; the revision history on pages 13 to 15
lists the rest.

The consequence for a decoder: `Orientation` is an Exif tag and is in
neither ITU document, so a decoder that reads only those two standards
displays a large share of photographs rotated ninety degrees while being
right about every byte it read. Whether a stream is YCbCr can likewise be
settled by a tag rather than by a JFIF segment that is absent.

## Terms

This document is not covered by this repository's licence. It carries
"Copyright © 2023 CIPA All Rights Reserved", and CIPA states no
permission to redistribute it. The copy here is kept under D-124 on the
same footing as the ITU Recommendations: the file is unmodified so that
its notice travels inside it, nothing is republished from this
repository, and the copy goes if CIPA objects.

The disclaimer conditioning the download disclaims rather than permits.
CIPA warrants nothing about the standard, and in particular nothing about
essential intellectual property rights being licensed or identified. Its
full text is in the document's front matter, before the contents.

Software written from the standard is a separate matter. Nothing is
transcribed from it yet; when something is, the clause is named at the
point of transcription, as D-40 requires.

## Why it is here

Nothing in the workspace decodes JPEG. The file is kept ahead of that so
that a clause a test or a comment cites is readable from the repository
at the wording that was read.
