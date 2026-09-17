# 17. Text and fonts

## 17.0 How to read this document

Each step states Status, Depends on, Size, Needs, Does, Produces, and Done
when, in that order. T1–T12 build `text-core`; R1–R3 describe later work
outside this task. A step ends with its acceptance results before the next step starts.
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
| font instance | A face plus explicit variation coordinates, scale, and baseline offset. |
| role | `Ui` or `Mono`, selecting an ordered fallback chain. |
| generation | Caller-supplied `u64` identifying one immutable font-set snapshot. |
| rasterizer | Outline-to-pixel coverage conversion, outside `text-core`. |
| glyph cache | Rasterized glyph storage owned by the drawing side. |
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
4. T1–T12 add no rasterizer, glyph cache, compositor code, file access,
   memory mapping, settings access, external crate, or C implementation.

## 17.3 What is already built

| Component | Location | Decided in |
|-----------|----------|------------|
| Bitmap text and pixel surfaces | `crates/gfx/src/font.rs:1` | `docs/09-decisions.md:39`, D-29 |
| Logic-crate safety and checked parsing rules | `docs/04-safety-policy.md:62` | `docs/09-decisions.md:15`, D-05 |
| Host test and coverage tooling | `docs/06-testing-strategy.md:8` | `docs/09-decisions.md:33`, D-23 |
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
| R1–R3 drawing integration | Pixel output needs rendering policy and storage outside the pure library. |

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
any scaled metric is exposed. The rasterizer alone chooses pixel rounding,
hinting, antialiasing, and subpixel policy. GPOS Device pixel adjustments do not
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
Vertical layout, color emoji painting, bitmap strikes, SVG glyph painting, and
WOFF/WOFF2 decompression are refused; supported outline fonts remain usable.

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

Every resolved run carries its face index, variation coordinates, scale, and
baseline offset. Base ascent/descent use OS/2 typographic metrics when selected
by the documented T3 rule, otherwise hhea. Each run's scale is style size divided
by that face's units-per-em under D2; the relative fallback scale is therefore
the ratio of base to fallback units-per-em. Baseline offset is zero on the
shared alphabetic baseline in this track. Line extents include scaled fallback
ascent/descent. Weight selects the explicit `wght` instance where present;
other axes use font defaults, without automatic optical-size settings.
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
| `text-core`, T1–T12 | Those values, text, optional width, buffers | Glyph IDs, fixed positions/outlines, line/cluster geometry and box. |
| Rasterizer, R1 | Outlines and positions, explicit rendering policy | Pixel coverage; hinting and rounding cannot feed back into advances. |
| Glyph cache, R2 | Generation, face, instance, size, glyph, raster policy | Cached pixel data with bounded eviction and generation invalidation. |
| Toolkit/compositor integration, R3 | Same font-set snapshot and style | Measurement and drawing commands with checked generation agreement. |

## 17.10 The order of the steps

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
| R1 rasterizer | unscheduled, outside task | T12 | XL |
| R2 glyph cache | unscheduled, outside task | R1 | M |
| R3 integration | unscheduled, outside task | R2 | L |

## 17.11 T1: sfnt envelope

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

## 17.12 T2: cmap

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

## 17.13 T3: metrics and Fixed

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

## 17.14 T4: glyf and loca

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

## 17.15 T5: CFF and CFF2

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

## 17.16 T6: variations

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
**Done when:** defaults/endpoints/intermediate tuples match literal expected
numbers; malformed maps, stores, packed deltas, axis counts, and blend operands
return errors; no double application of advance deltas; report acceptance before continuing.

Acceptance: 62 host tests and one doctest pass in debug/release; Clippy,
`x86_64-unknown-none`, and 12 fuzz seeds pass. Fresh product coverage is
3381/3559 lines (95.00%) and 973/1114 branches (87.34%). A separate host
probe decodes all 4515 glyphs of Noto Sans at wght=900/wdth=75.
The full-system E2E failure recorded in T1 remains outside this crate.

## 17.17 T7: Unicode generation

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

## 17.18 T8: segmentation

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

## 17.19 T9: bidirectional algorithm

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

## 17.20 T10: shaping

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

## 17.21 T11: resolution

