# text-raster audit findings

Repository: AuDHSOS/AuDHSOS. Audit of text-raster at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #437
Title: text-raster: a gradient evaluated far from its geometry overflows and aborts the whole draw
Labels: bug
Body:
`Gradient::at` maps a pixel centre into the fill's own space with `self.inverse.apply` (`crates/text-raster/src/gradient.rs:191`) and then squares or dot-multiplies the result in Q32.32 (`crates/text-raster/src/gradient.rs:203-204`, `crates/text-raster/src/gradient.rs:400-402`, `crates/text-raster/src/gradient.rs:367-369`). `shift_for` narrows only the gradient's own coordinates to at most 128 (`crates/text-raster/src/gradient.rs:441-455`); the pixel's fill-space coordinate is narrowed by that same shift and is otherwise unbounded. `State::paint` propagates the error with `?` (`crates/text-raster/src/paint.rs:439`), `draw_color_glyph` propagates it (`crates/text-raster/src/paint.rs:132-140`), and `draw` propagates it out of the paragraph loop (`crates/text-raster/src/draw.rs:79`).

A font with `PaintGlyph` -> `PaintScale(1/64)` -> `PaintRadialGradient(c0 = c1 = (0, 0), r0 = 0, r1 = 100)` drawn at 16 px on a 1000-upem face gives a placement with `xx = 0.016 / 64`; the inverse maps the pixel at box position (0.5, 0.5) to a fill-space coordinate near 62000, whose square exceeds 2^31, and `dot` returns `Err(Font(Overflow))`. Confirmed by calling `Gradient::new` with that placement and `Gradient::at` at (0.5, 0.5) and at (16.5, 0.5): both return `Err(Font(Overflow))`. One such glyph anywhere in a `LayoutView` makes `draw` return an error, and every glyph after it in the view is not drawn. The comment at `crates/text-raster/src/gradient.rs:354-355` states the intent that a parameter far outside the line saturates, which `dot` and `apply` do not do.

Fix: evaluate `parameter` in `i128` from the raw bits and saturate the result to the Q32.32 range before `sample`, and treat an `Overflow` from `inverse.apply` as a parameter of `i64::MAX` with the sign of the coordinate; the option of choosing `shift` from the pixel coordinates as well as the gradient's keeps the arithmetic in Q32.32 but changes the shift per pixel and needs the geometry renormalized per pixel.

---

## F02 — issue #438
Title: text-raster: one glyph's font error aborts the paragraph
Labels: bug
Body:
`draw` calls `one_glyph` with `?` inside the loop over `view.glyphs` (`crates/text-raster/src/draw.rs:72-81`), so the first glyph whose outline or paint stream fails ends the call. The errors that reach that point from font data are `Font(_)` from `colr.paint` (`crates/text-raster/src/draw.rs:232-238`), from `outline_edges` (`crates/text-raster/src/outline.rs:317-323`, `crates/text-raster/src/outline.rs:338`) and from `Affine::compose` (`crates/text-raster/src/paint.rs:121`, `crates/text-raster/src/paint.rs:134`), and `Stream` from an unbalanced stream (`crates/text-raster/src/paint.rs:144-146`). `crates/text-raster/src/draw.rs:61` says a glyph the surface does not reach is skipped; a glyph the font describes badly is not skipped.

A colour glyph whose paint graph exceeds `MAX_PAINTS` of `text-core` makes `colr.paint` return `Err(LimitExceeded)`; `draw` returns `Err(Font(LimitExceeded))` after drawing the glyphs before it and none after it, including glyphs of other faces in the same view. A fallback face installed for one script therefore removes every line of a paragraph that contains one of its bad glyphs.

Fix: in `draw`, match the result of `one_glyph` and continue on `Font(_)`, `Stream` and `Overflow`, returning `BufferTooSmall` and `Face` as now because the caller can act on those; the option of returning the failing glyph index in the error and letting the caller re-draw without it costs a second pass per bad glyph.

