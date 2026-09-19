# text-core shaping and layout audit findings

Repository: AuDHSOS/AuDHSOS. Audit of text-core (shape, bidi, segment, layout, resolve, colr, unicode) at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #485
Title: text-core: the shaping work budget is per lookup application, so a hostile font multiplies it by the number of font runs
Labels: bug
Body:
`LayoutTable::apply` creates a fresh `Engine` with `work: 0` at `crates/text-core/src/shape/features.rs:133`, and `Engine::tick` refuses only when that one engine passes one million operations (`crates/text-core/src/shape/mod.rs:260-267`). Layout calls `apply` once for GSUB and once for GPOS per font run per candidate line (`crates/text-core/src/layout/line.rs:114-122` and `crates/text-core/src/layout/line.rs:158-166`). The layout budget `Context::charge` counts source scalars only (`crates/text-core/src/layout/engine.rs:96-103`), so engine operations are not bounded across a layout.

A font with 4,000 lookups of 240 single-substitution subtables whose coverage misses costs one glyph 4,000 × (2 + 240) = 968,000 ticks per `apply` (`crates/text-core/src/shape/mod.rs:354`, `:371`, `:418`), under the limit. The text `"a\n"` repeated 32,768 times (65,536 scalars) makes 32,768 lines; `choose` shapes each line once and `execute` shapes it again (`crates/text-core/src/layout/engine.rs:145-152`, `:208-215`), so GSUB and GPOS together run about 1.3 × 10^11 ticks, each with a coverage lookup, for one `layout` call. With DejaVu Sans the same text already takes 6.2 s (F10); the hostile font turns one call into hours.

Fix: draw engine ticks from the layout's own budget by passing a shared counter into `LayoutTable::apply` so one `layout` call is bounded as a whole; the option not taken, lowering the per-engine limit, breaks longer lines (F04) without bounding the product.

---

## F02 — issue #486
Title: text-core: ligature and multiple substitution move the buffer tail per glyph, O(n²) per lookup and outside the work budget
Labels: bug
Body:
`Buffer::insert` and `Buffer::remove` shift every glyph after the position with `copy_within` (`crates/text-core/src/shape/mod.rs:148-165`). Ligature substitution calls `remove` once per consumed component (`crates/text-core/src/shape/substitute.rs:203`), multiple substitution calls `insert` once per produced glyph (`crates/text-core/src/shape/substitute.rs:74`), and a zero-length sequence calls `remove` (`crates/text-core/src/shape/substitute.rs:61`). A `Glyph` is about 120 bytes. `tick` counts one operation per substitution, not the moved glyphs, so the budget does not see this work.

One line of n glyphs with a ligature at every second position moves n²/4 glyphs. Measured with DejaVu Sans at 16 px and `max_width == None`: `"fi"` × 4,096 takes 0.50 s, × 8,192 takes 1.50 s, × 16,384 takes 3.91 s, while `"a"` × 8,192 takes 0.03 s; `"fi"` × 32,768 (65,536 scalars, inside the input limit) needs about 15 s of memmove for one layout call.

Fix: substitute into a second glyph buffer per lookup pass (read cursor in the old buffer, write cursor in the new one) so each pass is O(n); the option not taken, charging the moved glyphs to `tick`, turns the time into a `LimitExceeded` on ordinary text.

---

## F03 — issue #487
Title: text-core: canonical composition compacts the glyph array once per composed pair, O(n²) and outside the work budget
Labels: bug
Body:
`normalize` removes a composed mark with `out.copy_within(i + 1..len, i)` (`crates/text-core/src/shape/script.rs:343-346`), which moves every following glyph, once per composition. The `work` counter of that function guards only the insertion sort (`crates/text-core/src/shape/script.rs:305-323`).

