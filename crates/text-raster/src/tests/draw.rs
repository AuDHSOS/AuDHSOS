// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

//! R13: drawing a `LayoutView`, the one entry point the compositor needs.

use text_core::{
    Fixed, Font, FontSet, TextStyle, bidi,
    cff::Command,
    colr::{ColorStop, PaintOp},
    glyf,
    layout::{
        ClusterBox, LayoutBuffers, LayoutView, Line, PositionedGlyph, Workspace, layout_into,
    },
    resolve::Run,
    segment,
    shape::{Caret, Glyph as ShapedGlyph},
    variation::VariationPoint,
};

use crate::{
    Bounds, Cell, Context, Edge, Entry, Format, Gamma, GlyphCache, OutlineScratch, Paint,
    PaintScratch, RasterError, Surface, draw,
};

/// A licensed `DejaVu` subset with real outlines, the same file `text-core`
/// shapes against.
const TEXT: &[u8] = include_bytes!("fixtures/DejaVu-shaping.ttf");

/// The `COLRv1` build of Noto Color Emoji, subset to five base glyphs.
const EMOJI: &[u8] = include_bytes!("fixtures/NotoEmoji-colr.ttf");

/// Everything `layout_into` borrows.
struct Memory {
    bidi: Vec<bidi::Unit>,
    indices: Vec<usize>,
    levels: Vec<u8>,
    ranks: Vec<usize>,
    line_units: Vec<segment::LineUnit>,
    breaks: Vec<segment::LineBoundary>,
    runs: Vec<Run>,
    glyphs: Vec<ShapedGlyph>,
    line_glyphs: Vec<PositionedGlyph>,
    line_clusters: Vec<ClusterBox>,
    points: Vec<glyf::Point>,
    contours: Vec<usize>,
    variation: Vec<VariationPoint>,
    carets: Vec<Caret>,
}

/// What `layout_into` writes its result into.
struct Output {
    lines: Vec<Line>,
    glyphs: Vec<PositionedGlyph>,
    clusters: Vec<ClusterBox>,
}

impl Output {
    fn new() -> Self {
        Self {
            lines: vec![Line::default(); 64],
            glyphs: vec![PositionedGlyph::default(); 2048],
            clusters: vec![ClusterBox::default(); 512],
        }
    }

    fn buffers(&mut self) -> LayoutBuffers<'_> {
        LayoutBuffers {
            lines: &mut self.lines,
            glyphs: &mut self.glyphs,
            clusters: &mut self.clusters,
        }
    }
}

impl Memory {
    fn new() -> Self {
        Self {
            bidi: vec![bidi::Unit::default(); 512],
            indices: vec![0; 512],
            levels: vec![0; 512],
            ranks: vec![0; 512],
            line_units: vec![segment::LineUnit::default(); 512],
            breaks: vec![segment::LineBoundary::default(); 513],
            runs: vec![Run::default(); 512],
            glyphs: vec![ShapedGlyph::default(); 2048],
            line_glyphs: vec![PositionedGlyph::default(); 2048],
            line_clusters: vec![ClusterBox::default(); 512],
            points: vec![glyf::Point::default(); 4096],
            contours: vec![0; 512],
            variation: vec![VariationPoint::default(); 4096],
            carets: vec![Caret::Coordinate(Fixed::ZERO); 512],
        }
    }

    fn workspace(&mut self) -> Workspace<'_> {
        Workspace {
            bidi: &mut self.bidi,
            indices: &mut self.indices,
            levels: &mut self.levels,
            ranks: &mut self.ranks,
            line_units: &mut self.line_units,
            breaks: &mut self.breaks,
            runs: &mut self.runs,
            glyphs: &mut self.glyphs,
            line_glyphs: &mut self.line_glyphs,
            line_clusters: &mut self.line_clusters,
            points: &mut self.points,
            contours: &mut self.contours,
            variation: &mut self.variation,
            caret_values: &mut self.carets,
        }
    }
}

/// Everything one call of [`draw`] borrows.
struct Storage {
    coverage: Vec<u8>,
    ops: Vec<PaintOp>,
    stops: Vec<ColorStop>,
    edges: Vec<Edge>,
    cells: Vec<Cell>,
    clips: Vec<u8>,
    groups: Vec<u8>,
    points: Vec<glyf::Point>,
    contours: Vec<usize>,
    variation: Vec<VariationPoint>,
    commands: Vec<Command>,
}

