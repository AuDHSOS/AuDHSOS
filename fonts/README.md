# fonts

The fonts of the system: two chains of fallback, fetched byte for byte by
`fetch.sh` from the commit or release each of its lines names, and pinned
in `SHA256SUMS`. Every file is unmodified. D-158 records the directory,
the chains and the rules of selection.

Nothing reads this directory yet. What will is the image build, the way
`cargo xtask image` reads [`anchors/`](../anchors/README.md) (D-148).

## Definitions

| Term | Meaning |
|------|---------|
| chain | An ordered list of fonts. A code point is drawn by the first font of the chain whose `cmap` maps it. |
| layer | One position of a chain. |
| variable font | One file carrying an axis (`wght`, `wdth`) instead of one file per weight; the name of the file carries the axes in brackets. |
| collection | A `.ttc` file: several fonts sharing tables. Both CJK files are collections of five regional fonts over one `glyf` table. |
| maps | The count of code points a font's `cmap` maps to a glyph, measured with fontTools over the files here. |
| adds | The count of code points a layer maps that no earlier layer of its chain maps. |

## The UI chain

| Layer | Font | File | Maps | Adds |
|-------|------|------|------|------|
| 1 | Atkinson Hyperlegible Next | `atkinson/AtkinsonHyperlegibleNext[wght].ttf` | 362 | 362 |
| 2 | Noto Sans | `noto/NotoSans[wdth,wght].ttf` | 3094 | 2746 |
| 3 | Noto Sans CJK, five regions | `noto/cjk/NotoSansCJK-VF.ttf.ttc` | 44810 | 44268 |
| 4 | one Noto font per script, 155 scripts | `noto/scripts/`, 157 files | 27279 | 26628 |
| 5 | Noto Sans Symbols, Symbols 2, Math, Music, Znamenny | `noto/symbols/`, 5 files | 6910 | 5675 |
| 6 | Noto Color Emoji | `noto/emoji/NotoColorEmoji.ttf` | 1501 | 1030 |
| 7 | Last Resort | `last-resort/LastResort-Regular.ttf` | 1114112 | 1033403 |

Layer 1 maps four Greek letters (U+0394, U+03A9, U+03BC, U+03C0) and no
Cyrillic. Layer 2 maps 121 code points of the Greek block and 304 of
Cyrillic and Cyrillic Supplement, so layer 2 is the font of every Greek
and Cyrillic letter. The Braille Institute's own served build,
`AtkinsonHyperlegibleNextVF-Variable.woff2`, maps 347 code points with the
same four Greek letters and no Cyrillic, so the coverage is the family's
and not the build's. Layer 1 maps 14 code points layer 2 does not:
twelve mathematical operators, `◊` and `♪`, each of which layer 5 maps
as well.

Layer 7 maps every code point of every plane to a glyph naming its block,
so a chain that reaches it draws no empty box.

## The terminal chain

| Layer | Font | File | Maps | Adds |
|-------|------|------|------|------|
| 1 | Atkinson Hyperlegible Mono | `atkinson/AtkinsonHyperlegibleMono[wght].ttf` | 359 | 359 |
| 2 | Noto Sans Mono | `noto/NotoSansMono[wdth,wght].ttf` | 3490 | 3136 |
| 3 | Noto Sans Mono CJK, five regions | `noto/cjk/NotoSansMonoCJK-VF.ttf.ttc` | 44810 | 44016 |
| 4 to 7 | the UI chain from its layer 4 | | | 26753 at layer 4 |

Advance widths, in units of an em of 1000, measured over the glyphs with a
nonzero advance:

| Layer | Advances | Grid |
|-------|----------|------|
| 1 | 632 for all 352 glyphs | one cell |
| 2 | 600 for 3340 glyphs, 1200 for 243, 1800 for 9 | one, two or three cells |
| 3 | 1000 for 51304 glyphs, 920 for 12744, 500 for 185, and 163 other widths over 548 glyphs | one cell for Han and kana, half a cell for ASCII; Hangul at 920 and the remainder do not fit the grid |
| 4 to 7 | proportional | none |

## What is here

The 22 files outside `noto/scripts/`, with the source each was fetched
from. The retrieval date of every file is 2026-09-17.

