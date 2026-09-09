# Reference documents: the ITU Telecommunication Standardization Sector

The JPEG standards, kept verbatim so that a marker or a table can be read
against its source without a network, and so that the source cannot
change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. Decision D-59 records the
arrangement for `docs/rfc/`, and this directory is the same arrangement
for a body that is neither the RFC Editor, OASIS, the Consortium, nor
Ecma. What the same picture format needs from a fifth body is in
[`docs/cipa/`](../cipa/README.md).

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `t81.pdf` | Recommendation ITU-T T.81 (09/92) \| ISO/IEC 10918-1, *Information technology – Digital compression and coding of continuous-tone still images – Requirements and guidelines*, 186 pages | 2026-09-09 from `https://www.w3.org/Graphics/JPEG/itu-t81.pdf` | 1058883 | `631031d4ba56b06abee3e312a0f235b9422da9c7267d1c8f7604418795768bf0` |
| `t871.pdf` | Recommendation ITU-T T.871 (05/2011) \| ISO/IEC 10918-5, *… JPEG File Interchange Format (JFIF)*, 18 pages | 2026-09-09 from `https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-T.871-201105-I!!PDF-E&type=items` | 201764 | `ca3d749e7e04016f7695ba9d319eeeab3ad80c99265a883c14f03b9daf3e1c9a` |
| `t84.pdf` | Recommendation ITU-T T.84 (07/96) \| ISO/IEC 10918-3, *… Extensions*, 84 pages | 2026-09-09 from `https://www.itu.int/rec/dologin_pub.asp?lang=e&id=T-REC-T.84-199607-I!!PDF-E&type=items` | 419607 | `cd714dd1590724a3a23d0d346358e5ad8eb85743a111b8795f16234989f6173f` |
| `jfif-1.02.pdf` | Eric Hamilton, C-Cube Microsystems, *JPEG File Interchange Format, Version 1.02*, 1 September 1992, 9 pages — the document T.871 was made from, kept for the reason given below | 2026-09-09 from `https://www.w3.org/Graphics/JPEG/jfif3.pdf` | 17183 | `1135325aa12ee6b3def5a374fecdf3d6d0de88dc4f205af7de8d25a9c949e173` |

The checksums are here so that a reader can tell a file has not been
edited since. Every file is byte for byte what the server delivered; each
was fetched twice and the two fetches agreed.

The two `itu.int` addresses are the ones the ITU's own catalogue pages
for those Recommendations redirect to, and a plain `curl` of them
returns the PDF.

`t81.pdf` comes from the Consortium rather than from the ITU because the
ITU serves T.81 only behind a TIES account: the same `dologin_pub.asp`
address for T.81 redirects to a login page instead of a file, while for
T.871 and T.84 it does not. The Consortium has published that file at that address
since the nineteen-nineties; it is the 1992 Recommendation with the
ITU's own 1993 imprint on it, and its text is the text the ITU sells.

## The two questions these documents answer

The request they were fetched for was in two parts — what the format is,
and how it is stored on disk — and the parts are in two different
documents, which is the thing about JPEG that catches people out.

**What the format is: T.81.** It defines the entropy-coded stream and
nothing about a file. Annex A is the mathematics, Annex B the compressed
data format — the markers, `SOI`, `SOF`n, `DHT`, `DQT`, `SOS`, `DRI`,
`EOI` and the rest, and the shape of every segment. Annexes C and D are
the two entropy coders, Huffman and arithmetic. Annexes F, G, H and J are
the four modes of operation: sequential DCT, progressive DCT, lossless,
hierarchical. Annex K carries the example quantization and Huffman tables
that nearly every encoder in the world ships. Annex L is the patent
statement, which is of historical interest only: the arithmetic coder's
patents, the reason almost nothing implements Annex D, have all expired.

What T.81 never says is how such a stream becomes a file — no signature
beyond `SOI`, no colour space, no pixel aspect ratio, no metadata. A
decoder that implements T.81 and nothing else cannot tell whether the
three components it decoded are YCbCr or RGB.

**How it is stored on disk: T.871.** Eighteen pages that close exactly
that gap. It is the 1992 JFIF specification of C-Cube, formalized in 2011
without changing what it says. Clause 6 is the application marker
segments, clause 7 the conversion to and from RGB — this is where the
colour space of an ordinary JPEG file is finally stated — clause 8 the
image orientation, clause 9 the spatial relationship of the components,
and clause 10 the format proper: the `APP0` segment with the identifier
`JFIF\0`, the version, the density unit and the two densities, the
thumbnail, and the JFIF extension `APP0` with its three thumbnail
encodings.

`jfif-1.02.pdf` is here beside it because T.871 says of itself that it
was made to formalize that document, and because the older text is what
every decoder written between 1992 and 2011 was written against. Where
the two differ, T.871 is the standard and the C-Cube paper is the
history. It is also the only file in this directory whose terms are not a
problem, on which see below.

**T.84** is the third answer, and the one nothing uses. Annex F defines
SPIFF, the Still Picture Interchange File Format, which is what the JPEG
committee intended as the file format and which lost to JFIF before it
was published. The rest of the document — variable quantization,
selective refinement, tiling — is the same story. It is kept because a
decoder that meets a SPIFF file has to recognize it rather than guess,
and because the `APP8` marker it claims is otherwise unexplained.

What is deliberately not here is Exif, which is what a camera actually
writes into `APP1` and is therefore what most JPEG files on a disk really
are. It is not an ITU document; it is in [`docs/cipa/`](../cipa/README.md).

## Terms

These documents are not covered by this repository's licence, and their
terms are stricter than those of every other directory beside this one.

T.81, T.871 and T.84 each print the ITU's notice — © ITU 1993, 2012 and
1997 respectively — and each states that no part of the publication may
be reproduced without permission in writing from the ITU. T.84 adds one
exception, in a footnote to Annex F, that concerns SPIFF and not the
document. The ITU serves the files at no charge; that is a matter of
access and not of licence, and the notice on the page is the licence.

So this directory holds copies that the notice on their own first pages
does not grant permission to make. That was decided knowingly rather than
overlooked: the alternative, which is what [`docs/pcisig/`](../pcisig/README.md)
does, is to keep no copy and record the provenance rule instead, and it
was not chosen here. The files are unmodified, the notices travel with
them inside the PDFs, and nothing is republished from this repository. If
the ITU asks for them to go, they go, and this README becomes the
pcisig-shaped one.

`jfif-1.02.pdf` is the exception. It carries no copyright notice at all.
C-Cube circulated it freely in 1992 and, in the words of T.871's own
summary, it "was widely and freely circulated, and became widely
recognized as a de facto standard".

Software written from any of these is a separate matter from the
documents. Nothing is transcribed from them yet; when something is, the
clause is named at the point of transcription, as decision D-40 requires.

## Why it is here

Nothing in the workspace decodes JPEG today, and the files are kept ahead
of that on the same reasoning as ECMA-262: the moment a test or a comment
cites a clause number, the clause it cited has to be readable from the
repository, at the wording that was read, without asking a server that
has moved on.

The work they are ahead of is the one `docs/w3c/png-3.html` is already
beside. `audhsos-deflate` writes what a PDF calls `FlateDecode`; the
other filter a PDF names for an image is `DCTDecode`, and `DCTDecode` is
a T.81 stream — without the JFIF wrapper, because a PDF states the colour
space itself. So T.81 is the document for reading an image out of a PDF,
and T.871 is the document for reading the same image out of a file. A
framebuffer that shows a picture needs both.
