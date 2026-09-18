// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

//! R10 and R12: the clip stack, the group stack and the paint stream.

use text_core::{
    Fixed, Font,
    cff::Command,
    colr::{Affine, Color, ColorSource, ColorStop, Colr, CompositeMode, Fill, PaintOp, Painted},
    glyf::Point,
    variation::VariationPoint,
};

use crate::{
    Bounds, Cell, Edge, Format, Gamma, LINEAR_ONE, OutlineScratch, Paint, PaintScratch,
    RasterError, Surface, Texel, draw_color_glyph,
};

/// The `COLRv1` build of Noto Color Emoji, subset to five base glyphs.
const EMOJI: &[u8] = include_bytes!("fixtures/NotoEmoji-colr.ttf");

/// Everything one call borrows, in one place a test can hand out.
struct Storage {
    edges: Vec<Edge>,
    cells: Vec<Cell>,
    clips: Vec<u8>,
    groups: Vec<u8>,
    points: Vec<Point>,
    contours: Vec<usize>,
    variation: Vec<VariationPoint>,
    commands: Vec<Command>,
}

impl Storage {
    fn new(bounds: Bounds, clip_levels: usize, group_levels: usize) -> Self {
        let plane = usize::try_from(bounds.width * bounds.height).unwrap();
        Self {
            edges: vec![Edge::default(); 8192],
            cells: vec![Cell::default(); usize::try_from(bounds.width).unwrap() + 1],
            clips: vec![0; plane * clip_levels],
            groups: vec![0; plane * group_levels * 8],
            points: vec![Point::default(); 2048],
            contours: vec![0; 256],
            variation: vec![VariationPoint::default(); 2048],
            commands: vec![Command::default(); 2048],
        }
    }

    fn scratch(&mut self) -> PaintScratch<'_> {
        PaintScratch {
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
        }
    }
}

fn solid(red: u8, green: u8, blue: u8) -> Fill {
    Fill::Solid(Color {
        source: ColorSource::Palette { red, green, blue },
        alpha: Fixed::ONE,
    })
}

fn bounded(ops: usize) -> Painted {
    Painted {
        ops,
        stops: 0,
        clip: None,
        bounded: true,
    }
}

/// A transform that maps the font-unit box `(x0, y0)` to `(x1, y1)` onto a
/// device box of `size` by `size`, flipping y because font space grows up.
fn fit(x0: i32, y0: i32, x1: i32, y1: i32, size: u32) -> Affine {
    let span = i64::from((x1 - x0).max(y1 - y0));
    let scale = Fixed::ONE.mul_ratio(i64::from(size), span).unwrap();
    Affine {
        xx: scale,
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: scale.checked_neg().unwrap(),
        dx: Fixed::from_i32(-x0).checked_mul(scale).unwrap(),
        dy: Fixed::from_i32(y1).checked_mul(scale).unwrap(),
    }
}

/// The ink box of glyph 6 of the fixture, which is a layer glyph with real
/// ink; the five base glyphs have empty outlines of their own.
const LAYER: (i32, i32, i32, i32) = (359, -6, 878, 706);

/// That layer glyph mapped onto a device box of `size` by `size`.
fn layer(size: u32) -> Affine {
    fit(LAYER.0, LAYER.1, LAYER.2, LAYER.3, size)
}

/// Draw one stream into a fresh `A8` surface of `bounds` and give its bytes.
fn drawn(
    bounds: Bounds,
    ops: &[PaintOp],
    stops: &[ColorStop],
    transform: Affine,
    clip_levels: usize,
    group_levels: usize,
) -> Result<Vec<u8>, RasterError> {
    let font = Font::parse(EMOJI).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let mut storage = Storage::new(bounds, clip_levels.max(1), group_levels.max(1));
    let pixels = usize::try_from(bounds.width * bounds.height).unwrap();
    let mut bytes = vec![0_u8; pixels * 4];
    {
        let mut destination = Surface::new(
            &mut bytes,
            bounds.width,
            bounds.height,
            bounds.width * 4,
            Format::Rgbx8888,
        )
        .unwrap();
        destination.clear();
        draw_color_glyph(
            &mut destination,
            bounds,
            transform,
            &font,
            &[],
            ops,
            stops,
            bounded(ops.len()),
            Paint::opaque(255, 255, 255),
            &gamma,
            &mut storage.scratch(),
        )?;
    }
    Ok(bytes)
}

