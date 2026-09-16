# Reference documents: the Unicode Consortium

The annexes, reports, character database and conformance test files that
text rendering is written against, kept verbatim so that a rule or a
property value can be read against its source without a network, and so
that the source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents and data files. Rule R8 of the safety policy is about
dependencies, and it is untouched: `Cargo.lock` still lists only
workspace members, and no manifest names anything outside the workspace.
D-59 records the arrangement, D-124 what decides whether a document is
kept under it, and D-151 this directory.

## Which version

**Unicode 18.0.0**, which every file here is of.

The seven annexes and reports each print `Version: Unicode 18.0.0` in
their header, except the two that carry a version of their own: UTS #37
is at 7.0 and UTS #51 at 18.0, both released with Unicode 18.0.0. The
database files come from `https://www.unicode.org/Public/18.0.0/ucd/`
and each prints `# Version: 18.0.0` in its header, except
`UnicodeData.txt`, which carries no header at all and is taken from the
same directory. The `latest` alias of the Public tree resolved to 18.0.0
on the day of the fetch, and the file it served for `UnicodeData.txt` was
byte for byte the file at the versioned address; the versioned address is
what `fetch.sh` uses, because it is fixed and the alias moves.

D-154 states what happens when Unicode 19.0.0 is published.

## What is here

The annexes and reports, as HTML, at the revision named in the file name:

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `reports/tr9/tr9-52.html` | UAX #9, *Unicode Bidirectional Algorithm*, revision 52 of 2026-09-01, version Unicode 18.0.0 | 2026-09-16 from `https://www.unicode.org/reports/tr9/tr9-52.html` | 188933 | `3c281b38800bad85b16bf3a169b8cb693d8a2dbceb60903feb26e68a2659ee8a` |
| `reports/tr11/tr11-46.html` | UAX #11, *East Asian Width*, revision 46 of 2026-07-31, version Unicode 18.0.0 | 2026-09-16 from `https://www.unicode.org/reports/tr11/tr11-46.html` | 36549 | `2c60f3da0010870eaa4d9665a4bbae3e473c016c82d27bb508ea075cbf2afa7a` |
| `reports/tr14/tr14-57.html` | UAX #14, *Unicode Line Breaking Algorithm*, revision 57 of 2026-09-01, version Unicode 18.0.0 | 2026-09-16 from `https://www.unicode.org/reports/tr14/tr14-57.html` | 226166 | `296df5332aefe695b4951b511288b118bc6a7b43db7fadfd60f6062096acc7c0` |
| `reports/tr29/tr29-49.html` | UAX #29, *Unicode Text Segmentation*, revision 49 of 2026-09-01, version Unicode 18.0.0 | 2026-09-16 from `https://www.unicode.org/reports/tr29/tr29-49.html` | 141203 | `60fb49ee640a154d9e40468306c6b283edcef73be3af5ede778107b4787bdec8` |
| `reports/tr37/tr37-16.html` | UTS #37, *Unicode Ideographic Variation Database*, revision 16 of 2026-04-30, version 7.0 | 2026-09-16 from `https://www.unicode.org/reports/tr37/tr37-16.html` | 33708 | `92ae55ff5f162ec416dbb5616556e9d4ebd7bce68f5c7c03c71467a7a7a448d4` |
| `reports/tr50/tr50-35.html` | UAX #50, *Unicode Vertical Text Layout*, revision 35 of 2026-07-31, version Unicode 18.0.0 | 2026-09-16 from `https://www.unicode.org/reports/tr50/tr50-35.html` | 528174 | `e8662f3394d20212beaa1862b505582cee5383c4b502a46966e317967f7bc65e` |
| `reports/tr51/tr51-31.html` | UTS #51, *Unicode Emoji*, revision 31 of 2026-08-28, version 18.0 | 2026-09-16 from `https://www.unicode.org/reports/tr51/tr51-31.html` | 207223 | `a12a8c986f50b97c2d82cbdabfe18928ac16375e74b6eb61b54654e132966658` |

Vertical text layout was UTR #50 and is now a Standard Annex: the file
the Consortium serves at `reports/tr50/` titles itself Unicode® Standard
Annex #50, which is why the table says UAX and not UTR. Its revision 35
carries a "Proposed Update" heading and a draft status block, and both
are inside HTML comments; `reports/tr50/` served this exact file, so
revision 35 is the published version and not a draft.

