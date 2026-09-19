# text-core font parsing audit findings

Repository: AuDHSOS/AuDHSOS. Audit of text-core (read, sfnt, cmap, glyf, metrics, fixed, cff, variation) at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #478
Title: text-core: gvar applies a non-intermediate tuple at full strength beyond its peak
Labels: bug
Body:
`tuple_scalar` in `crates/text-core/src/variation/gvar.rs:336-342` returns `Fixed::ONE` for a tuple without an intermediate region whenever the instance coordinate has the same sign as the peak and `|coordinate| >= |peak|`. The specification defines the implicit region of such a tuple as the interval between the zero origin and the peak (`docs/microsoft/otvaroverview.html:1171`) and sets the axis scalar to 0 for an instance coordinate outside the region (`docs/microsoft/otvaroverview.html:1222`). `ItemStore::scalar` in `crates/text-core/src/variation/store.rs:399-401` and `Store::scalar` in `crates/text-core/src/cff/blend.rs:533-535` implement the zero case for explicit regions; the gvar path is the only one that does not.

A glyph variation with a shared or embedded peak of 0.5 on one axis and an instance coordinate of 1.0 on that axis receives the full delta of that tuple in `Gvar::evaluate` (`crates/text-core/src/variation/gvar.rs:233-255`), where the specification yields no delta. `Glyf::outline_instance` (`crates/text-core/src/glyf.rs:180-230`) and `Instance::advance` (`crates/text-core/src/variation/instance.rs:67-71`) return an outline and a phantom advance that differ from every conforming renderer for that instance.

Fix: in the branch at `crates/text-core/src/variation/gvar.rs:338-342` return `Fixed::ZERO` when `|coordinate| > |peak|` and keep `Fixed::ONE` only for equality, the case the intermediate branch handles at `crates/text-core/src/variation/gvar.rs:324-325`; deriving explicit `start`/`end` from the peak and sharing the intermediate code path is the option not taken.

---

## F02 — issue #479
Title: text-core: USE_MY_METRICS adopts transformed component phantom points
Labels: bug
Body:
`Glyf::composite` in `crates/text-core/src/glyf.rs:420-425` applies the component matrix and offset to `child_phantoms` before assigning them to the composite's phantoms when flag `0x200` is set. The specification defines USE_MY_METRICS as forcing the composite's advance width and side bearings to equal those of the component glyph (`docs/microsoft/glyf.html:1043`, `docs/microsoft/glyf.html:1082`); the component's own metrics carry no scale or offset.

A composite whose component carries flags `0x0208` (WE_HAVE_A_SCALE and USE_MY_METRICS) with scale 0.5 and a child advance of 500 produces `phantoms[1].x - phantoms[0].x == 250` in `Outline::phantoms` (`crates/text-core/src/glyf.rs:53-60`). `Instance::advance` (`crates/text-core/src/variation/instance.rs:67-71`) returns that halved advance for a variable font without HVAR, and a component offset `(x, y)` shifts `phantoms[0].x` by `x`, so a left side bearing derived from the phantom points is wrong for every offset component with the flag. The test `point_matching_and_use_my_metrics` (`crates/text-core/src/tests/glyf.rs:139-165`) covers only an identity matrix with zero offset.

Fix: copy `child_phantoms` into `phantoms` at `crates/text-core/src/glyf.rs:423-425` before the transform loop at `crates/text-core/src/glyf.rs:420-422`, or keep an untransformed copy taken at `crates/text-core/src/glyf.rs:386-387`; transforming and then inverting the matrix is the option not taken.

---

## F03 — issue #480
Title: text-core: cmap rejects equal platform and encoding records that differ by language
Labels: bug
Body:
`Cmap::parse` in `crates/text-core/src/cmap.rs:44-48` returns `TableOrder` when two encoding records carry the same `(platform, encoding)` pair. The specification orders records by platform, then encoding, then the language field of the subtable, and requires only the triple to be unique (`docs/microsoft/cmap.html:810`).

A font whose cmap holds two Macintosh records `(1, 0)` with subtable languages 0 and 1 followed by a Windows `(3, 1)` format 4 record fails `Font::cmap()` (`crates/text-core/src/metrics.rs:367-370`) with `TableOrder`, although the code never selects a Macintosh subtable (`crates/text-core/src/cmap.rs:65-69`) and the Windows subtable is valid.

Fix: at `crates/text-core/src/cmap.rs:45` reject only `p > pair` and, for `p == pair`, compare the language field read at offset 4 of each record's subtable; ignoring order among Macintosh records entirely is the option not taken.

---