| File | Font, version | Source | Bytes | SHA-256 |
|------|---------------|--------|-------|---------|
| `atkinson/AtkinsonHyperlegibleNext[wght].ttf` | Atkinson Hyperlegible Next 2.001, `wght` 200–800 | `googlefonts/atkinson-hyperlegible-next` at `7925f50f`, `fonts/variable/` | 114552 | `5a455d1cfa099b601ab70751bb9673e8fe1854dc4500c80e1a220d0d75e31745` |
| `atkinson/AtkinsonHyperlegibleNext-Italic[wght].ttf` | the italic of the same | same | 123916 | `ce9cffed32742ad2d9238c561a93220385e5934cdc02b8eb4097a50efa957dc6` |
| `atkinson/AtkinsonHyperlegibleNext-OFL.txt` | its licence | same, `OFL.txt` | 4431 | `aca6a428580965d2297d1b718042dd427c2a9443ece3b0d02d758e161e0c4030` |
| `atkinson/AtkinsonHyperlegibleMono[wght].ttf` | Atkinson Hyperlegible Mono 2.001, `wght` 200–800 | `googlefonts/atkinson-hyperlegible-next-mono` at `154d5036`, `fonts/variable/` | 53960 | `5ce8b1698d1ded7dff2178c1a3ad159470085a58ea239e8b2cb88f4fb4a6f646` |
| `atkinson/AtkinsonHyperlegibleMono-Italic[wght].ttf` | the italic of the same | same | 56364 | `e18f523877530e4abe229df6a36d394326254da488a7f5abb9b7b5ea8f780cd4` |
| `atkinson/AtkinsonHyperlegibleMono-OFL.txt` | its licence | same, `OFL.txt` | 4436 | `1ebb31cf7393164f20d10c1d48406cddb5314feff8465531cf1e4ba37e9dd740` |
| `noto/NotoSans[wdth,wght].ttf` | Noto Sans 2.015, `wght` and `wdth` | `notofonts/notofonts.github.io` at `53486abe`, `fonts/NotoSans/full/variable-ttf/` | 2049764 | `ae80a5e79e4afbf493e9f4684ec4988e6a861b3598102ee48950b4b7a754cdff` |
| `noto/NotoSans-Italic[wdth,wght].ttf` | the italic of the same | same | 2323744 | `8bcb0ae904c973257ec957ca92559d425469d92e0913d489fe80fbb49c3b6ded` |
| `noto/NotoSansMono[wdth,wght].ttf` | Noto Sans Mono 2.014, `wght` and `wdth` | same commit, `fonts/NotoSansMono/unhinted/variable/` | 1710808 | `8b8b934f600dbd1da306eb4018e7406a4510cccd011a686c16680bcaeacb6617` |
| `noto/OFL.txt` | the licence of the Noto fonts | `notofonts/latin-greek-cyrillic` at tag `NotoSans-v2.015` | 4396 | `cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a` |
| `noto/cjk/NotoSansCJK-VF.ttf.ttc` | Noto Sans CJK 2.004, JP KR SC TC HK, `wght` | `notofonts/noto-cjk` at tag `Sans2.004`, `Sans/Variable/OTC/` | 38089916 | `2abbfc7ff74a086cf2c1f5be6130528791cfd2c8b83466db5a1463aa63252ca7` |
| `noto/cjk/NotoSansMonoCJK-VF.ttf.ttc` | Noto Sans Mono CJK 2.004, the same five | same | 37014640 | `b861b923e105a437f30ce12573350e899ee75766c4a6e9eef6d46788fb839e76` |
| `noto/cjk/LICENSE` | its licence | same tag, `LICENSE` | 4301 | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| `noto/symbols/NotoSansSymbols[wght].ttf` | Noto Sans Symbols 2.003, `wght` | `notofonts.github.io` at `53486abe`, `fonts/NotoSansSymbols/full/variable-ttf/` | 372684 | `e31a1468c4a76ba9b8fd15adeff108b5c115835e79590e1f9cc5fda5d375654b` |
| `noto/symbols/NotoSansSymbols2-Regular.ttf` | Noto Sans Symbols 2 2.008 | same commit, `fonts/NotoSansSymbols2/unhinted/ttf/` | 671568 | `c4a0a80f0041ce4be81e2478faad22776d23edb98ae3f0d19bd37044820ecf9d` |
| `noto/symbols/NotoSansMath-Regular.ttf` | Noto Sans Math 3.000 | same commit, `fonts/NotoSansMath/unhinted/ttf/` | 657440 | `b127e84699212b6b2ef50aff58e0ebebeec04ffe6db1b9eb9e209c8c3d97b4aa` |
| `noto/symbols/NotoMusic-Regular.ttf` | Noto Music 2.003 | same commit, `fonts/NotoMusic/unhinted/ttf/` | 82260 | `0dc5a0e2f2d6cde113607b60141bde4a80966901b4948f5b42363052bef7b06c` |
| `noto/symbols/NotoZnamennyMusicalNotation-Regular.ttf` | Noto Znamenny Musical Notation 1.003 | same commit, `fonts/NotoZnamennyMusicalNotation/unhinted/ttf/` | 45376 | `b6ed2a11d2a653e14137a35e4c6fdaf5093434ac12964791ee195270828e7508` |
| `noto/emoji/NotoColorEmoji.ttf` | Noto Color Emoji 2.051 | `googlefonts/noto-emoji` at tag `v2.051`, `fonts/` | 10673480 | `72a635cb3d2f3524c51620cdde406b217204e8a6a06c6a096ff8ed4b5fd6e27b` |
| `noto/emoji/LICENSE` | its licence | same tag, `fonts/LICENSE` | 4301 | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| `last-resort/LastResort-Regular.ttf` | Last Resort 18.000 for Unicode 18.0.0 | `unicode-org/last-resort-font` release `18.000` | 9592228 | `ca7df8948cec84240f19508a17a74de037c98ba3a54e1aaa50ea6edbbdc37f64` |
| `last-resort/LICENSE` | its licence | same tag, `LICENSE` | 4335 | `fc8fc512b27846bdb0d6645bed8069ac87ea599548b58a8d04bc3b0ea705a9c4` |

