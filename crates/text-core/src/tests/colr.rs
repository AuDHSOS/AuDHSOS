// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! COLR version 0 and version 1 over CPAL; `docs/microsoft/colr.html:1130`.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Bounded test fixture arithmetic"
)]

use crate::{
    Fixed, Font, FontError,
    cff::Command,
    colr::{
        Affine, Background, Color, ColorLine, ColorSource, ColorStop, Colr, CompositeMode, Cpal,
        Extend, Extents, Fill, MAX_DEPTH, PaintOp, Painted, PaletteUse, Scratch,
    },
    glyf::Point,
    variation::VariationPoint,
};

/// The `COLRv1` build of Noto Color Emoji, subset to five base glyphs.
const EMOJI: &[u8] = include_bytes!("fixtures/NotoEmoji-colr.ttf");

/// A big-endian byte assembler for synthetic tables.
///
/// Every offset of COLR is unsigned and forward, so a table is written before
/// its children and its offset fields are patched once they are placed.
#[derive(Default)]
struct Bytes(Vec<u8>);

impl Bytes {
    fn u8(&mut self, value: u8) -> &mut Self {
        self.0.push(value);
        self
    }
    fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }
    fn i16(&mut self, value: i16) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }
    fn u24(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes()[1..]);
        self
    }
    fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_be_bytes());
        self
    }
    fn at(&self) -> u32 {
        u32::try_from(self.0.len()).expect("small fixture")
    }
    fn slot32(&mut self) -> u32 {
        let at = self.at();
        self.u32(0);
        at
    }
    fn slot24(&mut self) -> u32 {
        let at = self.at();
        self.u24(0);
        at
    }
    fn patch32(&mut self, at: u32, value: u32) {
        let at = usize::try_from(at).expect("small fixture");
        self.0[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }
    fn patch24(&mut self, at: u32, value: u32) {
        let at = usize::try_from(at).expect("small fixture");
        self.0[at..at + 3].copy_from_slice(&value.to_be_bytes()[1..]);
    }
    /// Patch a forward offset field holding the distance from `base`.
    fn link32(&mut self, field: u32, base: u32, target: u32) {
        self.patch32(field, target.checked_sub(base).expect("forward offset"));
    }
    fn link24(&mut self, field: u32, base: u32, target: u32) {
        self.patch24(field, target.checked_sub(base).expect("forward offset"));
    }
}

/// A CPAL version 0 table with one palette of BGRA records.
fn palette() -> Vec<u8> {
    let records = [[1u8, 2, 3, 255], [4, 5, 6, 128], [7, 8, 9, 0]];
    let mut out = Bytes::default();
    out.u16(0).u16(3).u16(1).u16(3).u32(14).u16(0);
    for record in records {
        out.0.extend_from_slice(&record);
    }
    out.0
}

/// A COLR version 1 header; every offset is patched once its table is placed.
fn header() -> Bytes {
    let mut out = Bytes::default();
    out.u16(1)
        .u16(0)
        .u32(0)
        .u32(0)
        .u16(0)
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(0);
    out
}

/// A `BaseGlyphList` for `glyphs`; returns its start and each `paintOffset` field.
fn base_list(out: &mut Bytes, glyphs: &[u16]) -> (u32, Vec<u32>) {
    let at = out.at();
    out.patch32(14, at);
    out.u32(u32::try_from(glyphs.len()).expect("small fixture"));
    let fields = glyphs
        .iter()
        .map(|glyph| {
            out.u16(*glyph);
            out.slot32()
        })
        .collect();
    (at, fields)
}

/// A `LayerList` of `count` entries; returns its start and each offset field.
fn layer_list(out: &mut Bytes, count: usize) -> (u32, Vec<u32>) {
    let at = out.at();
    out.patch32(18, at);
    out.u32(u32::try_from(count).expect("small fixture"));
    (at, (0..count).map(|_| out.slot32()).collect())
}

/// A COLR version 1 table whose single base glyph 5 paints what `body` writes.
fn version_one(body: impl FnOnce(&mut Bytes) -> u32) -> Vec<u8> {
    let mut out = header();
    let (list, fields) = base_list(&mut out, &[5]);
    let root = body(&mut out);
    out.link32(fields[0], list, root);
    out.0
}

/// `PaintGlyph(glyph) -> PaintSolid(entry)`, returning the paint offset.
fn glyph_solid(out: &mut Bytes, glyph: u16, entry: u16) -> u32 {
    let at = out.at();
    out.u8(10).u24(6).u16(glyph);
    out.u8(2).u16(entry).i16(1 << 14);
    at
}

fn resolve(colr: &[u8], glyph: u16) -> Result<(Painted, Vec<PaintOp>, Vec<ColorStop>), FontError> {
    resolve_with(colr, glyph, &[], 256, 64)
}

fn resolve_with(
    colr: &[u8],
    glyph: u16,
    coords: &[Fixed],
    op_capacity: usize,
    stop_capacity: usize,
) -> Result<(Painted, Vec<PaintOp>, Vec<ColorStop>), FontError> {
    let cpal = palette();
    let table = Colr::parse_tables(colr, &cpal, 64, coords.len())?;
    let mut ops = vec![PaintOp::default(); op_capacity];
    let mut stops = vec![ColorStop::default(); stop_capacity];
    let painted = table.paint(glyph, 0, coords, &mut ops, &mut stops)?;
    ops.truncate(painted.ops);
    stops.truncate(painted.stops);
    Ok((painted, ops, stops))
}

/// The sRGB triple of palette entry `index` of [`palette`].
fn entry(index: u16) -> ColorSource {
    match index {
        0 => ColorSource::Palette {
            red: 3,
            green: 2,
            blue: 1,
        },
        1 => ColorSource::Palette {
            red: 6,
            green: 5,
            blue: 4,
        },
        _ => ColorSource::Palette {
            red: 9,
            green: 8,
            blue: 7,
        },
    }
}

fn clip(glyph: u16) -> PaintOp {
    PaintOp::Clip {
        glyph,
        transform: Affine::IDENTITY,
    }
}