`"e\u{301}"` repeated composes to `é` when the face covers it (`crates/text-core/src/shape/script.rs:335`). Measured with DejaVu Sans at 16 px and `max_width == None`: × 4,096 takes 0.12 s, × 8,192 takes 0.42 s, × 16,384 takes 2.18 s; × 32,768 (65,536 scalars, inside the input limit) needs about 9 s.

Fix: compose with a read cursor and a write cursor in one pass over the cluster and return the write length, which is O(n); the option not taken, charging moved glyphs to `work`, refuses ordinary text.

---

## F04 — issue #488
Title: text-core: a line of 65,536 glyphs exhausts the one-million operation budget after eight lookups
Labels: bug
Body:
`Engine::run` ticks once per glyph per lookup (`crates/text-core/src/shape/mod.rs:353-361`) and `Engine::apply` ticks again for every glyph the feature enables (`crates/text-core/src/shape/mod.rs:371`), so one lookup pass over a buffer of n glyphs costs at least 2n of the one-million ticks one `apply` may spend (`crates/text-core/src/shape/mod.rs:260-267`). `crates/text-core/README.md:85` and `docs/17-text-and-fonts.md:774` state that input is limited to 65,536 scalars; a run of that length fails as soon as the language system selects eight lookups.

Measured with DejaVu Sans at 16 px: `layout(&set, &style, &"a".repeat(65_536), None)` returns `Font(LimitExceeded)` after 0.15 s while `"a".repeat(32_768)` succeeds. Any paragraph of 65,536 scalars without a hard break, measured without a width, is refused by every real font with eight or more lookups in the selected features.

Fix: do not tick the linear per-glyph pass of `run` and `apply` (the buffer length already bounds it) and keep the budget for subtable, match and nested-lookup work; the option not taken, raising the constant, moves the failure to the next line length.

---

## F05 — issue #489
Title: text-core: greedy wrapping reshapes every candidate from the line start, O(k²) per line, and refuses a 65,536-scalar text at 2000 px
Labels: bug
Body:
`Context::choose` shapes the text from `start` to every allowed boundary in turn until one overflows (`crates/text-core/src/layout/engine.rs:114-180`, the call at `:145-152`), and each candidate charges its scalar count to the one-million budget (`crates/text-core/src/layout/line.rs:248`). A line with k break opportunities and w scalars therefore charges about k·w/2 scalars before it is chosen and shaped once more by `execute`.

Measured with DejaVu Sans at 16 px on `"abcd "` × 13,107 (65,535 scalars): `max_width == Some(2000)` returns `TextError::LimitExceeded` after 2.2 s; `Some(1000)` succeeds in 2.6 s, `Some(500)` in 2.2 s. A text inside the documented input limit fails at a width a wide window offers, and a text that succeeds spends seconds re-shaping prefixes.

Fix: shape the remaining paragraph run once per line, choose the boundary from the accumulated cluster advances of that one shaping, and reshape only the chosen line, so a line costs O(w) plus one reshape; the option not taken, raising the budget, keeps the quadratic time.

---

## F06 — issue #490
Title: text-core: a contextual match scans the whole glyph buffer three times, so long lines with many matches exhaust the budget
Labels: bug
Body:
After a rule matches, `Engine::rule` clears the context bit of every glyph in the buffer with a tick per glyph (`crates/text-core/src/shape/context.rs:167-170`), `Engine::actions` scans from glyph zero to find each action's target with a tick per glyph (`crates/text-core/src/shape/context.rs:198-207`) and clears every glyph again (`crates/text-core/src/shape/context.rs:214-217`). The matched positions are already collected in `positions` (`crates/text-core/src/shape/context.rs:126`), so the whole-buffer scans do no work the array does not already bound.

A line of L glyphs with M contextual matches spends about 3·L·M ticks of the one million per `apply`: a 2,000-glyph line with a `calt` lookup that matches 170 times, or a 65,536-glyph line with six matches, returns `Font(LimitExceeded)`. Fonts with `calt` chains over every letter pair (programming ligature fonts) match at a large share of positions.

