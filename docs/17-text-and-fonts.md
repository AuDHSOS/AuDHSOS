# 17. Text and fonts

## 17.0 How to read this document

Each step states Status, Depends on, Size, Needs, Does, Produces, and Done
when, in that order. T1–T13 build `text-core`. Section 17.28 says where
the three steps outside this track, R1 to R3, are now specified. A step ends with its acceptance results before the next step starts.
The owner authorized continuous execution through T12 on 2026-09-17 (D-163). Status describes implemented behavior, not the target API.

## 17.1 Terms

| Term | Meaning |
|------|---------|
| font file | An immutable borrowed byte slice containing one sfnt or a collection. |
| sfnt | OpenType's big-endian header, table directory, and table payloads. |
| collection | A TTC/OTC file whose directories use offsets from the whole file. |
| face | One directory and the tables that directory references. |
| glyph | A face-local numbered shape, distinct from a Unicode character. |
| cluster | An extended grapheme cluster, identified by UTF-8 byte boundaries. |
| run | Adjacent clusters with one script, language, direction, and resolved face. |
| shaping | Character-to-glyph substitution and glyph positioning using OpenType. |
| advance | Fractional distance to the next glyph origin. |
| Fixed | Signed `i64` with 32 fractional bits; one unit is `1 << 32`. |
| font instance | A face plus explicit variation coordinates and scale. |
| role | `Ui` or `Mono`, selecting an ordered fallback chain. |
| generation | Caller-supplied `u64` identifying one immutable font-set snapshot. |
| colour glyph | A glyph a face draws from `COLR` and `CPAL` rather than from one outline. |
| paint graph | The acyclic graph of paint tables one `COLR` version 1 colour glyph is defined by. |
| paint stream | The flat list of paint operations `text-core` resolves a paint graph into. |
| palette | One row of `CPAL`: an ordered set of sRGB colour records. |
| clip box | The precomputed box of a colour glyph in the `ClipList`. |
| ink extents | The box of what a glyph paints, in font units, distinct from its advance. |
| subpixel position | One of the quantized horizontal glyph origins within a pixel (D-174). |
| gamma | The exponent of the transfer function the compositor owns (D-175, D-183). |
| rasterizer | `text-raster` of [document 18](18-rasterization.md), outside `text-core`. |
| glyph cache | Rasterized glyph storage owned by the drawing side (D-184). |
| compositor | Drawing-command consumer and framebuffer owner. |
| workspace | Caller-owned mutable slices for intermediate numeric records. |
| UCD | Unicode Character Database, pinned to 18.0.0 by D-156. |
| gate | A required complete test suite, with no skipped failing cases. |

## 17.2 Goal

1. The compositor and an application's toolkit obtain identical numeric
   layout from identical font bytes, style, text, width, and generation.
2. `text-core` parses outlines, resolves fonts, shapes text, and returns
   fractional geometry, lines, cluster carets, selection rectangles, and a box.
3. Host tests exercise every operation without a running AuDHSOS system.
4. A colour glyph is measured and its paint graph resolved without a
   rasterizer, so an emoji can be laid out by a program that draws nothing.
5. T1–T13 add no rasterizer, glyph cache, compositor code, file access,
   memory mapping, settings access, external crate, or C implementation.

## 17.3 What is already built

| Component | Location | Decided in |
|-----------|----------|------------|
| Bitmap text and pixel surfaces | `crates/gfx/src/font.rs:1` | `docs/09-decisions.md:39`, D-29 |
| Logic-crate safety and checked parsing rules | `docs/04-safety-policy.md:62` | `docs/09-decisions.md:15`, D-05 |
| Host test and coverage tooling | `docs/06-testing-strategy.md:8` | `docs/09-decisions.md:33`, D-23 |
| T13 colour glyphs | `crates/text-core/src/colr/mod.rs:1` | D-173, D-176 |
| T8–T12 segmentation, bidi, shaping, resolution, and layout | `crates/text-core/src/lib.rs:12` | D-160, D-162, D-164 |
| T7 Unicode properties | `crates/text-core/src/unicode/mod.rs:1` | D-160 |
| T2–T6 payload readers | `crates/text-core/src/lib.rs:8` | D-159 and D-161 |
| T1 sfnt/TTC envelope parser | `crates/text-core/src/sfnt.rs:66` | `docs/09-decisions.md:171`, D-161 |

| Reference group | Location | Decided in |
|-----------------|----------|------------|
| Unicode annexes, properties, and test files | `docs/unicode/README.md:1` | `docs/09-decisions.md:163`, D-153 and D-156 |
| OpenType 1.9.1 | `docs/microsoft/README.md:1` | `docs/09-decisions.md:164`, D-154 |
| Normative Open Font Format and CFF notes | `docs/iso/README.md:1`, `docs/adobe/README.md:1` | `docs/09-decisions.md:165`, D-155 and D-157 |

## 17.4 What is missing

| Component | Why required |
|-----------|--------------|
| R3 drawing integration | A drawing protocol and immutable font distribution are outside this track and outside document 18. |
| Gradient and compositing rasterization | Specified as steps R9 and R11 of document 18; D-176 states the price and why it is paid. |
| Gamma correction | The value is a setting, which `text-core` does not read; D-175 names its owner and D-183 states where it enters. |

## 17.5 Decision D1: ownership and allocation (D-158)

**Decision:** `text-core` is a `no_std`, `forbid(unsafe_code)` logic crate.
Font parsing borrows bytes and allocates nothing. The primary processing
operations write to caller-provided slices and report capacity exhaustion.
An optional `alloc` convenience layer owns `Layout` output and scratch
vectors through Rust's caller-installed allocator, using fallible reservation.
The `alloc` feature is enabled by default; disabling default features removes
the allocator requirement (D-164).

1. Reason: a borrowed face can be parsed once and reused in either address space.
2. Reason: shaping expansion and paragraph-length intermediate data require
   variable capacity; callers can provision slices or choose growing vectors.
3. Reason: a four-argument `layout` returning owned arrays needs storage that
   survives the call; only the convenience layer owns growing structures.

**Option not taken:** implicit allocation in the parser, which would require
an allocator before any font can be inspected.

Public API, with lifetimes elided here only for readability:

```text
Font::parse(&[u8]) -> Result<Font, FontError>     // face zero, validates file
FontCollection::parse(&[u8]) -> Result<FontCollection, FontError>
FontCollection::font(index: u32) -> Result<Font, FontError> // no revalidation
FontSet { ui: &[Font], mono: &[Font], generation: u64 }
TextStyle { role: Ui | Mono, size: Fixed, lang: Language, weight: u16 }
layout(&FontSet, &TextStyle, &str, Option<Fixed>) -> Result<Layout, TextError>
measure(&FontSet, &TextStyle, &str, Option<Fixed>) -> Result<(Fixed, Fixed), TextError>
layout_into(...same inputs..., &mut Workspace, &mut LayoutBuffers) -> Result<LayoutView, TextError>
measure_into(...same inputs..., &mut Workspace) -> Result<(Fixed, Fixed), TextError>
```