#[test]
fn cpal_version_zero_reads_bgra_records_and_multiplies_alpha() {
    let bytes = palette();
    let table = Cpal::parse(&bytes).expect("valid");
    assert_eq!(table.palette_count(), 1);
    assert_eq!(table.entry_count(), 3);
    let opaque = table.color(0, 0, Fixed::ONE).expect("entry");
    assert_eq!(opaque.source, entry(0));
    assert_eq!(opaque.alpha, Fixed::ONE);
    // The record's own alpha is a fraction of 255.
    let half = table.color(0, 1, Fixed::ONE).expect("entry");
    assert_eq!(half.alpha, Fixed::ONE.mul_ratio(128, 255).expect("exact"));
    // The paint table's alpha multiplies the record's own.
    let quarter = Fixed::ONE.mul_ratio(1, 2).expect("exact");
    assert_eq!(
        table.color(0, 1, quarter).expect("entry").alpha,
        quarter.mul_ratio(128, 255).expect("exact")
    );
    let foreground = table.color(0, 0xffff, Fixed::ONE).expect("foreground");
    assert_eq!(foreground.source, ColorSource::Foreground);
    assert_eq!(foreground.alpha, Fixed::ONE);
    assert_eq!(
        table.color(0, 3, Fixed::ONE).unwrap_err(),
        FontError::InvalidTable
    );
    assert_eq!(
        table.color(1, 0, Fixed::ONE).unwrap_err(),
        FontError::InvalidTable
    );
    // Alpha outside [0, 1] is reserved and clamps.
    assert_eq!(
        table.color(0, 0, Fixed::from_i32(4)).expect("entry").alpha,
        Fixed::ONE
    );
    assert_eq!(
        table.color(0, 0, Fixed::from_i32(-4)).expect("entry").alpha,
        Fixed::ZERO
    );
}

#[test]
fn cpal_version_one_selects_a_palette_by_background() {
    let mut out = Bytes::default();
    out.u16(1).u16(1).u16(3).u16(3);
    let records = out.slot32();
    out.u16(0).u16(1).u16(2);
    let types = out.slot32();
    out.u32(0).u32(0);
    let at = out.at();
    out.patch32(records, at);
    for record in [[1u8, 1, 1, 255], [2, 2, 2, 255], [3, 3, 3, 255]] {
        out.0.extend_from_slice(&record);
    }
    let at = out.at();
    out.patch32(types, at);
    out.u32(0).u32(1).u32(2);
    let table = Cpal::parse(&out.0).expect("valid");
    assert_eq!(table.palette_count(), 3);
    assert_eq!(
        table.usage(0).expect("palette"),
        PaletteUse {
            light: false,
            dark: false
        }
    );
    assert!(table.usage(1).expect("palette").light);
    assert!(table.usage(2).expect("palette").dark);
    assert_eq!(table.palette_for(Background::Light).expect("any"), 1);
    assert_eq!(table.palette_for(Background::Dark).expect("any"), 2);
    assert_eq!(
        table.color(2, 0, Fixed::ONE).expect("entry").source,
        ColorSource::Palette {
            red: 3,
            green: 3,
            blue: 3
        }
    );
    // Without a types array every palette suits either background.
    let bytes = palette();
    let plain = Cpal::parse(&bytes).expect("valid");
    assert_eq!(plain.palette_for(Background::Dark).expect("any"), 0);
    assert_eq!(
        plain.usage(0).expect("palette"),
        PaletteUse {
            light: true,
            dark: true
        }
    );
}

#[test]
fn cpal_rejects_malformed_tables() {
    assert_eq!(Cpal::parse(&[]).unwrap_err(), FontError::Truncated);
    let mut two = palette();
    two[1] = 2;
    assert_eq!(Cpal::parse(&two).unwrap_err(), FontError::UnsupportedFormat);
    let mut empty = palette();
    empty[3] = 0;
    assert_eq!(Cpal::parse(&empty).unwrap_err(), FontError::InvalidTable);
    // A palette whose entries run past the record array.
    let mut short = palette();
    short[13] = 2;
    assert_eq!(Cpal::parse(&short).unwrap_err(), FontError::InvalidTable);
}

#[test]
fn version_zero_layers_resolve_to_clipped_solid_fills() {
    let mut out = Bytes::default();
    out.u16(0).u16(1).u32(14).u32(20).u16(2);
    assert_eq!(out.at(), 14);
    out.u16(7).u16(0).u16(2);
    assert_eq!(out.at(), 20);
    out.u16(11).u16(2).u16(12).u16(0xffff);
    let (painted, ops, stops) = resolve(&out.0, 7).expect("colour glyph");
    assert_eq!(painted.clip, None);
    assert!(stops.is_empty());
    assert_eq!(ops.len(), 6);
    assert_eq!(ops[0], clip(11));
    let PaintOp::Fill { fill, transform } = ops[1] else {
        panic!("a solid fill");
    };
    assert_eq!(transform, Affine::IDENTITY);
    let Fill::Solid(color) = fill else {
        panic!("a solid fill");
    };
    assert_eq!(color.source, entry(2));
    // Palette entry two is fully transparent.
    assert_eq!(color.alpha, Fixed::ZERO);
    assert_eq!(ops[2], PaintOp::Unclip);
    assert_eq!(ops[3], clip(12));
    let PaintOp::Fill {
        fill: Fill::Solid(color),
        ..
    } = ops[4]
    else {
        panic!("a solid fill");
    };
    assert_eq!(color.source, ColorSource::Foreground);
    assert_eq!(ops[5], PaintOp::Unclip);
    // A glyph with no record of either version is not a colour glyph.
    assert_eq!(resolve(&out.0, 8).unwrap_err(), FontError::MissingTable);
    let cpal = palette();
    let table = Colr::parse_tables(&out.0, &cpal, 64, 0).expect("valid");
    assert!(table.covers(7).expect("record"));
    assert!(!table.covers(8).expect("record"));
}

#[test]
fn version_zero_rejects_unsorted_records_and_out_of_range_indices() {
    let build = |bases: &[(u16, u16, u16)], layers: &[(u16, u16)]| {
        let mut out = Bytes::default();
        let count = u16::try_from(bases.len()).expect("small fixture");
        let after = 14 + u32::from(count) * 6;
        out.u16(0)
            .u16(count)
            .u32(14)
            .u32(after)
            .u16(u16::try_from(layers.len()).expect("small fixture"));
        for (glyph, first, count) in bases {
            out.u16(*glyph).u16(*first).u16(*count);
        }
        for (glyph, entry) in layers {
            out.u16(*glyph).u16(*entry);
        }
        let cpal = palette();
        Colr::parse_tables(&out.0, &cpal, 64, 0).map(|_| ())
    };
    assert_eq!(build(&[(9, 0, 1)], &[(1, 0)]), Ok(()));
    assert_eq!(
        build(&[(9, 0, 1), (9, 0, 1)], &[(1, 0)]).unwrap_err(),
        FontError::TableOrder
    );
    assert_eq!(
        build(&[(9, 0, 2)], &[(1, 0)]).unwrap_err(),
        FontError::InvalidTable
    );
    assert_eq!(
        build(&[(9, 0, 1)], &[(64, 0)]).unwrap_err(),
        FontError::GlyphIndex
    );
    assert_eq!(
        build(&[(9, 0, 1)], &[(1, 3)]).unwrap_err(),
        FontError::InvalidTable
    );
}

