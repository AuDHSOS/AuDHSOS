# Reference documents: Microsoft

The OpenType specification, kept verbatim so that a table format or a
field offset can be read against its source without a network, and so
that the source cannot change under anything that cites it.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies, and it is
untouched: `Cargo.lock` still lists only workspace members, and no
manifest names anything outside the workspace. D-59 records the
arrangement, D-124 what decides whether a document is kept under it, and
D-152 this directory.

## Which specification, and which text of it

**OpenType 1.9.1**, the version every page here prints in its title.

The normative form of OpenType is ISO/IEC 14496-22, *Open Font Format*.
The text Microsoft serves is the same specification and is what every
implementation is written against; its index page states the relation in
its own words:

> OpenType 1.9.1 incorporates revisions in a preliminary working draft of
> the 5th edition of the ISO/IEC 14496-22 "Open Font Format" standard.

[`docs/iso/`](../iso/README.md) holds that standard, the fourth edition
of 2019, so the two texts can be read against each other.

## What was kept, and how

Microsoft serves the specification as 84 HTML pages, one per chapter,
table or registry, and not as one document. Each page is kept as one
file, byte for byte as the server delivered it, named after the last
segment of its address with `.html` appended. Nothing is cut and nothing
is rewritten.

Three consequences follow, and they are the whole of what this
arrangement costs:

- The site around the document travels with each page. Roughly half of
  each file is the navigation, the feedback controls and the script tags
  of `learn.microsoft.com`, which reference assets on Microsoft's servers
  and do not load without a network. The specification text, its tables
  and its anchors are all present and a browser lays them out.
- A cross-reference between pages is a relative link without an
  extension, `href="cmap#cmap-header"`, and resolves to a file
  `cmap` that is not here. The mapping is mechanical: the part before the
  `#` is the file name here without its `.html`, so that link is
  `cmap.html`, heading `cmap-header`.
- A figure is named by a relative path and does resolve. The 399 files
  those paths name are below `images/`.

The alternative was to cut each page to its article body, as
[`docs/ecma/`](../ecma/README.md) did for ECMA-262, and rewrite the
cross-links so that they resolve. It was not taken: that page had one
container holding the whole specification and needed no link rewriting,
whereas here the cut would have to be made 84 times and the rewrite would
modify every page, which costs more than it buys and weakens the position
the Terms section below rests on.

The authored source was looked for and is not public. Each page names its
own source file in a `MicrosoftDocs/typography` repository, and both that
repository and the raw file it names answer `404` to a request that is
not signed in, so the Markdown the pages are rendered from cannot be
fetched.

## What is lost against the ISO text, which is beside it

- **The clause numbering.** ISO/IEC 14496-22 numbers its clauses; these
  pages are titled by table tag and have anchors rather than clause
  numbers. A citation of this copy names the page and the heading, not a
  clause.
- **The edition.** OpenType 1.9.1 follows a preliminary working draft of
  the fifth edition, so it is ahead of the fourth edition of 2019 and is
  not identical to the fifth. Where the two differ, the ISO text governs,
  and this copy does not say where that is.
- **The front matter.** Scope, normative references, terms and
  definitions, and the conformance clause are parts of an ISO standard
  and are not pages here.
