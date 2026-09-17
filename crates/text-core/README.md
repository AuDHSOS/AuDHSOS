# text-core

Deterministic sans-I/O text and font logic. T1–T12 implement the pure stack
specified in `docs/17-text-and-fonts.md`; rasterization and drawing integration
remain outside this crate.

`Font::parse(bytes)` validates the complete file and selects face zero.
`FontCollection::parse(bytes)` validates once; `font(index)` selects other faces
without repeating overlap checks. `Font::table(tag)` returns a borrowed payload.
Offsets are relative to the whole file, including for collection faces.

The library performs no I/O and has no floating point, dependencies, or unsafe
code. Parsing and caller-buffer operations allocate nothing. The default
`alloc` feature provides fallible owned layout and measurement wrappers;
`default-features = false` removes the allocator requirement.
Callers keep immutable font bytes alive.
Host tests may read files; the parser only receives their bytes.

Supported envelopes: sfnt `0x00010000`, `OTTO`, and TTC versions 1.0 and 2.0.
Limits: 64 faces, 256 tables per face, 4096 total table records. Validation takes
O(R² + F²) time and O(1) extra storage; table lookup takes O(log T) time.
Tags must be valid, sorted, and unique. Metadata and payload ranges must be
disjoint; identical payloads may be shared across faces with matching tags and
recorded checksums. Directory aliases and partial table overlaps are refused.
Zero-length tables are accepted at bounded aligned offsets outside metadata.
TTC v2 DSIG data must be disjoint and at EOF; signatures are not verified.

Search hints, checksum correctness, gap bytes, and trailing padding are not
validated. Successful parsing certifies the envelope, not the contents of any
table or the presence of tables needed for rendering. Unknown valid tags are
available unchanged. Each payload reader validates the tables needed by its operation.

```rust
use text_core::{Font, FontError};

assert_eq!(Font::parse(&[]).err(), Some(FontError::Truncated));
```

`Font::cmap()` supports formats 0, 4, 6, 12, 13, and variation selectors (14).
`Font::metrics()` validates head/hhea/hmtx/maxp and optional OS/2/post tables.
`Fixed` is signed Q32.32 with checked arithmetic and nearest-even rounding.

`glyf::Glyf` decodes simple/composite contours into caller-owned point and
contour buffers. `cff::Cff` decodes CFF/CFF2 into cubic path commands, including
local/global subroutines, CID fonts, and blend operands. Recursion and work
limits reject hostile structures. Both readers return unhinted font units.

`variation::Axes` normalizes explicit axes through fvar/avar v1.
`Glyf::outline_instance` applies gvar and IUP using caller-owned scratch.
`Cff::outline_instance` applies CFF2 blends. Both accept a static face with an
empty coordinate slice and decode its default outline.
`variation::Instance` selects HVAR
or gvar phantom advances once and applies MVAR to selected typo metrics.
The caller supplies the same normalized coordinates to outlines and metrics.

`unicode` exposes Unicode 18.0.0 properties, script extensions, bidi brackets,
mirroring, and canonical decompositions from static checked-in tables.
Regenerate with `cargo run -p text-core --example unicode_gen`; add `-- --check`
to verify. Only this host tool and host tests read the retained UCD files.

`segment` provides allocation-free grapheme/word boundary iterators and
caller-buffer line break classification. All Unicode 18.0.0 segmentation
conformance cases run as host tests. Offsets refer to the original UTF-8 text.

`bidi` implements UAX #9, including isolates, paired brackets, paragraph
resolution, and per-line L1/L2 ordering. Both complete Unicode bidi suites are
host-test gates.

`shape` implements GDEF, GSUB types 1–8, and GPOS types 1–9, including feature
variations, contextual lookups, mark filtering, and mark/cursive attachments.
Full horizontal shaping covers Latin, Greek, Cyrillic, Hebrew, Arabic, Han,
Hiragana, Katakana, and Hangul. Other scripts are explicitly flagged for simple
cmap/advance placement. Unicode segmentation and bidi still apply.

`FontSet` supplies ordered UI/mono chains and a generation. `TextStyle` supplies
role, positive size, explicit language, and weight. Resolution preserves whole
graphemes, selects regional Han language systems, and returns face indices,
normalized weight coordinates, and scales. Every run sits on one alphabetic
baseline; the crate reads no `BASE` table.

`layout` and `measure` share the same shaping, greedy wrapping, and geometry
path. Results retain fractional advances, visual glyphs, line boxes, grapheme
carets with affinity, and selection rectangles. `layout_into` and `measure_into`
use caller-owned `layout::Workspace` and `layout::LayoutBuffers`.
Input is limited to 65,536 scalars and candidate shaping to one million scalar
visits. Short buffers, malformed fonts, numeric overflow, and work exhaustion
return typed errors. Failed operations invalidate scratch and output contents.
Owned wrappers cap each growing buffer at 1,048,576 entries.

```rust,no_run
# #[cfg(feature = "alloc")]
# fn example() -> Result<(), Box<dyn std::error::Error>> {
use text_core::{Fixed, Font, FontSet, Language, TextStyle, layout, measure};
let bytes = std::fs::read("font.otf")?; // The host owns I/O.
let fonts = [Font::parse(&bytes)?];
let set = FontSet { ui: &fonts, mono: &fonts, generation: 1 };
let style = TextStyle {
    size: Fixed::from_i32(16),
    lang: Language::parse("ja")?,
    ..TextStyle::default()
};
let result = layout(&set, &style, "Hello 世界", Some(Fixed::from_i32(200)))?;
assert_eq!(measure(&set, &style, "Hello 世界", Some(Fixed::from_i32(200)))?,
           (result.info().width, result.info().height));
let view = result.view();
let mut encoded = vec![0; view.encoded_len()?];
view.write_bytes(&mut encoded)?; // Versioned little-endian numbers, no padding.
# Ok(())
# }
```

The box measures advances and line metrics, not rasterized ink. Soft-wrap
trailing ASCII spaces/tabs collapse; tabs use four base-face space advances.
A final hard break adds an empty final line; empty input has a zero box.
Pixel rounding, hinting, antialiasing, and display settings belong to the caller's
rasterizer. Vertical text, dictionary breaking, color/bitmap/SVG painting, and
WOFF decompression are refused by this track.
