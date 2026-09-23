// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::resolve::font_for;
use crate::{
    Fixed, Font, FontSet, TextError, TextStyle, bidi, glyf,
    layout::{self, ClusterBox, LayoutBuffers, Line, PositionedGlyph, Workspace},
    resolve::Run,
    segment,
    shape::{Caret, Glyph},
    variation::VariationPoint,
};

#[cfg(feature = "alloc")]
use crate::{
    Language,
    layout::{Affinity, Cursor, Rect},
};

struct Memory {
    bidi: Vec<bidi::Unit>,
    indices: Vec<usize>,
    levels: Vec<u8>,
    ranks: Vec<usize>,
    line_units: Vec<segment::LineUnit>,
    breaks: Vec<segment::LineBoundary>,
    runs: Vec<Run>,
    glyphs: Vec<Glyph>,
    line_glyphs: Vec<PositionedGlyph>,
    line_clusters: Vec<ClusterBox>,
    points: Vec<glyf::Point>,
    contours: Vec<usize>,
    variation: Vec<VariationPoint>,
    carets: Vec<Caret>,
}
impl Memory {
    fn new() -> Self {
        Self {
            bidi: vec![bidi::Unit::default(); 256],
            indices: vec![0; 256],
            levels: vec![0; 256],
            ranks: vec![0; 256],
            line_units: vec![segment::LineUnit::default(); 256],
            breaks: vec![segment::LineBoundary::default(); 257],
            runs: vec![Run::default(); 256],
            glyphs: vec![Glyph::default(); 1024],
            line_glyphs: vec![PositionedGlyph::default(); 1024],
            line_clusters: vec![ClusterBox::default(); 256],
            points: vec![glyf::Point::default(); 1024],
            contours: vec![0; 256],
            variation: vec![VariationPoint::default(); 2048],
            carets: vec![Caret::Coordinate(Fixed::ZERO); 256],
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
#[test]
fn caller_buffers_fractional_boxes_wrap_and_measure() {
    let bytes = font_for(&[' ', 'A', 'é'], 1000);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 19,
    };
    let style = TextStyle {
        size: Fixed::from_i32(10),
        ..TextStyle::default()
    };
    let advance = style.size.mul_ratio(601, 1000).unwrap();
    let two = advance.mul_ratio(2, 1).unwrap();
    for (text, width, line_count, box_width) in [
        ("", None, 0, Fixed::ZERO),
        ("AA", None, 1, two),
        ("AA AA", Some(two), 2, two),
        ("AAAA", Some(two), 2, two),
        ("e\u{301}", Some(Fixed::ZERO), 1, advance),
        ("AA\r\nA\n", None, 3, two),
    ] {
        let mut memory = Memory::new();
        let mut workspace = memory.workspace();
        let mut lines = [Line::default(); 16];
        let mut glyphs = [PositionedGlyph::default(); 32];
        let mut clusters = [ClusterBox::default(); 32];
        let mut buffers = LayoutBuffers {
            lines: &mut lines,
            glyphs: &mut glyphs,
            clusters: &mut clusters,
        };
        let view =
            layout::layout_into(&set, &style, text, width, &mut workspace, &mut buffers).unwrap();
        assert_eq!(view.lines.len(), line_count, "{text:?}");
        assert_eq!(view.info.width, box_width, "{text:?}");
        assert_eq!(
            view.info.height,
            Fixed::from_i32(11)
                .mul_ratio(i64::try_from(line_count).unwrap(), 1)
                .unwrap()
        );
        let result = (view.info.width, view.info.height);
        assert_eq!(
            view.clusters.len(),
            segment::grapheme_boundaries(text)
                .count()
                .checked_sub(1)
                .unwrap()
        );
        if text == "AA AA" {
            assert_eq!(
                view.lines
                    .iter()
                    .map(|l| (l.start, l.end))
                    .collect::<Vec<_>>(),
                [(0, 3), (3, 5)]
            );
            assert_eq!(view.glyphs.len(), 4);
        }
        if text == "e\u{301}" {
            assert!(view.lines[0].overflow);
            assert_eq!((view.clusters[0].start, view.clusters[0].end), (0, 3));
        }
        assert_eq!(
            layout::measure_into(&set, &style, text, width, &mut memory.workspace()).unwrap(),
            result
        );
    }
}
#[cfg(feature = "alloc")]
#[test]
fn owned_layout_bidi_carets_selections_and_ligature_reshaping() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/tests/fixtures/DejaVu-shaping.ttf"
    ))
    .unwrap();
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 55,
    };
    let style = TextStyle {
        size: Fixed::from_i32(2048),
        ..TextStyle::default()
    };
    let output = layout::layout(&set, &style, "office", None).unwrap();
    let view = output.view();
    assert_eq!(
        view.glyphs.iter().map(|g| g.id).collect::<Vec<_>>(),
        [10, 43, 6, 7]
    );
    assert_eq!(view.clusters.len(), 6);
    assert_eq!(view.info.width, Fixed::from_i32(5619));
    let ligature = view.glyphs.iter().find(|g| g.id == 43).unwrap();
    assert_eq!((ligature.start, ligature.end), (1, 4));
    let mut cursors = [Cursor::default(); 8];
    let n = view.carets(2, &mut cursors).unwrap();
    assert_eq!(n, 2);
    assert!(cursors[..n].iter().all(|c| c.x == Fixed::from_i32(1913)));
    let limit = layout::measure(&set, &style, "of", None).unwrap().0;
    let wrapped = layout::layout(&set, &style, "office", Some(limit)).unwrap();
    let line = wrapped.view().lines[0];
    assert_eq!(line.end, 2);
    assert_eq!(
        wrapped.view().glyphs[..line.glyph_count]
            .iter()
            .map(|g| g.id)
            .collect::<Vec<_>>(),
        [10, 8]
    );
    let output = layout::layout(&set, &style, "A שלום", None).unwrap();
    let view = output.view();
    let n = view.carets(2, &mut cursors).unwrap();
    assert_eq!(n, 2);
    assert_ne!(cursors[0].x, cursors[1].x);
    let mut rectangles = [Rect::default(); 8];
    assert_eq!(view.selection(0..4, &mut rectangles).unwrap(), 2);
    assert!(view.carets(3, &mut cursors).is_err());
    assert!(view.selection(0..3, &mut rectangles).is_err());
    assert_eq!(
        layout::measure(&set, &style, "A שלום", None).unwrap(),
        (view.info.width, view.info.height)
    );
    let empty = layout::layout(&set, &style, "", None).unwrap();
    assert_eq!(empty.view().carets(0, &mut cursors), Ok(1));
    assert_eq!(empty.view().selection(0..0, &mut rectangles), Ok(0));
    let newline = layout::layout(&set, &style, "A\n", None).unwrap();
    let n = newline.view().carets(2, &mut cursors).unwrap();
    assert!(
        cursors[..n]
            .iter()
            .any(|c| c.line == 1 && c.affinity == Affinity::Downstream)
    );
}
#[cfg(feature = "alloc")]
#[test]
fn layout_fallback_regional_han_variations_and_determinism() {
    let latin = include_bytes!("fixtures/DejaVu-shaping.ttf").to_vec();
    let cjk = include_bytes!("fixtures/NotoCJK-shaping.otf").to_vec();
    let variable = include_bytes!("fixtures/noto-sans-variable-subset.ttf").to_vec();
    let chain = [Font::parse(&latin).unwrap(), Font::parse(&cjk).unwrap()];
    let set = FontSet {
        ui: &chain,
        mono: &chain,
        generation: 123,
    };
    for (language, id) in [("ja", 23), ("ko", 23), ("zh-Hans", 24), ("zh-Hant", 25)] {
        let style = TextStyle {
            lang: Language::parse(language).unwrap(),
            ..TextStyle::default()
        };
        let output = layout::layout(&set, &style, "A骨", None).unwrap();
        assert_eq!(output.view().glyphs[1].id, id);
        assert_eq!(output.view().runs[1].face, 1);
        assert_eq!(
            layout::measure(&set, &style, "A骨", None).unwrap(),
            (output.info().width, output.info().height)
        );
    }
    let fonts = [Font::parse(&variable).unwrap()];
    let variable_set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 2,
    };
    let regular = layout::layout(&variable_set, &TextStyle::default(), "Agé", None).unwrap();
    let bold = layout::layout(
        &variable_set,
        &TextStyle {
            weight: 900,
            ..TextStyle::default()
        },
        "Agé",
        None,
    )
    .unwrap();
    assert_ne!(regular.info().width, bold.info().width);
    let style = TextStyle {
        lang: Language::parse("ja").unwrap(),
        ..TextStyle::default()
    };
    let text = "office A骨 שלום\n가각";
    let first = layout::layout(&set, &style, text, Some(Fixed::from_i32(90))).unwrap();
    let latin2 = latin.clone();
    let cjk2 = cjk.clone();
    let chain2 = [Font::parse(&latin2).unwrap(), Font::parse(&cjk2).unwrap()];
    let set2 = FontSet {
        ui: &chain2,
        mono: &chain2,
        generation: 123,
    };
    let separately_allocated_text = text.to_owned();
    let second = layout::layout(
        &set2,
        &style,
        &separately_allocated_text,
        Some(Fixed::from_i32(90)),
    )
    .unwrap();
    let mut a = vec![0; first.view().encoded_len().unwrap()];
    let mut b = vec![0; second.view().encoded_len().unwrap()];
    assert_eq!(first.view().write_bytes(&mut a), Ok(a.len()));
    assert_eq!(second.view().write_bytes(&mut b), Ok(b.len()));
    assert_eq!(a, b);
    assert_eq!(&a[..8], b"TEXT\x02\0\0\0");
    assert_eq!(&a[8..10], crate::unicode::VERSION_MAJOR.to_le_bytes());
    assert_eq!(crate::unicode::VERSION_MAJOR, 18);
    let digest = a.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
    });
    assert_eq!(digest, 0x937f_a806_1318_a96c);
    assert!(first.view().write_bytes(&mut []).is_err());
}
#[test]
fn layout_invalid_settings_capacity_and_tab_geometry() {
    let bytes = font_for(&[' ', 'A'], 1000);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle::default();
    let mut memory = Memory::new();
    assert_eq!(
        layout::measure_into(
            &set,
            &style,
            "A",
            Some(Fixed::from_i32(-1)),
            &mut memory.workspace()
        ),
        Err(TextError::InvalidInput)
    );
    let mut workspace = memory.workspace();
    workspace.glyphs = &mut [];
    assert!(layout::measure_into(&set, &style, "A", None, &mut workspace).is_err());
    let space = style.size.mul_ratio(601, 1000).unwrap();
    let expected = space.mul_ratio(5, 1).unwrap();
    assert_eq!(
        layout::measure_into(&set, &style, "A\tA", None, &mut memory.workspace())
            .unwrap()
            .0,
        expected
    );
    let mut workspace = memory.workspace();
    let mut buffers = LayoutBuffers {
        lines: &mut [],
        glyphs: &mut [],
        clusters: &mut [],
    };
    assert_eq!(
        layout::layout_into(&set, &style, "A", None, &mut workspace, &mut buffers).unwrap_err(),
        TextError::BufferTooSmall
    );
    assert_eq!(
        TextError::LimitExceeded.to_string(),
        "text work limit exceeded"
    );
    assert_eq!(TextError::Allocation.to_string(), "text allocation failed");
}