- **Nothing of the table formats.** Every table, field, offset, flag and
  algorithm the format is made of is in these pages, which is why this is
  the copy an implementation is written against and the ISO edition the
  one a disagreement is settled by.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `index.html` | *OpenType specification (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/` | 50949 | `298c975ca368ef8c416088fbd0ee83d030738ab57e0096443de727a1d0cf3815` |
| `overview.html` | *OpenType specification overview (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/overview` | 54692 | `cd02b22314d59efc4e8195ec0f6e85e2e9a2cc67c1a2d46d51d7a8ade7412eb3` |
| `ttochap1.html` | *Advanced typographic tables - OpenType Layout (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ttochap1` | 67737 | `f20c5ff8d191a72c16e7cddec86dbb688938692c47dd08483b408dbf4fd00738` |
| `otvaroverview.html` | *OpenType Font Variations overview (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/otvaroverview` | 133645 | `6d9ba5377f4522abd3d483d897757e256d20aeb5184c251c3fab5709b23f3994` |
| `otff.html` | *OpenType font file (OpenType 1.9.1)*, updated 2025-03-19 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/otff` | 87017 | `a00a96504da4592f7c08e43963e59e613ab6cf4ac76aa463152e49f249256a4c` |
| `chapter2.html` | *OpenType layout common table formats (OpenType 1.9.1)*, updated 2025-03-18 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/chapter2` | 177800 | `bb61da9f3ffe83a053c20557d0aae6f488d46db6d9a6a4c7e28bcb564674c7c5` |
| `otvarcommonformats.html` | *OpenType Font Variations Common Table Formats (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/otvarcommonformats` | 108722 | `0d057c0c41cc8ddc023c17a900f3b96fcad888dabaf40c7f12b3f190809f3340` |
| `avar.html` | *avar — Axis variations table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/avar` | 56946 | `b2e39d71375a6ff90ad6e26c8328211ea6febe2200a2f330ca5440116af12369` |
| `base.html` | *BASE - Baseline table (OpenType 1.9.1)*, updated 2024-07-07 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/base` | 115826 | `36ea9bacf70dc9e4e6ffc25bb1109afa9670e3f8040f91850588bf15776a05c1` |
| `cbdt.html` | *CBDT - Color Bitmap Data Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cbdt` | 56141 | `d82220feb2c6ae0971dc8b9d176be1a9de43a8d5d94acfa695ff7efb276688aa` |
| `cblc.html` | *CBLC - Color Bitmap Location Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cblc` | 51840 | `62f9bec7fc36c599dc877e48632ebafeeab27d8b9990c95a11996af798774f03` |
| `cff.html` | *CFF - Compact font format table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cff` | 50616 | `852c31132becef0276e8fa6cede46e7027aa0d2cb39eb7f0b0f5be8cd66a8ef0` |
| `cff2.html` | *CFF2 - Compact font format version 2 table (OpenType 1.9.1)*, updated 2024-10-11 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cff2` | 228071 | `97c70846b010c4fa1ea7c3f6dcf6bf0b3157f4865e04373f0bd53c5bc3540e10` |
| `cmap.html` | *cmap - Character To Glyph Index Mapping Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cmap` | 97662 | `4c4f8470fffa5b4b2fad0f7303a30d29f4e66efaae0177c1ff878dccb8250a8a` |
| `colr.html` | *COLR - Color Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/colr` | 227022 | `1b5da242df4b7396a2a1b8223b928d3ccd9e87b6779d37be1230fd23213a85dc` |
| `cpal.html` | *CPAL - Color Palette Table (OpenType 1.9.1)*, updated 2025-03-19 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cpal` | 65766 | `694adf752a0b012ce744e94cb1181704686309057586b58a4ba6d22fb1fb35c7` |
| `cvar.html` | *cvar — CVT Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cvar` | 54203 | `e93b71434efc032e6dc02661c43e38415a082be05c8ce75830201e5afba1b759` |
| `cvt.html` | *cvt - Control value table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/cvt` | 48494 | `a7d4e2d29743301191c8784d2f14b16690530e059693ef934674a42c0a61c379` |
| `dsig.html` | *DSIG — Digital signature table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dsig` | 56780 | `a53832d6762ec7dfe372966e6a8ed3cfed26ea5e8e8e30e59afc011697f17a8d` |
| `ebdt.html` | *EBDT - Embedded Bitmap Data Table (OpenType 1.9.1)*, updated 2025-03-19 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ebdt` | 61262 | `a23735a190ac216d0e140def6db12cb8a11f72485fcd2b8552322fcc2a03a8c5` |
| `eblc.html` | *EBLC - Embedded Bitmap Location Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/eblc` | 70438 | `cd44ebb5456c3279dee0d1778720c5f19cb94a7f6801b49c416816b7efabba12` |
| `ebsc.html` | *EBSC - Embedded Bitmap Scaling Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ebsc` | 51190 | `a11d2d4a3499b37ed6d3fc4675abb313f833cdc1cd0d4e669ddeeaac033d4742` |
| `fpgm.html` | *fpgm - Font program table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/fpgm` | 48838 | `ef6aa82d3169f301d2dc9fc0b5dd09c2637f248a4065da1f236c033a04e689f2` |
| `fvar.html` | *fvar — Font Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/fvar` | 70087 | `df90e0a7435dd6e19b941678e7bfe12e199cb1b03661e39cb916a28f2cda82bd` |
| `gasp.html` | *gasp — Grid-fitting And Scan-conversion Procedure Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/gasp` | 55747 | `bd5093355ad1b342d4b02d4f2aebf60aa04aaf4b2c259b768e2088707e773fb3` |
| `gdef.html` | *GDEF — Glyph Definition Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/gdef` | 87749 | `43214b736ef86f90ef16369c9395d388e39c09fb8c2bdcefaf45f869b634585f` |
| `glyf.html` | *glyf - Glyf data table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/glyf` | 77555 | `bc4f97de0d6384c6663e329dd427ad6b641cee19d2dc5a10f84c5931d413e016` |
| `gpos.html` | *GPOS — Glyph Positioning Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/gpos` | 198793 | `30c12aae51bae7449da6ec2eb8fa0a6703c301860ec016451862c187eb5d0315` |
| `gsub.html` | *GSUB — Glyph Substitution Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/gsub` | 137950 | `c28921469aa25f4934b96cda197538cfdff3bcb5a371e3c7dabe0084dcd09425` |
| `gvar.html` | *gvar — Glyph Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/gvar` | 83378 | `491255050a4d50c3107a05571e5788a1b39fc0eb0a15cc3ea18e9c735f69ca0b` |
| `hdmx.html` | *hdmx - Horizontal Device Metrics (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/hdmx` | 50620 | `fc30cf86f4fef6ca0869415a3085b76f6ade7955cd9cad3dec288f929bf51b12` |
| `head.html` | *head - Font header table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/head` | 57600 | `06385bb6ad979318b16becb0e8a9b26e60442a1e0bf4a384e1aad53c22cc60c7` |
| `hhea.html` | *hhea - Horizontal header table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/hhea` | 52822 | `4d33bf0771a5504960e7aa7c03d29bf4e1327dd8c7c89d8898e216b8e762b5f0` |
| `hmtx.html` | *hmtx - Horizontal metrix table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/hmtx` | 53836 | `9cda830a5769ba79dfc3d02717dee45c65f00cab5aa960aa0590b62a052e59c0` |
| `hvar.html` | *HVAR — Horizontal Metrics Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/hvar` | 58085 | `34e9a1043cb43ad3524f3a9e8bce696fc78fa4f091779871d23355ac24e0839f` |
| `jstf.html` | *JSTF — Justification Table (OpenType 1.9.1)*, updated 2025-03-20 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/jstf` | 81980 | `9f080b6364ece4bf50e467c329f18f00e1c8ea896a8daabf0db68d7f2bb69580` |
| `kern.html` | *kern - Kerning (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/kern` | 58523 | `f5af09998913823de9aadd1b1d852d616032b219772f79a58b902c7490c13494` |
| `loca.html` | *loca - Index-to-location (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/loca` | 50025 | `1befbbef4cb691dfaf7faad2e6e758e6cafd68a43198ba85fc8c6ba2735e7700` |
| `ltsh.html` | *LTSH - Linear Threshold (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ltsh` | 50282 | `d552c9a8f6ed01007289c0c6adc0b9b8649fbc0f2d461ebe7dac51261c941c8e` |
| `math.html` | *MATH - The mathematical typesetting table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/math` | 100971 | `1df0fecb24050446d2ed00ac48b6a407ff2c89c45cfecf847bb3d92e168057ef` |
| `maxp.html` | *maxp - Maximum Profile table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/maxp` | 50921 | `b0f8c6d1d0318b63b164b1fdc73d73f4d72b04d02ad0dbbb29caf2df376486b6` |
| `merg.html` | *MERG — Merge Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/merg` | 63926 | `63e016b1b47d90d865ba08a1889282b3c4dba639a5adaa93c538fa6f638b1a86` |
| `meta.html` | *meta — Meta table (OpenType 1.9.1)*, updated 2025-03-20 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/meta` | 64236 | `d204cdcd94cf89a13017bc5ea6beecdbd01637464623113b9bfc3dbd4c3f16e7` |
| `mvar.html` | *MVAR — Metrics Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/mvar` | 65033 | `12d5aa600e3563464e7e4607065917db11f2011d68a3d829d9dea75b2fdab933` |
| `name.html` | *name - Naming table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/name` | 88912 | `c96f4720346e577c64d7d8c6f37702068fc6e0b6ca30119830637f4ab681346d` |
| `namesmp.html` | *Name string examples (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/namesmp` | 51470 | `eebc734135af71ebb741803b0ac983df30edddbacbf1eb2a874ec80f855db416` |
| `os2.html` | *OS/2 - OS/2 and Windows metrics table (OpenType 1.9.1)*, updated 2025-03-20 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/os2` | 160571 | `6b46c8d487bb4d926d5065aab8a33feb3b666fa01ae7275da47dbbb61f56cbdb` |
| `ibmfc.html` | *IBM Font Family Classifications (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ibmfc` | 82135 | `c9f552ad0bae19171b35783b8eb063645cd804bf5b511c0478f07749d9be27cc` |
| `pclt.html` | *PCLT - PCL 5 Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/pclt` | 62787 | `b807fc605d73d4a3fff9b168190665419a855a2d6159d9013e9fcb7fa2d713a4` |
| `post.html` | *post — PostScript Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/post` | 59395 | `892f716f6c5e4d77e6834d29b361bd5727ac3d2fe26a5745e197a9ef78c0edfd` |
| `prep.html` | *prep - Control value program table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/prep` | 49429 | `a73244146af0de1cea8c4209c4dd95592fa3de4d81a87a69a3d0b12ed3fda718` |
| `sbix.html` | *sbix - Standard Bitmap Graphics Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/sbix` | 56478 | `cb1143eb86c1c30c77c15c03c3d50b20efd0c5b5c38f6439549122deba74d4f2` |
| `stat.html` | *STAT — Style Attributes Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/stat` | 98142 | `c79cdbdaacd20d41e7d8af766e75f149c1b37ce230b51d203fd425e1ff650ab2` |
| `svg.html` | *SVG - Scalable vector graphics table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/svg` | 95683 | `a550f44c60587d04ea74c26d5b47314e676fddf675853c0122afcdb5aa717bf8` |
| `vdmx.html` | *VDMX - Vertical Device Metrics (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/vdmx` | 55077 | `c679f605f83e7a7317755e9b4a41f9e14b9e1e2438ae2e38613ed2187d6e6a3c` |
| `vhea.html` | *vhea — Vertical header table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/vhea` | 59866 | `3f9fb0d58e13d0cb8eb467757f3743af207988ab4bce8f0bfb50ccf78e6dd4f6` |
| `vmtx.html` | *vmtx — Vertical metrics table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/vmtx` | 53317 | `949d1bf0b264951af0f7d8444149654aa7c799b0b20eb549f804fd64405bb79a` |
| `vorg.html` | *VORG — Vertical origin table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/vorg` | 52643 | `e3444d29d71eee85e6695c9b614d4aa4e040c54d4aba7d1b37e971742b2b7250` |
| `vvar.html` | *VVAR — Vertical Metrics Variations Table (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/vvar` | 54566 | `9c2a648385a6fa279fa4665f109008828868e4b8920aaaa0fcfac382ad9446d5` |
| `ttoreg.html` | *OpenType Layout tag registry (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ttoreg` | 49431 | `6f612b7d7c1e726c79fbf90bfd5fd6b4a607852c35f5c28930dd8ffae15112af` |
| `scripttags.html` | *Script tags (OpenType 1.9.1)*, updated 2025-09-04 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/scripttags` | 60603 | `bbf0c92f4d6f553bcdb405a7d5f088fae5e523891d45a9287a955c2ba6569d67` |
| `languagetags.html` | *Language system tags (OpenType 1.9.1)*, updated 2024-12-06 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/languagetags` | 96344 | `108a24348189eced4190ef7a2e1290beecd5757b3426a7c94186854206b1556a` |
| `featuretags.html` | *Feature tags (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/featuretags` | 53267 | `02e8c09cb8a14fd0c10f3ca3210ab77623518e614d258e8317f6a610ee027f90` |
| `features_ae.html` | *Registered features, a-e (OpenType 1.9.1)*, updated 2025-03-17 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/features_ae` | 115281 | `fe66c3c07a06be79dd38b1eefd33b5989959091181fc26f96f11adf97b0070f0` |
| `features_fj.html` | *Registered features, f-j (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/features_fj` | 99239 | `6d2e0f55efa7f8f4305c425c56ffd822f7e943e8818a62023512a2d02fae8431` |
| `features_ko.html` | *Registered features, k-o (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/features_ko` | 93548 | `69bb70eb961b735d4482d97a95aa9c85d472bfc3be569cf4b663841fe7e17ca2` |
| `features_pt.html` | *Registered features, p-t (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/features_pt` | 135413 | `65bca7420f494079d80907e1feb869225dd0646e06c2b8b01962e841ec463251` |
| `features_uz.html` | *Registered features, u-z (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/features_uz` | 88558 | `a54055d431bc308184922b28d5b9eb7bf523c61655dc86289d953c92dd97c66f` |
| `baselinetags.html` | *Baseline tags (OpenType 1.9.1)*, updated 2024-07-07 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/baselinetags` | 63317 | `a34d051809aabf97d9a4101a095375e9c0ea4db62605865d991035c5e5dd75f5` |
| `dvaraxisreg.html` | *OpenType Design-Variation Axis Tag Registry (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxisreg` | 59136 | `e6ffe14aa20704d2dd71f744481deadca6a6db5f5c952863bced8aa3cde521e1` |
| `dvaraxistag_ital.html` | *ital design-variation axis tag (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_ital` | 49665 | `2729b154be7ad995c4adf15de23e343931dd4cf9665329ba7c5683215c8e53ea` |
| `dvaraxistag_opsz.html` | *opsz design-variation axis tag (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_opsz` | 53329 | `113a044354337ba9a411b6aac62193679f17f41857f716c0d9fafcc170f6b4d6` |
| `dvaraxistag_slnt.html` | *slnt design-variation axis tag (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_slnt` | 50313 | `af8273701f787a23018aabb15150aa591188a426f237415f7374affdf4378e10` |
| `dvaraxistag_wdth.html` | *wdth design-variation axis tag (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_wdth` | 52166 | `c246af4647466f2298613de438217dbe18ff2b13b092c73371a4d05349bc9fe1` |
| `dvaraxistag_wght.html` | *wght design-variation axis tag (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_wght` | 50197 | `a1e9001931bd55eb5e7c9aa11780f4ec576ae4109b456936c64f0380f7fe9a77` |
| `errata.html` | *OpenType 1.9.1 errata*, updated 2025-03-20 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/errata` | 51753 | `96a31970354dd4b00d5a0d2a0c2b758689df44b6fbbccb2abc91a2f2f41df002` |
| `recom.html` | *Recommendations for OpenType Fonts (OpenType 1.9.1)*, updated 2026-08-17 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/recom` | 89983 | `91511955d491e4049c2ec93cadd22402354aa929c7c46c4122d47ccc12096ff1` |
| `ttch01.html` | *TrueType fundamentals (OpenType 1.9.1)*, updated 2026-08-17 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ttch01` | 90417 | `78f792a0447ba825f2edd1111e27e2bb7cc913a48a87fc7e3516ebb5ca46d572` |
| `tt_instructing_glyphs.html` | *Instructing TrueType Glyphs (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/tt_instructing_glyphs` | 77163 | `a1f68a6787f3efda2dc190809d3990342f7b9dbab77a805f8e58e58de1c1d783` |
| `tt_instructions.html` | *TrueType Instruction Set (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/tt_instructions` | 234195 | `cc960aa6ec8d52e59d894146929638048f99e0ca916a46595218c8f9a1cff17e` |
| `tt_graphics_state.html` | *Graphics State Summary (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/tt_graphics_state` | 52086 | `0b37b4cb359bf535855e12bdf653221d185a67276a98d14ec0429c4d9b2cd2de` |
| `ompl.html` | *OpenType Mirroring Pairs List (OMPL) (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/ompl` | 64599 | `c0c946b79dd465077add69d2f9e57907593966c6358d2c5b4df2ff635614babd` |
| `glyphformatcomparison.html` | *Comparison of 'glyf', 'CFF ' and CFF2 tables  (OpenType 1.9.1)*, updated 2024-05-31 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/glyphformatcomparison` | 52892 | `6ea6a703d2ee2edf2a6679c088ab94c7e4314f88ec5cb7a7742eb278e5ac36a4` |
| `changes.html` | *OpenType change log (OpenType 1.9.1)*, updated 2025-09-04 | 2026-09-16 from `https://learn.microsoft.com/en-us/typography/opentype/spec/changes` | 240662 | `140d45c2282d65750010b031bad9f3dd12900c7048e5994a6ff3bc4a19cd8f8d` |

The checksums are here so that a reader can tell a file has not been
edited since. Each file is byte for byte what the server delivered; every
one was fetched twice, by the script below, and the two fetches agreed.
The `updated_at` date in each row is the one the page carries for itself
and says which text was read; the retrieval date says when it was taken.

### The figure files

399 files sit below `images/`, at the relative paths the pages name them
by: the figures of the layout chapters, the diagrams of the variations
chapters, and the 195 inset images of the TrueType instruction set, which
are the operand stack drawings that page is made of. They are listed one
per line with its own checksum by `fetch.sh`, and not in a table here,
because 399 rows of `ttinst_inset_p2_47.png` would bury the 84 documents
above without telling a reader anything. What pins them is one digest
over that listing:

```sh
LC_ALL=C find images -type f | LC_ALL=C sort | xargs shasum -a 256 |
	shasum -a 256
```

answers
`da06bf4c1fb3df1be2110716e28dab944cfd7c7225e890af23bfc10c717562ad`.

## How the files were fetched

`sh fetch.sh`, which writes every page of the table above and every
figure file, and prints `<sha256>  <path>` for each. Comparing that
output against the table is how a reader checks that this directory is
what it says it is. The page list is in the script, in the order the
specification's own table of contents gives.

## What the pages defer to

`cff.html` and `cff2.html` state that a CFF table is structured according
to Adobe Technical Note #5176, *The Compact Font Format Specification*,
and Adobe Technical Note #5177, *Type 2 Charstring Format*, and defer to
#5176 for the INDEX, DICT and FontSet structures they name without
defining; the index page names #5902 for the PostScript name of a
variable font instance. All three are in
[`docs/adobe/`](../adobe/README.md), which is where they belong under the
rule that one directory holds one publishing body (D-155).

## Why it is here

Nothing in the workspace reads a font file. `gfx` draws with the
project's own bitmap font of 95 glyphs, one per printable ASCII
character, each eight pixels by sixteen. The specification is kept ahead
of that work, for the reason D-59 gives: an offset a test or a comment
cites must be readable from the repository at the wording that was read.

The pages that matter first are the ones a reader of a font file needs
before anything else: `otff.html`, which is the file header and the table
directory every other table is found through, and `cmap.html`, `head.html`,
`hhea.html`, `hmtx.html`, `maxp.html`, `loca.html` and `glyf.html`, which
are the tables a TrueType outline is drawn from.

## Terms

These documents are not covered by this repository's licence. Each page
carries Microsoft's notice, © Microsoft 2026, and points at the
Microsoft Terms of Use, which state the restriction:

> Unless otherwise specified, the Services are for your personal and
> non-commercial use. You may not modify, copy, distribute, transmit,
> display, perform, reproduce, publish, license, create derivative works
> from, transfer, or sell any information, software, products or services
> obtained from the Services.

The same terms grant a narrower permission for documents, and it does not
reach a public repository either:

> Permission to use Documents (such as white papers, press releases,
> datasheets and FAQs) from the Services is granted, provided that (1) the
> below copyright notice appears in all copies and that both the copyright
> notice and this permission notice appear, (2) unless explicitly covered
> by another license or agreement, use of such Documents from the Services
> is for informational and non-commercial or personal use only and will
> not be copied or posted on any network computer or broadcast in any
> media, and (3) no modifications of any Documents are made.

The copies here are unmodified and the use is informational and
non-commercial, which is two of the three conditions; the third, *not
copied or posted on any network computer*, a repository does not meet. So
these copies stand where the ITU documents in
[`docs/itu/`](../itu/README.md) and the UEFI documents in
[`docs/uefi/`](../uefi/README.md) stand, under the second case of D-124
and for the same reason: the copy is what D-59 is for, a clause readable
without a network at wording that cannot change under what cites it, and
no arrangement that respects the restriction delivers it. What follows is
stated rather than assumed — the files are unmodified so that their
notices travel inside them, nothing is republished from this repository,
and a copy goes if Microsoft objects.

OpenType is a trademark of Microsoft Corporation.

Software written from these pages is a separate matter. Nothing is
transcribed from them yet; when something is, the page and the heading
are named at the point of transcription, as D-40 requires.
