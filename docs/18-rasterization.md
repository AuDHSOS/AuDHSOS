# 18. Rasterization

## 18.0 How to read this document

Every step below has the same parts, in the same order: Status, Depends
on, Size, Needs, Does, Produces, Done when. A step that creates something
nameable carries Produces; the others do not. Nothing is implied. Every
term is defined in 18.1 and has exactly one name, used everywhere.

Steps R1–R13 replace the three placeholder steps R1–R3 of
[document 17](17-text-and-fonts.md), which named this work and did not
specify it. Document 17 owns `text-core`: the tables, the paint graph and
the ink extents. This document owns `text-raster`: everything from an
outline to an alpha value.

## 18.1 Terms

| Term | Meaning |
|------|---------|
| surface | Width, height, stride, a pixel format, and a borrowed mutable byte slice. |
| stride | Bytes from the start of one row to the start of the next. |
| device space | The coordinates of a surface: x grows right, y grows down, one unit is one pixel. |
| font space | The coordinates `text-core` reports: y grows up, one unit is one font unit. |
| run scale | `Run::scale` of `text-core`: style size divided by the face's units-per-em. |
| contour | A closed sequence of points; a font states it as line and curve segments. |
| flattening | Replacing the curve segments of a contour by line segments. |
| tolerance | The largest distance in device space between a curve and the polyline that replaces it. |
| coverage | The fraction of one pixel that filled contours cover, in `[0, 1]`. |
| alpha | The weight a source pixel carries in a blend, in `[0, 1]`. |
| linear light | A colour value proportional to light energy. |
| display space | A colour value encoded by the transfer function, which is what a panel is driven with. |
| transfer function | `display = linear^(1/γ)` and `linear = display^γ`, for the one value γ of D-175 and D-183. |
| gamma context | The tables that evaluate the transfer function, built once from γ. |
| premultiplied | A colour value already multiplied by its own alpha. |
| cell | One pixel column of one scanline, holding the two accumulated numbers of R3. |
| edge | One line segment of a flattened contour, in device space. |
| subpixel position | One of the four quantized horizontal glyph origins within a pixel (D-174). |
| clip mask | An `A8` surface that weights every write of the operations under it. |
| group | An offscreen `Rgba16` surface that `Group` opens and `Compose` consumes. |
| paint stream | The flat list of `PaintOp` that `Colr::paint` of `text-core` writes. |
| glyph cache | Coverage stored against the key of D-184, in caller-owned bytes. |
| ring | The caller's cache bytes, written in order and reclaimed in the same order. |
| golden image | The exact bytes of a surface, checked in as a test vector. |
| gate | A required complete test suite, with no skipped failing cases. |

## 18.2 Goal

1. The compositor draws a `LayoutView` through one call and obtains the
   same bytes on every machine, in debug and in release.
2. A colour glyph of `fonts/noto/emoji/Noto-COLRv1.ttf` reaches pixels:
   three gradients, three extend modes, twenty-eight composite modes, the
   clip stack and the group stack.
3. Every test runs on the host against a `Vec` surface, without a
   display, a window, or a running AuDHSOS system.
4. `text-raster` reads no font table, no setting and no file: `text-core`
   gives it outlines and paint streams, the caller gives it the rest.
5. R1–R13 add no allocation, no external crate, no C, and no `unsafe`.

## 18.3 What is already built

| Component | Location | Decided in |
|-----------|----------|------------|
| Outline decoding, TrueType | `crates/text-core/src/glyf.rs:150` | D-159, D-161 |
| Outline decoding, CFF and CFF2 | `crates/text-core/src/cff/type2.rs:33` | D-157, D-167 |
| Paint stream, clip box, boundedness, ink extents | `crates/text-core/src/colr/mod.rs:220` | D-173, D-176 |
| Sine and cosine in half-turns | `crates/text-core/src/colr/trig.rs:31` | D-159 |
| Q32.32 arithmetic with checked widening | `crates/text-core/src/fixed.rs:15` | D-159 |
| Positioned glyphs, lines, runs, cluster boxes | `crates/text-core/src/layout/mod.rs:146` | D-162, D-171 |
| Opaque display surfaces and the two channel orders | `crates/gfx/src/surface.rs:63` | D-29 |
| Host test and coverage tooling | `docs/06-testing-strategy.md:8` | D-23 |

| Reference group | Location | Decided in |
|-----------------|----------|------------|
| COLR rendering algorithm and composite modes | `docs/microsoft/colr.html:3094` | D-154 |
| Gradient interpolation in linear light | `docs/microsoft/cpal.html:985` | D-154 |
| Thirteen compositing operators, sixteen blend modes | `docs/w3c/compositing-1.html:1635` | D-124, kept for D-180 |

## 18.4 What is missing

| Component | Why required |
|-----------|--------------|
| A surface with an alpha channel | `gfx::Surface` is opaque and carries damage tracking; a group of R11 needs alpha and no damage. |
| Flattening with a tolerance | `text-demo` uses twelve steps per curve, which is visibly angular at large sizes and wasteful at small ones. |
| Antialiased coverage | Nothing in the workspace converts a contour into a coverage value. |
| Gamma tables | D-175 names the owner of γ and no code evaluates the transfer function. |
| A glyph cache | D-174 fixes the four subpixel positions of its key and no storage holds them. |
| An inverse tangent and a square root | A sweep gradient and a radial gradient need them; `colr::trig` has neither. |
| The twenty-eight composite modes | `text-demo` draws the four non-separable ones as source-over. |