#[cfg(feature = "alloc")]
#[test]
fn layout_gdef_carets_deleted_clusters_and_malformed_fonts() {
    use super::metrics::sfnt;
    use super::shape::{Bin, coverage, lookup, with_features};
    let base = font_for(&['A', 'B'], 1000);
    let base = Font::parse(&base).unwrap();
    let outline = super::glyf::font(&[Vec::new(), Vec::new(), super::glyf::simple()], true);
    let outline = Font::parse(&outline).unwrap();
    let mut ts: Vec<_> = base.tables().map(|t| (t.tag, t.data.to_vec())).collect();
    for tag in [*b"head", *b"glyf", *b"loca"] {
        ts.retain(|t| t.0 != tag);
        ts.push((tag, outline.table(tag).unwrap().data.to_vec()));
    }
    let mut lig = Bin::words(&[1, 0, 1, 0]);
    lig.child(2, coverage(&[1]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[2, 2, 1]));
    lig.child(6, set);
    let sub = with_features(super::shape::layout(vec![lookup(4, 0, lig)]), 0);
    ts.push((*b"GSUB", sub.0));
    for (format, value, expected) in [(1, 123, 123), (2, 1, 30), (2, 999, -1)] {
        let mut gdef = Bin::words(&[1, 0, 0, 0, 0, 0]);
        let mut carets = Bin::words(&[0, 1, 0]);
        carets.child(0, coverage(&[2]));
        let mut values = Bin::words(&[1, 0]);
        values.child(2, Bin::words(&[format, value]));
        carets.child(4, values);
        gdef.child(8, carets);
        let mut tables = ts.clone();
        tables.push((*b"GDEF", gdef.0));
        let bytes = sfnt(tables);
        let fonts = [Font::parse(&bytes).unwrap()];
        let set = FontSet {
            ui: &fonts,
            mono: &fonts,
            generation: 0,
        };
        let style = TextStyle {
            size: Fixed::from_i32(1000),
            ..TextStyle::default()
        };
        let output = layout::layout(&set, &style, "AB", None);
        if expected < 0 {
            assert!(output.is_err());
            continue;
        }
        let output = output.unwrap();
        assert_eq!(output.view().glyphs.len(), 1);
        let mut cursors = [Cursor::default(); 2];
        assert_eq!(output.view().carets(1, &mut cursors), Ok(2));
        assert!(cursors.iter().all(|c| c.x == Fixed::from_i32(expected)));
        assert_eq!(
            output.view().carets(1, &mut []),
            Err(TextError::BufferTooSmall)
        );
        assert_eq!(
            output.view().selection(0..2, &mut []),
            Err(TextError::BufferTooSmall)
        );
    }
    let mut deletion = Bin::words(&[1, 0, 1, 0]);
    deletion.child(2, coverage(&[1]));
    deletion.child(6, Bin::words(&[0]));
    let sub = with_features(super::shape::layout(vec![lookup(2, 0, deletion)]), 0);
    ts.retain(|t| t.0 != *b"GSUB");
    ts.push((*b"GSUB", sub.0));
    let bytes = sfnt(ts);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let output = layout::layout(&set, &TextStyle::default(), "AB", None).unwrap();
    assert_eq!(output.info().width, Fixed::ZERO);
    assert!(output.view().glyphs.is_empty());
    assert_eq!(output.view().clusters.len(), 2);
    let mut cursors = [Cursor::default(); 2];
    assert_eq!(output.view().carets(1, &mut cursors), Ok(2));
}
#[cfg(feature = "alloc")]
#[test]
fn layout_gvar_phantoms_arabic_wrap_and_work_limits() {
    let bytes = include_bytes!("fixtures/noto-sans-variable-subset.ttf");
    let original = Font::parse(bytes).unwrap();
    let removed = super::metrics::sfnt(
        original
            .tables()
            .filter(|t| t.tag != *b"HVAR")
            .map(|t| (t.tag, t.data.to_vec()))
            .collect(),
    );
    let original = [original];
    let fallback = [Font::parse(&removed).unwrap()];
    let a = FontSet {
        ui: &original,
        mono: &original,
        generation: 0,
    };
    let b = FontSet {
        ui: &fallback,
        mono: &fallback,
        generation: 0,
    };
    let style = TextStyle {
        weight: 900,
        ..TextStyle::default()
    };
    assert_eq!(
        layout::measure(&a, &style, "Agé", None),
        layout::measure(&b, &style, "Agé", None)
    );
    let fonts = [Font::parse(include_bytes!("fixtures/DejaVu-shaping.ttf")).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle {
        size: Fixed::from_i32(2048),
        ..TextStyle::default()
    };
    let width = layout::measure(&set, &style, "سل", None).unwrap().0;
    let output = layout::layout(&set, &style, "سلام", Some(width)).unwrap();
    for line in output.view().lines {
        let standalone = layout::layout(&set, &style, &"سلام"[line.start..line.end], None).unwrap();
        let glyphs = &output.view().glyphs
            [line.glyph_start..line.glyph_start.checked_add(line.glyph_count).unwrap()];
        assert_eq!(
            glyphs
                .iter()
                .map(|g| (g.id, g.x, g.advance))
                .collect::<Vec<_>>(),
            standalone
                .view()
                .glyphs
                .iter()
                .map(|g| (g.id, g.x, g.advance))
                .collect::<Vec<_>>()
        );
    }
    assert!(output.view().lines.len() > 1);
    let huge = "A".repeat(layout::MAX_SCALARS.checked_add(1).unwrap());
    assert_eq!(
        layout::measure(&set, &style, &huge, None),
        Err(TextError::LimitExceeded)
    );
    let mut memory = Memory::new();
    assert_eq!(
        layout::measure_into(&set, &style, &huge, None, &mut memory.workspace()),
        Err(TextError::LimitExceeded)
    );
    let quadratic = "A ".repeat(1100);
    assert_eq!(
        layout::measure(&set, &style, &quadratic, Some(Fixed::from_i32(10_000_000))),
        Err(TextError::LimitExceeded)
    );
}

#[cfg(feature = "alloc")]
#[test]
fn layout_growth_controls_and_cluster_integrity() {
    use super::shape::{Bin, coverage, lookup, with_features};
    let base = font_for(&['A'], 1000);
    let base = Font::parse(&base).unwrap();
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    let mut sequence = Bin::words(&[64]);
    sequence.0.extend(std::iter::repeat_n([0, 1], 64).flatten());
    multiple.child(6, sequence);
    let sub = with_features(super::shape::layout(vec![lookup(2, 0, multiple)]), 0);
    let mut tables: Vec<_> = base.tables().map(|t| (t.tag, t.data.to_vec())).collect();
    tables.push((*b"GSUB", sub.0));
    let bytes = super::metrics::sfnt(tables);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle::default();
    let output = layout::layout(&set, &style, "A", None).unwrap();
    assert_eq!(output.view().glyphs.len(), 64);
    assert_eq!(output.view().clusters.len(), 1);
    assert_eq!(
        layout::measure(&set, &style, "A", None),
        Ok((output.info().width, output.info().height))
    );
    let fonts = [Font::parse(include_bytes!("fixtures/DejaVu-shaping.ttf")).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    for text in [
        "لاَ",
        "بَت",
        "שָׁלוֹם",
        "\u{202e}A\u{202c}",
        "\u{200d}",
        "\t",
        "\r\n",
        "\n\n",
        "A\u{2028}A",
        "अि",
    ] {
        for width in [None, Some(Fixed::ONE)] {
            let output = layout::layout(&set, &style, text, width).unwrap();
            let mut boundaries = segment::grapheme_boundaries(text);
            let mut start = boundaries.next().unwrap();
            for end in boundaries {
                assert_eq!(
                    output
                        .view()
                        .clusters
                        .iter()
                        .filter(|c| c.start == start && c.end == end)
                        .count(),
                    1,
                    "{text:?} {start}..{end}"
                );
                start = end;
            }
            assert_eq!(
                layout::measure(&set, &style, text, width),
                Ok((output.info().width, output.info().height))
            );
        }
    }
}

#[cfg(feature = "alloc")]
#[test]
fn layout_merges_overlapping_substitution_clusters() {
    use super::shape::{Bin, coverage, lookup, with_features};
    let base = font_for(&['A'], 1000);
    let base = Font::parse(&base).unwrap();
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    multiple.child(6, Bin::words(&[2, 2, 1]));
    let mut ligature = Bin::words(&[1, 0, 1, 0]);
    ligature.child(2, coverage(&[1]));
    let mut set = Bin::words(&[1, 0]);
    set.child(2, Bin::words(&[1, 2, 2]));
    ligature.child(6, set);
    let mut sub = with_features(
        super::shape::layout(vec![lookup(2, 0, multiple), lookup(4, 0, ligature)]),
        0,
    );
    let mut features = Bin::words(&[1, 0x6c69, 0x6761, 0]);
    features.child(6, Bin::words(&[0, 2, 0, 1]));
    sub.child(6, features);
    let mut tables: Vec<_> = base.tables().map(|t| (t.tag, t.data.to_vec())).collect();
    tables.push((*b"GSUB", sub.0));
    let bytes = super::metrics::sfnt(tables);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let mut memory = Memory::new();
    let mut workspace = memory.workspace();
    let mut lines = [Line::default(); 1];
    let mut glyphs = [PositionedGlyph::default(); 8];
    let mut clusters = [ClusterBox::default(); 2];
    let mut out = LayoutBuffers {
        lines: &mut lines,
        glyphs: &mut glyphs,
        clusters: &mut clusters,
    };
    let view = layout::layout_into(
        &set,
        &TextStyle::default(),
        "AA",
        None,
        &mut workspace,
        &mut out,
    )
    .unwrap();
    assert_eq!(view.glyphs.len(), 3);
    assert_eq!(view.clusters.len(), 2);
    assert!(view.glyphs.iter().all(|g| g.start == 0 && g.end == 2));
    assert_eq!(view.clusters[0].end, view.clusters[1].start);
    let owned = layout::layout(&set, &TextStyle::default(), "AA", None).unwrap();
    assert_eq!(view.info, owned.info());
}

#[cfg(feature = "alloc")]
#[test]
fn tabs_are_shaping_barriers() {
    use super::shape::{Bin, coverage, lookup, with_features};
    let base = font_for(&[' ', '\t', 'A'], 1000);
    let base = Font::parse(&base).unwrap();
    let mut ligature = Bin::words(&[1, 0, 1, 0]);
    ligature.child(2, coverage(&[1]));
    let mut ligatures = Bin::words(&[1, 0]);
    ligatures.child(2, Bin::words(&[2, 2, 1]));
    ligature.child(6, ligatures);
    let sub = with_features(super::shape::layout(vec![lookup(4, 0, ligature)]), 0);
    let mut tables: Vec<_> = base.tables().map(|t| (t.tag, t.data.to_vec())).collect();
    tables.push((*b"GSUB", sub.0));
    let bytes = super::metrics::sfnt(tables);
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle {
        size: Fixed::from_i32(1000),
        ..TextStyle::default()
    };
    assert_eq!(
        layout::layout(&set, &style, "AA", None)
            .unwrap()
            .view()
            .glyphs
            .len(),
        1
    );
    let output = layout::layout(&set, &style, "A\tA", None).unwrap();
    let glyphs = output.view().glyphs;
    assert_eq!(glyphs.len(), 3);
    assert_eq!(glyphs[1].start..glyphs[1].end, 1..2);
    assert!(glyphs[1].hidden);
    assert_eq!(glyphs[2].x, Fixed::from_i32(2404));
    assert_eq!(output.info().width, Fixed::from_i32(3005));
}

fn emergency_case(
    bytes: &[u8],
    text: &str,
    width: i32,
    expected_lines: &[(usize, usize, i32)],
    expected_glyphs: &[(usize, usize)],
) {
    let style = TextStyle {
        size: Fixed::from_i32(1000),
        ..TextStyle::default()
    };
    let width = Some(Fixed::from_i32(width));
    let mut encoded = Vec::new();
    for repeat in 0..2 {
        let bytes = bytes.to_vec();
        let text = text.to_owned();
        let fonts = [Font::parse(&bytes).unwrap()];
        let set = FontSet {
            ui: &fonts,
            mono: &fonts,
            generation: 91,
        };
        let mut memory = Memory::new();
        let mut workspace = memory.workspace();
        let mut lines = [Line::default(); 16];
        let mut glyphs = [PositionedGlyph::default(); 32];
        let mut clusters = [ClusterBox::default(); 32];
        let mut buffers = LayoutBuffers {
            lines: &mut lines,
            glyphs: &mut glyphs,
            clusters: &mut clusters,
        };
        let view =
            layout::layout_into(&set, &style, &text, width, &mut workspace, &mut buffers).unwrap();
        assert_eq!(
            view.lines
                .iter()
                .map(|line| (line.start, line.end, line.width))
                .collect::<Vec<_>>(),
            expected_lines
                .iter()
                .map(|&(start, end, width)| (start, end, Fixed::from_i32(width)))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            view.glyphs
                .iter()
                .map(|g| (g.start, g.end))
                .collect::<Vec<_>>(),
            expected_glyphs
        );
        let boundaries: Vec<_> = segment::grapheme_boundaries(&text).collect();
        for line in view.lines {
            assert!(line.start < line.end);
            assert!(boundaries.contains(&line.start));
            assert!(boundaries.contains(&line.end));
            assert_eq!(line.overflow, width.is_some_and(|w| line.width > w));
            let end = line.glyph_start.checked_add(line.glyph_count).unwrap();
            for glyph in &view.glyphs[line.glyph_start..end] {
                assert!(glyph.start >= line.start && glyph.end <= line.end);
            }
        }
        for pair in boundaries.windows(2) {
            assert_eq!(
                view.clusters
                    .iter()
                    .filter(|c| c.start == pair[0] && c.end == pair[1])
                    .count(),
                1
            );
        }
        let size = (view.info.width, view.info.height);
        let mut output = vec![0; view.encoded_len().unwrap()];
        assert_eq!(view.write_bytes(&mut output), Ok(output.len()));
        if repeat == 0 {
            encoded = output;
        } else {
            assert_eq!(output, encoded);
        }
        assert_eq!(
            layout::measure_into(&set, &style, &text, width, &mut memory.workspace()),
            Ok(size)
        );
        #[cfg(feature = "alloc")]
        {
            let owned = layout::layout(&set, &style, &text, width).unwrap();
            let mut output = vec![0; owned.view().encoded_len().unwrap()];
            assert_eq!(owned.view().write_bytes(&mut output), Ok(output.len()));
            assert_eq!(output, encoded);
            assert_eq!(layout::measure(&set, &style, &text, width), Ok(size));
        }
    }
}

#[test]
fn emergency_breaks_preserve_clusters_and_measurement() {
    let bytes = font_for(&['A', '\u{301}', 'א', 'ב', 'ג', 'ד'], 1000);
    emergency_case(
        &bytes,
        "AAAAA",
        1202,
        &[(0, 2, 1202), (2, 4, 1202), (4, 5, 601)],
        &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)],
    );
    for width in [0, 600] {
        emergency_case(
            &bytes,
            "AAA",
            width,
            &[(0, 1, 601), (1, 2, 601), (2, 3, 601)],
            &[(0, 1), (1, 2), (2, 3)],
        );
        emergency_case(
            &bytes,
            "A\u{301}A\u{301}",
            width,
            &[(0, 3, 601), (3, 6, 601)],
            &[(0, 3), (0, 3), (3, 6), (3, 6)],
        );
    }
    emergency_case(
        &bytes,
        "אבגד",
        1202,
        &[(0, 4, 1202), (4, 8, 1202)],
        &[(2, 4), (0, 2), (6, 8), (4, 6)],
    );
}