Every file of `noto/scripts/` has its own row in `SHA256SUMS`. One digest
over those 157 rows, sorted by path, stands in for a table here:

```sh
grep 'noto/scripts/' SHA256SUMS | sort -k2 | shasum -a 256
```

prints `31a9a98d77fccf3446e43ff470ea501f6ba175daddeaadd43ed9127e544b8041`.

All 157 were fetched from `notofonts/notofonts.github.io` at commit
`53486abe78fc4d44acde82d4b2d6e902f298e016` of 2026-09-14, under
`fonts/<Family>/full/variable-ttf/` for a variable font and
`fonts/<Family>/unhinted/ttf/` for a static one. The script column is the
key of the family in `noto.json` of that site, the version the family's
release the site named at that commit.

| Script | Family | Version | Files |
|--------|--------|---------|-------|
| `adlam` | Noto Sans Adlam | v3.002 | `NotoSansAdlam[wght].ttf` |
| `ahom` | Noto Serif Ahom | v2.007 | `NotoSerifAhom-Regular.ttf` |
| `anatolian-hieroglyphs` | Noto Sans Anatolian Hieroglyphs | v2.001 | `NotoSansAnatolianHieroglyphs-Regular.ttf` |
| `arabic` | Noto Sans Arabic | v2.013 | `NotoSansArabic[wdth,wght].ttf` |
| `armenian` | Noto Sans Armenian | v2.008 | `NotoSansArmenian[wdth,wght].ttf` |
| `avestan` | Noto Sans Avestan | v2.003 | `NotoSansAvestan-Regular.ttf` |
| `balinese` | Noto Sans Balinese | v2.006 | `NotoSansBalinese[wght].ttf` |
| `bamum` | Noto Sans Bamum | v2.002 | `NotoSansBamum[wght].ttf` |
| `bassa-vah` | Noto Sans Bassa Vah | v2.002 | `NotoSansBassaVah[wght].ttf` |
| `batak` | Noto Sans Batak | v2.005 | `NotoSansBatak-Regular.ttf` |
| `bengali` | Noto Sans Bengali | v3.011 | `NotoSansBengali[wdth,wght].ttf` |
| `bhaiksuki` | Noto Sans Bhaiksuki | v2.002 | `NotoSansBhaiksuki-Regular.ttf` |
| `brahmi` | Noto Sans Brahmi | v2.004 | `NotoSansBrahmi-Regular.ttf` |
| `buginese` | Noto Sans Buginese | v2.002 | `NotoSansBuginese-Regular.ttf` |
| `buhid` | Noto Sans Buhid | v2.001 | `NotoSansBuhid-Regular.ttf` |
| `canadian-aboriginal` | Noto Sans Canadian Aboriginal | v2.004 | `NotoSansCanadianAboriginal[wght].ttf` |
| `carian` | Noto Sans Carian | v2.002 | `NotoSansCarian-Regular.ttf` |
| `caucasian-albanian` | Noto Sans Caucasian Albanian | v2.005 | `NotoSansCaucasianAlbanian-Regular.ttf` |
| `chakma` | Noto Sans Chakma | v2.003 | `NotoSansChakma-Regular.ttf` |
| `cham` | Noto Sans Cham | v2.005 | `NotoSansCham[wght].ttf` |
| `cherokee` | Noto Sans Cherokee | v2.001 | `NotoSansCherokee[wght].ttf` |
| `chorasmian` | Noto Sans Chorasmian | v1.004 | `NotoSansChorasmian-Regular.ttf` |
| `coptic` | Noto Sans Coptic | v2.004 | `NotoSansCoptic-Regular.ttf` |
| `cuneiform` | Noto Sans Cuneiform | v2.001 | `NotoSansCuneiform-Regular.ttf` |
| `cypriot` | Noto Sans Cypriot | v2.002 | `NotoSansCypriot-Regular.ttf` |
| `cypro-minoan` | Noto Sans Cypro Minoan | v1.503 | `NotoSansCyproMinoan-Regular.ttf` |
| `deseret` | Noto Sans Deseret | v2.001 | `NotoSansDeseret-Regular.ttf` |
| `devanagari` | Noto Sans Devanagari | v2.007 | `NotoSansDevanagari[wdth,wght].ttf` |
| `dives-akuru` | Noto Serif Dives Akuru | v2.000 | `NotoSerifDivesAkuru-Regular.ttf` |
| `dogra` | Noto Serif Dogra | v1.007 | `NotoSerifDogra-Regular.ttf` |
| `duployan` | Noto Sans Duployan | v3.002 | `NotoSansDuployan-Bold.ttf`, `NotoSansDuployan-Regular.ttf` |
| `egyptian-hieroglyphs` | Noto Sans Egyptian Hieroglyphs | v2.002 | `NotoSansEgyptianHieroglyphs-Regular.ttf` |
| `elbasan` | Noto Sans Elbasan | v2.004 | `NotoSansElbasan-Regular.ttf` |
| `elymaic` | Noto Sans Elymaic | v1.002 | `NotoSansElymaic-Regular.ttf` |
| `ethiopic` | Noto Sans Ethiopic | v2.102 | `NotoSansEthiopic[wdth,wght].ttf` |
| `georgian` | Noto Sans Georgian | v2.005 | `NotoSansGeorgian[wdth,wght].ttf` |
| `glagolitic` | Noto Sans Glagolitic | v2.004 | `NotoSansGlagolitic-Regular.ttf` |
| `gothic` | Noto Sans Gothic | v2.001 | `NotoSansGothic-Regular.ttf` |
| `grantha` | Noto Sans Grantha | v2.005 | `NotoSansGrantha-Regular.ttf` |
| `gujarati` | Noto Sans Gujarati | v2.106 | `NotoSansGujarati[wdth,wght].ttf` |
| `gunjala-gondi` | Noto Sans Gunjala Gondi | v1.004 | `NotoSansGunjalaGondi[wght].ttf` |
| `gurmukhi` | Noto Sans Gurmukhi | v2.004 | `NotoSansGurmukhi[wdth,wght].ttf` |
| `hanifi-rohingya` | Noto Sans Hanifi Rohingya | v2.102 | `NotoSansHanifiRohingya[wght].ttf` |
| `hanunoo` | Noto Sans Hanunoo | v2.004 | `NotoSansHanunoo-Regular.ttf` |
| `hatran` | Noto Sans Hatran | v2.001 | `NotoSansHatran-Regular.ttf` |
| `hebrew` | Noto Sans Hebrew | v3.001 | `NotoSansHebrew[wdth,wght].ttf` |
| `imperial-aramaic` | Noto Sans Imperial Aramaic | v2.002 | `NotoSansImperialAramaic-Regular.ttf` |
| `indic-siyaq-numbers` | Noto Sans Indic Siyaq Numbers | v2.002 | `NotoSansIndicSiyaqNumbers-Regular.ttf` |
| `inscriptional-pahlavi` | Noto Sans Inscriptional Pahlavi | v2.004 | `NotoSansInscriptionalPahlavi-Regular.ttf` |
| `inscriptional-parthian` | Noto Sans Inscriptional Parthian | v2.004 | `NotoSansInscriptionalParthian-Regular.ttf` |
| `javanese` | Noto Sans Javanese | v2.005 | `NotoSansJavanese[wght].ttf` |
| `kaithi` | Noto Sans Kaithi | v2.006 | `NotoSansKaithi-Regular.ttf` |
| `kannada` | Noto Sans Kannada | v2.006 | `NotoSansKannada[wdth,wght].ttf` |
| `kawi` | Noto Sans Kawi | v1.000 | `NotoSansKawi[wght].ttf` |
| `kayah-li` | Noto Sans Kayah Li | v2.002 | `NotoSansKayahLi[wght].ttf` |
| `kharoshthi` | Noto Sans Kharoshthi | v2.004 | `NotoSansKharoshthi-Regular.ttf` |
| `khitan-small-script` | Noto Serif Khitan Small Script | v1.000 | `NotoSerifKhitanSmallScript-Regular.ttf` |
| `khmer` | Noto Sans Khmer | v2.004 | `NotoSansKhmer[wdth,wght].ttf` |
| `khojki` | Noto Sans Khojki | v2.005 | `NotoSansKhojki-Regular.ttf` |
| `khudawadi` | Noto Sans Khudawadi | v2.004 | `NotoSansKhudawadi-Regular.ttf` |
| `lao` | Noto Sans Lao | v2.003 | `NotoSansLao[wdth,wght].ttf` |
| `lepcha` | Noto Sans Lepcha | v2.006 | `NotoSansLepcha-Regular.ttf` |
| `limbu` | Noto Sans Limbu | v2.005 | `NotoSansLimbu-Regular.ttf` |
| `linear-a` | Noto Sans Linear A | v2.002 | `NotoSansLinearA-Regular.ttf` |
| `linear-b` | Noto Sans Linear B | v2.002 | `NotoSansLinearB-Regular.ttf` |
| `lisu` | Noto Sans Lisu | v2.102 | `NotoSansLisu[wght].ttf` |
| `lycian` | Noto Sans Lycian | v2.002 | `NotoSansLycian-Regular.ttf` |
| `lydian` | Noto Sans Lydian | v2.002 | `NotoSansLydian-Regular.ttf` |
| `mahajani` | Noto Sans Mahajani | v2.003 | `NotoSansMahajani-Regular.ttf` |
| `makasar` | Noto Serif Makasar | v1.001 | `NotoSerifMakasar-Regular.ttf` |
| `malayalam` | Noto Sans Malayalam | v2.104 | `NotoSansMalayalam[wdth,wght].ttf` |
| `mandaic` | Noto Sans Mandaic | v2.003 | `NotoSansMandaic-Regular.ttf` |
| `manichaean` | Noto Sans Manichaean | v2.005 | `NotoSansManichaean-Regular.ttf` |
| `marchen` | Noto Sans Marchen | v2.004 | `NotoSansMarchen-Regular.ttf` |
| `masaram-gondi` | Noto Sans Masaram Gondi | v1.005 | `NotoSansMasaramGondi-Regular.ttf` |
| `mayan-numerals` | Noto Sans Mayan Numerals | v2.001 | `NotoSansMayanNumerals-Regular.ttf` |
| `medefaidrin` | Noto Sans Medefaidrin | v1.002 | `NotoSansMedefaidrin[wght].ttf` |
| `meetei-mayek` | Noto Sans Meetei Mayek | v2.002 | `NotoSansMeeteiMayek[wght].ttf` |
| `mende-kikakui` | Noto Sans Mende Kikakui | v2.003 | `NotoSansMendeKikakui-Regular.ttf` |
| `meroitic` | Noto Sans Meroitic | v2.002 | `NotoSansMeroitic-Regular.ttf` |
| `miao` | Noto Sans Miao | v2.004 | `NotoSansMiao-Regular.ttf` |
| `modi` | Noto Sans Modi | v2.004 | `NotoSansModi-Regular.ttf` |
| `mongolian` | Noto Sans Mongolian | v3.002 | `NotoSansMongolian-Regular.ttf` |
| `mro` | Noto Sans Mro | v2.001 | `NotoSansMro-Regular.ttf` |
| `multani` | Noto Sans Multani | v2.002 | `NotoSansMultani-Regular.ttf` |
| `myanmar` | Noto Sans Myanmar | v2.107 | `NotoSansMyanmar[wdth,wght].ttf` |
| `nabataean` | Noto Sans Nabataean | v2.001 | `NotoSansNabataean-Regular.ttf` |
| `nag-mundari` | Noto Sans Nag Mundari | v1.001 | `NotoSansNagMundari[wght].ttf` |
| `nandinagari` | Noto Sans Nandinagari | v1.003 | `NotoSansNandinagari-Regular.ttf` |
| `new-tai-lue` | Noto Sans New Tai Lue | v2.004 | `NotoSansNewTaiLue[wght].ttf` |
| `newa` | Noto Sans Newa | v2.007 | `NotoSansNewa-Regular.ttf` |
| `nko` | Noto Sans NKo | v2.004 | `NotoSansNKo-Regular.ttf` |
| `nushu` | Noto Sans Nushu | v1.003 | `NotoSansNushu-Regular.ttf` |
| `nyiakeng-puachue-hmong` | Noto Serif NPHmong | v1.001 | `NotoSerifNPHmong[wght].ttf` |
| `ogham` | Noto Sans Ogham | v2.001 | `NotoSansOgham-Regular.ttf` |
| `ol-chiki` | Noto Sans Ol Chiki | v2.003 | `NotoSansOlChiki[wght].ttf` |
| `old-hungarian` | Noto Sans Old Hungarian | v2.005 | `NotoSansOldHungarian-Regular.ttf` |
| `old-italic` | Noto Sans Old Italic | v2.004 | `NotoSansOldItalic-Regular.ttf` |
| `old-north-arabian` | Noto Sans Old North Arabian | v2.001 | `NotoSansOldNorthArabian-Regular.ttf` |
| `old-permic` | Noto Sans Old Permic | v2.001 | `NotoSansOldPermic-Regular.ttf` |
| `old-persian` | Noto Sans Old Persian | v2.001 | `NotoSansOldPersian-Regular.ttf` |
| `old-sogdian` | Noto Sans Old Sogdian | v2.003 | `NotoSansOldSogdian-Regular.ttf` |
| `old-south-arabian` | Noto Sans Old South Arabian | v2.001 | `NotoSansOldSouthArabian-Regular.ttf` |
| `old-turkic` | Noto Sans Old Turkic | v2.004 | `NotoSansOldTurkic-Regular.ttf` |
| `old-uyghur` | Noto Serif Old Uyghur | v1.005 | `NotoSerifOldUyghur-Regular.ttf` |
| `oriya` | Noto Sans Oriya | v2.007 | `NotoSansOriya[wdth,wght].ttf` |
| `osage` | Noto Sans Osage | v2.002 | `NotoSansOsage-Regular.ttf` |
| `osmanya` | Noto Sans Osmanya | v2.001 | `NotoSansOsmanya-Regular.ttf` |
| `ottoman-siyaq-numbers` | Noto Serif Ottoman Siyaq | v1.006 | `NotoSerifOttomanSiyaq-Regular.ttf` |
| `pahawh-hmong` | Noto Sans Pahawh Hmong | v2.001 | `NotoSansPahawhHmong-Regular.ttf` |
| `palmyrene` | Noto Sans Palmyrene | v2.001 | `NotoSansPalmyrene-Regular.ttf` |
| `pau-cin-hau` | Noto Sans Pau Cin Hau | v2.002 | `NotoSansPauCinHau-Regular.ttf` |
| `phags-pa` | Noto Sans Phags Pa | v2.004 | `NotoSansPhagsPa-Regular.ttf` |
| `phoenician` | Noto Sans Phoenician | v2.001 | `NotoSansPhoenician-Regular.ttf` |
| `psalter-pahlavi` | Noto Sans Psalter Pahlavi | v2.003 | `NotoSansPsalterPahlavi-Regular.ttf` |
| `rejang` | Noto Sans Rejang | v2.003 | `NotoSansRejang-Regular.ttf` |
| `runic` | Noto Sans Runic | v2.002 | `NotoSansRunic-Regular.ttf` |
| `samaritan` | Noto Sans Samaritan | v2.001 | `NotoSansSamaritan-Regular.ttf` |
| `saurashtra` | Noto Sans Saurashtra | v2.002 | `NotoSansSaurashtra-Regular.ttf` |
| `sharada` | Noto Sans Sharada | v2.006 | `NotoSansSharada-Regular.ttf` |
| `shavian` | Noto Sans Shavian | v2.001 | `NotoSansShavian-Regular.ttf` |
| `siddham` | Noto Sans Siddham | v2.005 | `NotoSansSiddham-Regular.ttf` |
| `sign-writing` | Noto Sans Sign Writing | v2.005 | `NotoSansSignWriting-Regular.ttf` |
| `sinhala` | Noto Sans Sinhala | v3.000 | `NotoSansSinhala[wdth,wght].ttf` |
| `sogdian` | Noto Sans Sogdian | v2.002 | `NotoSansSogdian-Regular.ttf` |
| `sora-sompeng` | Noto Sans Sora Sompeng | v2.101 | `NotoSansSoraSompeng[wght].ttf` |
| `soyombo` | Noto Sans Soyombo | v2.001 | `NotoSansSoyombo-Regular.ttf` |
| `sundanese` | Noto Sans Sundanese | v2.005 | `NotoSansSundanese[wght].ttf` |
| `sunuwar` | Noto Sans Sunuwar | v1.000 | `NotoSansSunuwar-Regular.ttf` |
| `syloti-nagri` | Noto Sans Syloti Nagri | v2.004 | `NotoSansSylotiNagri-Regular.ttf` |
| `syriac` | Noto Sans Syriac | v3.000 | `NotoSansSyriac[wght].ttf` |
| `tagalog` | Noto Sans Tagalog | v2.002 | `NotoSansTagalog-Regular.ttf` |
| `tagbanwa` | Noto Sans Tagbanwa | v2.001 | `NotoSansTagbanwa-Regular.ttf` |
| `tai-le` | Noto Sans Tai Le | v2.002 | `NotoSansTaiLe-Regular.ttf` |
| `tai-tham` | Noto Sans Tai Tham | v2.002 | `NotoSansTaiTham[wght].ttf` |
| `tai-viet` | Noto Sans Tai Viet | v2.004 | `NotoSansTaiViet-Regular.ttf` |
| `takri` | Noto Sans Takri | v2.005 | `NotoSansTakri-Regular.ttf` |
| `tamil` | Noto Sans Tamil | v2.004 | `NotoSansTamil[wdth,wght].ttf` |
| `tamil` | Noto Sans Tamil Supplement | v2.001 | `NotoSansTamilSupplement-Regular.ttf` |
| `tangsa` | Noto Sans Tangsa | v1.506 | `NotoSansTangsa[wght].ttf` |
| `tangut` | Noto Serif Tangut | v2.170 | `NotoSerifTangut-Regular.ttf` |
| `telugu` | Noto Sans Telugu | v2.005 | `NotoSansTelugu[wdth,wght].ttf` |
| `thaana` | Noto Sans Thaana | v3.001 | `NotoSansThaana[wght].ttf` |
| `thai` | Noto Sans Thai | v2.002 | `NotoSansThai[wdth,wght].ttf` |
| `tibetan` | Noto Serif Tibetan | v2.103 | `NotoSerifTibetan[wght].ttf` |
| `tifinagh` | Noto Sans Tifinagh | v2.006 | `NotoSansTifinagh-Regular.ttf` |
| `tirhuta` | Noto Sans Tirhuta | v2.003 | `NotoSansTirhuta-Regular.ttf` |
| `toto` | Noto Serif Toto | v2.003 | `NotoSerifToto[wght].ttf` |
| `ugaritic` | Noto Sans Ugaritic | v2.001 | `NotoSansUgaritic-Regular.ttf` |
| `vai` | Noto Sans Vai | v2.001 | `NotoSansVai-Regular.ttf` |
| `vithkuqi` | Noto Sans Vithkuqi | v1.001 | `NotoSansVithkuqi[wght].ttf` |
| `wancho` | Noto Sans Wancho | v2.001 | `NotoSansWancho-Regular.ttf` |
| `warang-citi` | Noto Sans Warang Citi | v3.002 | `NotoSansWarangCiti-Regular.ttf` |
| `yezidi` | Noto Serif Yezidi | v1.001 | `NotoSerifYezidi[wght].ttf` |
| `yi` | Noto Sans Yi | v2.002 | `NotoSansYi-Regular.ttf` |
| `zanabazar-square` | Noto Sans Zanabazar Square | v2.006 | `NotoSansZanabazarSquare-Regular.ttf` |