impl Storage {
    fn new(plane: usize, levels: usize) -> Self {
        Self::with_coverage(plane, levels, 128 * 128)
    }

    fn with_coverage(plane: usize, levels: usize, coverage: usize) -> Self {
        Self {
            coverage: vec![0; coverage],
            ops: vec![PaintOp::default(); 512],
            stops: vec![ColorStop::default(); 512],
            edges: vec![Edge::default(); 16_384],
            cells: vec![Cell::default(); 1024],
            clips: vec![0; plane * levels],
            groups: vec![0; plane * levels * 8],
            points: vec![glyf::Point::default(); 4096],
            contours: vec![0; 512],
            variation: vec![VariationPoint::default(); 4096],
            commands: vec![Command::default(); 4096],
        }
    }

    fn context<'a, 'b>(
        &'a mut self,
        gamma: &'a Gamma,
        cache: Option<&'a mut GlyphCache<'b>>,
    ) -> Context<'a, 'b> {
        Context {
            gamma,
            palette: 0,
            foreground: Paint::opaque(255, 255, 255),
            cache,
            coverage: &mut self.coverage,
            ops: &mut self.ops,
            stops: &mut self.stops,
            paint: PaintScratch {
                edges: &mut self.edges,
                cells: &mut self.cells,
                clips: &mut self.clips,
                groups: &mut self.groups,
                outline: OutlineScratch {
                    points: &mut self.points,
                    contours: &mut self.contours,
                    variation: &mut self.variation,
                    commands: &mut self.commands,
                },
            },
        }
    }
}

/// Lay out `text` at `size` pixels and draw it into a surface of `width` by
/// `height`, returning the bytes and the box `measure` reported.
fn render(
    bytes: &'static [u8],
    text: &str,
    size: i32,
    width: u32,
    height: u32,
    wrap: Option<Fixed>,
    cache: bool,
) -> (Vec<u8>, (Fixed, Fixed)) {
    let fonts = [Font::parse(bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 3,
    };
    let style = TextStyle {
        size: Fixed::from_i32(size),
        ..TextStyle::default()
    };
    let mut memory = Memory::new();
    let mut output = Output::new();
    let mut surface = vec![0_u8; usize::try_from(width * height).unwrap() * 4];
    let mut storage = Storage::new(usize::try_from(width * height).unwrap(), 4);
    let gamma = Gamma::default_value().unwrap();
    let mut ring = vec![0_u8; 1 << 16];
    let mut entries = vec![Entry::default(); 128];
    let mut store = GlyphCache::new(&mut ring, &mut entries);
    let extent;
    {
        let mut buffers = output.buffers();
        let mut workspace = memory.workspace();
        let view = layout_into(&set, &style, text, wrap, &mut workspace, &mut buffers).unwrap();
        extent = (view.info.width, view.info.height);
        let mut destination =
            Surface::new(&mut surface, width, height, width * 4, Format::Rgbx8888).unwrap();
        destination.clear();
        let mut context = storage.context(&gamma, cache.then_some(&mut store));
        draw(
            &mut destination,
            &view,
            &fonts,
            (Fixed::from_i32(2), Fixed::ZERO),
            &mut context,
        )
        .unwrap();
    }
    (surface, extent)
}

/// The pixels that carry ink.
fn inked(bytes: &[u8]) -> usize {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel.iter().take(3).any(|byte| *byte != 0))
        .count()
}

#[test]
fn a_line_of_latin_text_reaches_pixels() {
    let (bytes, _) = render(TEXT, "AVAWA", 16, 96, 24, None, false);
    let count = inked(&bytes);
    assert!(count > 40, "only {count} pixels carry ink");
    assert!(count < 96 * 24, "the whole surface was inked");
    super::golden::check("latin", &bytes, 96, 24);
}