## 18.5 Decision D1: the numeric contract (D-177)

**Decision:** `text-raster` is deterministic the way `text-core` is. Every
coordinate, coverage value, colour channel and transcendental of the
product code is `Fixed` Q32.32 or a narrower integer. No product path
contains an `f32` or an `f64`. A test may evaluate a formula the
specification states in real arithmetic with the host's floating point
and compare to a tolerance, because what it then checks is a bound and
not a bit pattern; a golden image may not.

1. Reason: one numeric contract for the whole stack, because the same
   rounding rule of D-159 then governs a glyph origin, an advance and a
   coverage value, and a disagreement between layout and drawing cannot
   come from the arithmetic.
2. Reason: `f64` addition, subtraction, multiplication, division and
   square root are exact under IEEE-754, but the inverse tangent and the
   power this crate needs are not specified bit for bit, so a golden
   image would assert the host's libm rather than this crate.
3. Reason: an integer implementation states its own error bound, which a
   test can check, whereas a transcendental of the host states none.

**Option not taken:** `f64`, which is easier to write and faster to run,
and whose cost is that a golden image can assert only a tolerance.

What a golden-image test may then assert: the exact bytes of the surface.
A golden image is a gate under this decision, on every host, in debug and
in release, and a change to its bytes is a change to the rendering and is
reviewed as one. The one floating-point reference in the crate is the
twenty-eight mixing formulas of R11, which
`docs/w3c/compositing-1.html:1832` and `docs/w3c/compositing-1.html:1967`
state in real arithmetic; the test compares the crate's integer answer to
them within a two-hundredth of a channel.

## 18.6 Decision D2: surface formats (D-178)

**Decision:** a surface carries one of four formats. Alpha is
premultiplied. Every blend of this crate runs on linear-light values.

| Format | Bytes per pixel | Contents | Used by |
|--------|-----------------|----------|---------|
| `A8` | 1 | Coverage or alpha, linear, `0` to `255`. | R3 output, the clip mask of R10, the glyph cache of R7 |
| `Rgba16` | 8 | Red, green, blue, alpha, little-endian `u16` each, premultiplied, linear light. | Every group of R11; every colour glyph |
| `Rgbx8888` | 4 | Red, green, blue, one unused byte, opaque, display space. | The compositor's surface |
| `Bgrx8888` | 4 | Blue, green, red, one unused byte, opaque, display space. | The compositor's surface |

1. Reason: `docs/microsoft/cpal.html:985` requires a COLR gradient to
   interpolate on linear-light values with alpha premultiplied into each
   component, so the format every gradient is written into holds exactly
   that.
2. Reason: the same page states that greater than eight-bit precision is
   required in the linearization, the premultiply and the interpolation.
   At γ = 2.2 an eight-bit linear channel maps the first fifteen display
   codes onto zero and leaves 184 distinct levels of 256; a sixteen-bit
   one leaves 255, and the one pair it cannot separate, the two darkest
   codes, R5 separates by one level.
3. Reason: premultiplied is the form the general Porter-Duff equation of
   `docs/w3c/compositing-1.html:1635` is written in, so an operator is a
   multiply-add per channel and no division.
4. Reason: the two display formats are the two `gfx::PixelFormat` values
   of D-29, so the compositor's framebuffer is a destination of this
   crate without a conversion pass.

**Option not taken:** one display-space premultiplied format for
everything, whose cost is an unpremultiply, a decode, an encode and a
re-premultiply per blended pixel, and banding in the darks where an
antialiased edge lives.

A display-space destination is opaque, so drawing onto it decodes three
bytes, blends, and encodes three bytes, and never divides by an alpha.
`gfx::Surface` is not reused: it has no alpha channel, it tracks damage
this crate has no use for, and it sits a layer above `text-core`.

## 18.7 Decision D3: allocation (D-179)

**Decision:** `text-raster` is a `no_std`, `forbid(unsafe_code)` logic
crate that allocates nothing and has no `alloc` feature. The caller owns
the surface bytes, the edge buffer, the cell buffer, the clip mask
bytes, the group bytes, the paint stream, the colour stops, the outline
scratch and the glyph cache. Every entry point reports capacity
exhaustion as a typed error.

1. Reason: the glyph cache cannot allocate, because its whole purpose is
   a bounded amount of memory and a bounded eviction rule, and an
   allocator would replace both with the allocator's behaviour.
2. Reason: `text-core` writes into caller-owned buffers (D-158), and one
   rule across the two crates means the compositor provisions once.
3. Reason: no program of this system uses `alloc` (D-89), so the
   convenience layer D-164 gives `text-core` has no consumer here.

**Option not taken:** an `alloc` layer mirroring `text-core`'s, whose
cost is a second code path with no caller and a cache whose bound is the
heap.

## 18.8 Decision D4: what is refused (D-180)

**Decision:** four things are refused. A refusal is stated, not omitted,
and each names what is drawn instead.

| Refused | What is drawn instead | Reason |
|---------|-----------------------|--------|
| Hinting | The outline as the font states it, scaled. | D-174 already refuses it; no outline is fitted to the grid. |
| Subpixel antialiasing over RGB stripes | Grayscale coverage, one value per pixel. | The result depends on the physical stripe order of the panel, which `server-display` does not report; the per-channel filter needs a transfer function per channel, which the one value of D-175 does not describe; and the cache key of D-184 would carry the stripe order. |
| Vertical layout | Nothing: this crate has no such input. | `text-core` refuses vertical layout (D-160) and reports no vertical origin and no vertical advance, so no run reaches this crate with one. |
| Dilation for faux-bold | The face at the weight the style asked for. | Dilation changes the ink `text-core` reported, so `measure` and the drawing would disagree; D-162 selects the `wght` instance instead. |