The checksums are here so that a reader can tell a file has not been
edited since. Every file was fetched twice, by `sh fetch.sh`, which prints
`<sha256>  <path>` per file, and the two fetches agreed. `shasum -a 256 -c
SHA256SUMS`, run here, checks all 179 files.

## Rules of selection

| Rule | Option not taken | Cost of the option not taken |
|------|------------------|------------------------------|
| One family per script: `Noto Sans <Script>` when the index has it, else `Noto Serif <Script>`, else the only family. | Kufi, Naskh and Nastaliq for Arabic; Rashi for Hebrew; Serif for 22 scripts; Looped for Thai and Lao; Unjoined for Adlam and NKo; Eastern and Western for Syriac; Traditional for Nushu; the two Fangsong builds for Khitan; the eleven regional builds for Tifinagh. | Each is a second style of code points the chain already reaches. |
| The variable font when the family builds one, else the static Regular and Bold of the family's own name. | Every static instance: 36 files for Armenian, 72 for Gujarati. | Weights and widths the variable font carries in one file. |
| TrueType outlines throughout: the `.ttf.ttc` CJK collections, the CBDT emoji. | `NotoSansCJK-VF.otf.ttc` (CFF2, 32682580 bytes) and `Noto-COLRv1.ttf` (4991984 bytes). | A second outline format, CFF2, and a second colour format, COLRv1, for one rasterizer to carry. |
| Unhinted builds. | `hinted/ttf/`. | Instructions in `fpgm`, `prep` and `glyf` that a rasterizer of this system does not run. |
| An italic for Atkinson Next, Atkinson Mono and Noto Sans only. | The italics of the script fonts, where they exist. | The fonts a UI sets text in are layers 1 and 2. Noto Sans Mono builds no italic. |
| `Noto Nastaliq Urdu`, `test`, `old-hungarian-ui` skipped. | | Nastaliq is a style of Arabic; `test` is a test family; `old-hungarian-ui` lists no file. |
| Last Resort at release 18.000. | `LastResortHE-Regular.ttf`, 587864 bytes, `cmap` format 13. | The full build maps every code point through formats 4 and 12, which every other font here uses too. |