/// The red channel of every pixel.
fn reds(bytes: &[u8]) -> Vec<u8> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| pixel[0])
        .collect()
}

fn box_of(width: u32, height: u32) -> Bounds {
    Bounds {
        x: 0,
        y: 0,
        width,
        height,
    }
}

#[test]
fn a_fill_without_a_clip_covers_the_whole_box() {
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: solid(255, 0, 0),
    }];
    let bytes = drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 1, 1).unwrap();
    assert_eq!(reds(&bytes), vec![255_u8; 16]);
}

#[test]
fn a_clip_limits_the_fill_to_the_outline() {
    // Glyph 6 of the fixture is a layer glyph with real ink; clipping to it
    // and filling must leave pixels outside it alone.
    let ops = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
        PaintOp::Unclip,
    ];
    let bounds = box_of(32, 32);
    let bytes = drawn(bounds, &ops, &[], layer(32), 2, 1).unwrap();
    let inked = reds(&bytes).into_iter().filter(|red| *red > 0).count();
    assert!(inked > 0, "the clip drew nothing");
    assert!(inked < 32 * 32, "the clip drew everything");
}

#[test]
fn nested_clips_give_the_product_of_the_two_coverages() {
    // Two clips to the same outline: the second is the square of the first,
    // so a partly covered edge pixel comes out darker than with one clip.
    let outer = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 255, 255),
        },
        PaintOp::Unclip,
    ];
    let inner = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 255, 255),
        },
        PaintOp::Unclip,
        PaintOp::Unclip,
    ];
    let bounds = box_of(24, 24);
    let single = reds(&drawn(bounds, &outer, &[], layer(24), 2, 1).unwrap());
    let double = reds(&drawn(bounds, &inner, &[], layer(24), 2, 1).unwrap());
    let mut partial = 0_usize;
    for (one, two) in single.iter().zip(&double) {
        assert!(two <= one, "the nested clip drew more: {two} over {one}");
        if *one > 0 && *one < 255 {
            partial += 1;
        }
    }
    assert!(partial > 0, "no pixel was partly covered");
    let total = |row: &[u8]| row.iter().map(|byte| usize::from(*byte)).sum::<usize>();
    assert!(total(&double) <= total(&single));
}

#[test]
fn an_unclip_restores_the_region_the_clip_narrowed() {
    let ops = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Unclip,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
    ];
    let bytes = drawn(box_of(8, 8), &ops, &[], layer(8), 2, 1).unwrap();
    assert_eq!(reds(&bytes), vec![255_u8; 64]);
}

#[test]
fn an_unmatched_unclip_is_an_error() {
    let ops = [PaintOp::Unclip];
    assert_eq!(
        drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 1, 1).unwrap_err(),
        RasterError::Stream
    );
    let unbalanced = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Unclip,
        PaintOp::Unclip,
    ];
    assert_eq!(
        drawn(box_of(4, 4), &unbalanced, &[], Affine::IDENTITY, 2, 1).unwrap_err(),
        RasterError::Stream
    );
}

#[test]
fn a_clip_left_open_at_the_end_is_an_error() {
    let ops = [PaintOp::Clip {
        glyph: 6,
        transform: Affine::IDENTITY,
    }];
    assert_eq!(
        drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 2, 1).unwrap_err(),
        RasterError::Stream
    );
    let group = [PaintOp::Group];
    assert_eq!(
        drawn(box_of(4, 4), &group, &[], Affine::IDENTITY, 1, 2).unwrap_err(),
        RasterError::Stream
    );
}

