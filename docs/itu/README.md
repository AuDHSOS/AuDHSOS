# Reference documents: the ITU Telecommunication Standardization Sector

The JPEG standards, kept verbatim so that a marker or a table can be read
against its source without a network, and so that the source cannot
change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, and D-124 records why these documents are kept under it
despite their terms.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `t81.pdf` | Recommendation ITU-T T.81 (09/92) \| ISO/IEC 10918-1, *Information technology – Digital compression and coding of continuous-tone still images – Requirements and guidelines*, 186 pages | 2026-09-09 from `https://www.w3.org/Graphics/JPEG/itu-t81.pdf` | 1058883 | `631031d4ba56b06abee3e312a0f235b9422da9c7267d1c8f7604418795768bf0` |
| `t871.pdf` | Recommendation ITU-T T.871 (05/2011) \| ISO/IEC 10918-5, *… JPEG File Interchange Format (JFIF)*, 18 pages | 2026-09-09 from `https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-T.871-201105-I!!PDF-E&type=items` | 201764 | `ca3d749e7e04016f7695ba9d319eeeab3ad80c99265a883c14f03b9daf3e1c9a` |
| `t84.pdf` | Recommendation ITU-T T.84 (07/96) \| ISO/IEC 10918-3, *… Extensions*, 84 pages | 2026-09-09 from `https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-T.84-199607-I!!PDF-E&type=items` | 419607 | `cd714dd1590724a3a23d0d346358e5ad8eb85743a111b8795f16234989f6173f` |
| `jfif-1.02.pdf` | Eric Hamilton, C-Cube Microsystems, *JPEG File Interchange Format, Version 1.02*, 1 September 1992, 9 pages — the document T.871 was made from, and what a decoder predating T.871 was written against | 2026-09-09 from `https://www.w3.org/Graphics/JPEG/jfif3.pdf` | 17183 | `1135325aa12ee6b3def5a374fecdf3d6d0de88dc4f205af7de8d25a9c949e173` |

The checksums are here so that a reader can tell a file has not been
edited since. Every file is byte for byte what the server delivered; each
was fetched twice and the two fetches agreed. The two `itu.int` addresses
are what the ITU's catalogue pages for those Recommendations redirect to.

`t81.pdf` comes from the Consortium because the ITU serves T.81 only
behind a TIES account: the same `dologin_pub.asp` address for T.81
redirects to a login page instead of a file, while for T.871 and T.84 it
does not.

## What is in which document

**T.81 defines the entropy-coded stream and nothing about a file.**
Annex A is the mathematics, Annex B the compressed data format: the
markers — `SOI`, `SOF`n, `DHT`, `DQT`, `SOS`, `DRI`, `EOI` and the rest —
and the shape of every segment. Annexes C and D are the two entropy
coders, Huffman and arithmetic. Annexes F, G, H and J are the four modes
of operation: sequential DCT, progressive DCT, lossless, hierarchical.
Annex K carries the example quantization and Huffman tables. Annex L is
the patent statement.

What T.81 does not state is any of what makes a stream a file: no colour
space, no pixel aspect ratio, no metadata. A decoder that implements T.81
and nothing else cannot tell whether the three components it decoded are
YCbCr or RGB.

**T.871 defines the file.** Clause 6 is the application marker segments,
clause 7 the conversion to and from RGB, which is where the colour space
is stated, clause 8 the image orientation, clause 9 the spatial
relationship of the components, clause 10 the format: the `APP0` segment
with the identifier `JFIF\0`, the version, the density unit and the two
densities, the thumbnail, and the JFIF extension `APP0` with its three
thumbnail encodings.

**T.84 defines SPIFF**, in Annex F: a second file format, which claims
the `APP8` marker. The rest of the document is variable quantization,
selective refinement and tiling. A decoder that meets a SPIFF file has to
recognize it rather than guess.

**`jfif-1.02.pdf`** is the text T.871 formalizes. Where the two differ,
T.871 governs.

What a camera writes is none of these. It is Exif, in
[`docs/cipa/`](../cipa/README.md).

## Terms

These documents are not covered by this repository's licence.

T.81, T.871 and T.84 each print the ITU's notice — © ITU 1993, 2012 and
1997 — and each states that no part of the publication may be reproduced
without permission in writing from the ITU. T.84 adds one exception in a
footnote to Annex F, which concerns SPIFF and not the document. The ITU
serves the files at no charge, which is access and not licence.

So the copies here are ones the notices do not permit. D-124 is the
decision that keeps them and states what follows: the files are
unmodified so that their notices travel inside them, nothing is
republished from this repository, and a copy goes if the ITU objects.

`jfif-1.02.pdf` carries no copyright notice. T.871's summary records that
the document was widely and freely circulated.

Software written from any of these is a separate matter. Nothing is
transcribed from them yet; when something is, the clause is named at the
point of transcription, as D-40 requires.

## Why it is here

Nothing in the workspace decodes JPEG. The files are kept ahead of that
so that a clause a test or a comment cites is readable from the
repository at the wording that was read.

The work they are ahead of is the one `docs/w3c/png-3.html` is beside.
`audhsos-deflate` writes what a PDF calls `FlateDecode`; the other filter
a PDF names for an image is `DCTDecode`, which is a T.81 stream without
the JFIF wrapper, because a PDF states the colour space itself. T.81 is
therefore the document for reading an image out of a PDF, and T.871 the
document for reading the same image out of a file.