Fix: set and clear the context bit only at the entries of `positions`, adjusted by the buffer length change after each action, and start the action scan at `i`; the option not taken, leaving the scans unticked, keeps O(L·M) time.

---

## F07 — issue #491
Title: text-core: mark-to-base attachment walks back over every preceding mark for each mark, O(n²) over a mark run
Labels: enhancement
Body:
`Engine::mark` searches backward from the mark with `next(.., back = true)` until a non-mark glyph (`crates/text-core/src/shape/position.rs:229-242`), and `next` ticks once per skipped glyph (`crates/text-core/src/shape/mod.rs:304`). Over a base followed by n marks the k-th mark skips k − 1 marks, and every subtable whose base coverage misses repeats the walk (`crates/text-core/src/shape/position.rs:243`).

Measured with DejaVu Sans at 16 px: `"a"` followed by 800 × U+0301 lays out, 1,000 × U+0301 returns `Font(LimitExceeded)`. HarfBuzz walks the same way but has no budget, so a text it renders is refused here.

Fix: remember, per lookup pass, the index of the nearest preceding eligible non-mark glyph and reuse it for every mark that follows without an intervening non-mark, which is O(n).

---

## F08 — issue #492
Title: text-core: attachment resolution walks each glyph's full parent chain, O(n²) for stacked mark-to-mark chains
Labels: enhancement
Body:
`finish` resolves each glyph by walking its `parent` chain to the root (`crates/text-core/src/shape/position.rs:317-340`) and counts every step against one million (`crates/text-core/src/shape/position.rs:320`). A mark-to-mark lookup attaches each mark to the previous mark, so the k-th mark walks k + 1 glyphs and n marks cost n²/2 steps.

A base with 1,414 marks attached in one chain returns `LimitExceeded` at `crates/text-core/src/shape/position.rs:320-321` even in a face whose `mark` lookup is O(n) (F07 hides this with DejaVu Sans). Every position of a chain is recomputed for every descendant although it changes nothing between glyphs.

Fix: memoize each glyph's resolved offset with a three-state flag (unresolved, in progress, done) and resolve a parent on demand, which is O(n) and still reports a cycle through the in-progress state.

---

## F09 — issue #493
Title: text-core: canonical ordering uses insertion sort, O(n²) over the marks of one cluster
Labels: enhancement
Body:
`normalize` orders the marks of a cluster by combining class with an insertion sort (`crates/text-core/src/shape/script.rs:305-323`) and refuses past one million inner steps (`crates/text-core/src/shape/script.rs:309`). A cluster of n marks in reverse class order costs n²/2 steps.

`"a"` followed by 1,000 × U+0301 (class 230) and 1,000 × U+0316 (class 220), 2,001 scalars, costs 1,002,000 steps and returns `Font(LimitExceeded)` from `normalize` before any lookup runs.

Fix: sort each cluster's marks by a stable counting sort over the 256 combining classes (or a merge sort in the caller-owned scratch), O(n) per cluster.

---

## F10 — issue #494
Title: text-core: every font run of every candidate line re-validates the face metrics and instance
Labels: enhancement
Body:
`font_run` calls `Instance::new`, `line_metrics` and `font.metrics()` for each run of each candidate (`crates/text-core/src/layout/line.rs:86-89`), and `carets` parses GDEF and metrics again per multi-grapheme cluster (`crates/text-core/src/layout/line.rs:609`, `:628`). With DejaVu Sans (6,253 glyphs) `font.metrics()` costs 65 µs and `Instance::new` 65 µs per call, both proportional to the face's glyph count.

Measured at 16 px: `layout` of `"a"` takes 410 µs, of `"a\na"` 690 µs, and of `"a\n"` × 32,768 takes 6.2 s, about 190 µs per one-character line, all of it validation the first line already did.