## What a parser meets

| Property | Value |
|----------|-------|
| outline table | `glyf` in every font, `CBDT` and `CBLC` bitmaps in `NotoColorEmoji.ttf`, which has no outlines |
| `cmap` subtable formats | over the 171 `.ttf` files: 4 and 12 in 114, 4 alone in 53, 0, 4 and 12 in 2, 4, 12 and 14 in Math, 12 and 14 in the emoji font; 4, 6, 12 and 14 in the two collections |
| colour | `COLR` version 0 in `NotoZnamennyMusicalNotation-Regular.ttf`; `CBDT` in the emoji font |
| variation tables | 56 variable fonts, 48 of them in `noto/scripts/`; `fvar`, `gvar`, `HVAR` and `STAT` in all 56, `avar` in 45, `MVAR` in 18 |
| collections | two, `ttcf` header, five fonts each, 65535 glyphs each, sharing `glyf` |
| largest `cmap` | Last Resort, 1114112 code points, 5776 glyphs, through format 12 groups mapping a range to one glyph |
| licence in the file | name ID 13 of every font carries the OFL notice, name ID 0 the copyright |

## Terms

Every font here is under the SIL Open Font License, Version 1.1, which
each subdirectory carries as a file and each font in its `name` table.
The licence permits copying and redistribution of the unmodified files,
and this repository modifies none. Its one condition on a copy is
attribution, which the copyright string inside each file and the source
column above meet. Its condition on a modified font, that a Reserved Font
Name is not reused, is no constraint here. The Noto CJK fonts name Adobe
as copyright holder with Reserved Font Name `Source`; the Atkinson fonts
name the Braille Institute's project authors; Last Resort names Unicode,
Inc.