#[test]
fn a_clip_stack_deeper_than_its_storage_is_refused() {
    let ops = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Unclip,
        PaintOp::Unclip,
    ];
    assert_eq!(
        drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 1, 1).unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn a_clip_on_a_glyph_without_ink_draws_nothing() {
    // Glyphs 1 to 5 are the base glyphs; their own outlines are empty.
    let ops = [
        PaintOp::Clip {
            glyph: 1,
            transform: Affine::IDENTITY,
        },
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
        PaintOp::Unclip,
    ];
    let bytes = drawn(box_of(8, 8), &ops, &[], layer(8), 2, 1).unwrap();
    assert_eq!(reds(&bytes), vec![0_u8; 64]);
}

#[test]
fn a_group_composes_onto_what_was_under_it() {
    // Two groups, a red one under a green one, combined with source-over and
    // then drawn onto a surface that already holds blue.
    let ops = [
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(0, 0, 255),
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(0, 255, 0),
        },
        PaintOp::Compose(CompositeMode::SrcOver),
    ];
    let bytes = drawn(box_of(2, 2), &ops, &[], Affine::IDENTITY, 1, 2).unwrap();
    // Green over red is green; drawn over blue with source-over it stays green.
    assert_eq!(&bytes[0..3], &[0, 255, 0]);
}

#[test]
fn a_compose_keeps_the_ink_that_was_under_the_groups() {
    // `DestOut` of two groups leaves nothing, so what the surface held must
    // still be there afterwards.
    let ops = [
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(0, 0, 255),
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(255, 0, 0),
        },
        PaintOp::Compose(CompositeMode::DestOut),
    ];
    let bytes = drawn(box_of(2, 2), &ops, &[], Affine::IDENTITY, 1, 2).unwrap();
    assert_eq!(&bytes[0..3], &[0, 0, 255]);
}

#[test]
fn a_compose_with_fewer_than_two_groups_is_an_error() {
    let ops = [PaintOp::Compose(CompositeMode::SrcOver)];
    assert_eq!(
        drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 1, 2).unwrap_err(),
        RasterError::Stream
    );
    let one = [PaintOp::Group, PaintOp::Compose(CompositeMode::SrcOver)];
    assert_eq!(
        drawn(box_of(4, 4), &one, &[], Affine::IDENTITY, 1, 2).unwrap_err(),
        RasterError::Stream
    );
}

#[test]
fn a_group_stack_deeper_than_its_storage_is_refused() {
    let ops = [
        PaintOp::Group,
        PaintOp::Group,
        PaintOp::Compose(CompositeMode::SrcOver),
    ];
    assert_eq!(
        drawn(box_of(4, 4), &ops, &[], Affine::IDENTITY, 1, 1).unwrap_err(),
        RasterError::BufferTooSmall
    );
}

#[test]
fn an_unbounded_glyph_draws_nothing() {
    let font = Font::parse(EMOJI).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let bounds = box_of(4, 4);
    let mut storage = Storage::new(bounds, 1, 1);
    let mut bytes = vec![0_u8; 4 * 4 * 4];
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: solid(255, 0, 0),
    }];
    {
        let mut destination = Surface::new(&mut bytes, 4, 4, 16, Format::Rgbx8888).unwrap();
        destination.clear();
        draw_color_glyph(
            &mut destination,
            bounds,
            Affine::IDENTITY,
            &font,
            &[],
            &ops,
            &[],
            Painted {
                ops: 1,
                stops: 0,
                clip: None,
                bounded: false,
            },
            Paint::opaque(255, 255, 255),
            &gamma,
            &mut storage.scratch(),
        )
        .unwrap();
    }
    assert_eq!(reds(&bytes), vec![0_u8; 16]);
}

#[test]
fn an_empty_stream_writes_nothing() {
    let bytes = drawn(box_of(4, 4), &[], &[], Affine::IDENTITY, 1, 1).unwrap();
    assert_eq!(bytes, vec![0_u8; 4 * 4 * 4]);
}

#[test]
fn the_foreground_reaches_a_solid_fill() {
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: Fill::Solid(Color {
            source: ColorSource::Foreground,
            alpha: Fixed::ONE,
        }),
    }];
    let font = Font::parse(EMOJI).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let bounds = box_of(2, 2);
    let mut storage = Storage::new(bounds, 1, 1);
    let mut bytes = vec![0_u8; 2 * 2 * 4];
    {
        let mut destination = Surface::new(&mut bytes, 2, 2, 8, Format::Rgbx8888).unwrap();
        destination.clear();
        draw_color_glyph(
            &mut destination,
            bounds,
            Affine::IDENTITY,
            &font,
            &[],
            &ops,
            &[],
            bounded(1),
            Paint::opaque(0x10, 0x20, 0x30),
            &gamma,
            &mut storage.scratch(),
        )
        .unwrap();
    }
    assert_eq!(&bytes[0..3], &[0x10, 0x20, 0x30]);
}