**Status:** implemented and accepted (2026-09-17).
**Depends on:** T10.
**Size:** M.
**Needs:** generated script properties; `docs/microsoft/languagetags.html:1`
and `docs/unicode/reports/tr37/tr37-16.html:1`, variation sequences.
**Does:** apply D5 chain ordering, script/language runs, cluster coverage,
instance scale/baseline metadata, and missing-glyph reporting.
**Produces:** resolved runs indexed by role, face, and generation.
**Done when:** one Han code point resolves differently under zh-Hans, zh-Hant,
ja, and ko fixtures where appropriate; fallback baselines/scales and combining
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

Acceptance: role order, whole combining clusters, fallback scales/baselines,
regional language systems, variable weight, script inheritance, bidi levels,
UVS coverage, missing glyphs, caller capacities, and language extensions pass
six host tests.
Debug/release, strict Clippy, and the bare target pass.

## 17.22 T12: layout

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
substitution followed by ligation merge before grapheme geometry is computed. Output serialization is explicit
little-endian, versioned, and contains no padding.

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

Acceptance: eleven layout tests cover fractional advances, legal/emergency
breaks, line-end Arabic/ligature reshaping, bidi carets and selections,
regional fallback, GDEF coordinate/point carets, deleted clusters, gvar metrics
without HVAR, overlapping substitution clusters, buffer growth/exhaustion,
tab shaping barriers, and input/work limits. Emergency-break tests cover an
unbreakable run, an overlong first cluster, combining marks, multiple glyphs
from GSUB, and RTL visual order; every case checks progress, exact overflow,
measurement agreement, and deterministic serialization through caller buffers
and owned wrappers. Layout and
measure agree in every case. Separately allocated inputs serialize identically;
debug/release assert FNV-1a-64 `15cce28391e7c99d` for the mixed-script fixture.
The complete pure stack has 108 host tests and two doctests; allocator-free
builds have 101 host tests. All Unicode gates, regeneration, Clippy, bare-target
builds, and fourteen fuzz regression seeds pass. Product coverage is
95.99% of lines and 89.45% of branches after review.

Review traced the earlier E2E disk timeouts to concurrent BAR size probing by
`app-lspci`. Read-only BAR inspection restores disk notifications during
startup (D-165); the main E2E run passes with the fix.


## 17.23 R1: rasterizer (outside this task)

**Status:** unscheduled.
**Depends on:** T12.
**Size:** XL.
**Needs:** accepted outline and position output, explicit rendering policy.
**Does:** convert outlines into bounded pixel coverage buffers.
**Produces:** rasterizer component separate from `text-core`.
**Done when:** reference images and policy changes preserve T12 measurements.

## 17.24 R2: glyph cache (outside this task)

**Status:** unscheduled.
**Depends on:** R1.
**Size:** M.
**Needs:** full rasterization key and caller-supplied capacity.
**Does:** cache coverage with generation invalidation and bounded eviction.
**Produces:** drawing-side glyph storage.
**Done when:** cached and uncached pixels agree and changed instances cannot
reuse stale glyphs.

## 17.25 R3: integration (outside this task)

**Status:** unscheduled.
**Depends on:** R2.
**Size:** L.
**Needs:** immutable font distribution and drawing protocol design.
**Does:** connect adapters, toolkit, and compositor through explicit snapshots.
**Produces:** on-system text drawing and matching application measurements.
**Done when:** both address spaces agree on serialized geometry; stale
generation commands are rejected or retried against the current snapshot.

## 17.26 Risks

| # | Risk | Effect | Reduction |
|---|------|--------|-----------|
| 1 | Malformed font offsets/counts | Panic or excessive work | Checked borrowed reads; bounded traversal; negative tests at T1–T6. |
| 2 | Rounding drift | Toolkit and compositor disagree | One Fixed implementation; explicit serialization determinism tests. |
| 3 | Unicode version mismatch | Incorrect boundaries or bidi | Pinned inputs and complete conformance gates. |
| 4 | Generic lookups mistaken for full shaping | Broken complex scripts | Explicit full-shaping refusal and run status. |
| 5 | Fallback or generation mismatch | Wrong glyphs and line geometry | Explicit language, ordered chains, instance metadata, generation. |
| 6 | Strict envelope policy | Some readable fonts refused | Documented overlap/limit errors; valid TTC sharing tests. |
| 7 | Growing layout data | Allocation failure or resource abuse | Caller buffers first; fallible optional vectors; work limits. |