---

## F03 — issue #439
Title: text-raster: quadratic flattening uses the wrong second difference and exceeds the D-181 tolerance
Labels: bug
Body:
`Sink::quadratic` passes `(start, control, control, end)` to `second_difference` (`crates/text-raster/src/flatten.rs:138`), which computes `a - 2*b + c` over `(p0, p1, p2)` and `(p1, p2, p3)` (`crates/text-raster/src/flatten.rs:176-195`). With the control point repeated those two values are `start - control` and `end - control`, the two legs of the control polygon, and the comment at `crates/text-raster/src/flatten.rs:173-175` claims they equal the second difference. D-181 defines `d = |p0 - 2·p1 + p2|` for a quadratic (`docs/18-rasterization.md:206-207`), and `|start - 2·control + end|` reaches twice the longer leg when the legs point the same way.

A quarter circle of radius 256 px stated as one quadratic, `start = (256, 0)`, `control = (256, 256)`, `end = (0, 256)`, has `d = 362` and needs 27 segments by `quadratic_segments` (`crates/text-raster/src/flatten.rs:42-44`); the code passes `d = 256`, takes 23 segments, and the polyline deviates 0.171 px from the curve where `TOLERANCE` is 0.125 px (`crates/text-raster/src/flatten.rs:17`). Confirmed by flattening that contour through `flatten_glyf` and measuring the perpendicular distance from the curve to each segment. The test at `crates/text-raster/src/tests/flatten.rs:194-224` uses `control = (1000, 552)`, for which the longer leg exceeds the true `d`, so it does not reach this case.

Fix: in `Sink::quadratic` compute `length(difference(start, control, end))` and pass it to `quadratic_segments`; the option of keeping `second_difference` for both curve kinds needs a quadratic-specific branch inside it.

---

## F04 — issue #440
Title: text-raster: a colour line with every stop at one offset renders under repeat and reflect
Labels: bug
Body:
`Gradient::sample` treats a line whose first and last stops share one offset as a step for every extend mode (`crates/text-raster/src/gradient.rs:240-250`). `docs/microsoft/colr.html:816` states that this line is a step only under pad, and that under repeat or reflect the colour line is ill-formed and nothing must be rendered.

A linear gradient with two stops at offset 0.5 and `Extend::Repeat` returns `Some` from `Gradient::new` and paints the last colour at every pixel at or past the offset. Confirmed with `Gradient::new` over `Affine::IDENTITY` and `at(8, 0)`, which returns a pixel with red `65535`.

Fix: in `Gradient::new` return `Ok(None)` when the first and last stop of the line share one offset and `line.extend` is not `Pad`; the option of checking in `sample` repeats the test per pixel.

---

## F05 — issue #441
Title: text-raster: a linear gradient whose rotation point equals p0 renders
Labels: bug
Body:
`perpendicular_part` treats a zero rotation vector as "no rotation" and uses `p1` as `p3` (`crates/text-raster/src/gradient.rs:379-384`). `docs/microsoft/colr.html:837` states that a linear gradient with `p2` equal to `p0` is ill-formed and must not be rendered.

A linear gradient with `p0 = (0, 0)`, `p1 = (8, 0)`, `p2 = (0, 0)` returns `Some` from `Gradient::new` and paints the ramp. Confirmed with `Gradient::new` over `Affine::IDENTITY` and `at(4, 0)`, which returns a pixel with red `32767`.

Fix: return `Ok(None)` from `perpendicular_part` when `square == Fixed::ZERO`, and list `p2` on `p0` in the doc comment at `crates/text-raster/src/gradient.rs:79-81`; the option of keeping the fallback contradicts the specification.

---