The four non-separable blend modes are **not** refused. Hue, Saturation,
Color and Luminosity are implemented from the auxiliary functions of
`docs/w3c/compositing-1.html:1967`, because a colour glyph that names one
and receives source-over is drawn wrongly and silently.

## 18.9 Decision D5: the flattening tolerance (D-181)

**Decision:** a curve is flattened to a maximum deviation of **one eighth
of a device pixel**, measured in device space after the glyph transform.
The segment count is computed from the control points; it is not fixed.

For a quadratic segment with control points `p0`, `p1`, `p2`, let
`d = |p0 - 2·p1 + p2|` in device units. For a cubic with `p0`, `p1`,
`p2`, `p3`, let `d = max(|p0 - 2·p1 + p2|, |p1 - 2·p2 + p3|)`. The
segment count is

| Segment | Bound on the chord deviation | Count |
|---------|------------------------------|-------|
| Quadratic | `d / (4·n²)` | `n = ceil(sqrt(d / (4·tol)))` |
| Cubic | `3·d / (4·n²)` | `n = ceil(sqrt(3·d / (4·tol)))` |

Both bounds come from `|B(t) - chord| ≤ h²·max|B''| / 8` over a parameter
interval of length `h = 1/n`, with `|B''| = 2·d` for a quadratic and
`|B''| ≤ 6·d` for a cubic.

1. Reason: `d` is measured after the transform, so `n` scales as the
   square root of the run scale: a glyph at four times the size gets
   twice the segments, which is what a constant deviation costs.
2. Reason: one eighth of a pixel is half the quantization step of R4,
   so flattening contributes less positional error than the subpixel
   quantization the same glyph already carries.
3. Reason: the count needs one square root, which R8 writes for the
   radial gradient anyway.

**Option not taken:** a fixed step count, which `text-demo` uses at
twelve. Its cost is both directions at once: visible facets on a glyph
above about 100 pixels, and eleven wasted segments on a glyph at 10.

`n` is clamped to **256**. A segment whose `d` exceeds `4·256²·tol`,
which is 32768 device units for a quadratic, is flattened at the clamp
and its deviation then exceeds the tolerance. The clamp bounds the work
a hostile transform can ask for; no glyph of a display reaches it.

## 18.10 Decision D6: how coverage is computed (D-182)

**Decision:** coverage is computed by exact area accumulation over a
scanline, with the non-zero winding rule, in integer arithmetic. There is
no supersampling.

1. Reason: exact area is the analytic answer for one edge crossing one
   pixel, so it has no sampling error to state, whereas `k×k`
   supersampling quantizes coverage to `k²+1` levels and shows steps on a
   near-horizontal stem.
2. Reason: the work is proportional to the cells the edges touch, not to
   `k²` times the area, so a large glyph costs its perimeter and not its
   interior.
3. Reason: the accumulator is two integers per cell and the caller
   provisions one row of them, so the memory is `O(width)` and not
   `O(width · k)`.

**Option not taken:** supersampling, which is shorter to write and whose
cost is the quantization above and a memory or time factor of `k²`.

The rule, stated once: each edge deposits into every cell it crosses a
signed `cover`, the height of the edge inside that cell, and a signed
`area`, twice the area between the edge and the right border of the
cell. A left-to-right sweep of one row accumulates `cover` into a running
winding number, and the coverage of a pixel is that winding number
combined with the cell's `area`, taken as `min(|value|, 1)`, which is the
non-zero rule.

Malformed geometry is font data and is drawn, not rejected: a
self-intersecting contour is what the non-zero rule is defined for, an
unclosed contour is closed by an edge from its last point to its first,
a contour of fewer than two points deposits nothing, and a horizontal
edge deposits nothing because its `cover` is zero. None of the four
panics and none returns an error.

## 18.11 Decision D7: where the gamma value enters (D-183)

**Decision:** every blend of this crate runs on linear-light values.
Display-space channels are decoded by `x^γ` on the way in and encoded by
`x^(1/γ)` on the way out; coverage becomes alpha unchanged. The value γ
reaches the crate as a gamma context the caller builds and passes per
call. The crate reads it from nowhere. **This supersedes D-175 in its
mechanism.** Its owner, `server-display`; its default, 2.2; and its
prohibition on this crate reading a setting are unchanged.

1. Reason: D-175 gives as its own first reason that uncorrected coverage
   composites light text on a dark background heavier than dark text on a
   light one, and no mapping of coverage alone fixes that. Take γ = 2.2,
   coverage 0.5, and a blend performed in display space. Black on white
   needs an output of `0.5^(1/2.2) = 0.73`, which requires an alpha of
   0.27; white on black needs the same 0.73, which requires an alpha of
   0.73. One coverage value would have to become two alphas. Blending in
   linear light with alpha equal to coverage gives 0.73 in both
   directions, which is the area average the coverage states.
2. Reason: `docs/microsoft/cpal.html:985` requires a COLR gradient to
   interpolate on linear-light values, so the transfer function is
   already at this boundary and applying it once serves both.
3. Reason: the same page states that alpha is on a linear scale and needs
   no linearization, so correcting coverage would correct a quantity the
   format says is already linear.