#[test]
fn version_one_resolves_layers_of_clipped_solid_fills() {
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(1).u8(2).u32(0);
        let (list, fields) = layer_list(out, 2);
        let first = glyph_solid(out, 21, 0);
        let second = glyph_solid(out, 22, 1);
        out.link32(fields[0], list, first);
        out.link32(fields[1], list, second);
        root
    });
    let (painted, ops, _) = resolve(&colr, 5).expect("colour glyph");
    assert_eq!(painted.ops, 6);
    assert_eq!(ops[0], clip(21));
    assert_eq!(ops[2], PaintOp::Unclip);
    assert_eq!(ops[3], clip(22));
    // A slice that runs past the LayerList is refused.
    let mut past = colr;
    past[45] = 3;
    assert_eq!(resolve(&past, 5).unwrap_err(), FontError::InvalidTable);
}

/// A colour line of `stops` as `(offset in 2.14, palette entry)`.
fn color_line(out: &mut Bytes, extend: u8, stops: &[(i16, u16)]) {
    out.u8(extend)
        .u16(u16::try_from(stops.len()).expect("small fixture"));
    for (offset, index) in stops {
        out.i16(*offset).u16(*index).i16(1 << 14);
    }
}

#[test]
fn gradients_carry_geometry_stops_and_extend_modes() {
    for (value, extend) in [
        (0, Extend::Pad),
        (1, Extend::Repeat),
        (2, Extend::Reflect),
        (9, Extend::Pad),
    ] {
        let colr = version_one(|out| {
            let root = out.at();
            out.u8(4);
            let line = out.slot24();
            out.i16(10).i16(20).i16(30).i16(40).i16(-50).i16(-60);
            let at = out.at();
            out.link24(line, root, at);
            color_line(out, value, &[(0, 0), (1 << 13, 1), (1 << 14, 2)]);
            root
        });
        let (_, ops, stops) = resolve(&colr, 5).expect("colour glyph");
        let PaintOp::Fill {
            fill:
                Fill::Linear {
                    x0,
                    y0,
                    x1,
                    y1,
                    x2,
                    y2,
                    line,
                },
            ..
        } = ops[0]
        else {
            panic!("a linear gradient");
        };
        assert_eq!(line.extend, extend);
        assert_eq!((line.first, line.count), (0, 3));
        assert_eq!(x0, Fixed::from_i32(10));
        assert_eq!(y0, Fixed::from_i32(20));
        assert_eq!(x1, Fixed::from_i32(30));
        assert_eq!(y1, Fixed::from_i32(40));
        assert_eq!(x2, Fixed::from_i32(-50));
        assert_eq!(y2, Fixed::from_i32(-60));
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[0].offset, Fixed::ZERO);
        assert_eq!(stops[1].offset, Fixed::from_bits(1 << 31));
        assert_eq!(stops[2].offset, Fixed::ONE);
        assert_eq!(stops[2].color.source, entry(2));
    }
}

#[test]
fn radial_and_sweep_gradients_read_their_own_fields() {
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(6);
        let line = out.slot24();
        out.i16(1).i16(2).u16(3).i16(4).i16(5).u16(60000);
        let at = out.at();
        out.link24(line, root, at);
        color_line(out, 0, &[(0, 0), (1 << 14, 1)]);
        root
    });
    let (_, ops, _) = resolve(&colr, 5).expect("colour glyph");
    assert_eq!(
        ops[0],
        PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: Fill::Radial {
                x0: Fixed::from_i32(1),
                y0: Fixed::from_i32(2),
                r0: Fixed::from_i32(3),
                x1: Fixed::from_i32(4),
                y1: Fixed::from_i32(5),
                // A UFWORD radius is unsigned and exceeds an i16.
                r1: Fixed::from_i32(60000),
                line: ColorLine {
                    extend: Extend::Pad,
                    first: 0,
                    count: 2,
                },
            },
        }
    );
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(8);
        let line = out.slot24();
        out.i16(7).i16(8).i16(-1 << 14).i16(1 << 14);
        let at = out.at();
        out.link24(line, root, at);
        color_line(out, 2, &[(0, 0), (1 << 14, 1)]);
        root
    });
    let (_, ops, _) = resolve(&colr, 5).expect("colour glyph");
    let PaintOp::Fill {
        fill: Fill::Sweep {
            x, y, start, end, ..
        },
        ..
    } = ops[0]
    else {
        panic!("a sweep gradient");
    };
    assert_eq!((x, y), (Fixed::from_i32(7), Fixed::from_i32(8)));
    // Angles carry a bias of one half-turn: -1.0 is zero, +1.0 is two.
    assert_eq!(start, Fixed::ZERO);
    assert_eq!(end, Fixed::from_i32(2));
}

/// A transform paint of `format` over `fields`, wrapping a solid fill.
fn transform_paint(format: u8, fields: &[i16]) -> Vec<u8> {
    version_one(|out| {
        let root = out.at();
        out.u8(format);
        let child = out.slot24();
        for field in fields {
            out.i16(*field);
        }
        let at = out.at();
        out.link24(child, root, at);
        out.u8(2).u16(0).i16(1 << 14);
        root
    })
}

fn transform_of(colr: &[u8]) -> Affine {
    let (_, ops, _) = resolve(colr, 5).expect("colour glyph");
    let PaintOp::Fill { transform, .. } = ops[0] else {
        panic!("a fill");
    };
    transform
}