## F04 — issue #481
Title: text-core: post 2.0 glyph names outside [A-Za-z0-9._] or empty refuse the whole face
Labels: bug
Body:
`validate_names` in `crates/text-core/src/metrics.rs:235-242` returns `InvalidTable` for a Pascal string of length 0, of length above 63, or containing any byte other than ASCII alphanumerics, `.` and `_`. The specification defines the strings as opaque Pascal strings (`docs/microsoft/post.html:831-832`) and places no constraint on their bytes or length. `Metrics::parse` propagates the error for an optional table (`crates/text-core/src/metrics.rs:295-298`), and `Glyf::parse` (`crates/text-core/src/glyf.rs:79`), `Cff::parse` (`crates/text-core/src/cff/mod.rs:38`) and `Instance::new` (`crates/text-core/src/variation/instance.rs:35`) depend on it.

A font with a version 2.0 post table whose glyphNameIndex references a custom name `a-b` or an empty string fails every metrics, outline and instance operation of the crate, while the names affect no layout result (`docs/17-text-and-fonts.md:459`).

Fix: keep the length and bounds walk in `crates/text-core/src/metrics.rs:231-244` and drop the byte-class and zero-length conditions at `crates/text-core/src/metrics.rs:235-239`; validating names against the Adobe Glyph List conventions on a separate query path is the option not taken.

---

## F05 — issue #482
Title: text-core: seac component lookup walks the charset once per glyph, O(G × R)
Labels: enhancement
Body:
`Cff::standard_glyph` in `crates/text-core/src/cff/mod.rs:399-403` calls `sid(id)` for every glyph id from 1 to the glyph count until the SID matches. `sid` for charset formats 1 and 2 (`crates/text-core/src/cff/mod.rs:424-444`) walks the range records from the start on each call. The combined cost is O(G × R) range reads per component, with G the glyph count and R the range count.

A CFF font with 65,535 glyphs, a format 1 charset of 65,534 one-glyph ranges, and a glyph whose charstring ends with the four-operand `endchar` performs about 2 × 10⁹ range reads in `sid` for each of the two components of one `Cff::outline` call (`crates/text-core/src/cff/mod.rs:363-368`), and repeats the work on every layout of that glyph. The operation limit in `crates/text-core/src/cff/type2.rs:211-214` does not count this walk.

Fix: resolve the SID by one pass over the charset ranges in `standard_glyph`, keeping a running glyph counter and returning `first_glyph + (sid - first_sid)` for the range that contains the SID, O(R); building a SID-to-glyph map at parse time is the option not taken because parsing allocates nothing.

---

## F06 — issue #483
Title: text-core: FDSelect format 3 and 4 lookup is linear per outline
Labels: enhancement
Body:
`Cff::select` in `crates/text-core/src/cff/mod.rs:187-203` scans the range records from the first until `first > glyph`, O(R) per call. `validate_select` (`crates/text-core/src/cff/mod.rs:232-246`) proves the `first` values strictly increasing, so a binary search is valid. `outline_inner` calls `select` on every outline (`crates/text-core/src/cff/mod.rs:348`), and `docs/17-text-and-fonts.md:516` documents the linear cost.

A CID-keyed font with one range per glyph (R = G = 65,535) reads up to 65,535 records before decoding its last glyph, on every `Cff::outline` or `Cff::outline_instance` call for that glyph.

Fix: replace the scan in `select` with `read::lower_bound` over the `first` fields, O(log R), and update `docs/17-text-and-fonts.md:516`; caching the selected FD per glyph is the option not taken because the crate keeps no mutable state.

---

## F07 — issue #484
Title: text-core: Type 2 machine and DICT parser reserve 513-entry operand stacks on the call stack for CFF
Labels: enhancement
Body:
`type2::decode` in `crates/text-core/src/cff/type2.rs:70-86` builds a `Machine` with `stack: [Fixed; 513]` (4,104 bytes), `transient: [Option<Fixed>; 32]` (512 bytes) and `path: [(bool, usize); 10]` (160 bytes), about 4.8 KiB in one frame. `Dict::parse_with` in `crates/text-core/src/cff/dict.rs:174` holds another `[Fixed; 513]` (4,104 bytes). Both sizes apply to CFF fonts, whose operand limit is 48 entries (`crates/text-core/src/cff/type2.rs:117`, `crates/text-core/src/cff/dict.rs:180`).

Every `Cff::outline` call pays the 4.8 KiB frame plus up to ten `run` frames (`crates/text-core/src/cff/type2.rs:196-300`), and every `Cff::parse` pays 4.1 KiB per dictionary parse; a userland thread with a small stack that lays out text through this crate carries that reservation at the deepest point of every glyph.

Fix: keep the operand stack in a caller-owned workspace passed into `decode`, or size the array by `cff2` through a const generic; leaving the `Machine` on the stack and documenting the frame size is the option not taken.