These fonts are not covered by this repository's licence. Rule R8 of the
safety policy is untouched: nothing here is compiled or linked, and
`Cargo.lock` still lists only workspace members.

## Sources

| Publisher | Repository | Pinned at | Date of the pin |
|-----------|------------|-----------|-----------------|
| Braille Institute | `github.com/googlefonts/atkinson-hyperlegible-next` | commit `7925f50f649b3813257faf2f4c0b381011f434f1` | 2025-02-21 |
| Braille Institute | `github.com/googlefonts/atkinson-hyperlegible-next-mono` | commit `154d50362016cc3e873eb21d242cd0772384c8f9` | 2024-11-20 |
| Noto Project | `github.com/notofonts/notofonts.github.io` | commit `53486abe78fc4d44acde82d4b2d6e902f298e016` | 2026-09-14 |
| Noto Project | `github.com/notofonts/latin-greek-cyrillic` | tag `NotoSans-v2.015` | 2024-11-20 |
| Noto Project | `github.com/notofonts/noto-cjk` | tag `Sans2.004` | 2022-01-27 |
| Google | `github.com/googlefonts/noto-emoji` | tag `v2.051` | 2025-09-15 |
| Unicode, Inc. | `github.com/unicode-org/last-resort-font` | release `18.000` | 2026-09-16 |

The two Atkinson commits are the ones Google Fonts names in its
`METADATA.pb` for the families, and the files are byte for byte the ones
Google Fonts serves. Last Resort moves with the Unicode version D-156
tracks: 19.0.0 brings release `19.000` and a new checksum here.

Refetching everything, 139 MiB in 179 files:

```sh
sh fonts/fetch.sh
```