The property files of the character database, which an implementation is
written against rather than cited by:

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `ucd/UnicodeData.txt` | The main database: one line per code point, fifteen fields, the General_Category, the canonical combining class, the Bidi_Class and the decomposition among them | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/UnicodeData.txt` | 2243593 | `0736451de439ae7baf1425136617da495e09ee5afbe6e394374db7009ea08950` |
| `ucd/EastAsianWidth.txt` | The East_Asian_Width property, which UAX #11 defines | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/EastAsianWidth.txt` | 204705 | `a0cf29eacd00cfcaec4381c6b7c281685f18dbb4e7ff82b4076ccb342ca839aa` |
| `ucd/LineBreak.txt` | The Line_Break property, which UAX #14 defines | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/LineBreak.txt` | 266206 | `ae8cf1970c73f3f1a12d77852df96c4b2b1723ca8b388e36bb5739250c6849de` |
| `ucd/auxiliary/GraphemeBreakProperty.txt` | The Grapheme_Cluster_Break property, which UAX #29 defines | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/GraphemeBreakProperty.txt` | 100009 | `0839dcb79e4ac639ecd538b1abf7c9d22e3f9dd265b7e182d33627aa4d75b45a` |
| `ucd/auxiliary/WordBreakProperty.txt` | The Word_Break property, which UAX #29 defines | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/WordBreakProperty.txt` | 116210 | `8dbfa17063e11084201f33c3e76d485d3b9166930c71db8e39ed1c9234171aec` |
| `ucd/emoji/emoji-data.txt` | The five emoji properties, which UTS #51 defines: Emoji, Emoji_Presentation, Emoji_Modifier, Emoji_Modifier_Base, Extended_Pictographic | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/emoji/emoji-data.txt` | 108719 | `80d00f8e616a0ef27fd6b8de3b758c06383b5d917e2977709578e68baf733bf1` |

The conformance test files, which say what a correct implementation
answers:

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `ucd/auxiliary/GraphemeBreakTest.txt` | Every grapheme cluster boundary case of UAX #29, each line a string with `÷` at a break and `×` where there is none | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/GraphemeBreakTest.txt` | 137090 | `b0cf047ee94485bbdc846de2b902f5f8a815f6b674f9d04223cddadd91c9df31` |
| `ucd/auxiliary/WordBreakTest.txt` | The same for the word boundaries of UAX #29 | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/WordBreakTest.txt` | 322136 | `3dd70c071781276067c680d87303f60434adce7b067bd063194af374edad86a5` |
| `ucd/auxiliary/LineBreakTest.txt` | The same for the line break opportunities of UAX #14 | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/LineBreakTest.txt` | 3178750 | `fd9ff9eca411080085b41471bba79ae19d9db2e16924f9770b8509f1affd3f95` |
| `ucd/BidiTest.txt` | The bidirectional algorithm over every string of Bidi_Class values up to length four, with the levels and the reorder each paragraph direction produces | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/BidiTest.txt` | 7959988 | `9af2f882a4ab50912e388f069a673b94eacd82fa6d07d20a3ff7f3c759e905aa` |
| `ucd/BidiCharacterTest.txt` | The same algorithm over strings of actual code points, which is the case `BidiTest.txt` cannot state | 2026-09-16 from `https://www.unicode.org/Public/18.0.0/ucd/BidiCharacterTest.txt` | 6880771 | `045b24d2c8ab066951bd32fe8c6b4de34647f72b5b1c7df0265f24ab53573e01` |

The checksums are here so that a reader can tell a file has not been
edited since. Each file is byte for byte what the server delivered; every
one was fetched twice, by the script below, and the two fetches agreed.
The seven reports are 3479, 340, 4623, 3326, 491, 2208, and 4356 lines,
in the order of their table; the six property files 41341, 2754, 3740,
1519, 1558, and 1305; the five test files 883, 1974, 19376, 497590, and
96465.

### The figure files

A report names its figures by a relative path, so 139 further files sit
below `reports/`, at the paths the reports name them by: 132 for UTS #51,
which illustrates each emoji sequence with how four vendors render it,
three for UAX #50, two for UAX #11, and one each for UAX #29 and UTS #37.
They are listed one per line with its own checksum by `fetch.sh`, and not
in a table here, because 139 rows of `apple_1f469.png` would bury the
documents above without telling a reader anything. What pins them is one
digest over that listing:

```sh
LC_ALL=C find reports -type f ! -name '*.html' | LC_ALL=C sort |
	xargs shasum -a 256 | shasum -a 256
```

answers
`35505943891193cceccbbc7534fe19b3d72c9a540bc13276b8657797ff2959fc`.