`Result` refines the requested shape to expose allocation, capacity, malformed
font, work-limit, and arithmetic failures without panics. The four-argument
functions belong to the `alloc` layer and call the same processing implementation.
Failure publishes no successful partial layout; caller scratch contents become
unspecified. An empty role chain is an error. Missing characters use glyph zero
of the base face and produce a missing-glyph flag. No system font is consulted.
`Font` remains a borrowed face; resolution attaches instance metadata to runs.

## 17.6 Decision D2: numeric contract (D-159)

**Decision:** all coordinates, advances, scales, normalized variations, and
transforms use `Fixed` Q32.32. Arithmetic widens to checked `i128`; division
and precision reduction round to nearest, ties to even, for both signs.
Exact additions retain every bit. Overflow and division by zero return errors.
Integers remain integers for identifiers, counts, indices, and Unicode properties.

1. Reason: fractional advances must survive shaping, fallback scaling, and layout.
2. Reason: one signed rounding rule avoids differing negative-transform results.
3. Reason: explicit integer evaluation order gives the same answer on each target.

**Option not taken:** floating point followed by final conversion, which makes
intermediate arithmetic part of platform behavior.

Font 16.16 and 2.14 values convert exactly. CFF decimal operands are parsed as
bounded integer ratios and rounded once. Type 2 random uses a specified constant
seed and integer sequence per glyph invocation, never time. Fixed-point square
root rounds by the same nearest-even rule. Size is positive; width is nonnegative.
T3 introduces the shared arithmetic and its signed tie/overflow vectors before
any scaled metric is exposed. The rasterizer alone rounds to pixels; D7 states
where. Hinting stays refused, so no outline is fitted to the grid. GPOS Device pixel adjustments do not
change layout; VariationIndex adjustments do.

## 17.7 Decision D3: shaping scope and Unicode (D-160)

**Decision:** this track fully shapes horizontal Latin, Greek, Cyrillic, Hebrew,
Arabic, Han, Hiragana, Katakana, and Hangul, including combining marks and the
script-specific joining, composition, and feature order those scripts need.
All other scripts receive explicitly flagged simple cmap/advance placement.
The track refuses full Indic, Southeast Asian, Tibetan, Mongolian, and other
unlisted-script shaping. Simple placement does not claim readable typography
for those scripts. Bidi and segmentation still apply to every Unicode character.

1. Reason: executing every GSUB/GPOS lookup type is insufficient to implement
   every script's syllable analysis and reordering.
2. Reason: the explicit refusal makes support testable without treating missing
   script algorithms as successful shaping.

**Option not taken:** a claim of universal shaping based on lookup coverage,
which would silently misrender scripts requiring additional processing.

Generated tables are Unicode **18.0.0**, matching the checked-in files.
T7's host generator reads UCD sources only at generation/check time, emits
sorted nonoverlapping Rust ranges, records input checksums and version, and
checks regenerated source byte for byte. Product builds use checked-in Rust;
product execution reads no standards directory. Defaults, range endpoints,
UnicodeData First/Last ranges, and surrogate/noncharacter handling are tested.
Version changes replace sources, generated tables, and conformance answers
together under D-156. Default UAX #29 and #14 suites are complete gates; dictionary
word breaking and locale-specific line-break tailoring are refused in this track.
Vertical layout, bitmap strikes, `sbix`, SVG glyph painting, and WOFF/WOFF2
decompression are refused; supported outline fonts remain usable. Colour
glyphs are no longer refused: T13 reads `COLR` and `CPAL` under D9.

## 17.8 Decision D4: validation and limits (D-161)

**Decision:** T1 validates the entire sfnt/TTC envelope before exposing any face.
Every later table reader validates its own payload before using its numbers.
T1 accepts sfnt `0x00010000` and `OTTO`, and TTC versions 1.0 and 2.0.
Legacy `true`/`typ1`, nested collections, and other signatures return errors.

1. Reason: a directory offset is an untrusted number, never a pointer or recursive instruction.
2. Reason: whole-collection validation catches references into another face's directory.
3. Reason: explicit work bounds prevent small malicious files from requesting unbounded validation.

**Option not taken:** interpreting all payloads in T1, which would couple the
envelope acceptance gate to every later step.

| Rule | T1 contract |
|------|-------------|
| Reads | Checked addition, multiplication, conversion, and slice access; big-endian byte arrays. |
| Limits | 64 faces, 256 tables per face, 4096 total table records; excess returns `LimitExceeded`. |
| Directory | Nonempty, sorted unique valid tags; complete records; four-byte aligned face offsets. |
| Search hints | Ignore `searchRange`, `entrySelector`, and `rangeShift`; derive searches from validated counts. |
| Payload | Four-byte aligned offset; checked actual length; offset plus length must fit `u32`; an empty payload may point at EOF. |
| Overlap | Reject overlapping directories, payload/header intersections, and partial payload overlap across all faces. Empty payload offsets inside metadata are refused too. |
| Sharing | Permit an identical nonempty range across different faces only with the same tag and recorded checksum; reject aliases within one face. |
| TTC signature | Version 2 has all three DSIG fields zero or a bounded, nonempty `DSIG` range at EOF disjoint from headers, directories, and payloads. No cryptographic signature verification. |
| Checksums/padding | Expose recorded checksum; do not verify checksum, gap contents, or trailing padding. These are not memory-safety guarantees. |
| Required tables | Deferred to the operation needing them; a structurally valid envelope is not a usable-font certificate. |
| Complexity | O(R² + F²) validation, O(1) extra storage, O(log T) tag lookup; R total records, F faces, T face tables. |

Source: `docs/microsoft/otff.html:905`, “Table Directory”;
`docs/microsoft/otff.html:1042`, “The Font Collection File Structure”;
`docs/microsoft/otff.html:1054`, “TTC Header”. The parser deliberately rejects
interleaved/overlapping tables that the general format can represent.
Subtable cycles belong to their owning step: composite glyphs in T4,
subroutines in T5, variation indirections in T6, contextual lookups in T10.
Those steps use explicit depth and operation limits, tested at equality and
one beyond, before their acceptance. Malformed input tests ship with T1–T6.

## 17.9 Decision D5: resolution, layout, and drawing boundary (D-162)

**Decision:** resolution segments by script, language, and bidi level while
preserving clusters. `TextStyle.lang` is an explicit language tag, including
`und`; multiple-language documents make explicit style runs. Common/Inherited
characters inherit surrounding script by a fixed documented rule in T11.
The requested role's chain is searched from the front for cluster coverage and
language suitability. Han chooses Chinese, Japanese, or Korean instances and
OpenType language systems from `lang`; equal code points need not choose equal
glyphs under different languages.