#[test]
fn affine_paints_compose_the_transform_of_their_subgraph() {
    // PaintTransform names an Affine2x3 of six 16.16 numbers.
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(12);
        let child = out.slot24();
        let matrix = out.slot24();
        let at = out.at();
        out.link24(child, root, at);
        out.u8(2).u16(0).i16(1 << 14);
        let at = out.at();
        out.link24(matrix, root, at);
        out.u32(2 << 16)
            .u32(0)
            .u32(0)
            .u32(3 << 16)
            .u32(4 << 16)
            .u32(5 << 16);
        root
    });
    assert_eq!(
        transform_of(&colr),
        Affine {
            xx: Fixed::from_i32(2),
            yx: Fixed::ZERO,
            xy: Fixed::ZERO,
            yy: Fixed::from_i32(3),
            dx: Fixed::from_i32(4),
            dy: Fixed::from_i32(5),
        }
    );
    let translate = transform_of(&transform_paint(14, &[7, -9]));
    assert_eq!(
        translate.apply(Fixed::ZERO, Fixed::ZERO).expect("exact"),
        (Fixed::from_i32(7), Fixed::from_i32(-9))
    );
    let scale = transform_of(&transform_paint(16, &[1 << 13, 1 << 14]));
    assert_eq!(
        scale
            .apply(Fixed::from_i32(4), Fixed::from_i32(4))
            .expect("exact"),
        (Fixed::from_i32(2), Fixed::from_i32(4))
    );
    let uniform = transform_of(&transform_paint(20, &[1 << 13]));
    assert_eq!(
        uniform
            .apply(Fixed::from_i32(4), Fixed::from_i32(4))
            .expect("exact"),
        (Fixed::from_i32(2), Fixed::from_i32(2))
    );
    // Scaling about a centre leaves that centre where it was.
    for (format, fields) in [
        (18u8, vec![1 << 13, 1 << 13, 100, 200]),
        (22, vec![1 << 13, 100, 200]),
    ] {
        let around = transform_of(&transform_paint(format, &fields));
        assert_eq!(
            around
                .apply(Fixed::from_i32(100), Fixed::from_i32(200))
                .expect("exact"),
            (Fixed::from_i32(100), Fixed::from_i32(200))
        );
    }
}

/// A variable transform paint of `format`, its `varIndexBase` naming a store
/// that holds one delta per field.
fn var_transform_paint(format: u8, fields: &[i16], deltas: &[i8]) -> Vec<u8> {
    let mut out = header();
    let (list, slots) = base_list(&mut out, &[5]);
    let root = out.at();
    out.u8(format);
    let child = out.slot24();
    for field in fields {
        out.i16(*field);
    }
    out.u32(0);
    let at = out.at();
    out.link24(child, root, at);
    out.u8(2).u16(0).i16(1 << 14);
    let store = variation_store(&mut out, deltas);
    out.patch32(30, store);
    out.link32(slots[0], list, root);
    out.0
}

fn varied_transform_of(colr: &[u8]) -> Affine {
    let (_, ops, _) = resolve_with(colr, 5, &[Fixed::ONE], 16, 8).expect("colour glyph");
    let PaintOp::Fill { transform, .. } = ops[0] else {
        panic!("a fill");
    };
    transform
}

#[test]
fn every_variable_transform_format_maps_its_fields_to_the_sequence() {
    // Each variable format applied at one end of the axis equals its
    // non-variable twin built from the stored numbers plus the deltas.
    let cases: [(u8, u8, &[i16], &[i8]); 9] = [
        (15, 14, &[100, 200], &[3, -4]),
        (17, 16, &[1 << 13, 1 << 13], &[5, -6]),
        (19, 18, &[1 << 13, 1 << 13, 40, 50], &[5, -6, 7, -8]),
        (21, 20, &[1 << 13], &[9]),
        (23, 22, &[1 << 13, 40, 50], &[9, 7, -8]),
        (25, 24, &[1 << 12], &[11]),
        (27, 26, &[1 << 12, 40, 50], &[11, 7, -8]),
        (29, 28, &[1 << 11, 1 << 11], &[13, -14]),
        (31, 30, &[1 << 11, 1 << 11, 40, 50], &[13, -14, 7, -8]),
    ];
    for (variable, plain, fields, deltas) in cases {
        let applied: Vec<i16> = fields
            .iter()
            .zip(deltas)
            .map(|(field, delta)| field + i16::from(*delta))
            .collect();
        assert_eq!(
            varied_transform_of(&var_transform_paint(variable, fields, deltas)),
            transform_of(&transform_paint(plain, &applied)),
            "format {variable}"
        );
    }
    // PaintVarTransform names a VarAffine2x3 of six 16.16 numbers.
    let mut out = header();
    let (list, slots) = base_list(&mut out, &[5]);
    let root = out.at();
    out.u8(13);
    let child = out.slot24();
    let matrix = out.slot24();
    let at = out.at();
    out.link24(child, root, at);
    out.u8(2).u16(0).i16(1 << 14);
    let at = out.at();
    out.link24(matrix, root, at);
    out.u32(1 << 16)
        .u32(0)
        .u32(0)
        .u32(1 << 16)
        .u32(0)
        .u32(0)
        .u32(0);
    let store = variation_store(&mut out, &[1, 2, 3, 4, 5, 6]);
    out.patch32(30, store);
    out.link32(slots[0], list, root);
    let unit = Fixed::from_bits(1 << 16);
    let step = |n: i64| unit.mul_ratio(n, 1).expect("exact");
    assert_eq!(
        varied_transform_of(&out.0),
        Affine {
            xx: Fixed::ONE.checked_add(step(1)).expect("exact"),
            yx: step(2),
            xy: step(3),
            yy: Fixed::ONE.checked_add(step(4)).expect("exact"),
            dx: step(5),
            dy: step(6),
        }
    );
}

#[test]
fn rotation_and_skew_use_fixed_point_trigonometry() {
    // A quarter turn maps (1000, 0) onto (0, 1000).
    let quarter = transform_of(&transform_paint(24, &[1 << 13]));
    let (x, y) = quarter
        .apply(Fixed::from_i32(1000), Fixed::ZERO)
        .expect("exact");
    assert!(x.bits().abs() < 16, "{x:?}");
    assert_eq!(y, Fixed::from_i32(1000));
    // A half turn maps (1000, 0) onto (-1000, 0).
    let half = transform_of(&transform_paint(24, &[1 << 14]));
    let (x, y) = half
        .apply(Fixed::from_i32(1000), Fixed::ZERO)
        .expect("exact");
    assert_eq!(x, Fixed::from_i32(-1000));
    assert!(y.bits().abs() < 16, "{y:?}");
    // Rotation about a centre leaves that centre where it was.
    let around = transform_of(&transform_paint(26, &[1 << 13, 300, 400]));
    let (x, y) = around
        .apply(Fixed::from_i32(300), Fixed::from_i32(400))
        .expect("exact");
    assert!(near(x, Fixed::from_i32(300)), "{x:?}");
    assert!(near(y, Fixed::from_i32(400)), "{y:?}");
    // An eighth of a turn of x skew has tangent one, so xy is minus one.
    let skew = transform_of(&transform_paint(28, &[1 << 12, 0]));
    assert!(near(skew.xy, Fixed::from_i32(-1)), "{:?}", skew.xy);
    assert_eq!(skew.yx, Fixed::ZERO);
    let around = transform_of(&transform_paint(30, &[1 << 12, 0, 50, 60]));
    let (x, y) = around
        .apply(Fixed::from_i32(50), Fixed::from_i32(60))
        .expect("exact");
    assert!(near(x, Fixed::from_i32(50)), "{x:?}");
    assert_eq!(y, Fixed::from_i32(60));
    // A quarter turn of skew is a pole of the tangent.
    assert_eq!(
        resolve(&transform_paint(28, &[1 << 13, 0]), 5).unwrap_err(),
        FontError::InvalidTable
    );
}