Four references reach outside these files and are left as they stand,
because the copies are unmodified: the stylesheet `reports-v2.css`, the
Consortium's logo, the icon that marks an external link in UTS #51, and
the two figures of UAX #11, which that report names by their address on
the server rather than by a relative path. A browser lays a report out
with default styling and shows nothing where those images are; the text,
the rules, the tables and the anchors are all there.
The two UAX #11 figures are kept anyway, at `reports/tr11/images/`, so
that what they show is in the repository even though the document does
not point there.

## How the files were fetched

`sh fetch.sh`, which writes every file of the two tables above and every
figure file, and prints `<sha256>  <path>` for each. Comparing that
output against the tables is how a reader checks that this directory is
what it says it is.

## What is in which document

**UAX #9** is the bidirectional algorithm: the Bidi_Class values, the
paragraph level, the explicit embedding and isolate formatting
characters, the resolution of weak, neutral and implicit levels, and the
reordering of a line by its resolved levels. It is the one document here
whose output is a permutation rather than a boundary set.

**UAX #11** is the East_Asian_Width property and the six values it takes.
It is what says a character occupies one column or two in a terminal,
which is the only question this project's display asks of it today.

**UAX #14** is line breaking: the Line_Break property, the pair table of
section 6, and the rules LB1 to LB31 that say where a line may be broken
and where it may not. It is the longest chain of rules of the seven and
the one with the most exceptions.

**UAX #29** is text segmentation: grapheme cluster, word and sentence
boundaries. The grapheme cluster is what a user calls a character, and it
is what a cursor moves over and a backspace deletes, which is why it
comes before any of the rest.

**UTS #37** is the Ideographic Variation Database: how a registered
collection of variation sequences is identified, and what a variation
selector after an ideograph selects.

**UAX #50** is vertical text layout: the Vertical_Orientation property
and its four values, which say whether a character is drawn upright or
rotated when a line runs top to bottom.

**UTS #51** is emoji: the five properties of `emoji-data.txt`, the
presentation selectors, the modifier sequences, and the ZWJ sequences a
renderer has to treat as one glyph or fail visibly.

## Why it is here

Nothing in the workspace implements any of these yet. `gfx` draws with
the project's own bitmap font of 95 glyphs, one per printable ASCII
character, and `audhsos-encoding` and `audhsos-utf16` convert between
encodings without asking any property of a code point. The files are kept
ahead of that work, for the reason D-59 gives: a rule a test or a comment
cites must be readable from the repository at the wording that was read.

The test files are the part that will not come from anywhere else. A
segmentation or bidirectional implementation checked only against its
author's reading of the rules is checked against nothing; these five
files are the Consortium's own answer for hundreds of thousands of cases,
and D-40 has them read rather than restated.

## Terms

These files are not covered by this repository's licence, and the two
groups stand under different terms.

The property and test files are Unicode Data Files, which the Terms of
Use define as the computer data files under
`https://www.unicode.org/Public/`. They are licensed under the Unicode
License v3, which permits copying, modification and redistribution
provided the copyright notice and the permission notice travel with the
data. Each file carries that notice in its own header, and the files here
are unmodified. That is the first case of D-124, and there is nothing
further to state.

The seven annexes and reports are not Data Files. Each prints the
Consortium's notice at its end — © 1999–2026 Unicode, Inc. for UAX #9,
with the year of first publication differing per report — and each states
the restriction in the same words:

> Specifically, you may make copies of this publication and may annotate
> and translate it solely for personal or internal business purposes and
> not for public distribution, provided that any such permitted copies
> and modifications fully reproduce all copyright and other legal notices
> contained in the original. You may not make copies of or modifications
> to this publication for public distribution, or incorporate it in whole
> or in part into any product or publication without the express written
> permission of Unicode.

So the seven copies here are ones the notice does not permit, and they
stand where the ITU documents in [`docs/itu/`](../itu/README.md) stand,
under the second case of D-124 and for the same reason: the copy is what
D-59 is for, and no arrangement that respects the restriction delivers
it. What follows is stated rather than assumed — the files are unmodified
so that their notices travel inside them, nothing is republished from
this repository, and a copy goes if Unicode objects. The figure files
belong to the reports and stand with them.

Unicode and the Unicode Logo are registered trademarks of Unicode, Inc.

Software written from any of these is a separate matter. Nothing is
transcribed from them yet; when something is, the document, the version
and the rule are named at the point of transcription, as D-40 requires.
