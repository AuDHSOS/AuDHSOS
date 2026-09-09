# Reference documents: the Camera & Imaging Products Association

Exif, the metadata format that a camera writes into a JPEG file, kept
verbatim so that a tag can be read against its source without a network,
and so that the source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement for `docs/rfc/`, and this directory is the same arrangement
for the one part of the JPEG picture that is not published by the ITU.
The rest of it is in [`docs/itu/`](../itu/README.md).

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `dc-008-2023.pdf` | CIPA DC-008-Translation-2023, *Exchangeable image file format for digital still cameras: Exif Version 3.0*, established May 2023, 255 pages — the English translation of the original Japanese standard CIPA DC-008-2023 | 2026-09-09 from `https://www.cipa.jp/std/documents/download_e.html?DC-008-Translation-2023-E` | 4476137 | `563e0ccb3b3700b5e35f104a0207ef6d577dbad10b1f1dc91116a80682bda39c` |

The checksum is here so that a reader can tell the file has not been
edited since. It is byte for byte what the server delivered; it was
fetched twice and the two fetches agreed.

CIPA serves it behind a disclaimer that has to be accepted before the
download starts, which is a click and not an account: the file is public
and free. The address in the table is the disclaimer page; the download
itself is a `POST` to `https://www.cipa.jp/std/documents/dll.cgi` with
the single field `dlltarget=DC-008-Translation-2023-E`, which is what
that page's own form submits once the box is ticked.

The document says of itself that it is a translation, and that "in the
event of any doubts arising as the contents, the original Standard is to
be the final authority". The original is CIPA DC-008-2023, in Japanese,
and it is not kept here. CIPA and JEITA formulated the standard jointly,
which is why the same text is met in the wild under the JEITA name.

## Why it is here

Because a JPEG file on a disk is usually not the file that
[`docs/itu/`](../itu/README.md) describes.

T.871 defines JFIF, which puts an `APP0` segment after the `SOI` marker.
A camera, a phone, or anything that has ever passed through one writes
`APP1` instead, and the contents of that segment are this document: a
whole TIFF Rev. 6.0 file — byte-order mark, image file header, and a
chain of image file directories — embedded in a JPEG marker segment.
Clause 4.7.2 is the internal structure of `APP1`; clauses 4.6.5 to 4.6.8
are the tag tables, the TIFF ones the format inherits and the Exif, GPS
and Interoperability directories it adds. Version 3.0, of May 2023, is
the edition that admitted UTF-8 into tags that had been ASCII since 1995,
and that opened `APP11` to the box-structured data of the JPEG Systems
standard; its revision history, on pages 13 to 15, lists the rest.

The consequence for a decoder is the practical reason the file is here.
Orientation is the clearest case: the `Orientation` tag lives in Exif,
not in T.81 and not in T.871, so a decoder that reads only those two
standards displays a large fraction of the world's photographs rotated
ninety degrees and is right about every byte it read. Whether a stream is
YCbCr or something else can likewise be decided by a tag rather than by
the JFIF segment that is absent.

Nothing in the workspace decodes JPEG today. The file is kept ahead of
that on the same reasoning as ECMA-262 and the JPEG Recommendations: the
moment a test or a comment cites a clause number, the clause it cited has
to be readable from the repository, at the wording that was read.

## Terms

This document is not covered by this repository's licence. It carries the
notice "Copyright © 2023 CIPA All Rights Reserved", and CIPA states no
permission to redistribute it. The copy here was made knowingly on the
same footing as the ITU Recommendations next door, and what is written in
[`docs/itu/`](../itu/README.md) under *Terms* applies here without
change: the file is unmodified, its notice travels inside it, nothing is
republished from this repository, and if CIPA asks for it to go, it goes.

The download is conditioned on a disclaimer, and the disclaimer is what
it disclaims rather than what it permits. CIPA warrants nothing about the
standard, and in particular warrants nothing about essential intellectual
property rights being licensed or even identified. The full text of it is
in the document's own front matter, before the contents.

Software written from the standard is a separate matter from the
document. Nothing is transcribed from it yet; when something is, the
clause is named at the point of transcription, as decision D-40 requires.
