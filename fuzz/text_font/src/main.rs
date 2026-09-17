// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Accepted envelopes expose exactly the bounded records of every face.

#![forbid(unsafe_code)]

use text_core::FontCollection;
use text_core::cmap::Cmap;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = core::str::from_utf8(bytes) {
        let base = text_core::Font::parse(include_bytes!(
            "../../../crates/text-core/src/tests/fixtures/DejaVu-shaping.ttf"
        ))
        .expect("fixture");
        if text.chars().count() <= 32 {
            exercise_layout(base, text);
        }
        for byte in text_core::segment::grapheme_boundaries(text)
            .chain(text_core::segment::word_boundaries(text))
        {
            assert!(text.is_char_boundary(byte));
        }
        let mut units = vec![text_core::bidi::Unit::default(); 512];
        let mut seq = vec![0; 512];
        if let Ok(info) =
            text_core::bidi::resolve(text, text_core::bidi::Direction::Auto, &mut units, &mut seq)
        {
            let mut levels = [0; 512];
            let mut order = [0; 512];
            let _ = text_core::bidi::reorder_line(
                &units[..info.count],
                0..info.count,
                &mut levels,
                &mut order,
            );
        }
        let mut work = vec![text_core::segment::LineUnit::default(); 512];
        let mut out = vec![text_core::segment::LineBoundary::default(); 513];
        if let Ok(n) = text_core::segment::line_breaks(text, &mut work, &mut out) {
            for boundary in &out[..n] {
                assert!(text.is_char_boundary(boundary.byte));
            }
        }
    }
    if let Ok(cmap) = Cmap::parse(bytes, 512) {
        for ch in ['\0', 'A', '\u{ffff}', '\u{1f600}', '\u{10ffff}'] {
            assert!(cmap.glyph_index(ch) < 512);
            assert!(
                cmap.variation_glyph(ch, '\u{fe0f}')
                    .is_none_or(|id| id < 512)
            );
        }
    }
    for substitution in [true, false] {
        if let Ok(table) = text_core::shape::LayoutTable::parse(bytes, substitution) {
            let mut glyphs = [text_core::shape::Glyph::default(); 32];
            for (i, g) in glyphs.iter_mut().take(8).enumerate() {
                g.id = u16::try_from(i).expect("bounded");
            }
            let mut buffer = text_core::shape::Buffer::new(&mut glyphs, 8).expect("capacity");
            let _ = table.apply_lookup(
                0,
                text_core::shape::Gdef::default(),
                &[],
                false,
                &mut buffer,
            );
            let _ = text_core::shape::finish(&mut buffer, false);
        }
    }
    for cff2 in [false, true] {
        if let Ok(cff) = text_core::cff::Cff::parse_table(bytes, cff2, 1000) {
            let _ = cff.outline(0, &mut [text_core::cff::Command::Close; 256]);
        }
    }
    if let Ok(file) = FontCollection::parse(bytes) {
        for index in 0..file.face_count() {
            let font = file.font(index).expect("validated face");
            let _ = font.cmap();
            exercise_layout(font, "office A骨 سلام שָׁלוֹם");
            if let Some(fvar) = font.table(*b"fvar") {
                if let Ok(axes) = text_core::variation::Axes::parse(
                    fvar.data,
                    font.table(*b"avar").map(|t| t.data),
                ) {
                    let mut coords = [text_core::Fixed::ZERO; 64];
                    if let Ok(n) = axes.normalize(&[], &mut coords) {
                        let _ = text_core::variation::Instance::new(&font, &coords[..n]);
                        if let Ok(glyf) = text_core::glyf::Glyf::parse(&font) {
                            let mut points = [text_core::glyf::Point::default(); 256];
                            let mut contours = [0; 64];
                            let mut scratch =
                                vec![text_core::variation::VariationPoint::default(); 512];
                            let _ = glyf.outline_instance(
                                0,
                                &coords[..n],
                                &mut points,
                                &mut contours,
                                &mut scratch,
                            );
                        }
                    }
                }
            }
            if let Ok(cff) = text_core::cff::Cff::parse(&font) {
                let _ = cff.outline(0, &mut [text_core::cff::Command::Close; 256]);
            }
            if let Ok(metrics) = font.metrics() {
                let _ = metrics.advance(0, text_core::Fixed::from_i32(13));
                let _ = metrics.advance(metrics.maxp.glyph_count, text_core::Fixed::ONE);
            }
            if let Some(table) = font.table(*b"cmap") {
                let _ = Cmap::parse(table.data, u16::MAX);
            }
            if let Ok(glyf) = text_core::glyf::Glyf::parse(&font) {
                let mut points = [text_core::glyf::Point::default(); 256];
                let mut contours = [0; 64];
                let _ = glyf.outline(0, &mut points, &mut contours);
            }
            assert_eq!(font.tables().count(), usize::from(font.table_count()));
            for table in font.tables() {
                let start = usize::try_from(table.offset).expect("bounded offset");
                let end = start.checked_add(table.data.len()).expect("bounded range");
                assert_eq!(bytes.get(start..end), Some(table.data));
                assert_eq!(font.table(table.tag), Some(table));
            }
        }
    }
});

fn exercise_layout(font: text_core::Font<'_>, text: &str) {
    use text_core::{
        Fixed, bidi, glyf, layout::*, resolve::Run, segment, shape, variation::VariationPoint,
    };
    let fonts = [font];
    let set = text_core::FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = text_core::TextStyle::default();
    let mut bidi = [bidi::Unit::default(); 64];
    let mut indices = [0; 64];
    let mut levels = [0; 64];
    let mut ranks = [0; 64];
    let mut line_units = [segment::LineUnit::default(); 64];
    let mut breaks = [segment::LineBoundary::default(); 65];
    let mut runs = [Run::default(); 64];
    let mut shape = [shape::Glyph::default(); 128];
    let mut line_glyphs = [PositionedGlyph::default(); 128];
    let mut line_clusters = [ClusterBox::default(); 64];
    let mut points = [glyf::Point::default(); 256];
    let mut contours = [0; 64];
    let mut variation = [VariationPoint::default(); 512];
    let mut carets = [shape::Caret::Coordinate(Fixed::ZERO); 64];
    let mut workspace = Workspace {
        bidi: &mut bidi,
        indices: &mut indices,
        levels: &mut levels,
        ranks: &mut ranks,
        line_units: &mut line_units,
        breaks: &mut breaks,
        runs: &mut runs,
        glyphs: &mut shape,
        line_glyphs: &mut line_glyphs,
        line_clusters: &mut line_clusters,
        points: &mut points,
        contours: &mut contours,
        variation: &mut variation,
        caret_values: &mut carets,
    };
    let mut lines = [Line::default(); 65];
    let mut glyphs = [PositionedGlyph::default(); 256];
    let mut clusters = [ClusterBox::default(); 64];
    let mut out = LayoutBuffers {
        lines: &mut lines,
        glyphs: &mut glyphs,
        clusters: &mut clusters,
    };
    let result = layout_into(
        &set,
        &style,
        text,
        Some(Fixed::from_i32(50)),
        &mut workspace,
        &mut out,
    )
    .map(|v| (v.info.width, v.info.height));
    let measure = measure_into(
        &set,
        &style,
        text,
        Some(Fixed::from_i32(50)),
        &mut workspace,
    );
    if result.is_ok() {
        assert_eq!(result, measure);
    }
}