/// Whether two numbers agree to within sixteen Q32.32 units.
fn near(left: Fixed, right: Fixed) -> bool {
    left.checked_sub(right).expect("exact").bits().abs() < 16
}

#[test]
fn composite_brackets_two_groups_and_names_every_mode() {
    for (value, mode) in [
        (0u8, CompositeMode::Clear),
        (3, CompositeMode::SrcOver),
        (12, CompositeMode::Plus),
        (23, CompositeMode::Multiply),
        (27, CompositeMode::Luminosity),
        (28, CompositeMode::Clear),
        (255, CompositeMode::Clear),
    ] {
        let colr = version_one(|out| {
            let root = out.at();
            out.u8(32);
            let source = out.slot24();
            out.u8(value);
            let backdrop = out.slot24();
            let at = out.at();
            out.link24(source, root, at);
            glyph_solid(out, 21, 0);
            let at = out.at();
            out.link24(backdrop, root, at);
            glyph_solid(out, 22, 1);
            root
        });
        let (_, ops, _) = resolve(&colr, 5).expect("colour glyph");
        assert_eq!(ops.len(), 9);
        assert_eq!(ops[0], PaintOp::Group);
        // The backdrop is resolved first, then the source.
        assert_eq!(ops[1], clip(22));
        assert_eq!(ops[4], PaintOp::Group);
        assert_eq!(ops[5], clip(21));
        assert_eq!(ops[8], PaintOp::Compose(mode));
    }
    // Every documented value names a distinct mode.
    for value in 1..=27u8 {
        assert_ne!(CompositeMode::from_value(value), CompositeMode::Clear);
    }
}

#[test]
fn colr_glyph_reuses_another_base_glyph_definition() {
    let mut out = header();
    let (list, fields) = base_list(&mut out, &[5, 6]);
    let root = out.at();
    out.u8(11).u16(6);
    let shared = glyph_solid(&mut out, 21, 0);
    out.link32(fields[0], list, root);
    out.link32(fields[1], list, shared);
    let (_, ops, _) = resolve(&out.0, 5).expect("colour glyph");
    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0], clip(21));
    // A referenced glyph without a record of its own is malformed.
    let mut missing = header();
    let (list, fields) = base_list(&mut missing, &[5]);
    let root = missing.at();
    missing.u8(11).u16(7);
    missing.link32(fields[0], list, root);
    assert_eq!(resolve(&missing.0, 5).unwrap_err(), FontError::InvalidTable);
}

#[test]
fn a_cyclic_graph_is_refused_before_it_recurses() {
    // A PaintColrGlyph that names its own base glyph.
    let mut own = header();
    let (list, fields) = base_list(&mut own, &[5]);
    let root = own.at();
    own.u8(11).u16(5);
    own.link32(fields[0], list, root);
    assert_eq!(resolve(&own.0, 5).unwrap_err(), FontError::Cycle);
    // Two base glyphs that name each other.
    let mut pair = header();
    let (list, fields) = base_list(&mut pair, &[5, 6]);
    let first = pair.at();
    pair.u8(11).u16(6);
    let second = pair.at();
    pair.u8(11).u16(5);
    pair.link32(fields[0], list, first);
    pair.link32(fields[1], list, second);
    assert_eq!(resolve(&pair.0, 5).unwrap_err(), FontError::Cycle);
    // A translation whose child is the translation itself.
    let mut loop_back = header();
    let (list, fields) = base_list(&mut loop_back, &[5]);
    let root = loop_back.at();
    loop_back.u8(14).u24(0).i16(1).i16(1);
    loop_back.link32(fields[0], list, root);
    assert_eq!(resolve(&loop_back.0, 5).unwrap_err(), FontError::Cycle);
}

#[test]
fn an_overlong_graph_exhausts_the_depth_limit() {
    // A chain of translations, each naming the eight bytes after it.
    let chain = |links: usize| {
        let mut out = header();
        let (list, fields) = base_list(&mut out, &[5]);
        let root = out.at();
        for _ in 0..links {
            out.u8(14).u24(8).i16(1).i16(1);
        }
        out.u8(2).u16(0).i16(1 << 14);
        out.link32(fields[0], list, root);
        out.0
    };
    assert_eq!(
        resolve(&chain(MAX_DEPTH), 5).unwrap_err(),
        FontError::LimitExceeded
    );
    let (painted, _, _) = resolve(&chain(MAX_DEPTH - 1), 5).expect("colour glyph");
    assert_eq!(painted.ops, 1);
}

#[test]
fn caller_storage_bounds_the_operations_and_the_stops() {
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(1).u8(2).u32(0);
        let (list, fields) = layer_list(out, 2);
        let first = glyph_solid(out, 21, 0);
        let second = glyph_solid(out, 22, 1);
        out.link32(fields[0], list, first);
        out.link32(fields[1], list, second);
        root
    });
    assert_eq!(
        resolve_with(&colr, 5, &[], 5, 4).unwrap_err(),
        FontError::BufferTooSmall
    );
    let (painted, _, _) = resolve_with(&colr, 5, &[], 6, 0).expect("colour glyph");
    assert_eq!(painted.ops, 6);
    let gradient = version_one(|out| {
        let root = out.at();
        out.u8(4);
        let line = out.slot24();
        out.i16(0).i16(0).i16(1).i16(0).i16(0).i16(1);
        let at = out.at();
        out.link24(line, root, at);
        color_line(out, 0, &[(0, 0), (1 << 14, 1)]);
        root
    });
    assert_eq!(
        resolve_with(&gradient, 5, &[], 8, 1).unwrap_err(),
        FontError::BufferTooSmall
    );
    assert!(resolve_with(&gradient, 5, &[], 8, 2).is_ok());
}

#[test]
fn a_format_this_version_does_not_define_is_ignored() {
    let colr = version_one(|out| {
        let root = out.at();
        out.u8(1).u8(2).u32(0);
        let (list, fields) = layer_list(out, 2);
        let unknown = out.at();
        out.u8(33).u16(0);
        let known = glyph_solid(out, 21, 0);
        out.link32(fields[0], list, unknown);
        out.link32(fields[1], list, known);
        root
    });
    let (painted, ops, _) = resolve(&colr, 5).expect("colour glyph");
    assert_eq!(painted.ops, 3);
    assert_eq!(ops[0], clip(21));
}