#[test]
fn emergency_breaks_keep_all_substituted_glyphs_of_a_cluster() {
    use super::shape::{Bin, coverage, lookup, with_features};
    let bytes = font_for(&['A'], 1000);
    let font = Font::parse(&bytes).unwrap();
    let mut multiple = Bin::words(&[1, 0, 1, 0]);
    multiple.child(2, coverage(&[1]));
    multiple.child(6, Bin::words(&[2, 1, 1]));
    let sub = with_features(super::shape::layout(vec![lookup(2, 0, multiple)]), 0);
    let mut tables: Vec<_> = font.tables().map(|t| (t.tag, t.data.to_vec())).collect();
    tables.push((*b"GSUB", sub.0));
    let bytes = super::metrics::sfnt(tables);
    for width in [601, 1803] {
        emergency_case(
            &bytes,
            "AAA",
            width,
            &[(0, 1, 1202), (1, 2, 1202), (2, 3, 1202)],
            &[(0, 1), (0, 1), (1, 2), (1, 2), (2, 3), (2, 3)],
        );
    }
}

/// BASE v1.0 with an ideographic default baseline and a nonzero `romn`
/// coordinate (`docs/microsoft/base.html:1`).
fn base_table() -> Vec<u8> {
    use super::metrics::put16;
    let mut d = vec![0; 52];
    put16(&mut d, 0, 1); // majorVersion
    put16(&mut d, 4, 8); // horizAxisOffset
    put16(&mut d, 8, 4); // baseTagListOffset, from the axis table at 8
    put16(&mut d, 10, 14); // baseScriptListOffset
    put16(&mut d, 12, 2); // baseTagCount
    d[14..22].copy_from_slice(b"ideoromn");
    put16(&mut d, 22, 1); // baseScriptCount
    d[24..28].copy_from_slice(b"latn");
    put16(&mut d, 28, 8); // baseScriptOffset, from the script list at 22
    put16(&mut d, 30, 6); // baseValuesOffset, from the script table at 30
    put16(&mut d, 36, 0); // defaultBaselineIndex: ideographic, not alphabetic
    put16(&mut d, 38, 2); // baseCoordCount
    put16(&mut d, 40, 8); // baseCoords[0], from the values table at 36
    put16(&mut d, 42, 12); // baseCoords[1]
    put16(&mut d, 44, 1); // ideo BaseCoord format 1
    put16(&mut d, 46, 0); // ideo coordinate
    put16(&mut d, 48, 1); // romn BaseCoord format 1
    put16(&mut d, 50, (-120_i16).cast_unsigned()); // romn coordinate
    d
}