## F06 — issue #443
Title: text-raster: a radial gradient paints the circle of radius zero
Labels: bug
Body:
`admissible` accepts a root whose interpolated radius is zero (`crates/text-raster/src/gradient.rs:431-436`), and the doc comment at `crates/text-raster/src/gradient.rs:390-391` says "not negative". `docs/microsoft/colr.html:877` paints only where `r(ω) > 0`, and `docs/microsoft/colr.html:882` states that two circles of radius zero paint nothing.

A radial gradient with `c0 = (0, 0)`, `c1 = (8, 0)`, `r0 = r1 = 0` returns `Some` from `Gradient::new`, and a pixel whose centre lies on the line through the two centres is painted: `at(4, 0)` returns red `32767` while `at(4, 1)` returns `None`. Confirmed over `Affine::IDENTITY`. The tip of every cone, where `r(ω) = 0`, is painted the same way.

Fix: compare `radius > Fixed::ZERO` in `admissible`; the option of testing `r0 == r1 == 0` in `Gradient::new` covers the two-zero case and not the cone tip.

---

## F07 — issue #445
Title: text-raster: an overflow in the font's fixed-point arithmetic surfaces as `Font(Overflow)` where the docs say `Overflow`
Labels: bug
Body:
`From<FontError> for RasterError` wraps every `FontError` in `RasterError::Font` (`crates/text-raster/src/error.rs:86-90`), and every `?` on a `text_core::Fixed` or `Affine` operation goes through it. The doc comments promise `Overflow`: `crates/text-raster/src/gradient.rs:82-83`, `crates/text-raster/src/gradient.rs:182-183`, `crates/text-raster/src/transform.rs:363-364`, `crates/text-raster/src/flatten.rs:276-279`, `crates/text-raster/src/outline.rs:300-303`. `RasterError::Overflow` is documented as "a surface larger than an address of this machine" (`crates/text-raster/src/error.rs:58-59`) and is what the crate's own `checked_*` calls return, so one condition has two variants depending on whose arithmetic hit it.

`Affine::inverse` on the placement of F08 returns `Err(Font(Overflow))`, and `Gradient::at` in F01 returns `Err(Font(Overflow))`; `Fixed::checked_div` by zero returns `Font(InvalidTable)` for an argument that is not a table. A caller matching `RasterError::Overflow` does not see these.

Fix: map `FontError::Overflow` to `RasterError::Overflow` in the `From` impl and keep the other variants under `Font`; the option of correcting every doc comment leaves the caller with two variants for one condition.

---

## F08 — issue #446
Title: text-raster: a solid fill inverts its placement and fails on an inverse it does not use
Labels: bug
Body:
`State::paint` calls `Gradient::new` for every fill (`crates/text-raster/src/paint.rs:420`), and `Gradient::new` inverts the placement before it looks at the fill kind (`crates/text-raster/src/gradient.rs:89-93`). `Affine::inverse` maps an overflow of the division to `None` but propagates an overflow of the translation products (`crates/text-raster/src/transform.rs:389-396`). A `Fill::Solid` needs no inverse.

A font with `PaintGlyph` -> `PaintTranslate(dx = 32767)` -> `PaintTransform(xx = 1/65536, yy = 1)` -> `PaintSolid`, drawn at 16 px on a 1000-upem face with the glyph box starting 1000 px left of the origin, gives `placed.xx = 1049` bits and `placed.dx = 1524` px; the inverse's `xx` is near 4.1e6 and `xx * dx` exceeds 2^31. Confirmed: `inverse()` on that placement returns `Err(Font(Overflow))` and `Gradient::new(Fill::Solid(..), ..)` returns the same error, so `draw` aborts (F02) for a fill that draws one colour.

Fix: match `Fill::Solid` in `State::paint` before calling `Gradient::new`, and move the `Fill::Solid => return Ok(None)` arm of `Gradient::new` above the inverse; the option of making `inverse` saturate the translation hides the overflow for gradients that do use it.

---