/// An item variation store of one axis, one region and one delta row.
fn variation_store(out: &mut Bytes, deltas: &[i8]) -> u32 {
    let at = out.at();
    out.u16(1);
    let regions = out.slot32();
    out.u16(1);
    let rows = out.slot32();
    let here = out.at();
    out.link32(regions, at, here);
    out.u16(1).u16(1).i16(0).i16(1 << 14).i16(1 << 14);
    let here = out.at();
    out.link32(rows, at, here);
    out.u16(u16::try_from(deltas.len()).expect("small fixture"))
        .u16(0)
        .u16(1)
        .u16(0);
    for delta in deltas {
        out.u8(delta.to_be_bytes()[0]);
    }
    at
}

/// `PaintVarTranslate(dx, dy, base)` over a solid fill, with a delta store.
fn var_translate(dx: i16, dy: i16, base: u32, deltas: &[i8], map: bool) -> Vec<u8> {
    let mut out = header();
    let (list, fields) = base_list(&mut out, &[5]);
    let root = out.at();
    out.u8(15);
    let child = out.slot24();
    out.i16(dx).i16(dy).u32(base);
    let at = out.at();
    out.link24(child, root, at);
    out.u8(2).u16(0).i16(1 << 14);
    let store = variation_store(&mut out, deltas);
    out.patch32(30, store);
    if map {
        // Format 0, one byte per entry, one inner bit: entries 1 then 0.
        let at = out.at();
        out.u8(0).u8(0).u16(2).u8(1).u8(0);
        out.patch32(26, at);
    }
    out.link32(fields[0], list, root);
    out.0
}

fn translation_of(colr: &[u8], coords: &[Fixed]) -> (Fixed, Fixed) {
    let (_, ops, _) = resolve_with(colr, 5, coords, 16, 8).expect("colour glyph");
    let PaintOp::Fill { transform, .. } = ops[0] else {
        panic!("a fill");
    };
    (transform.dx, transform.dy)
}

#[test]
fn variable_paints_add_interpolated_deltas_to_their_fields() {
    let colr = var_translate(100, 200, 0, &[10, -20], false);
    assert_eq!(
        translation_of(&colr, &[Fixed::ONE]),
        (Fixed::from_i32(110), Fixed::from_i32(180))
    );
    // At the default instance the stored numbers stand.
    assert_eq!(
        translation_of(&colr, &[Fixed::ZERO]),
        (Fixed::from_i32(100), Fixed::from_i32(200))
    );
    // Half the axis interpolates half the delta.
    let half = Fixed::ONE.mul_ratio(1, 2).expect("exact");
    assert_eq!(
        translation_of(&colr, &[half]),
        (Fixed::from_i32(105), Fixed::from_i32(190))
    );
}

#[test]
fn a_reserved_variation_base_leaves_the_stored_numbers_alone() {
    let colr = var_translate(100, 200, 0xffff_ffff, &[10, -20], false);
    assert_eq!(
        translation_of(&colr, &[Fixed::ONE]),
        (Fixed::from_i32(100), Fixed::from_i32(200))
    );
}

#[test]
fn a_delta_set_index_map_redirects_the_variation_sequence() {
    let colr = var_translate(0, 0, 0, &[7, 9], true);
    assert_eq!(
        translation_of(&colr, &[Fixed::ONE]),
        (Fixed::from_i32(9), Fixed::from_i32(7))
    );
}

#[test]
fn variable_stop_offsets_are_sorted_after_the_instance_is_derived() {
    let mut out = header();
    let (list, fields) = base_list(&mut out, &[5]);
    let root = out.at();
    out.u8(5);
    let line = out.slot24();
    out.i16(0)
        .i16(0)
        .i16(1)
        .i16(0)
        .i16(0)
        .i16(1)
        .u32(0xffff_ffff);
    let at = out.at();
    out.link24(line, root, at);
    // The first record's offset falls below the second at this instance.
    out.u8(0).u16(2);
    out.i16(1 << 14).u16(0).i16(1 << 14).u32(0);
    out.i16(1 << 13).u16(1).i16(1 << 14).u32(0xffff_ffff);
    let store = variation_store(&mut out, &[-1 << 7, 0]);
    out.patch32(30, store);
    out.link32(fields[0], list, root);
    let (_, _, stops) = resolve_with(&out.0, 5, &[Fixed::ONE], 16, 4).expect("colour glyph");
    assert_eq!(stops.len(), 2);
    assert_eq!(stops[0].color.source, entry(1));
    assert_eq!(stops[0].offset, Fixed::from_bits(1 << 31));
    assert_eq!(stops[1].color.source, entry(0));
    // One times 2.14 less 128 units of 2^-14 is 0.9921875.
    assert_eq!(
        stops[1].offset,
        Fixed::ONE
            .checked_sub(Fixed::from_bits(128 << 18))
            .expect("exact")
    );
}

#[test]
fn clip_boxes_cover_their_glyph_range_and_may_vary() {
    let mut out = header();
    let (list, fields) = base_list(&mut out, &[5]);
    let root = glyph_solid(&mut out, 21, 0);
    out.link32(fields[0], list, root);
    let clips = out.at();
    out.patch32(22, clips);
    out.u8(1).u32(2);
    out.u16(4).u16(6);
    let static_box = out.slot24();
    out.u16(9).u16(9);
    let variable_box = out.slot24();
    let at = out.at();
    out.link24(static_box, clips, at);
    out.u8(1).i16(-10).i16(-20).i16(30).i16(40);
    let at = out.at();
    out.link24(variable_box, clips, at);
    out.u8(2).i16(0).i16(0).i16(100).i16(100).u32(0);
    let store = variation_store(&mut out, &[5, 5, 5, 5]);
    out.patch32(30, store);
    let cpal = palette();
    let table = Colr::parse_tables(&out.0, &cpal, 64, 1).expect("valid");
    let default = [Fixed::ZERO];
    assert_eq!(
        table.clip_box(5, &default).expect("clip"),
        Some(Extents {
            x_min: Fixed::from_i32(-10),
            y_min: Fixed::from_i32(-20),
            x_max: Fixed::from_i32(30),
            y_max: Fixed::from_i32(40),
        })
    );
    // The record covers glyphs four to six and nothing outside it.
    assert!(table.clip_box(4, &default).expect("clip").is_some());
    assert!(table.clip_box(6, &default).expect("clip").is_some());
    assert_eq!(table.clip_box(7, &default).expect("clip"), None);
    assert_eq!(table.clip_box(3, &default).expect("clip"), None);
    assert_eq!(
        table.clip_box(9, &[Fixed::ONE]).expect("clip"),
        Some(Extents {
            x_min: Fixed::from_i32(5),
            y_min: Fixed::from_i32(5),
            x_max: Fixed::from_i32(105),
            y_max: Fixed::from_i32(105),
        })
    );
}