1. Reason: codepoint-only fallback cannot resolve Han regional forms.
2. Reason: a mark and base need one shaping context whenever a face covers both.
3. Reason: a generation and ordered numeric face index identify output without addresses.

**Option not taken:** host locale and font discovery, which would make the two
processes depend on hidden state.

Every resolved run carries its face index, variation coordinates, and scale.
Base ascent/descent use OS/2 typographic metrics when selected by the documented
T3 rule, otherwise hhea. Each run's scale is style size divided by that face's
units-per-em under D2; the relative fallback scale is therefore the ratio of
base to fallback units-per-em. Every run sits on one alphabetic baseline and
carries no baseline offset (D-171): `Run` has no such field, and a face whose
`BASE` table declares a different default baseline is placed on the alphabetic
baseline regardless, because no reader in this crate reads `BASE`.
`Line::baseline` stays the computed alphabetic coordinate of a line. Line
extents include scaled fallback ascent/descent. Weight selects the explicit
`wght` instance where present; other axes use font defaults, without automatic
optical-size settings.
Geometry uses `design_value.mul_div(style.size, units_per_em)` for one final
precision reduction; the rounded run scale describes the instance.

T12 shapes candidate lines, chooses legal UAX #14 breaks by shaped advances,
then reshapes at the chosen boundaries. Bidi L1/L2 reordering applies per line.
An overlong indivisible cluster remains intact and marks overflow. Carets refer
to logical cluster byte boundaries with visual affinity; selections return
possibly disjoint visual rectangles. Ligature caret data are used when present;
otherwise cluster subdivisions use deterministic fractional interpolation.
`measure` uses the same layout path and returns the identical box. Empty text
has a zero box. Output serialization writes fields explicitly in a specified
byte order; Rust padding and addresses are never compared or transmitted.

| Owner | Inputs | Outputs and responsibilities |
|-------|--------|------------------------------|
| File/settings adapter, later | Files, explicit user settings | Immutable font bytes, ordered FontSet, generation, and style. |
| `text-core`, T1–T13 | Those values, text, optional width, buffers | Glyph IDs, fixed positions/outlines, line/cluster geometry and box; a colour glyph's paint stream and ink extents (D6). |
| Rasterizer, R1 | Outlines, positions and paint streams, explicit rendering policy | Pixel coverage; hinting and rounding cannot feed back into advances. D7 states the rounding, D8 the gamma. |
| Glyph cache, R2 | Generation, face, instance, size, glyph, subpixel position, raster policy | Cached pixel data with bounded eviction and generation invalidation. Four horizontal positions per pixel (D7). |
| Toolkit/compositor integration, R3 | Same font-set snapshot and style | Measurement and drawing commands with checked generation agreement. |

## 17.10 Decision D6: colour glyphs belong to `text-core` (D-173)

**Decision:** `text-core` reads `COLR` and `CPAL`, resolves the paint graph,
and reports ink extents. The rasterizer turns a resolved paint stream into
pixels and reads no font table. Colour glyph work is step T13 of this track.

1. Reason: an application measures text without rasterizing it, and an emoji
   whose box no reader can report cannot be laid out.
2. Reason: a paint graph is untrusted input with the cycle and depth hazards
   the composite decoder of T4 and the subroutine interpreter of T5 already
   bound in this crate.
3. Reason: a colour glyph's advance and vertical origin are the base glyph's
   own, which T3 reads; a second reader would repeat T3.

**Option not taken:** the colour tables in the rasterizer, which gives the
toolkit and the compositor two readers of one byte range and makes a
measurement depend on a rasterizer the measuring program does not run.

The boundary of D1 is unchanged. The palette index and the text colour are
parameters; palette entry `0xFFFF` resolves to a named foreground, never to a
colour. `Colr::paint` writes into caller storage and allocates nothing.

| Owner | Reads | Produces |
|-------|-------|----------|
| `text-core`, T13 | `COLR`, `CPAL`, the outline tables, an explicit palette index and instance | Paint operations in font units, colour stops, the clip box, the boundedness verdict, ink extents |
| Rasterizer, R1 | That stream, an explicit rendering policy | Pixel coverage; it walks no graph and meets no cycle |

## 17.11 Decision D7: where rounding happens (D-174)

**Decision:** a horizontal glyph origin is quantized to one of **four**
subpixel positions per pixel. A vertical glyph origin is a whole pixel. An
advance stays fractional through shaping and layout and is never rounded
there. Hinting stays refused, so no outline is fitted to the grid.

1. Reason: a whole-pixel horizontal origin moves each glyph by up to half a
   pixel against the fractional advance layout reported, once per glyph, so a
   run drifts from the box `measure` returned; a quarter of a pixel bounds
   that error at an eighth of a pixel.
2. Reason: every glyph of a line shares one baseline (D-171), so a whole-pixel
   vertical origin is one rounding per line and keeps a horizontal stem on a
   pixel row.
3. Reason: four positions bound the glyph cache of R2 at four entries per
   glyph, instance and size; a free fractional origin has an unbounded key.

**Option not taken:** sixteen subpixel positions, which quadruple that cache
for a shift of a sixteenth of a pixel.

The number four is part of the R2 cache key, so changing it invalidates every
cached entry. A glyph at a subpixel position is the same outline translated;
this rounding changes no advance, no line break and no box.

## 17.12 Decision D8: gamma (D-175)

**Decision:** coverage is gamma-corrected before it becomes alpha. One value
does it for the whole system. `server-display` owns that value and reports it
to its clients through the display protocol of `user-proto`. Its default is
2.2.

1. Reason: uncorrected coverage composites light text on a dark background
   heavier than dark text on a light one, so with a switchable appearance the
   apparent stroke weight of one font at one size moves when the theme
   changes.
2. Reason: one value system-wide is what makes the text of two programs match
   on one screen.
3. Reason: a COLR gradient interpolates in linear light, which
   `docs/microsoft/cpal.html`, "Interpolation of colors", requires, so the
   compositor already holds that transfer function.

**Option not taken:** correction per program, whose cost is text that differs
between one window and the window beside it.

`text-core` cannot hold the value: it is a setting, and this crate reads no
setting (D1). It reaches the rasterizer as a parameter of R1's rendering
policy, the way the palette index reaches T13.

## 17.13 Decision D9: COLRv1 as the colour format (D-176)

**Decision:** T13 implements COLR version 1 with CPAL, and COLR version 0 as
the same table's degenerate case through the same emitter. `CBDT`/`CBLC`,
`sbix` and OpenType-SVG are refused. Layer 6 of both chains of D-169 becomes
`fonts/noto/emoji/Noto-COLRv1.ttf`.

1. Reason: this system allows fractional scaling factors, and a bitmap strike
   does not survive them.
2. Reason: version 0 is version 1 with one solid fill per clipped layer, so
   one emitter answers both and no second path can disagree with the first.

**Option not taken:** the CBDT build, whose cost is that scaling; and
`Noto-COLRv1-noflags.ttf`, which drops 26 regional indicators and ten
plane 15 code points so that flags fall to a later layer of the chain.