**Option not taken:** correcting coverage and blending in display space,
which is what D-175 read literally describes. Its cost is the asymmetry
D-175 was written to remove.

γ is a pure power, not the piecewise curve of sRGB. `server-display`
reports one number and this crate raises to it; the piecewise form is not
used and would need a second number the display protocol does not carry.

## 18.12 Decision D8: the glyph cache key, eviction and bound (D-184)

**Decision:** a cache entry is keyed by generation, face index, glyph
identifier, size, variation coordinates and subpixel position. The bytes
are a ring the caller owns and eviction is first-in, first-out.

The key, in the order it is compared:

| Field | Type | Why it is in the key |
|-------|------|----------------------|
| generation | `u64` | D-162: one immutable font-set snapshot; a changed generation invalidates every entry. |
| face | `u32` | The ordered index in the role chain; two faces share glyph numbers. |
| glyph | `u16` | The face-local glyph. |
| size | `Fixed` | The run scale; the same glyph at two sizes is two shapes. |
| coordinates | `[Fixed]` | D-162: a run carries its variation coordinates; two instances are two shapes. |
| subpixel | `u8`, `0` to `3` | D-174: four horizontal positions per pixel. |

1. Reason: the ring reclaims exactly the bytes a new entry needs, in
   allocation order, in time proportional to the entries it evicts, and a
   byte arena with entries of different sizes cannot free a middle range
   without compaction.
2. Reason: a changed generation is answered by resetting the ring, which
   is `O(1)` and needs no sweep.
3. Reason: the coordinates are stored in the ring beside the coverage and
   compared on a candidate hit, so a digest collision cannot return
   another instance's pixels.

**Option not taken:** least recently used, which needs either a
compacting arena, at a cost proportional to the bytes per insertion, or
fixed-size slots, which waste the difference on every glyph below the
slot size.

The memory bound is exactly two numbers the caller states: the length of
the ring in bytes, and the number of entry records. Nothing else grows.
An entry larger than the ring is not cached and its glyph is drawn
directly, every time.

## 18.13 The order of the steps

| Step | Status | Depends on | Size |
|------|--------|------------|------|
| R1 the surface | implemented | D1–D4 | S |
| R2 flattening | implemented | R1 acceptance, R8 | M |
| R3 coverage | implemented | R2 | L |
| R4 subpixel positioning | implemented | R3 | S |
| R5 gamma | implemented | R1 | M |
| R6 monochrome glyphs | implemented | R4, R5 | M |
| R7 the glyph cache | implemented | R6 | M |
| R8 the transcendentals | implemented | D1 | M |
| R9 gradients | implemented | R5, R8 | L |
| R10 the clip stack | implemented | R3 | M |
| R11 groups and composition | implemented | R5, R10 | L |
| R12 the paint stream | implemented | R9, R11 | M |
| R13 drawing a `LayoutView` | implemented | R7, R12 | M |

R8 depends on D1 alone and on no other step, so it is written where it is
first needed, which is the segment count of R2.

## 18.14 R1: the surface

**Status:** implemented and accepted (2026-09-18).
**Depends on:** decisions D1–D4 above.
**Size:** S.
**Needs:** the four formats of D2; the opaque surface of
`crates/gfx/src/surface.rs:63` for the two display channel orders.
**Does:** validate dimensions, stride and slice length; give
bounds-checked access to one pixel and to one row; clear a surface.
**Produces:** `crates/text-raster`, `Surface`, `Format`, `Texel`,
`RasterError`.
**Done when:** host tests over `Vec` surfaces construct every format,
accept an exact-length and a longer slice, and reject a zero width, a
zero height, a stride below the row length, a row whose bytes leave
`u32`, and a slice shorter than `height · stride`; read and write every
corner pixel; get `None` and `Err` one past every edge and at `u32::MAX`;
find the padding bytes of every row untouched after a full clear and a
full write; round-trip every `u8` through `A8` and the channel extremes
through `Rgba16`; read the channel order of `Rgbx8888` and `Bgrx8888`
literally; and write byte-identical results into two independently
allocated `Vec`s. The crate passes strict lints, the `no_std` build and
the bare target; report the results before R2.

Access is by texel, which is the stored value of the format and not a
colour: `Texel::Coverage(u8)` for `A8`, `Texel::Linear([u16; 4])` for
`Rgba16`, and `Texel::Display([u8; 3])` for the two display formats.
A `set_texel` whose texel does not match the surface's format returns
`RasterError::Format`. The linear reading of a display texel arrives with
R5, which is where the gamma context is defined.

`stride` is in bytes, not in pixels, because the four formats have four
pixel sizes and a caller who computes one number computes it once. A
surface never writes the bytes of a row past its last visible column.

`RasterError` is the one error type of the crate,
`docs/05-code-organization.md:377`. R1 defines the six variants a surface
needs; a later step adds the variant it names, `BufferTooSmall` with R2,
`Gamma` with R5, `Domain` with R8, `Stream` with R10 and `Face` with R13.

Complexity: `O(1)` per texel, `O(width · height)` for a clear, `O(1)`
extra storage.