#[test]
fn a_gradient_fill_covers_the_clip_region() {
    let stops = [
        ColorStop {
            offset: Fixed::ZERO,
            color: Color {
                source: ColorSource::Palette {
                    red: 0,
                    green: 0,
                    blue: 0,
                },
                alpha: Fixed::ONE,
            },
        },
        ColorStop {
            offset: Fixed::ONE,
            color: Color {
                source: ColorSource::Palette {
                    red: 255,
                    green: 255,
                    blue: 255,
                },
                alpha: Fixed::ONE,
            },
        },
    ];
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: Fill::Linear {
            x0: Fixed::ZERO,
            y0: Fixed::ZERO,
            x1: Fixed::from_i32(8),
            y1: Fixed::ZERO,
            x2: Fixed::ZERO,
            y2: Fixed::from_i32(8),
            line: text_core::colr::ColorLine {
                extend: text_core::colr::Extend::Pad,
                first: 0,
                count: 2,
            },
        },
    }];
    let bytes = drawn(box_of(8, 1), &ops, &stops, Affine::IDENTITY, 1, 1).unwrap();
    let row = reds(&bytes);
    assert!(row[0] < row[4], "{row:?} is not increasing");
    assert!(row[4] < row[7], "{row:?} is not increasing");
    // The gradient is read at pixel centres, so the last pixel sits at 7.5 of
    // 8 along the line and does not reach the end colour.
    assert!(row[7] > 240, "{row:?} does not reach the end");
}

#[test]
fn the_same_stream_draws_identically_twice() {
    let ops = [
        PaintOp::Clip {
            glyph: 6,
            transform: Affine::IDENTITY,
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(200, 100, 50),
        },
        PaintOp::Group,
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: solid(50, 100, 200),
        },
        PaintOp::Compose(CompositeMode::Multiply),
        PaintOp::Unclip,
    ];
    let bounds = box_of(16, 16);
    let first = drawn(bounds, &ops, &[], layer(16), 2, 2).unwrap();
    let second = drawn(bounds, &ops, &[], layer(16), 2, 2).unwrap();
    assert_eq!(first, second);
    assert!(first.iter().any(|byte| *byte > 0), "nothing was drawn");
}

#[test]
fn every_base_glyph_of_the_fixture_reaches_pixels() {
    let font = Font::parse(EMOJI).unwrap();
    let colr = Colr::parse(&font).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let bounds = box_of(24, 24);
    for glyph in 1..=5_u16 {
        let mut ops = vec![PaintOp::default(); 512];
        let mut stops = vec![ColorStop::default(); 512];
        let painted = colr.paint(glyph, 0, &[], &mut ops, &mut stops).unwrap();
        assert!(painted.bounded, "glyph {glyph} is not bounded");
        let clip = painted.clip.expect("the fixture states a clip box");
        let whole = |value: Fixed| i32::try_from(value.bits() >> 32).unwrap();
        let placement = fit(
            whole(clip.x_min),
            whole(clip.y_min),
            whole(clip.x_max),
            whole(clip.y_max),
            24,
        );
        let mut storage = Storage::new(bounds, 8, 8);
        let mut bytes = vec![0_u8; 24 * 24 * 4];
        {
            let mut destination =
                Surface::new(&mut bytes, 24, 24, 24 * 4, Format::Rgbx8888).unwrap();
            destination.clear();
            draw_color_glyph(
                &mut destination,
                bounds,
                placement,
                &font,
                &[],
                &ops[..painted.ops],
                &stops[..painted.stops],
                painted,
                Paint::opaque(255, 255, 255),
                &gamma,
                &mut storage.scratch(),
            )
            .unwrap();
        }
        let inked = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel.iter().take(3).any(|byte| *byte != 0))
            .count();
        assert!(inked > 0, "glyph {glyph} drew nothing");
        assert!(inked < 24 * 24, "glyph {glyph} covered everything");
        super::golden::check(&format!("emoji-{glyph}"), &bytes, 24, 24);
    }
}