The price of the decision falls on the rasterizer, which now needs gradients
and the compositing and blending modes of W3C Compositing and Blending
Level 1 that it would not otherwise need. It is paid deliberately.

A chain layer whose only glyph data is a refused colour format covers no
cluster: T11 skips it and the chain moves to its next layer, so a face this
track cannot draw never yields a blank. `Colr::parse` returns `MissingTable`
for such a face.

## 17.14 The order of the steps

| Step | Status | Depends on | Size |
|------|--------|------------|------|
| T1 sfnt envelope | implemented | D1–D5 | M |
| T2 cmap | implemented | T1 acceptance | M |
| T3 metrics and Fixed | implemented | T2 | M |
| T4 glyf/loca | implemented | T3 | L |
| T5 CFF/CFF2 | implemented | T4 | XL |
| T6 variations | implemented | T5 | XL |
| T7 Unicode generation | implemented | T6 | M |
| T8 segmentation | implemented | T7 | L |
| T9 bidi | implemented | T8 | L |
| T10 shaping | implemented | T9 | XL |
| T11 resolution | implemented | T10 | M |
| T12 layout | implemented | T11 | L |
| T13 colour glyphs | implemented | T12 | XL |
| R1 rasterizer | specified as document 18 | T13 | XL |
| R2 glyph cache | specified as step R7 of document 18 | R1 | M |
| R3 integration | unscheduled, outside both documents | R2 | L |

## 17.15 T1: sfnt envelope

**Status:** implemented.
**Depends on:** decisions D1–D5 above.
**Size:** M.
**Needs:** OpenType file organization, `docs/microsoft/otff.html:905`.
**Does:** validate headers, directories, collections, all referenced ranges,
sharing, limits, and DSIG envelope through borrowed checked reads.
**Produces:** `crates/text-core`, `Font`, `FontCollection`, `FontError`, table views.
**Done when:** host tests accept synthetic sfnt and mixed-outline TTC v1/v2,
read a font fixture through host file I/O, check every truncated prefix,
reject overflow, overlaps, self-references, and attempted collection cycles,
and accept legitimate shared tables. Separate allocations produce identical
field-by-field serialized results. The crate passes strict lints, no_std build,
and parser fuzz regression; report the results before T2.

Acceptance results: 18 unit tests and one doctest pass in debug and release;
strict Clippy and `x86_64-unknown-none` checking pass. Instrumented product
coverage is 267/267 lines and 74/74 branches. `text_font` replays all seven
regression seeds. A separate host probe accepts eight installed DejaVu fonts
and verifies the expected `head`/`cmap` table presence. The checked-in fixture
provides the reproducible file-I/O test; installed fonts are additional evidence.

Repository check on 2026-09-17: `sh tools/xtask-check.sh --quiet` passes lint,
layering, dependency checks, unsafe budgets, host tests, coverage, Miri,
documentation, and QEMU tests. The existing end-to-end run fails after
`[files] sector 436 was not answered`; the reported violation is
“the network server did not start”. The full check therefore exits 1 before
its workspace fuzz-regression phase. The separate `text_font` regression
command exits 0. `text-core` has no system-image consumer in T1.

## 17.16 T2: cmap

**Status:** implemented; 28 crate tests, release and bare-target checks, and nine fuzz regression seeds pass.
**Depends on:** T1 acceptance.
**Size:** M.
**Needs:** `docs/microsoft/cmap.html:1`, formats 0, 4, 6, 12, 13, 14.
**Does:** select Unicode subtables deterministically; map scalar values and
variation-selector pairs, including default and nondefault UVS ranges.
**Produces:** bounded character-to-glyph lookup.

T2 API: `Cmap::parse(table_bytes, num_glyphs)` validates a borrowed table;
T3 supplies `num_glyphs` from maxp. Selection prefers formats 12, 13, 4, 6,
then 0; equal formats prefer platform 0, then the higher encoding ID.
Unsupported legacy encodings are not interpreted as Unicode. Parsing validates
the selected primary and optional platform 0/encoding 5 variation table;
unselected payloads are ignored. The encoding directory is limited to 64
records and format 14 to 260 selectors and 16 MiB of cumulative validation
work. Mapping is O(log N), except constant-time formats 0 and 6. Format 4
validation examines at most 65,536 character positions; other validation is
linear in selected table records. Glyph zero means missing; unsupported
variation sequences return `None`.

**Done when:** all six formats pass literal vectors and malformed count,
range, offset, sentinel, and glyph-bound tests; report acceptance before continuing.

## 17.17 T3: metrics and Fixed

**Status:** implemented; 34 crate tests and strict Clippy pass; debug/release and bare-target builds pass. Eight installed DejaVu fonts pass the host metrics/cmap probe.
**Depends on:** T2.
**Size:** M.
**Needs:** `docs/microsoft/head.html:1`, `hhea.html`, `hmtx.html`, `maxp.html`,
`os2.html`, `post.html` in the same directory.
**Does:** implement D2 arithmetic; validate units-per-em, glyph counts,
horizontal metric counts, repeated advances, and table-version lengths.
**Produces:** fractional advances and explicit ascent/descent/line-gap policy.

`Metrics::parse` requires head, hhea, hmtx, and maxp. OS/2 and post are
optional; present tables must have valid version lengths. OS/2 version 0
may use the documented 68-byte legacy form. Line metrics use OS/2 typo
values when USE_TYPO_METRICS is set and available, otherwise hhea.
Negative line gaps become zero. Metrics remain in font units until scaled
by `Fixed::mul_ratio(size, units, units_per_em)`, with one rounding.
PostScript glyph names are validated but do not affect layout.
**Done when:** exact scaled metrics, negative ties, overflow, truncated metric
arrays, and invalid count relationships pass host tests; report acceptance before continuing.

## 17.18 T4: glyf and loca

**Status:** implemented; 41 crate tests and strict Clippy pass in debug/release; bare-target builds pass. Host probes decode all 31,597 glyphs of eight DejaVu fonts.
**Depends on:** T3.
**Size:** L.
**Needs:** `docs/microsoft/glyf.html:1` and `docs/microsoft/loca.html:1`.
**Does:** decode short/long loca, simple contour flags and deltas, composite
point attachment and transforms into caller-owned outline buffers.
**Produces:** numeric quadratic contours with bounded composite traversal.

The caller supplies point and contour-end slices. The decoder returns used
lengths and four phantom points; an error invalidates the output buffers.
Traversal permits 32 active glyphs and 1024 component visits per outline.
The decoder rejects cycles before recursion and checks every output write.
Absent component-offset flags select unscaled offsets. Multiple USE_MY_METRICS
components select the last component, as required by existing DejaVu glyphs. Grid-fitting flags
and bytecode remain rasterizer inputs; this decoder retains fractional
coordinates. Optional vhea/vmtx supply vertical phantom metrics; absent
vertical metrics use the selected line ascender and descender. Work is
O(decoded points + component visits); loca validation is O(glyph count).
**Done when:** empty glyphs, repeated flags, transforms, point matching,
out-of-order offsets, cycles, self-reference, depth exhaustion, and truncated
coordinates have exact results or typed errors; report acceptance before continuing.