## 18.15 R2: flattening

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R1 acceptance, and `sqrt` of R8.
**Size:** M.
**Needs:** D5; `sqrt` of R8 for the segment count; `Glyf::outline_instance` at
`crates/text-core/src/glyf.rs:180` for quadratic segments; the `Command`
stream of `crates/text-core/src/cff/type2.rs:33` for cubic segments;
`Fixed::checked_mul` for the transform.
**Does:** transform a decoded outline from font space into device space
and replace every curve segment by line segments within the tolerance.
**Produces:** `flatten_glyf`, `flatten_cff`, `Edge`, `Transform`.
**Done when:** host tests check the segment count of a quadratic and a
cubic against the closed form of D5 at the boundary where the count
changes and one unit either side; check that the largest perpendicular
distance from the curve to the polyline, sampled sixteen times inside
every segment, is at most one eighth of a pixel for a quarter circle
stated as one quadratic and as one cubic, at 8, 16, 64 and 256 pixels;
check that the count doubles when the size quadruples; check that a
second difference beyond the clamp gives 256; check that a contour of one
point and a curve whose control points all coincide each produce no edge;
check that a contour closes without a closing command and that two
contours both close; check that contour ends which do not partition the
points are refused; and check that the same outline flattened twice into
two buffers gives identical edges. A buffer too small returns
`RasterError::BufferTooSmall` and leaves the buffer unspecified.

A segment whose two endpoints coincide is dropped: it encloses no area
and deposits no coverage, so keeping it would cost the caller a slot and
change no pixel.

The transform composes the run scale, the glyph origin of R4, and the
`Affine` a `PaintOp::Fill` carries, and flips y, because font space
grows up and device space grows down.

Complexity: `O(P + Σ nᵢ)` for `P` points and `nᵢ` segments per curve,
`O(1)` extra storage beyond the caller's edge buffer.

## 18.16 R3: coverage

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R2.
**Size:** L.
**Needs:** D6; the edges of R2; an `A8` surface from R1.
**Does:** accumulate the signed cover and area of every edge into one row
of cells at a time and write the non-zero coverage of each pixel.
**Produces:** `fill`, `Cell`, `EdgeList`.
**Done when:** host tests assert coverage 255 inside and 0 outside an
axis-aligned rectangle on integer boundaries; assert the exact coverage
of a rectangle inset by a quarter, a half and three quarters of a pixel
on each of the four sides; assert that a triangle's coverage sums to its
area within one unit per pixel; assert that two nested contours of the
same direction give 255 in the overlap and two of opposite direction give
0, which is the non-zero rule; assert that a self-intersecting contour,
an unclosed contour, a contour of one point, a contour of two identical
points and a horizontal-only contour each return `Ok` and panic on
neither; assert that no pixel outside the reported box is written;
assert that no coverage value exceeds 255; and assert that the same edge
list filled twice gives identical bytes. A fuzz target drives arbitrary
edge coordinates, including the extremes of `Fixed`, through the filler.

Edges are ordered by their top y with `slice::sort_unstable_by_key`,
which is in-place and allocates nothing. The key is a total order over
the edge's own four numbers — the top, the bottom, the smaller x and the
larger — so two edges the key cannot separate are interchangeable and the
result does not depend on the sort's stability. A row then activates
every edge whose top it has reached, retires every edge whose bottom it
has passed by swapping it to the front of the active range, and sweeps
its cells.

Complexity: `O(E log E + C)` time for `E` edges and `C` cells touched,
`O(width + E)` storage, all of it the caller's.

## 18.17 R4: subpixel positioning

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R3.
**Size:** S.
**Needs:** D-174.
**Does:** quantize the fractional glyph origin `text-core` produced into
one of four horizontal positions and one whole vertical pixel.
**Produces:** `Origin`, `quantize`.
**Done when:** host tests check the four positions and the pixel they
belong to for origins at every eighth of a pixel across two whole pixels,
for positive and negative x; check that a tie rounds to even, in both
signs, as D-159 requires; check that quantizing changes no advance, no
line break and no box, by laying out a line and asserting the
`LayoutView` before and after is byte-identical; and check that the four
positions of one glyph give four coverage buffers whose ink boxes differ
by at most one pixel.

The rule: for a device x in `Fixed`, let `q` be `x · 4` rounded to
nearest with ties to even. The pixel column is `q.div_euclid(4)` and the
subpixel position is `q.rem_euclid(4)`. For a device y, the pixel row is
`y` rounded to nearest with ties to even. This is the only place in the
stack where a position becomes an integer.

Complexity: `O(1)`.

## 18.18 R5: gamma

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R1.
**Size:** M.
**Needs:** D7; `pow` of R8.
**Does:** build the two tables of one γ value and convert a display texel
to linear and back.
**Produces:** `Gamma`, `Gamma::decode`, `Gamma::encode`.
**Done when:** host tests check that `encode(decode(v)) == v` for all 256
display values at γ = 2.2, at γ = 1.0 and at γ = 1.8; check that γ = 1.0
makes both tables the identity; check that `decode` is strictly
increasing over the 256 entries; check that the encode table is the
inverse of the decode table at every one of its entries, so no display
value is unreachable; check that an out-of-range γ, zero or negative,
returns `RasterError::Gamma`; and check that two contexts built from the
same γ hold identical tables.

`Gamma` holds two tables of 256 `u16` each. `decode` maps a display byte
to its linear value. `edges` holds, for each display code, the linear
midpoint between that code and the next, so encoding is a binary search
for the code nearest in linear light, which is the correctly rounded
answer and needs no wider table. Both are built once, by the `pow` of R8,
in `O(256)` operations; a blend then costs a lookup or eight comparisons
and no power. The context is passed per call and holds no state a draw
changes.