## F09 — issue #448
Title: text-raster: the glyph cache lookup scans every live entry where docs/18 states O(1)
Labels: bug
Body:
`GlyphCache::find` walks `0..self.count` and compares the digest of every live entry (`crates/text-raster/src/cache.rs:575-587`); `get` calls it once per lookup (`crates/text-raster/src/cache.rs:494-496`). `docs/18-rasterization.md:592-593` states "O(1) expected for a lookup over the entry table". The digest is computed and stored (`crates/text-raster/src/cache.rs:563`, `crates/text-raster/src/cache.rs:666-685`) but no table is indexed by it.

With an entry table of 4096 records and a full cache, `monochrome` performs 4096 comparisons per glyph on every hit and every miss (`crates/text-raster/src/draw.rs:135-137`); a view of 1000 glyphs costs 4 million comparisons before any pixel is drawn. The lookup is O(n) for n live entries.

Fix: index the entry table by `digest` with open addressing over a caller-owned slot slice, clearing a slot on eviction, which makes the lookup O(1) expected as the document states; the option of correcting the document to O(n) keeps the cost.

---

## F10 — issue #450
Title: text-raster: every glyph re-parses the COLR table and the outline table
Labels: enhancement
Body:
`one_glyph` calls `Colr::parse(font)` per glyph (`crates/text-raster/src/draw.rs:94`), and `outline_edges` calls `Glyf::parse(font)` or `Cff::parse(font)` per outline (`crates/text-raster/src/outline.rs:314`, `crates/text-raster/src/outline.rs:335`); the colour path calls `outline_edges` once per `Clip` op (`crates/text-raster/src/paint.rs:183-190`). `Glyf::parse` validates every `loca` entry, O(N) for N glyphs in the face (`crates/text-core/src/glyf.rs:94-101`); `Cff::parse` reads the whole charstring index and charset (`crates/text-core/src/cff/mod.rs:37-49`); `Colr::parse` validates every base glyph record, layer record and clip record and the item variation store (`crates/text-core/src/colr/mod.rs:135-201`, `crates/text-core/src/colr/mod.rs:449-534`).

A view of G glyphs from a face of N glyphs costs O(G·N) in `loca` validation alone; 1000 glyphs of a 65535-glyph CJK face read 65 million `loca` entries per `draw`, and a colour glyph with k clip ops reads N·k. `docs/18-rasterization.md:836-837` states O(G) dispatches plus each glyph's own cost, which does not include N.

Fix: parse `Colr`, `Glyf` and `Cff` once per face at the top of `draw` into a fixed-size per-face array in `Context`, and pass the parsed tables to `one_glyph` and `outline_edges`; the option of caching inside `text-core` moves per-call state into a crate that has none.

---

## F11 — issue #451
Title: text-raster: ring allocation scans every live entry per attempt
Labels: enhancement
Body:
`GlyphCache::allocate` calls `free` for each candidate range (`crates/text-raster/src/cache.rs:612-626`), and `free` walks every live entry (`crates/text-raster/src/cache.rs:629-639`). The loop evicts one entry per failed attempt, so an insertion that evicts k entries out of n live ones costs O(k·n); `docs/18-rasterization.md:592-593` states O(k).

The ring allocates in address order within one lap and evicts oldest first, so the only live entry a candidate range can overlap first is the oldest one; a full cache of 4096 entries that wraps costs up to 4096 scans of 4096 entries for one insertion.

Fix: test the candidate range against the oldest live entry only, the entry at `self.slot(0)`, which is O(1) per attempt and O(k) per insertion as the document states; the option of a free-space counter needs the same oldest-entry rule to stay exact.

---