## 17.19 T5: CFF and CFF2

**Status:** implemented; 50 tests, strict Clippy, release/bare-target builds, and 11 fuzz seeds pass. The full Noto Sans CJK JP host probe decodes 65,535 glyphs and 4,336,282 commands; checked-in fixtures verify literal coordinates and the CFF2 worked example.
**Depends on:** T4.
**Size:** XL.
**Needs:** `docs/microsoft/cff.html:1`, `docs/microsoft/cff2.html:1`,
`docs/adobe/5176.CFF.pdf`, INDEX/DICT sections, and
`docs/adobe/5177.Type2.pdf`, charstring operators and subroutines.
**Does:** decode INDEX, DICT, charset, FDSelect/FDArray, local/global subroutines,
Type 2 widths, stacks, masks, and cubic paths; validate CFF2 blend structure.
**Produces:** numeric cubic contours; explicit instance coordinates follow in T6.

`Cff` validates INDEX arrays and all font dictionaries once; outline decoding
reuses validated immutable arrays. Limits: 65,536 INDEX objects, 256 font
dictionaries, 48 CFF or 513 CFF2 operands, ten subroutine calls, 65,536
operations per glyph. Type 2 `random` uses xorshift32 with shifts 13, 17, 5,
seed 1 per glyph invocation, and result `(state + 1) / 2^32` in Q32.32.
Determinism takes precedence over the specified randomness in Adobe #5177,
section 4.4, page 26 (`docs/adobe/5177.Type2.pdf`); D-166 fixes the sequence
across address spaces. Uninitialized transient reads are errors.
CFF2 deliberately ignores unrecognized operators and clears the operand
stack, including escaped operators, as required by “Stack-based CFF2 decoding”
(`docs/microsoft/cff2.html:1039`, D-167). Truncated encodings, invalid operands
for recognized operators, and exhausted limits remain errors.
Deprecated endchar composites use
StandardEncoding and permit one component level. CFF2 DICT/charstring blends
validate the variation store and evaluate the default instance in T5.
The caller owns the cubic-command buffer; errors invalidate its contents.
Parsing is O(INDEX entries + dictionary bytes); outline work is bounded by
the operation limit. FDSelect lookup is O(range count).
**Done when:** CFF-based Noto Sans CJK host input yields checked glyph contours;
INDEX offSize/count errors, operand overflow, invalid operators, recursive
subroutines, stack and depth limits all pass negative tests; report acceptance before continuing.

## 17.20 T6: variations

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T5.
**Size:** XL.
**Needs:** `docs/microsoft/fvar.html:1`, `avar.html`, `gvar.html`, `hvar.html`,
`mvar.html`, `otvarcommonformats.html`, and `cff2.html` in the same directory.
**Does:** normalize explicit axes, apply avar, gvar/IUP and phantom-point
deltas, HVAR/MVAR stores, and CFF2 blends in fixed point.
**Produces:** instance-specific outlines and metrics without a mutable face.

`Axes` normalizes tagged user values into caller storage; unknown/duplicate
requests are errors and out-of-range values clamp. The local avar specification
is version 1; other avar versions return an error. Limits: 64 axes, 4096 shared
regions/store subtables, 16 MiB cumulative item-store/CFF2-store validation, and 16 million
point/tuple operations per gvar glyph. gvar supports repeated packed point
numbers cumulatively and infers sparse deltas per contour before scaling.
Caller `VariationPoint` storage holds the current glyph plus component records
of active ancestors; recursive decoding partitions that buffer.
`Instance::advance` chooses HVAR when present, otherwise gvar phantom widths
for nondefault instances, otherwise hmtx. Missing required phantom data returns
`MissingOutline`. `Fixed::mul_div` scales fractional design values with one
wide division. MVAR updates the selected OS/2 typo metrics before gap clamping.
CFF2 instance coordinates are explicit; default outlines use zero coordinates.
`Glyf::outline_instance` treats an absent `fvar` as zero axes and decodes the
same outline as `outline` for an empty coordinate slice, rejecting a nonempty
one with `InvalidTable` (D-170); `Cff::outline_instance` accepts a face without
a CFF2 variation store the same way. A caller passes `Run::coordinates()`
without testing whether a face is variable.
**Done when:** defaults/endpoints/intermediate tuples match literal expected
numbers; malformed maps, stores, packed deltas, axis counts, and blend operands
return errors; no double application of advance deltas; report acceptance before continuing.

Acceptance: 62 host tests and one doctest pass in debug/release; Clippy,
`x86_64-unknown-none`, and 12 fuzz seeds pass. Fresh product coverage is
3381/3559 lines (95.00%) and 973/1114 branches (87.34%). A separate host
probe decodes all 4515 glyphs of Noto Sans at wght=900/wdth=75.
The full-system E2E failure recorded in T1 remains outside this crate.

## 17.21 T7: Unicode generation

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T6.
**Size:** M.
**Needs:** sources named in 17.3, retained with 18.0.0 checksums and license
under `docs/unicode/README.md:1`; existing property-file formats and defaults.
**Does:** implement the D3 host generator and regeneration check.
**Produces:** checked-in versioned Rust property tables, including script,
joining, paired-bracket, mirroring, segmentation, and bidi properties.
**Done when:** regeneration is byte-identical, every property range/default
agrees with the source, and ordinary builds need no standards files; report acceptance before continuing.

The host generator is `cargo run -p text-core --example unicode_gen`;
`--check` compares without writing. Host tests perform the same regeneration
check. The generator allocates dense property vectors; product lookups use
sorted static ranges in O(log R) time and O(1) storage. Generated comments
record FNV-1a-64 source fingerprints; the standards register records SHA-256.
Version headers are checked; headerless UnicodeData has a pinned fingerprint.
Runtime code exposes numeric property enums and borrowed numeric mappings.

Acceptance: all 12 properties agree with the parsed UCD at all 1,114,112
code points; Script_Extensions ranges and gaps, mirroring, brackets, and
canonical decompositions agree. First/Last and malformed source tests pass.
Regeneration, Clippy, and the bare-target build pass. Generated Rust carries
AGPL-3.0-only AND Unicode-3.0 and both copyright notices; xtask checks the exact generated path against that header.

Product coverage after T7 is 3461/3639 lines (95.11%) and 975/1116 branches (87.37%).

## 17.22 T8: segmentation

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T7.
**Size:** L.
**Needs:** `docs/unicode/reports/tr29/tr29-49.html:427`, conformance and boundary
rules; `docs/unicode/reports/tr14/tr14-57.html:845`, conformance and line-break rules.
**Does:** extended graphemes, default words, and default line opportunities.
**Produces:** UTF-8 boundary iterators and line-break classes.
**Done when:** every case of GraphemeBreakTest.txt, WordBreakTest.txt, and
LineBreakTest.txt passes, with executed case counts and no exclusions;
empty text and multibyte byte offsets also pass; report acceptance before continuing.