`decode` is strictly increasing by construction. At γ = 2.2 the second
display code is 0.33 of 65535 and rounds onto the first, so one level is
added wherever a code would otherwise collide with the code below it. The
adjustment is a few parts in 65535, far below one display code anywhere,
and without it a code would have no linear value of its own and could not
be encoded back.

Complexity: `O(1)` to decode, `O(log 256)` to encode, `O(256)` to
construct; 1024 bytes per context, on the caller's stack or in the
caller's storage.

## 18.19 R6: monochrome glyphs

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R4, R5.
**Size:** M.
**Needs:** the coverage of R3; the gamma context of R5.
**Does:** composite an `A8` coverage buffer with one text colour onto a
surface of any format, source-over, in linear light.
**Produces:** `blend_mono`.
**Done when:** host tests check that coverage 0 leaves the destination
unchanged and coverage 255 with an opaque colour replaces it exactly, on
each of the four formats; check the middle value against the linear-light
average, computed independently; check that black text on white and white
text on black at coverage 128 give the same distance from their
backgrounds, which is what D7 exists for and what an uncorrected blend
fails; check that a coverage buffer placed so that it overhangs each of
the four edges writes only inside the surface; and check that the same
draw twice gives identical bytes.

Complexity: `O(w · h)` for the coverage buffer's dimensions, `O(1)`
storage.

## 18.20 R7: the glyph cache

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R6.
**Size:** M.
**Needs:** D8; the key fields D-174 and D-162 fix.
**Does:** store the coverage of one glyph at one key in the caller's ring
and return it on a later request with the same key.
**Produces:** `GlyphCache`, `CacheKey`, `CacheEntry`.
**Done when:** host tests check that a hit returns bytes identical to a
fresh rasterization, for every one of the four subpixel positions; check
that a changed generation, face, glyph, size, coordinate or subpixel
position misses; check that two coordinate slices with the same digest
and different values do not collide, using a constructed pair; check that
inserting more bytes than the ring holds evicts in allocation order and
that the evicted entries then miss; check that an entry larger than the
ring is never stored and its glyph still draws; check that a full ring
and a zero-length ring both draw correctly; and check that a sequence of
draws with and without the cache gives byte-identical surfaces, which is
the acceptance D-162's R2 row states.

A ring block holds the coordinate bytes and then the coverage bytes; a
candidate hit compares the digest, then the axis count, then the
coordinate bytes, then returns the coverage. The digest is FNV-1a-64 over
the little-endian bits of the coordinates.

Complexity: `O(1)` expected for a lookup over the entry table, `O(k)` for
an insertion that evicts `k` entries, `O(1)` for a generation change.

## 18.21 R8: the transcendentals

**Status:** implemented and accepted (2026-09-18).
**Depends on:** D1.
**Size:** M.
**Needs:** the contract of `crates/text-core/src/colr/trig.rs:31`: a
stated error bound, a test against it, and no floating point.
**Does:** compute a square root, an inverse tangent in half-turns, and a
power, in `Fixed`.
**Produces:** `sqrt`, `atan2_turns`, `pow`.
**Done when:** each of the three has its bound checked. `sqrt` is
correctly rounded, so the test asserts that the truncated root `t` the
algorithm reaches satisfies `t² ≤ x < (t+1)²` and that the returned value
is whichever of `t` and `t+1` is nearer to the real root, ties to even,
at 10,000 values spanning the range and at every power of two; a negative
argument returns `RasterError::Domain`. `atan2_turns` is within
`2^-28` of the real value, which the test checks by feeding its result to
`text_core::colr::trig::sin_cos` and asserting the direction returned
agrees with `(y, x)` to the combined bound, over a dense sweep of both
signs of both arguments and over the four axis directions and the eight
octant boundaries literally; `atan2_turns(0, 0)` returns zero. `pow` is
within `2^-24` over base `(0, 1]` and exponent `[1/8, 8]`, which the test
checks against the identities `pow(x, 1) == x`, `pow(x, 2) == x·x` to the
bound, and `pow(pow(x, γ), 1/γ) == x` to the bound.

This is the only genuinely new numeric code in the crate.
`sqrt` is bit-by-bit restoring over `i128`, 64 iterations, with one
comparison at the end that rounds to nearest, ties to even, as D-159
requires. `atan2_turns` reduces to an octant by sign and magnitude
comparison, evaluates the odd Taylor series of `atan` on
`[0, tan(π/8)]`, and divides by π to reach half-turns. `pow` is
`exp2(exponent · log2(base))`, with `log2` by the classical squaring
loop, one fractional bit per iteration, and `exp2` by an integer shift
plus the Taylor series of `2^f` on `[0, 1)`.

Complexity: `O(1)` each, with a fixed iteration count: 64 for `sqrt`, 32
for `log2`, and a fixed series length for `exp2` and for `atan`.