#[test]
fn a_declared_baseline_does_not_move_a_run_off_the_alphabetic_baseline() {
    let latin = font_for(&['A'], 1000);
    let plain = font_for(&['中'], 2000);
    let mut tables: Vec<_> = Font::parse(&plain)
        .unwrap()
        .tables()
        .map(|t| (t.tag, t.data.to_vec()))
        .collect();
    tables.push((*b"BASE", base_table()));
    let declared = super::metrics::sfnt(tables);
    let style = TextStyle {
        size: Fixed::from_i32(100),
        ..TextStyle::default()
    };
    let mut encoded = Vec::new();
    for second in [&plain, &declared] {
        let fonts = [Font::parse(&latin).unwrap(), Font::parse(second).unwrap()];
        let set = FontSet {
            ui: &fonts,
            mono: &fonts,
            generation: 5,
        };
        let mut memory = Memory::new();
        let mut workspace = memory.workspace();
        let mut lines = [Line::default(); 4];
        let mut glyphs = [PositionedGlyph::default(); 8];
        let mut clusters = [ClusterBox::default(); 8];
        let mut buffers = LayoutBuffers {
            lines: &mut lines,
            glyphs: &mut glyphs,
            clusters: &mut clusters,
        };
        let view =
            layout::layout_into(&set, &style, "A中", None, &mut workspace, &mut buffers).unwrap();
        assert_eq!(view.runs.len(), 2);
        assert_ne!(view.runs[0].scale, view.runs[1].scale);
        let line = view.lines[0];
        assert_eq!(line.baseline, Fixed::from_i32(80));
        assert!(view.glyphs.iter().all(|g| g.y == line.baseline));
        let mut output = vec![0; view.encoded_len().unwrap()];
        assert_eq!(view.write_bytes(&mut output), Ok(output.len()));
        if encoded.is_empty() {
            encoded = output;
        } else {
            assert_eq!(output, encoded);
        }
    }
}

#[test]
fn one_layout_shares_one_shaping_budget() {
    use super::shape::{Bin, coverage, featured, lookup};
    let mut none = Bin::words(&[1, 0, 0]);
    none.child(2, coverage(&[]));
    let gsub = featured(vec![lookup(1, 0, none)], *b"ccmp", &vec![0; 65_535]);
    let original = Font::parse(include_bytes!("fixtures/DejaVu-shaping.ttf")).unwrap();
    let bytes = super::metrics::sfnt(
        original
            .tables()
            .filter(|t| t.tag != *b"GSUB")
            .map(|t| (t.tag, t.data.to_vec()))
            .chain([(*b"GSUB", gsub.0)])
            .collect(),
    );
    let fonts = [Font::parse(&bytes).unwrap()];
    let set = FontSet {
        ui: &fonts,
        mono: &fonts,
        generation: 0,
    };
    let style = TextStyle::default();
    assert!(layout::measure(&set, &style, &"a\n".repeat(100), None).is_ok());
    assert_eq!(
        layout::measure(&set, &style, &"a\n".repeat(1000), None),
        Err(TextError::Font(crate::FontError::LimitExceeded))
    );
}