## F12 — issue #453
Title: text-raster: the monochrome path rasterizes the whole glyph box even where the surface does not reach
Labels: enhancement
Body:
`monochrome` computes the glyph's box from its edges with `edge_bounds` (`crates/text-raster/src/draw.rs:172`), demands `width * height` bytes of `context.coverage` (`crates/text-raster/src/draw.rs:175-182`) and fills the whole box (`crates/text-raster/src/draw.rs:189-192`); the colour path clips its box to the destination first (`crates/text-raster/src/draw.rs:262`, `crates/text-raster/src/draw.rs:289-309`). `blend_mono` then visits every mask pixel, inside or outside the destination (`crates/text-raster/src/mono.rs:345-361`).

A glyph at a run scale that places its box at 8000 x 8000 px over a 1920 x 1080 destination costs 64 M cells in `fill` and 64 MB of `coverage`; with a smaller `coverage` slice `draw` returns `BufferTooSmall` and the whole view is not drawn (F02), where the colour path draws the visible 2 M pixels. The cost of `fill` is O(E log E + W·H + Σ over rows of the active edges) for a box of W x H and E edges, so the box and not the destination bounds the work.

Fix: when the box's pixel count exceeds `context.coverage.len()`, clip the box to the destination as `clipped` does, rasterize that part and skip the cache insert; the option of always clipping loses the cache for every glyph that overhangs an edge.

---

## F13 — issue #455
Title: text-raster: a surface demands `height * stride` bytes where the last row needs only its visible bytes
Labels: enhancement
Body:
`Surface::new` rejects a slice shorter than `height * stride` (`crates/text-raster/src/surface.rs:293-300`) and `Mask::new` does the same (`crates/text-raster/src/surface.rs:214-221`); the doc at `crates/text-raster/src/surface.rs:272-276` states it. A surface writes only the visible bytes of each row (`crates/text-raster/src/surface.rs:352-365`), so the bytes after the last row's visible columns are never read or written.

A compositor that hands this crate a window region of a larger framebuffer, as the slice from the region's first byte to its last visible byte, has `(height - 1) * stride + width * bytes_per_pixel` bytes and is refused with `TooShort`; it must extend the slice past the region's last row into the neighbouring window's bytes to satisfy the check.

Fix: compute `needed` as `(height - 1) * stride + row` with `row = width * bytes_per_pixel`, in both constructors and the doc comment; the option of keeping `height * stride` needs every caller to own the padding of its last row.

---

## F14 — issue #456
Title: text-raster: the `None` arm of `extend` is dead
Labels: enhancement
Body:
`extend` returns `Option<Fixed>` and its doc says `None` means a position the mode cannot extend (`crates/text-raster/src/gradient.rs:330-345`), but every arm returns `Some`. `sample` handles the `None` by returning the first colour (`crates/text-raster/src/gradient.rs:252-254`), which no input reaches.

A reader of `sample` looks for the extend mode that pads to the first colour and finds none; the ill-formed repeat and reflect step line of F04 is the case the arm reads as written for, and it is not routed there.

Fix: return `Fixed` from `extend` and remove the `let Some(position) = ... else` in `sample`, with F04 handled in `Gradient::new`; the option of returning `None` from `extend` for F04 tests the stops per pixel.

---

## F15 — issue #458
Title: text-raster: `expand` and `narrow` are defined twice
Labels: enhancement
Body:
`pixel.rs` defines `expand` and `narrow` (`crates/text-raster/src/pixel.rs:240-255`) and `paint.rs` defines the same two functions with the same bodies (`crates/text-raster/src/paint.rs:507-519`). The clip weight of the paint stream (`crates/text-raster/src/paint.rs:220`, `crates/text-raster/src/paint.rs:245`) and the coverage of an `A8` surface (`crates/text-raster/src/pixel.rs:193`, `crates/text-raster/src/pixel.rs:225`) go through different copies.

A change to the rounding of one copy leaves a clip plane and an `A8` surface with two conversions of the same byte, and the golden images of `crates/text-raster/src/tests/golden/` do not show which one moved.

Fix: make the two in `pixel.rs` `pub(crate)` and import them in `paint.rs`; the option of a shared module for two functions is more files for the same effect.