## 18.22 R9: gradients

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R5, R8.
**Size:** L.
**Needs:** `Fill` of `crates/text-core/src/colr/paint.rs:148`; the
interpolation rule of `docs/microsoft/cpal.html:985`; the gradient
constructions of `docs/microsoft/colr.html:1714`.
**Does:** evaluate a colour line at a device pixel for the three gradient
shapes and the three extend modes.
**Produces:** `gradient`, `ColorLineEval`.
**Done when:** host tests check the linear gradient against the p₃
construction at points on and off the gradient line, including a
degenerate `p0 = p1` and a `p2` on the line `p0p1`, which makes the
gradient undefined and draws nothing; check the radial gradient at the
two circle centres, between them, outside both, and in the cone of a
two-circle form where no root has a non-negative interpolated radius,
which draws nothing; check the sweep gradient at the four axis
directions, at the centre, and across the wrap from `1` to `-1`
half-turns; check all three extend modes at `-2.5`, `-0.5`, `0.5` and
`2.5` of the colour line; check a colour line of one stop, of two stops
at the same offset, and of stops whose offsets are equal at the ends;
check that interpolation happens on linear-light premultiplied values by
comparing the midpoint of a black-to-white line against the independently
computed value, which differs from the display-space midpoint by about
58 of 255; and check that the same gradient evaluated twice gives identical
bytes.

Every gradient is evaluated in the fill's own coordinate space, reached
by inverting the `Affine` the `PaintOp::Fill` carries, composed with the
glyph's own. A singular `Affine`, whose determinant is zero, draws
nothing and returns `Ok`.

`Gradient::new` takes the inverse once and normalizes the geometry: every
coordinate is divided by the smallest power of two that brings it to 128
or below, which is exact in the exponent and rounds the mantissa once.
The parameter of all three shapes is unchanged by one scale applied to
every input, and the normalization is what keeps the products of the
two-circle form — `b` squared and `a` times `c` — inside Q32.32 for a
gradient stated in the font units of a 2048-unit em.

The linear gradient uses the p₃ construction: p₃ is the orthogonal
projection of p₀p₁ onto the line through p₀ perpendicular to p₀p₂, and
the gradient parameter of a point is its projection onto p₀p₃. A naive
projection onto p₀p₁ is wrong whenever p₂ is not perpendicular to p₀p₁
and is what `text-demo` would have to be corrected for.

The radial gradient is the two-circle form: for a point, the parameter
`t` solves the quadratic that places the point on the interpolated
circle, and the larger root whose interpolated radius
`r0 + t·(r1 - r0)` is non-negative is taken.

The sweep gradient takes the angle of the point about the centre with
`atan2_turns` of R8 and maps `[start, end]` in half-turns onto the colour
line.

Complexity: `O(1)` per pixel plus `O(log S)` for the binary search over
`S` stops.

## 18.23 R10: the clip stack

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R3.
**Size:** M.
**Needs:** `PaintOp::Clip` and `PaintOp::Unclip` of
`crates/text-core/src/colr/paint.rs:318`.
**Does:** intersect the current clip mask with the coverage of a glyph
outline under its transform, and restore the previous mask.
**Produces:** `ClipStack`.
**Done when:** host tests check that a `Clip` over a rectangle limits the
following fill to that rectangle exactly; check that two nested `Clip`
operations give the product of the two coverages, per pixel; check that
`Unclip` restores the mask the matching `Clip` narrowed, byte for byte;
check that an `Unclip` with no matching `Clip` returns
`RasterError::Stream` and writes nothing; check that a stack deeper than
the caller's mask storage returns `RasterError::BufferTooSmall`; and
check that a `Clip` on a glyph with no outline gives an empty mask, so
the operations under it draw nothing.

A mask is an `A8` plane of the glyph's box over the caller's bytes, one
per stack level, and the depth the caller's slice allows is the depth the
stream may reach. One box for every level is what makes a nested clip one
multiplication per pixel and a `Compose` one pass over two planes of
equal shape; bounding each level to its own outline's box would save
memory on a deep stack and cost an intersection on every access.

Complexity: `O(w · h)` per `Clip`, `O(1)` per `Unclip`, storage the
caller's mask slice.

## 18.24 R11: groups and composition

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R5, R10.
**Size:** L.
**Needs:** the thirteen operators of
`docs/w3c/compositing-1.html:1635`; the twelve separable blend modes of
`docs/w3c/compositing-1.html:1832`; the four non-separable ones and their
auxiliary functions at `docs/w3c/compositing-1.html:1967`; the rendering
algorithm of `docs/microsoft/colr.html:3332`.
**Does:** open an offscreen `Rgba16` surface for `Group`, combine the two
surfaces above with the mode of `Compose`, and draw the result onto the
surface below with source-over.
**Produces:** `composite`, and the group stack of R12.
**Done when:** each of the twenty-eight modes has a literal result vector
over a matrix of operands: opaque over opaque, opaque over transparent,
transparent over opaque, and two partial alphas; the thirteen operators
are checked against the general Porter-Duff equation with the `Fa` and
`Fb` of each; the twelve separable modes are checked against their
per-channel formulas; the four non-separable modes are checked against
`Lum`, `ClipColor`, `SetLum`, `Sat` and `SetSat` computed independently,
including the two clamping branches of `ClipColor` and the `Cmax == Cmin`
branch of `SetSat`; a `Compose` with fewer than two groups above it
returns `RasterError::Stream`; a group stack deeper than the caller's
storage returns `RasterError::BufferTooSmall`; the ink a surface held
before a `Compose` is still under the result, which is what the rendering
algorithm requires; and the same stream composed twice gives identical
bytes.

`composite` is a pure function of a mode and two premultiplied pixels, so
every one of the twenty-eight modes is checked without a surface. The
stack of offscreen planes belongs to R12, which is where the stream that
opens and closes them is read.

A blend mode runs on unpremultiplied values, which
`docs/w3c/compositing-1.html:1785` requires, so the two operands are
divided by their alphas first; that division is the only one in a blend,
and an operand of zero alpha skips it and contributes its own colour.
The thirteen operators need no division, because the equation of
`docs/w3c/compositing-1.html:1635` is written on premultiplied values.