#[test]
fn clip_lists_reject_overlapping_and_inverted_ranges() {
    let build = |first: (u16, u16), second: (u16, u16)| {
        let mut out = header();
        let (list, fields) = base_list(&mut out, &[5]);
        let root = glyph_solid(&mut out, 21, 0);
        out.link32(fields[0], list, root);
        let clips = out.at();
        out.patch32(22, clips);
        out.u8(1).u32(2);
        out.u16(first.0).u16(first.1);
        let one = out.slot24();
        out.u16(second.0).u16(second.1);
        let two = out.slot24();
        let at = out.at();
        out.link24(one, clips, at);
        out.link24(two, clips, at);
        out.u8(1).i16(0).i16(0).i16(1).i16(1);
        let cpal = palette();
        Colr::parse_tables(&out.0, &cpal, 64, 0).map(|_| ())
    };
    assert_eq!(build((1, 5), (6, 9)), Ok(()));
    assert_eq!(build((1, 5), (5, 9)).unwrap_err(), FontError::TableOrder);
    assert_eq!(build((5, 1), (6, 9)).unwrap_err(), FontError::TableOrder);
    assert_eq!(build((1, 5), (6, 64)).unwrap_err(), FontError::GlyphIndex);
}

#[test]
fn the_emoji_fixture_resolves_every_paint_format_it_uses() {
    let font = Font::parse(EMOJI).expect("fixture");
    let table = Colr::parse(&font).expect("colour tables");
    assert_eq!(table.palettes().palette_count(), 1);
    assert_eq!(table.palettes().entry_count(), 21);
    let mut ops = vec![PaintOp::default(); 512];
    let mut stops = vec![ColorStop::default(); 64];
    // The counts and the literal numbers below were read from the same file
    // with fontTools, which decodes the binary independently of this crate.
    for (glyph, count, stop_count, box_) in [
        (1u16, 3usize, 2usize, (352, -32, 896, 736)),
        (2, 199, 0, (64, -224, 1216, 928)),
        (3, 42, 0, (320, -224, 960, 928)),
        (4, 75, 0, (64, -224, 1216, 928)),
        (5, 27, 7, (64, -192, 1184, 896)),
    ] {
        let painted = table
            .paint(glyph, 0, &[], &mut ops, &mut stops)
            .expect("colour glyph");
        assert_eq!((glyph, painted.ops), (glyph, count));
        assert_eq!((glyph, painted.stops), (glyph, stop_count));
        assert_eq!(
            (glyph, painted.clip),
            (
                glyph,
                Some(Extents {
                    x_min: Fixed::from_i32(box_.0),
                    y_min: Fixed::from_i32(box_.1),
                    x_max: Fixed::from_i32(box_.2),
                    y_max: Fixed::from_i32(box_.3),
                })
            )
        );
    }
    // Glyph 1 is one clipped linear gradient of two stops.
    let painted = table.paint(1, 0, &[], &mut ops, &mut stops).expect("glyph");
    assert_eq!(ops[0], clip(6));
    let PaintOp::Fill {
        fill:
            Fill::Linear {
                x0,
                y0,
                x1,
                y1,
                x2,
                y2,
                line,
            },
        ..
    } = ops[1]
    else {
        panic!("a linear gradient");
    };
    assert_eq!(x0, Fixed::from_i32(619));
    assert_eq!(y0, Fixed::from_i32(714));
    assert_eq!(x1, Fixed::from_i32(619));
    assert_eq!(y1, Fixed::from_i32(6));
    assert_eq!(x2, Fixed::from_i32(1327));
    assert_eq!(y2, Fixed::from_i32(714));
    assert_eq!((line.extend, line.count), (Extend::Pad, 2));
    assert_eq!(ops[2], PaintOp::Unclip);
    assert_eq!(painted.stops, 2);
    assert_eq!(stops[0].offset, Fixed::ZERO);
    // Glyph 5 opens on a radial gradient of three stops.
    table.paint(5, 0, &[], &mut ops, &mut stops).expect("glyph");
    let PaintOp::Fill {
        fill:
            Fill::Radial {
                x0,
                y0,
                r0,
                x1,
                y1,
                r1,
                ..
            },
        ..
    } = ops[1]
    else {
        panic!("a radial gradient");
    };
    assert_eq!(
        (x0, y0, r0),
        (Fixed::from_i32(638), Fixed::from_i32(350), Fixed::ZERO)
    );
    assert_eq!(
        (x1, y1, r1),
        (
            Fixed::from_i32(638),
            Fixed::from_i32(350),
            Fixed::from_i32(534)
        )
    );
    // Glyph 2 composes four source-in groups.
    let painted = table.paint(2, 0, &[], &mut ops, &mut stops).expect("glyph");
    let ops = &ops[..painted.ops];
    assert_eq!(
        ops.iter()
            .filter(|op| **op == PaintOp::Compose(CompositeMode::SrcIn))
            .count(),
        4
    );
    assert_eq!(ops.iter().filter(|op| **op == PaintOp::Group).count(), 8);
}

#[test]
fn emoji_ink_extents_come_from_the_outlines_the_stream_clips() {
    let font = Font::parse(EMOJI).expect("fixture");
    let table = Colr::parse(&font).expect("colour tables");
    let mut ops = vec![PaintOp::default(); 512];
    let mut stops = vec![ColorStop::default(); 64];
    let mut points = vec![Point::default(); 1024];
    let mut contours = vec![0usize; 256];
    let mut variation = vec![VariationPoint::default(); 1024];
    let mut commands = vec![Command::Close; 256];
    let mut scratch = Scratch {
        points: &mut points,
        contours: &mut contours,
        variation: &mut variation,
        commands: &mut commands,
    };
    for glyph in 1..=5u16 {
        let painted = table
            .paint(glyph, 0, &[], &mut ops, &mut stops)
            .expect("glyph");
        let ink = table
            .extents(&font, &[], &ops[..painted.ops], &mut scratch)
            .expect("extents")
            .expect("a bounded glyph");
        let clip = painted.clip.expect("a clip box");
        // The ink of a bounded glyph lies inside the font's own clip box.
        assert!(ink.x_min >= clip.x_min, "{glyph}: {ink:?} {clip:?}");
        assert!(ink.y_min >= clip.y_min, "{glyph}: {ink:?} {clip:?}");
        assert!(ink.x_max <= clip.x_max, "{glyph}: {ink:?} {clip:?}");
        assert!(ink.y_max <= clip.y_max, "{glyph}: {ink:?} {clip:?}");
        assert!(ink.x_max > ink.x_min && ink.y_max > ink.y_min);
    }
    // A stream no outline clips has no ink box.
    let unclipped = [PaintOp::Fill {
        transform: Affine::IDENTITY,
        fill: Fill::Solid(Color::default()),
    }];
    assert_eq!(
        table
            .extents(&font, &[], &unclipped, &mut scratch)
            .expect("no error"),
        None
    );
    // An unmatched Unclip is malformed.
    assert_eq!(
        table
            .extents(&font, &[], &[PaintOp::Unclip], &mut scratch)
            .unwrap_err(),
        FontError::InvalidTable
    );
}