`grapheme_boundaries` and `word_boundaries` yield UTF-8 offsets with O(1)
state. `line_breaks` writes one `LineBoundary` per scalar plus end-of-text;
`LineUnit` scratch needs one record per scalar. All three take O(N log R)
time for N scalars and R property ranges. Empty iterators yield offset zero
once; empty line output contains one mandatory end boundary. Capacity errors
return `TextError::BufferTooSmall`. Unicode 18 GB9c uses Linker Extend* before
Consonant, as written in UAX #29 revision 49.

Acceptance: all 853 GraphemeBreakTest, 1944 WordBreakTest, and 19346
LineBreakTest cases pass, with no skipped cases. Empty input, fused iteration,
capacity failures, UTF-8 offsets, and mandatory breaks pass. Clippy, the
bare-target build, and 13 fuzz seeds pass. Product coverage is 3883/4063 lines
(95.57%) and 1341/1488 branches (90.12%).

## 17.23 T9: bidirectional algorithm

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T8.
**Size:** L.
**Needs:** `docs/unicode/reports/tr9/tr9-52.html:1`, UAX #9 including isolates,
paired brackets, explicit-level overflow, and per-line reordering.
**Does:** paragraph resolution, embedding levels, mirroring decisions, and
logical/visual mappings in caller workspace.
**Produces:** directional runs independent of glyph selection.
**Done when:** every enabled direction bit and case of BidiTest.txt and every
case of BidiCharacterTest.txt passes; paragraph splitting and line resets have
additional tests; report executed counts before continuing.

Acceptance: 770,241 enabled BidiTest direction cases and 91,707
BidiCharacterTest cases pass without exclusions. Paragraph, line-reset,
mirroring, bracket-stack and embedding-overflow vectors pass in debug/release.
Clippy, the bare target, and 13 fuzz seeds pass. Product coverage is
4487/4673 lines (96.02%) and 1515/1666 branches (90.94%).
`resolve` retains paragraph levels; `reorder_line` applies L1/L2 independently.
Both use caller buffers. Runtime is O(N(log R + 126 + 63)); storage is O(N).

## 17.24 T10: shaping

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T9.
**Size:** XL.
**Needs:** `docs/microsoft/gdef.html:1`, `gsub.html`, `gpos.html`, `chapter2.html`,
script/language/feature registries in the same directory.
**Does:** GDEF classification/filtering and ligature carets; GSUB 1–8 and
GPOS 1–9; script/LangSys selection, required/default features, variation
features, contextual application, marks, and cursive attachment. Apply D3's
script algorithms and flag every refused full-shaping script.
**Produces:** glyph sequences, cluster mapping, fractional advances and offsets.
**Done when:** each lookup format has literal result vectors; Arabic joining,
Hebrew marks, Hangul, Latin ligatures, language features, and mark/cursive
chains pass; cycles, expansion, recursion, and operation limits return errors;
report acceptance before continuing.

T10 exposes borrowed layout tables and a caller-owned glyph buffer. Glyphs
retain UTF-8 cluster ranges, joining forms, ligature components, and design-unit
Q32.32 positions. GSUB runs before instance advances are installed; GPOS runs
afterward. Each feature stage executes selected lookups in lookup-list order.
Context calls permit 16 active lookups, 64 matched input glyphs, and one million
lookup/match/feature-selection operations per application; capacity exhaustion
returns an error. Context actions index the sequence modified by preceding
actions (`docs/microsoft/gsub.html:1`, Lookup type 5). Insertions inherit active
context membership; deletions remove membership. GDEF classes override inferred
classes after substitutions and before advances.
Normalization preserves grapheme source ranges, decomposes canonically,
orders Hebrew pronunciation marks before vowels and Arabic shadda before
vowels, and recomposes only glyphs the face covers. CGJ blocks reordering.
The Hebrew order follows `docs/unicode/sbl-hebrew-manual-1.5.pdf`; script
feature orders follow `docs/microsoft/script-{arabic,hebrew,hangul}.html`.
Anchors use design coordinates; contour-point hinting and Device pixels remain
rasterizer inputs. GDEF VariationIndex deltas remain layout inputs.

Acceptance: 16 shaping tests cover every lookup format, required/language/
variation features, mark filtering, carets, anchor formats, attachments, and
context expansion/depth/work limits. Every layout-table byte in the DejaVu
fixture is mutated twice and compared across separate structures. Literal
font oracles cover all supported script groups, regional Han, archaic Hangul,
and tone marks. The default stages include legacy Arabic mset substitution.
Debug/release, Clippy, and bare-target checks pass. Product
coverage is 5883/6171 lines (95.33%) and 1926/2170 branches (88.76%).

## 17.25 T11: resolution

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T10.
**Size:** M.
**Needs:** generated script properties; `docs/microsoft/languagetags.html:1`
and `docs/unicode/reports/tr37/tr37-16.html:1`, variation sequences.
**Does:** apply D5 chain ordering, script/language runs, cluster coverage,
instance scale metadata, and missing-glyph reporting.
**Produces:** resolved runs indexed by role, face, and generation.
**Done when:** one Han code point resolves differently under zh-Hans, zh-Hant,
ja, and ko fixtures where appropriate; fallback scales and combining
clusters remain correct across both roles; report acceptance before continuing.

Resolution uses whole grapheme clusters. Common/Inherited clusters inherit the
preceding strong script; leading neutral clusters use the following strong
script, otherwise Common. Script_Extensions constrains that inheritance.
`Language` validates BCP-47 syntax, duplicate variants/extensions, and a
255-byte limit (`docs/rfc/rfc5646.txt:218`). Registry membership is unchecked.
Mapped language/script/region subtags select OpenType language tags; extensions
and private-use subtags cannot change regional Han selection. Unmapped tags
select the default language system; explicit OpenType tags remain available.
For regional Han, the first pass requires an explicit matching LangSys in GSUB
or GPOS; the second pass permits default LangSys when no such face covers the
cluster. Each pass preserves role-chain order. Coverage includes normalization
and variation-selector pairs. Bidi levels come from the whole paragraph.

Acceptance: role order, whole combining clusters, fallback scales,
regional language systems, variable weight, script inheritance, bidi levels,
UVS coverage, missing glyphs, caller capacities, and language extensions pass
six host tests.
Debug/release, strict Clippy, and the bare target pass.

## 17.26 T12: layout

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T11.
**Size:** L.
**Needs:** D5 layout rules and accepted T8–T11 algorithms.
**Does:** combine shaping and breaking; expose layout/measure and buffer forms.
**Produces:** lines, positioned glyphs, cluster carets, selections, and box.
**Done when:** measurement equals layout geometry for empty, wrapped, RTL,
mixed-language, fallback, ligature, and overlong-cluster text; separate font
allocations and workspaces serialize byte-identical output in debug/release;
capacity/work exhaustion and numeric limits return errors; report acceptance before continuing.

