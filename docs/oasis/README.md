# Reference documents: OASIS

The standards this system implements that OASIS publishes, kept verbatim
so that a constant can be checked against its source without a network,
and so that the source cannot change under a crate that cites it. This is
[`docs/rfc/`](../rfc/README.md) for a second standards body, under the
same rule and for the same reason (D-59, D-90).

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace.

## What is here

| File | Document | Retrieved | Bytes | Lines | SHA-256 |
|------|----------|-----------|-------|-------|---------|
| `virtio-v1.4-cs01.html` | *Virtual I/O Device (VIRTIO) Version 1.4*, M. S. Tsirkin, C. Huck, M. E. Vara Larsen (eds.), OASIS Committee Specification 01, 8 April 2026 | 2026-09-07, from the archive named below | 5030643 | 91336 | `a48dd4967dda13cf2cf698f59b41ba1c4da0a428a30e9dfcd90ff2af3fc5d1c8` |

## Why the file did not come from its own URL

The document is published at

    https://docs.oasis-open.org/virtio/virtio/v1.4/cs01/virtio-v1.4-cs01.html

and what that address serves is not the same bytes twice. A content
delivery network in front of it rewrites every `mailto:` address in the
page into an obfuscated form with a key it draws afresh for each
response; two fetches a second apart differ in 144 lines, all of them the
editors' and chairs' addresses in the front matter. A checksum of that
would record nothing a later reader could check.

The same directory carries the whole specification as an archive, which
is served as a file and therefore untouched:

    https://docs.oasis-open.org/virtio/virtio/v1.4/cs01/virtio-v1.4-cs01.zip

2777664 bytes, SHA-256
`e848f7aeabc8c7050b8e317cbcea54751c71145bb5c4bbedea829320e8089de8`,
fetched twice on 2026-09-07 with the two fetches identical. The file
kept here is `virtio-v1.4-cs01.html` extracted from it, unchanged, and
it is the page as OASIS wrote it: the addresses in it are plain, and
nothing of the delivery network is in it.

To check this copy: fetch that archive, compare its checksum against the
one above, extract the one file, and compare it against this one.

## What the archive holds that this directory does not

The archive carries 185 files: the LaTeX sources the specification is
written in, the images, the stylesheet, the PDF, and the HTML. The
document names the LaTeX sources as the authoritative form and the PDF
and the HTML as renderings of it. The rendering is what is kept here,
because a constant is looked up by reading and the sources are a build.
Anyone who needs the authoritative form has the archive's address and its
checksum above.

## Which version, and why not the newest standard

Version 1.4 is a Committee Specification, not an OASIS Standard. The last
version of this specification to become an OASIS Standard is 1.1 of
20 April 2019; 1.2 and 1.4 stopped at Committee Specification and 1.3 at
Committee Specification Draft. Waiting for a standard would mean citing a
document seven years old and two versions behind what devices are built
against, so the current published version is the one kept, and its stage
is named here rather than glossed.

A Committee Specification is fixed at its address: it is a stage that has
been published and does not change, which is the property this directory
needs. The stage directory `cs01/` is that fixed address; the file one
level up, `virtio-v1.4.html`, is the latest-stage alias and may be
replaced, so it is not what was fetched.

What this project implements is the split virtqueue and the device
initialization sequence, and neither has changed since version 1.0. The
version matters for the section numbers a crate cites, not for the
constants.

## Terms

This document is not covered by this repository's licence. It carries

> Copyright © OASIS Open 2023. All Rights Reserved.

and a notice permitting the document to be copied and furnished to others
in whole or in part without restriction of any kind, provided the
copyright notice and that section are included on every copy, and
forbidding modification of the document itself. It is kept here whole and
unmodified, which is what those two conditions together ask for.

The notice is in the document, in the section headed *Notices* that
follows the front matter, so the condition that it travel with the copy
is met by the copy itself.

## Who cites it

`virtio-queue` ([document 12](../12-parallel-work.md), section 12.7.1):
the descriptor table, the available and used rings, the descriptor flags,
the device status bits, the feature bits, and the region alignments. The
crate names the section beside each group of constants.