#[test]
fn the_emoji_fixture_refuses_a_glyph_it_gives_no_colour_definition() {
    let font = Font::parse(EMOJI).expect("fixture");
    let table = Colr::parse(&font).expect("colour tables");
    assert!(table.covers(1).expect("record"));
    assert!(!table.covers(0).expect("record"));
    let mut ops = vec![PaintOp::default(); 16];
    let mut stops = vec![ColorStop::default(); 16];
    assert_eq!(
        table.paint(0, 0, &[], &mut ops, &mut stops).unwrap_err(),
        FontError::MissingTable
    );
    // A face without the two tables is not a colour face.
    let plain = Font::parse(include_bytes!("fixtures/DejaVu-shaping.ttf")).expect("fixture");
    assert_eq!(Colr::parse(&plain).unwrap_err(), FontError::MissingTable);
}

#[test]
fn every_truncated_prefix_of_the_emoji_colr_table_is_refused_or_bounded() {
    let font = Font::parse(EMOJI).expect("fixture");
    let colr = font.table(*b"COLR").expect("table").data;
    let cpal = font.table(*b"CPAL").expect("table").data;
    let mut ops = vec![PaintOp::default(); 512];
    let mut stops = vec![ColorStop::default(); 64];
    for length in 0..colr.len() {
        if let Ok(table) = Colr::parse_tables(&colr[..length], cpal, 64, 0) {
            for glyph in 0..8 {
                let _ = table.paint(glyph, 0, &[], &mut ops, &mut stops);
                let _ = table.clip_box(glyph, &[]);
            }
        }
    }
    for length in 0..cpal.len() {
        let _ = Colr::parse_tables(colr, &cpal[..length], 64, 0);
    }
}

#[test]
fn boundedness_follows_the_format_of_every_node() {
    // A bare solid fill paints everywhere.
    let solid = version_one(|out| {
        let root = out.at();
        out.u8(2).u16(0).i16(1 << 14);
        root
    });
    let (painted, _, _) = resolve(&solid, 5).expect("colour glyph");
    assert!(!painted.bounded);
    // A glyph outline bounds it, and a transform of a bounded child stays so.
    let clipped = version_one(|out| glyph_solid(out, 21, 0));
    assert!(resolve(&clipped, 5).expect("colour glyph").0.bounded);
    let moved = version_one(|out| {
        let root = out.at();
        out.u8(14).u24(8).i16(1).i16(1);
        glyph_solid(out, 21, 0);
        root
    });
    assert!(resolve(&moved, 5).expect("colour glyph").0.bounded);
    // Source-in is bounded when either operand is; clear always is.
    let compose = |mode: u8, source_clipped: bool| {
        version_one(|out| {
            let root = out.at();
            out.u8(32);
            let source = out.slot24();
            out.u8(mode);
            let backdrop = out.slot24();
            let at = out.at();
            out.link24(source, root, at);
            if source_clipped {
                glyph_solid(out, 21, 0);
            } else {
                out.u8(2).u16(0).i16(1 << 14);
            }
            let at = out.at();
            out.link24(backdrop, root, at);
            out.u8(2).u16(1).i16(1 << 14);
            root
        })
    };
    assert!(
        resolve(&compose(5, true), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    assert!(
        !resolve(&compose(5, false), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    assert!(
        !resolve(&compose(3, true), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    assert!(
        resolve(&compose(0, false), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    assert!(
        resolve(&compose(1, true), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    assert!(
        !resolve(&compose(2, true), 5)
            .expect("colour glyph")
            .0
            .bounded
    );
    // Every glyph of the emoji fixture is bounded.
    let font = Font::parse(EMOJI).expect("fixture");
    let table = Colr::parse(&font).expect("colour tables");
    let mut ops = vec![PaintOp::default(); 512];
    let mut stops = vec![ColorStop::default(); 64];
    for glyph in 1..=5u16 {
        assert!(
            table
                .paint(glyph, 0, &[], &mut ops, &mut stops)
                .expect("glyph")
                .bounded
        );
    }
}

/// The same file without its `glyf` and `loca` records and with its `COLR`
/// tag renamed `CBDT`: a face whose only glyph data this track refuses.
fn bitmap_face(font: &[u8]) -> Vec<u8> {
    let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let mut out = font[..12].to_vec();
    let mut kept = 0u16;
    for index in 0..count {
        let at = 12 + index * 16;
        let record = &font[at..at + 16];
        if matches!(&record[..4], b"glyf" | b"loca") {
            continue;
        }
        let start = out.len();
        out.extend_from_slice(record);
        if &record[..4] == b"COLR" {
            out[start..start + 4].copy_from_slice(b"CBDT");
        }
        kept += 1;
    }
    out[4..6].copy_from_slice(&kept.to_be_bytes());
    // The removed records become a gap, so every payload offset still holds.
    out.resize(12 + count * 16, 0);
    out.extend_from_slice(&font[12 + count * 16..]);
    out
}

#[test]
fn a_chain_layer_of_a_refused_colour_format_covers_no_cluster() {
    let stripped = bitmap_face(EMOJI);
    let strike = Font::parse(&stripped).expect("valid envelope");
    assert!(strike.table(*b"glyf").is_none());
    assert!(strike.table(*b"CBDT").is_some());
    assert!(strike.cmap().expect("cmap").glyph_index('\u{263a}') != 0);
    // The chain falls through to the next layer instead of drawing nothing.
    let plain = Font::parse(include_bytes!("fixtures/DejaVu-shaping.ttf")).expect("fixture");
    let chain = [strike, plain];
    let set = crate::FontSet {
        ui: &chain,
        mono: &chain,
        generation: 0,
    };
    let style = crate::TextStyle::default();
    let layout = crate::layout(&set, &style, "A", None).expect("layout");
    let view = layout.view();
    assert_eq!(view.glyphs.len(), 1);
    assert_eq!(view.runs[view.glyphs[0].run].face, 1);
}