#[test]
fn a_line_of_arabic_text_reaches_pixels() {
    // The fixture covers two Arabic letters, which shape and join.
    let (bytes, _) = render(TEXT, "\u{645}\u{628}\u{645}", 20, 96, 32, None, false);
    assert!(inked(&bytes) > 20, "the Arabic line drew almost nothing");
    super::golden::check("arabic", &bytes, 96, 32);
}

#[test]
fn a_mixed_bidirectional_line_reaches_pixels() {
    let (bytes, _) = render(TEXT, "AV \u{645}\u{628} VA", 16, 128, 28, None, false);
    assert!(
        inked(&bytes) > 40,
        "the bidirectional line drew almost nothing"
    );
    super::golden::check("bidirectional", &bytes, 128, 28);
}

#[test]
fn the_ink_stays_inside_the_box_measure_reported() {
    let (bytes, extent) = render(TEXT, "AVAWA", 16, 128, 40, None, false);
    // The paragraph's origin is at `(2, 0)` and every glyph carries its own
    // baseline, so the ink lies inside the box `measure` reported, grown by
    // the flattening tolerance and the rounding of the box, which is under
    // two pixels.
    let right = 2 + (extent.0.bits() >> 32) + 2;
    let bottom = (extent.1.bits() >> 32) + 2;
    for (index, pixel) in bytes.as_chunks::<4>().0.iter().enumerate() {
        if pixel.iter().take(3).all(|byte| *byte == 0) {
            continue;
        }
        let x = i64::try_from(index % 128).unwrap();
        let y = i64::try_from(index / 128).unwrap();
        assert!(x <= right, "ink at x {x} past {right}");
        assert!(y <= bottom, "ink at y {y} past {bottom}");
    }
}

#[test]
fn an_empty_string_draws_nothing() {
    let (bytes, extent) = render(TEXT, "", 16, 32, 32, None, false);
    assert_eq!(inked(&bytes), 0);
    assert_eq!(extent, (Fixed::ZERO, Fixed::ZERO));
}

#[test]
fn a_wrapped_paragraph_draws_more_than_one_line() {
    let wide = render(TEXT, "AVA WAV AVA", 12, 160, 64, None, false).0;
    let narrow = render(
        TEXT,
        "AVA WAV AVA",
        12,
        160,
        64,
        Some(Fixed::from_i32(40)),
        false,
    )
    .0;
    assert_ne!(wide, narrow, "wrapping changed nothing");
    // The wrapped drawing reaches further down the surface.
    let lowest = |bytes: &[u8]| {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(_, pixel)| pixel.iter().take(3).any(|byte| *byte != 0))
            .map(|(index, _)| index / 160)
            .max()
            .unwrap_or(0)
    };
    assert!(lowest(&narrow) > lowest(&wide), "the text did not wrap");
    super::golden::check("wrapped", &narrow, 160, 64);
}

#[test]
fn a_colour_face_takes_the_colour_path() {
    // The emoji fixture maps no character of its own in this subset, so the
    // drawing goes through `notdef`; what this checks is that a face with a
    // `COLR` table draws without error and reaches pixels.
    let (bytes, _) = render(EMOJI, "A", 24, 64, 48, None, false);
    assert!(inked(&bytes) > 0, "the colour face drew nothing");
}

#[test]
fn the_cache_changes_no_pixel() {
    let plain = render(TEXT, "AVAWA VAV", 14, 128, 32, None, false).0;
    let cached = render(TEXT, "AVAWA VAV", 14, 128, 32, None, true).0;
    assert_eq!(plain, cached);
}

#[test]
fn the_same_view_draws_identically_twice() {
    let first = render(TEXT, "AVAWA", 16, 96, 24, None, false).0;
    let second = render(TEXT, "AVAWA", 16, 96, 24, None, false).0;
    assert_eq!(first, second);
}

#[test]
fn a_surface_too_small_draws_what_fits() {
    let (bytes, _) = render(TEXT, "AVAWA", 16, 8, 24, None, false);
    assert_eq!(bytes.len(), 8 * 24 * 4);
    assert!(inked(&bytes) > 0, "nothing of the first glyph fits");
}