Fix: validate metrics and instance once per resolved run face in `Context::new` and keep them in a per-face slot the runs index; the option not taken, caching inside `Font`, adds mutable state the crate excludes.

---

## F11 — issue #495
Title: text-core: composition scans the whole decomposition table for every starter-mark pair
Labels: enhancement
Body:
`unicode::composition` finds the primary composite by a linear scan over the 2,081-entry `DECOMPOSITIONS` table (`crates/text-core/src/unicode/mod.rs:81-93`), and `normalize` calls it for every mark that follows a starter in the same cluster whose class allows composition (`crates/text-core/src/shape/script.rs:335`).

A run of n graphemes of the form letter plus mark costs about 2,081·n comparisons per shaping; the candidate loop of F05 repeats it per candidate. A binary search over a table sorted by (first, second) costs 11 comparisons.

Fix: generate a second static table of pairs sorted by (first, second) in the Unicode generator and binary-search it, O(log D).

---

## F12 — issue #496
Title: text-core: composition ignores the composition exclusions of UAX #15
Labels: enhancement
Body:
`unicode::composition` composes any pair whose canonical decomposition matches (`crates/text-core/src/unicode/mod.rs:81-93`), and the table holds every canonical decomposition of `UnicodeData.txt` (`crates/text-core/generator/mod.rs:216-222`), including `docs/unicode/ucd/UnicodeData.txt:15846` (U+FB2A → U+05E9 U+05C1) and `:2325` (U+0958 → U+0915 U+093C). UAX #15 excludes these primary composites from composition (`Full_Composition_Exclusion`, from `CompositionExclusions.txt`, which `docs/unicode/ucd/` does not hold); NFC keeps U+05E9 U+05C1 as two characters and HarfBuzz composes through the same exclusion.

Hebrew `"שׁ"` (U+05E9 U+05C1) in a face that covers U+FB2A resolves to the presentation-form glyph and one cluster component, so a following vowel point attaches to the anchors of U+FB2A instead of the mark anchors of shin; `docs/17-text-and-fonts.md:665-668` states the intent as recomposing what the face covers and does not name this deviation.

Fix: add `CompositionExclusions.txt` to `docs/unicode/ucd/`, have the generator drop excluded composites and singletons from the composition table, and keep them in `DECOMPOSITIONS` for decomposition.

---

## F13 — issue #497
Title: text-core: feature selection keeps two 4 KiB bool arrays on the stack and clears them per feature stage
Labels: enhancement
Body:
`LayoutTable::apply` allocates `[bool; 4096]` inside its feature loop (`crates/text-core/src/shape/features.rs:144`) and `Engine::stage` allocates another (`crates/text-core/src/shape/features.rs:185`), 8 KiB of stack while `stage` runs, and each is zeroed once per feature stage: 18 substitution stages and 4 positioning stages per font run (`crates/text-core/src/shape/script.rs:47-139`), so about 90 KiB of memset per font run before any lookup runs.

The layout of `"a\n"` × 32,768 (F10) zeroes about 12 GB of stack this way. A userland thread with a small stack carries 8 KiB of this crate's frames below every lookup application.

Fix: replace both arrays with a `[u64; 64]` bitset (512 bytes) indexed by lookup number.

---

## F14 — issue #498
Title: text-core: a match arm binding named `coverage_index` shadows the parameter and reads as a comparison
Labels: enhancement
Body:
`Engine::mark` matches the GDEF class with `0 => glyph.class, coverage_index => coverage_index` (`crates/text-core/src/shape/position.rs:235-238`). The second arm is a catch-all binding that shadows the `coverage_index` parameter of the function (`crates/text-core/src/shape/position.rs:216-223`); it does not compare the class with the coverage index.

The code is correct today, and a reader who takes the arm as a guard reads a different algorithm; a later edit that uses `coverage_index` after the match gets the class instead.

Fix: rename the binding to `class` (`class => class`).