T12 exposes borrowed Workspace/LayoutBuffers and owned alloc wrappers.
The alloc feature is enabled by default and can be disabled for an entirely
allocation-free library. Owned vectors grow through try_reserve; no global
cache or mutable font state exists. Glyph expansion and outline scratch may
grow independently of input length; runs and Unicode scratch are linear in
input scalars. Output failure invalidates caller storage.

Greedy wrapping tests legal candidates until the first overflow, selects the
last fitting candidate, and reshapes at the chosen boundary. When no legal
UAX #14 candidate fits, layout unconditionally breaks at the last fitting
grapheme cluster boundary (D-168). Each nonempty line consumes at least one
whole cluster; an overlong first cluster remains intact, including every
glyph produced by shaping that cluster. `overflow` is set if and only if the
emitted line exceeds the requested width. No caller opt-out exists.
Emergency boundaries use the same reshaping, per-line bidi reordering, and
layout/measurement path as legal boundaries, without additional allocation.
`LineBreakTest.txt` is unaffected: segmentation still reports UAX #14
opportunities unchanged; only layout applies the fallback when none fits.
Soft-wrap trailing ASCII spaces and tabs
collapse; hard-break and final-line spaces retain advances. Tabs advance to
four base-face space widths, using em/4 when space is absent. A final hard
break creates an empty final line; an empty input retains a zero box.

Workspace stores paragraph bidi results, resolved runs, shaped candidate
glyphs, and per-line cluster geometry. L1/L2 supplies visual scalar ranks;
shaped clusters remain atomic during reordering. Layout outputs visual glyphs,
lines, and cluster boxes; cursor queries distinguish upstream/downstream
affinity, and selection queries merge adjacent visual cluster rectangles.
GDEF carets use instance coordinates, including unhinted TrueType point carets;
missing carets use fractional interpolation. Deleted glyph clusters retain
zero-width cursor intervals. Overlapping source ranges from multiple
substitution followed by ligation merge before grapheme geometry is computed.
Output serialization is explicit little-endian, versioned, and contains no
padding. The version number denotes one record layout, and a reader selects its
record layout by that number alone. Version 1, written as `TEXT\x01`, gives each
run record a `Fixed` baseline offset after its scale. Version 2, written as
`TEXT\x02`, is version 1 without that field, because a run carries no baseline
offset (D-171); every other record is unchanged. The `u16` after the magic is
the major Unicode version of the property tables, `unicode::VERSION_MAJOR`,
which the generator emits from the same pinned `VERSION` the tables are built
from (D-156). The two numbers answer different questions: the version byte
selects the record layout, and this word reports which property data produced
the result, because segmentation, bidi and line breaking are the answers of one
Unicode version and two streams of one record layout carry different boundaries
when their tables differ.

Input is capped at 65,536 Unicode scalars. Each owned growing buffer is capped
at 1,048,576 entries. No-width layout shapes only mandatory-break candidates.

Candidate work is O(Σ(Nᵢ + Gᵢ log Gᵢ + Sᵢ)), where Nᵢ counts source
scalars, Gᵢ shaped glyphs, and Sᵢ bounded font/shaping work. A one-million-scalar
candidate budget bounds rescanning. Without glyph expansion, greedy rescanning
and visual sorting take O(C² log C) worst-case time for C input clusters.
Break/run searches start at the current line using binary search. Scratch is
O(input scalars + largest shaped line + largest decoded outline).
Layout and measure execute the same line
and cluster geometry path. Limits return typed errors.

Tabs split shaping runs and occupy hidden glyph slots, preserving tab stops
across GSUB and GPOS.

Acceptance: twelve layout tests cover fractional advances, legal/emergency
breaks, line-end Arabic/ligature reshaping, bidi carets and selections,
regional fallback, GDEF coordinate/point carets, deleted clusters, gvar metrics
without HVAR, overlapping substitution clusters, buffer growth/exhaustion,
tab shaping barriers, and input/work limits. Emergency-break tests cover an
unbreakable run, an overlong first cluster, combining marks, multiple glyphs
from GSUB, and RTL visual order; every case checks progress, exact overflow,
measurement agreement, and deterministic serialization through caller buffers
and owned wrappers. One test lays out two faces of different units-per-em, one
of them carrying a `BASE` table whose default baseline is ideographic, and
asserts that every glyph's y equals `Line::baseline` and that the serialized
output is byte-identical with and without that table (D-171). Layout and
measure agree in every case. Separately allocated inputs serialize identically;
debug/release assert FNV-1a-64 `937fa8061318a96c` for the mixed-script fixture.
The complete pure stack has 110 host tests and two doctests; allocator-free
builds have 103 host tests. All Unicode gates, regeneration, Clippy, bare-target
builds, and fourteen fuzz regression seeds pass. Product coverage is
96.08% of lines and 89.58% of branches after review.

Review traced the earlier E2E disk timeouts to concurrent BAR size probing by
`app-lspci`. Read-only BAR inspection restores disk notifications during
startup (D-165); the main E2E run passes with the fix.


## 17.27 T13: colour glyphs

**Status:** implemented and accepted (2026-09-18).
**Depends on:** T12.
**Size:** XL.
**Needs:** `docs/microsoft/colr.html:1130` and `docs/microsoft/cpal.html:740`;
the item variation store of T6; the outline decoders of T4 and T5.
**Does:** validate `CPAL` and `COLR` of either version, resolve one glyph's
paint graph into a flat paint stream in font units, report the clip box, the
boundedness verdict, and ink extents.
**Produces:** `crates/text-core/src/colr`, `Colr`, `Cpal`, `PaintOp`, `Fill`,
`ColorLine`, `Affine`, `CompositeMode`, `Extents`.

`Cpal::parse` validates every palette's record range and the optional palette
types array; the two label arrays name `name` strings and are range-checked
and not read. `Cpal::color` multiplies the record's alpha by the paint
table's, clamped to `[0, 1]`; entry `0xFFFF` is the foreground.
`Cpal::palette_for` selects by light or dark background, and a table without
a types array declares no preference.

`Colr::parse_tables` validates the record arrays, the base glyph order, the
clip ranges and every glyph identifier once; each paint table is validated
when the traversal reaches it. Limits: 65,536 base glyph records, 1,048,576
`LayerList` entries, 65,536 clip records, 1,024 palettes, a path of 64
paint tables and 65,536 paint table visits per glyph.
A paint table that is its own ancestor returns `Cycle` before recursion. A
paint format this version does not define, and its sub-graph, are ignored and
count as bounded, which the format's own section requires and which D-167
already decided for the CFF2 interpreter.