Complexity: `O(1)` per pixel per mode, `O(w · h)` per `Compose` over the
glyph's box, storage the caller's group slice.

## 18.25 R12: the whole paint stream

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R9, R11.
**Size:** M.
**Needs:** `Colr::paint` at `crates/text-core/src/colr/mod.rs:220`;
`Painted` at `crates/text-core/src/colr/mod.rs:102`.
**Does:** consume `PaintOp` in order and drive R6, R9, R10 and R11.
**Produces:** `draw_color_glyph`, `Bounds`, `PaintScratch`.
**Done when:** the five base glyphs of the COLRv1 fixture, a copy of
`crates/text-core/src/tests/fixtures/NotoEmoji-colr.ttf` under
`crates/text-raster/src/tests/fixtures/`, draw through the whole stack —
clips, groups, composites and gradients — and their surfaces are golden
images the test asserts byte for byte, which D1 allows;
`Painted::bounded` false draws nothing and returns `Ok`;
`ColorSource::Foreground` resolves to the caller's text colour; a stream
whose `Clip` and `Unclip` do not match returns `RasterError::Stream`, and
so does one that leaves a bracket open at its end; a stack deeper than
the caller's storage returns `RasterError::BufferTooSmall`; a `Compose`
with fewer than two groups above it returns `RasterError::Stream`; the
ink a surface held before a `Compose` is still under the result; an empty
stream writes nothing; and the same stream drawn twice gives identical
bytes.

Complexity: `O(Σ over the operations)` with each operation's own cost
above; the stream carries no graph, so this step meets no cycle, which is
what D-173 bought.

## 18.26 R13: drawing a `LayoutView`

**Status:** implemented and accepted (2026-09-18).
**Depends on:** R7, R12.
**Size:** M.
**Needs:** `LayoutView` at `crates/text-core/src/layout/mod.rs:146`;
`Run::scale` at `crates/text-core/src/resolve/mod.rs:93`.
**Does:** walk lines and glyphs, apply each run's scale and variation
coordinates, quantize each origin through R4, and dispatch each glyph to
the monochrome path or the colour path.
**Produces:** `draw`, `Context`, `OutlineScratch`, `outline_edges`.
**Done when:** a line of Latin text at 16 pixels, a line of Arabic, a
mixed bidirectional line and a wrapped paragraph each draw into a `Vec`
surface and are golden images the test asserts byte for byte; the ink of
every drawing lies inside the box `measure` reported, which is checked
pixel by pixel; a glyph of a face with a `COLR` table takes the colour
path; a run whose face index is out of range returns `RasterError::Face`;
a coverage buffer too small returns `RasterError::BufferTooSmall`; a
surface too small for the layout draws the part that fits and writes
nothing outside; an empty string draws nothing; a larger size inks more
pixels; the four subpixel positions of D-174 each draw different pixels;
and drawing the same view twice, with and without a glyph cache, gives
identical bytes.

A line mixing text with an emoji is not among the golden images: the
COLRv1 fixture is subset to five base glyphs and maps no character, so no
string reaches them through `cmap`. The colour path's own golden images
are R12's, which drive the same code from the paint stream the fixture
does state.

This is the only entry point the compositor needs. It reads no font
table: `text-core` decodes the outline and resolves the paint stream, and
this step positions and fills.

A glyph's box is not known before its outline is flattened, so the edges
are produced against the pixel the origin sits in and the box is read off
them. The subpixel offset of R4 stays in that transform, because it is
what the four positions distinguish; only the whole pixels come off, and
they are added back when the coverage is composited. The box is grown by
two pixels on every side, so that the rounding of the box never clips a
stem.

Complexity: `O(G)` glyph dispatches plus each glyph's own cost, with a
cache hit costing `O(w · h)` of the glyph and no flattening and no fill.

## 18.27 Risks

| # | Risk | Effect | Reduction |
|---|------|--------|-----------|
| 1 | Hostile outline coordinates | Unbounded flattening or fill work | The clamp of D5 at 256 segments; the caller's edge and cell buffers return `BufferTooSmall`; the fuzz target of R3 drives the extremes of `Fixed`. |
| 2 | Malformed geometry | Panic | D6 states what each of four degenerate contours draws; every one has a test at R3; no indexing without a bound and no arithmetic without a check, which R7 of the safety policy already denies by lint. |
| 3 | The gamma mechanism of D-175 | Text of the two themes disagrees in weight | D7 supersedes the mechanism and states the arithmetic; R6's test compares black on white against white on black. |
| 4 | Golden images as a gate | A legitimate change fails the build | D1 makes the bytes reproducible, so a golden image changes only when the rendering changes; a changed image is reviewed as a rendering change. |
| 5 | Sixteen bits per channel per group | Memory a group costs | A group is bounded to the glyph's box, not the surface; the bound is the caller's slice and `BufferTooSmall` is a typed error. |
| 6 | The four non-separable blend modes | Wrong colours, drawn silently | D4 refuses to fall back to source-over; R11 checks all four against independently computed auxiliary functions. |
| 7 | Cache key omissions | A stale glyph drawn for a changed instance | D8 lists every key field and its reason; R7 checks a miss for each one and compares cached against uncached bytes. |
| 8 | `text-demo` mistaken for a reference | The shortcuts of the demo enter the crate | The demo is replaced, not ported; 18.4 lists each shortcut and the step that does not take it. |