#[test]
fn a_run_naming_a_face_the_caller_did_not_supply_is_refused() {
    let fonts = [Font::parse(TEXT).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 1,
    };
    let style = TextStyle {
        size: Fixed::from_i32(12),
        ..TextStyle::default()
    };
    let mut memory = Memory::new();
    let mut output = Output::new();
    let mut surface = vec![0_u8; 32 * 32 * 4];
    let mut storage = Storage::new(32 * 32, 2);
    let gamma = Gamma::default_value().unwrap();
    let mut buffers = output.buffers();
    let mut workspace = memory.workspace();
    let view = layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap();
    let mut destination = Surface::new(&mut surface, 32, 32, 128, Format::Rgbx8888).unwrap();
    let mut context = storage.context(&gamma, None);
    let none: [Font<'_>; 0] = [];
    assert_eq!(
        draw(
            &mut destination,
            &view,
            &none,
            (Fixed::ZERO, Fixed::ZERO),
            &mut context,
        )
        .unwrap_err(),
        RasterError::Face
    );
}

#[test]
fn a_coverage_buffer_too_small_is_refused() {
    let fonts = [Font::parse(TEXT).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 1,
    };
    let style = TextStyle {
        size: Fixed::from_i32(32),
        ..TextStyle::default()
    };
    let mut memory = Memory::new();
    let mut output = Output::new();
    let mut surface = vec![0_u8; 64 * 64 * 4];
    // One pixel of coverage cannot hold a glyph at thirty-two pixels.
    let mut storage = Storage::with_coverage(64 * 64, 1, 1);
    let gamma = Gamma::default_value().unwrap();
    let mut buffers = output.buffers();
    let mut workspace = memory.workspace();
    let view = layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap();
    let mut destination = Surface::new(&mut surface, 64, 64, 256, Format::Rgbx8888).unwrap();
    let mut context = storage.context(&gamma, None);
    assert_eq!(
        draw(
            &mut destination,
            &view,
            &fonts,
            (Fixed::ZERO, Fixed::ZERO),
            &mut context,
        )
        .unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn a_larger_size_inks_more_pixels() {
    let small = inked(&render(TEXT, "A", 12, 96, 64, None, false).0);
    let large = inked(&render(TEXT, "A", 32, 96, 64, None, false).0);
    assert!(large > small * 2, "{large} against {small}");
}

#[test]
fn the_four_subpixel_positions_move_the_glyph() {
    // Shifting the origin by a quarter of a pixel must change the coverage,
    // which is what the four positions of D-174 are for.
    let mut seen = Vec::new();
    for quarter in 0..4_i64 {
        let fonts = [Font::parse(TEXT).unwrap()];
        let set = FontSet {
            ui: &fonts,
            mono: &fonts,
            generation: 1,
        };
        let style = TextStyle {
            size: Fixed::from_i32(16),
            ..TextStyle::default()
        };
        let mut memory = Memory::new();
        let mut output = Output::new();
        let mut surface = vec![0_u8; 32 * 32 * 4];
        let mut storage = Storage::new(32 * 32, 2);
        let gamma = Gamma::default_value().unwrap();
        {
            let mut buffers = output.buffers();
            let mut workspace = memory.workspace();
            let view = layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap();
            let mut destination =
                Surface::new(&mut surface, 32, 32, 128, Format::Rgbx8888).unwrap();
            destination.clear();
            let mut context = storage.context(&gamma, None);
            draw(
                &mut destination,
                &view,
                &fonts,
                (Fixed::ONE.mul_ratio(quarter, 4).unwrap(), Fixed::ZERO),
                &mut context,
            )
            .unwrap();
        }
        seen.push(surface);
    }
    for (index, bytes) in seen.iter().enumerate().skip(1) {
        assert_ne!(
            bytes,
            seen.first().unwrap(),
            "position {index} drew the same pixels as position zero"
        );
    }
}

/// A `LayoutView` is what the compositor hands over; this keeps the type in
/// the test's own signature so a change to it is caught here.
fn _entry_point(view: &LayoutView<'_>, bounds: Bounds) -> usize {
    view.glyphs.len() + usize::try_from(bounds.width).unwrap_or(0)
}

#[test]
fn a_postscript_face_takes_the_cff_decoder() {
    // The CJK subset states its outlines in CFF, so drawing from it exercises
    // the cubic path of R2 and the second decoder of R13.
    const CJK: &[u8] = include_bytes!("fixtures/noto-cjk-subset.otf");
    let (bytes, _) = render(CJK, "A\u{4e2d}\u{65e5}", 24, 128, 48, None, false);
    assert!(inked(&bytes) > 20, "the CFF face drew almost nothing");
}

#[test]
fn a_cached_glyph_is_composited_where_the_fresh_one_was() {
    // The second occurrence of a letter comes out of the cache; the two must
    // land on the same pixels, offset only by the advance.
    let once = render(TEXT, "A", 16, 64, 24, None, true).0;
    let twice = render(TEXT, "AA", 16, 64, 24, None, true).0;
    assert_ne!(once, twice, "the second A drew nothing");
    // Every pixel the single A inked is still inked by the pair.
    for (left, right) in once
        .as_chunks::<4>()
        .0
        .iter()
        .zip(twice.as_chunks::<4>().0.iter())
    {
        if left.iter().take(3).any(|byte| *byte != 0) {
            assert_eq!(left, right, "the cached draw moved the first glyph");
        }
    }
}

#[test]
fn a_transform_that_collapses_the_plane_has_no_inverse() {
    use crate::{Affine, Invertible};
    let flat = Affine {
        xx: Fixed::ZERO,
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: Fixed::ZERO,
        dx: Fixed::from_i32(3),
        dy: Fixed::from_i32(4),
    };
    assert!(flat.inverse().unwrap().is_none());
    // A translation inverts to the opposite translation.
    let moved = Affine {
        dx: Fixed::from_i32(3),
        dy: Fixed::from_i32(-4),
        ..Affine::IDENTITY
    };
    let back = moved.inverse().unwrap().unwrap();
    assert_eq!(back.dx, Fixed::from_i32(-3));
    assert_eq!(back.dy, Fixed::from_i32(4));
    assert_eq!(
        back.apply(Fixed::from_i32(3), Fixed::from_i32(-4)).unwrap(),
        (Fixed::ZERO, Fixed::ZERO)
    );
    // A scale inverts to its reciprocal.
    let scaled = Affine {
        xx: Fixed::from_i32(4),
        yy: Fixed::from_i32(-2),
        ..Affine::IDENTITY
    };
    let back = scaled.inverse().unwrap().unwrap();
    assert_eq!(back.xx, Fixed::ONE.mul_ratio(1, 4).unwrap());
    assert_eq!(back.yy, Fixed::ONE.mul_ratio(-1, 2).unwrap());
    assert_eq!(Affine::IDENTITY.inverse().unwrap(), Some(Affine::IDENTITY));
}

#[test]
fn a_colour_glyph_whose_box_misses_the_surface_draws_nothing() {
    // The emoji fixture's glyphs sit above the baseline, so an origin far
    // below the surface puts the whole box outside it.
    let fonts = [Font::parse(EMOJI).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 1,
    };
    let style = TextStyle {
        size: Fixed::from_i32(24),
        ..TextStyle::default()
    };
    let mut memory = Memory::new();
    let mut output = Output::new();
    let mut surface = vec![0_u8; 32 * 32 * 4];
    let mut storage = Storage::new(32 * 32, 2);
    let gamma = Gamma::default_value().unwrap();
    {
        let mut buffers = output.buffers();
        let mut workspace = memory.workspace();
        let view = layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap();
        let mut destination = Surface::new(&mut surface, 32, 32, 128, Format::Rgbx8888).unwrap();
        destination.clear();
        let mut context = storage.context(&gamma, None);
        draw(
            &mut destination,
            &view,
            &fonts,
            (Fixed::from_i32(-500), Fixed::from_i32(-500)),
            &mut context,
        )
        .unwrap();
    }
    assert_eq!(inked(&surface), 0);
}

/// The same file with one table's tag renamed, so the face states no outline
/// of that kind. This is how a strike-only face looks to an outline decoder.
fn without_table(font: &[u8], tag: [u8; 4]) -> Vec<u8> {
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let mut out = font.to_vec();
    for index in 0..count {
        let at = 12 + index * 16;
        if font[at..at + 4] == tag {
            out[at..at + 4].copy_from_slice(b"glyg");
        }
    }
    out
}

#[test]
fn a_face_without_its_outline_table_draws_nothing() {
    for (bytes, tag) in [(TEXT, *b"glyf"), (EMOJI, *b"glyf")] {
        let stripped = without_table(bytes, tag);
        let fonts = [Font::parse(&stripped).unwrap()];
        let set = FontSet {
            ui: &fonts,
            mono: &fonts,
            generation: 1,
        };
        let style = TextStyle {
            size: Fixed::from_i32(16),
            ..TextStyle::default()
        };
        let mut memory = Memory::new();
        let mut output = Output::new();
        let mut surface = vec![0_u8; 32 * 32 * 4];
        let mut storage = Storage::new(32 * 32, 2);
        let gamma = Gamma::default_value().unwrap();
        {
            let mut buffers = output.buffers();
            let mut workspace = memory.workspace();
            let view = layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap();
            let mut destination =
                Surface::new(&mut surface, 32, 32, 128, Format::Rgbx8888).unwrap();
            destination.clear();
            let mut context = storage.context(&gamma, None);
            draw(
                &mut destination,
                &view,
                &fonts,
                (Fixed::from_i32(4), Fixed::ZERO),
                &mut context,
            )
            .unwrap();
        }
        assert_eq!(inked(&surface), 0, "a face without {tag:?} drew ink");
    }
}

#[test]
fn a_tab_draws_nothing_because_layout_hides_its_glyph() {
    // A tab occupies a hidden glyph slot, which the walk skips.
    let (bytes, extent) = render(TEXT, "\t", 16, 64, 24, None, false);
    assert_eq!(inked(&bytes), 0);
    assert!(extent.0 > Fixed::ZERO, "the tab advanced nothing");
}

#[test]
fn an_inverse_that_leaves_the_arithmetic_is_no_inverse() {
    use crate::{Affine, Invertible};
    // A determinant of one Q32.32 unit makes the reciprocal 2^32, which is
    // beyond the range, so the transform has no inverse this crate can state.
    let steep = Affine {
        xx: Fixed::from_bits(1),
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: Fixed::ONE,
        dx: Fixed::ZERO,
        dy: Fixed::ZERO,
    };
    assert!(steep.inverse().unwrap().is_none());
    let other = Affine {
        xx: Fixed::ONE,
        yy: Fixed::from_bits(1),
        ..Affine::IDENTITY
    };
    assert!(other.inverse().unwrap().is_none());
}

#[test]
fn a_glyph_the_face_does_not_state_draws_nothing() {
    // The emoji fixture maps no character, so every cluster falls to glyph
    // zero, whose own outline the subset does state; the CJK subset has no
    // notdef ink at all. Both draw without error.
    const CJK: &[u8] = include_bytes!("fixtures/noto-cjk-subset.otf");
    for bytes in [CJK, EMOJI] {
        let (surface, _) = render(bytes, "\u{10FFFD}", 16, 32, 32, None, false);
        assert_eq!(surface.len(), 32 * 32 * 4);
    }
}

#[test]
fn a_size_below_one_pixel_draws_almost_nothing() {
    // `text-core` refuses a size of zero, so the smallest a caller can ask
    // for is one unit; the glyph's box then rounds to the margin alone.
    let fonts = [Font::parse(TEXT).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 1,
    };
    let style = TextStyle {
        size: Fixed::from_bits(1),
        ..TextStyle::default()
    };
    let mut memory = Memory::new();
    let mut output = Output::new();
    let mut surface = vec![0_u8; 32 * 32 * 4];
    let mut storage = Storage::new(32 * 32, 2);
    let gamma = Gamma::default_value().unwrap();
    {
        let mut buffers = output.buffers();
        let mut workspace = memory.workspace();
        let view = layout_into(&set, &style, "AV", None, &mut workspace, &mut buffers).unwrap();
        let mut destination = Surface::new(&mut surface, 32, 32, 128, Format::Rgbx8888).unwrap();
        destination.clear();
        let mut context = storage.context(&gamma, None);
        draw(
            &mut destination,
            &view,
            &fonts,
            (Fixed::from_i32(4), Fixed::from_i32(8)),
            &mut context,
        )
        .unwrap();
    }
    assert_eq!(inked(&surface), 0);
}