`Colr::paint` writes `PaintOp` into caller storage: `Clip` and `Unclip`
bracket a clip region, `Group` and `Compose` bracket an offscreen surface.
A `Compose` consumes the two groups above it, combines them with its mode,
and draws the result onto the surface below them with source-over, which is
what the rendering algorithm of `docs/microsoft/colr.html:3332` does, so the
ink that surface already held stays under the result. `Fill` carries the
accumulated transform and either a solid colour or a gradient naming a range
of the caller's colour stop slice. Stops are written in increasing offset
order, which a variable font can change, so each is placed by binary search
into the part already written. Rotation and skew need a sine, a cosine and a
tangent; `colr::trig` computes them from integer Taylor series after an exact
reduction to a quarter turn, within 2⁻²⁸ of the real value, so D2 admits no
floating point here either.

Variation deltas are integers applied to the stored representation: a `FWORD`
takes one font unit per delta, an `F2DOT14` one unit of 2⁻¹⁴, a `Fixed` one
unit of 2⁻¹⁶. A `varIndexBase` of `0xFFFFFFFF`, a face without an item
variation store, and a non-variable format each leave the stored numbers
alone. Without a `DeltaSetIndexMap` the sequence is the delta-set index
itself, high word outer and low word inner.

`Colr::clip_box` is O(log N) over the `ClipList`. `Colr::extents` is the
union of the boxes of the outermost clipped outlines, decoded through T4 or
T5 into caller scratch; curved segments contribute their control points, so
the box can exceed the ink. `Painted::bounded` is the specification's answer:
a clip box bounds the glyph whatever its graph does, and without one the rule
of each format decides, computed during the traversal. Resolution is O(P) for P
paint table visits plus O(S²) worst case for S colour stops, a binary search
and a move of the tail per stop; S is bounded by the caller's stop slice.
Storage is the caller's two slices and nothing else.

Layout is unchanged: a colour glyph's advance and vertical origin are the
base glyph's own, which T3 already reads, so T12 needs no new field.

**Done when:** every paint format, every extend mode and every composite mode
has a literal result vector; cyclic, self-referential and overlong graphs
return typed errors; version 0 and version 1 of one table resolve through one
emitter; a real COLRv1 face's operation counts, clip boxes and gradient
geometry match numbers obtained independently; report acceptance.

Acceptance: 33 host tests. Synthetic tables cover CPAL versions 0 and 1,
palette selection, foreground entries, alpha multiplication and clamping,
version 0 layers and their malformed record arrays, version 1 layer lists,
all three gradients, all three extend modes, the ten non-variable affine
formats by the position they map, the ten variable ones against their twins
built from the stored numbers plus the deltas, every composite mode value
including the unrecognized ones, `PaintColrGlyph` reuse,
three shapes of cycle, the depth limit at equality and one beyond, caller
capacity exhaustion, an undefined paint format, variable paints with and
without a delta-set index map, a reserved variation base, stop reordering
under variation, clip boxes of both formats, and overlapping and inverted
clip ranges, a variation base at the end of its range whose sequence fits its
own field count and one whose sequence does not, a clip box bounding a glyph
whose graph alone does not, and an empty clipped outline contributing no ink.
The `NotoEmoji-colr.ttf` fixture, five base glyphs of the build
D-176 pins, exercises paint formats 1, 2, 4, 6, 10, 12, 14, 16, 18 and 32;
its operation and stop counts, clip boxes and gradient geometry were read
from the same file with fontTools. Every truncated prefix of its `COLR` and
`CPAL` tables is refused or bounded. A face whose only glyph data is a
refused colour format covers no cluster and the chain falls through to its
next layer, both for a face that states no outline table and for a strike
face that states a `glyf` whose every entry is empty, and the notdef of a
cluster no face covers comes from a face that can draw one. Debug/release, strict Clippy, the bare target and the
`text_font` fuzz regression, which gained the seeds `colr-emoji` and
`colr-cycle`, all pass. Product coverage after T13 is 96.09% of lines and
89.49% of branches.

`sh tools/xtask-check.sh --quiet` on 2026-09-18 passes lint, layering,
dependency checks, unsafe budgets, host tests, coverage, Miri, documentation
and the QEMU tests, and fails in its end-to-end run. The same run on the
unmodified default branch fails the same way, with a different violation on
each attempt — once "the application said nothing through the console
driver", once "the forwarded port refused" — so the failure is the
container's network and not this step: no program or server of the image
depends on `text-core`, which only `fuzz/text_font` links.

## 17.28 R1 to R3: the drawing side

The three steps below named the work outside this track and did not
specify it. [Document 18](18-rasterization.md) specifies R1 and R2 as
thirteen steps of a track of its own, and D-177 to D-184 are their
decisions.

| Step | Where it is now |
|------|-----------------|
| R1 rasterizer | Steps R1 to R13 of document 18. The crate is `text-raster`. Gradients and the twenty-eight compositing and blending modes of `docs/w3c/compositing-1.html` are what vector emoji cost, and D-176 states why the cost is accepted: this system allows fractional scaling factors and a bitmap strike does not survive them. |
| R2 glyph cache | Step R7 of document 18. D-184 states the key, the eviction rule and the memory bound. |
| R3 integration | Unscheduled. It needs `text-raster`, immutable font distribution and a drawing protocol, and it is done when both address spaces agree on serialized geometry and a stale generation is rejected or retried against the current snapshot. |

D-183 supersedes D-175 in its mechanism: the gamma value corrects the
colour channels around every blend rather than the coverage before it,
because no mapping of coverage alone removes the asymmetry D-175 was
written to remove. The owner of the value, its default and the rule that
`text-core` never holds it are unchanged.

## 17.31 Risks

| # | Risk | Effect | Reduction |
|---|------|--------|-----------|
| 1 | Malformed font offsets/counts | Panic or excessive work | Checked borrowed reads; bounded traversal; negative tests at T1–T6. |
| 2 | Rounding drift | Toolkit and compositor disagree | One Fixed implementation; explicit serialization determinism tests. |
| 3 | Unicode version mismatch | Incorrect boundaries or bidi | Pinned inputs and complete conformance gates. |
| 4 | Generic lookups mistaken for full shaping | Broken complex scripts | Explicit full-shaping refusal and run status. |
| 5 | Fallback or generation mismatch | Wrong glyphs and line geometry | Explicit language, ordered chains, instance metadata, generation. |
| 6 | Strict envelope policy | Some readable fonts refused | Documented overlap/limit errors; valid TTC sharing tests. |
| 7 | Growing layout data | Allocation failure or resource abuse | Caller buffers first; fallible optional vectors; work limits. |
| 8 | Malformed or cyclic paint graph | Unbounded recursion or work | Path tracking, depth 64, 65,536 visits, typed errors; cyclic, self-referential and overlong tests at T13. |
| 9 | Rasterizer cost of vector emoji | R1 grows gradients and 28 blend modes | The cost is stated in D9 and bounded by the paint stream, which carries no graph. |