#[test]
fn a_texel_of_the_destination_reads_back_what_was_drawn() {
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: solid(0x33, 0x66, 0x99),
    }];
    let mut bytes = drawn(box_of(2, 2), &ops, &[], Affine::IDENTITY, 1, 1).unwrap();
    let surface = Surface::new(&mut bytes, 2, 2, 8, Format::Rgbx8888).unwrap();
    assert_eq!(
        surface.texel(1, 1),
        Some(Texel::Display([0x33, 0x66, 0x99]))
    );
    assert_eq!(LINEAR_ONE, 65_535);
}

#[test]
fn a_box_of_no_pixels_draws_nothing() {
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: solid(255, 0, 0),
    }];
    let font = Font::parse(EMOJI).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let flat = Bounds {
        x: 0,
        y: 0,
        width: 0,
        height: 4,
    };
    let mut storage = Storage::new(box_of(4, 4), 1, 1);
    let mut bytes = vec![0_u8; 16 * 4];
    {
        let mut destination = Surface::new(&mut bytes, 4, 4, 16, Format::Rgbx8888).unwrap();
        destination.clear();
        draw_color_glyph(
            &mut destination,
            flat,
            Affine::IDENTITY,
            &font,
            &[],
            &ops,
            &[],
            bounded(1),
            Paint::opaque(255, 255, 255),
            &gamma,
            &mut storage.scratch(),
        )
        .unwrap();
    }
    assert_eq!(reds(&bytes), vec![0_u8; 16]);
}

#[test]
fn a_box_reaching_past_the_destination_writes_only_inside_it() {
    let ops = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: solid(255, 0, 0),
    }];
    let font = Font::parse(EMOJI).unwrap();
    let gamma = Gamma::default_value().unwrap();
    let wide = Bounds {
        x: -2,
        y: -2,
        width: 8,
        height: 8,
    };
    let mut storage = Storage::new(wide, 1, 1);
    let mut bytes = vec![0xa5_u8; 6 * 4 * 4];
    {
        let mut destination = Surface::new(&mut bytes, 4, 4, 24, Format::Rgbx8888).unwrap();
        destination.clear();
        draw_color_glyph(
            &mut destination,
            wide,
            Affine::IDENTITY,
            &font,
            &[],
            &ops,
            &[],
            bounded(1),
            Paint::opaque(255, 255, 255),
            &gamma,
            &mut storage.scratch(),
        )
        .unwrap();
    }
    for row in 0..4_usize {
        assert_eq!(
            &bytes[row * 24 + 16..row * 24 + 24],
            &[0xa5; 8],
            "row {row}"
        );
    }
}

#[test]
fn a_gradient_with_a_singular_transform_draws_nothing() {
    let stops = [ColorStop {
        offset: Fixed::ZERO,
        color: Color {
            source: ColorSource::Palette {
                red: 255,
                green: 0,
                blue: 0,
            },
            alpha: Fixed::ONE,
        },
    }];
    let flat = Affine {
        xx: Fixed::ZERO,
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: Fixed::ZERO,
        dx: Fixed::ZERO,
        dy: Fixed::ZERO,
    };
    let ops = [PaintOp::Fill {
        transform: flat,
        fill: Fill::Linear {
            x0: Fixed::ZERO,
            y0: Fixed::ZERO,
            x1: Fixed::from_i32(4),
            y1: Fixed::ZERO,
            x2: Fixed::ZERO,
            y2: Fixed::from_i32(4),
            line: text_core::colr::ColorLine {
                extend: text_core::colr::Extend::Pad,
                first: 0,
                count: 1,
            },
        },
    }];
    let bytes = drawn(box_of(4, 4), &ops, &stops, Affine::IDENTITY, 1, 1).unwrap();
    assert_eq!(reds(&bytes), vec![0_u8; 16]);
}
