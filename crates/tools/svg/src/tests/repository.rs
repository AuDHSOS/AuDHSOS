// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The figures of this repository, which are the drawings this crate has
//! to be right about.

use std::path::{Path, PathBuf};

use doc_pdf::units::Mils;

use crate::{Item, parse};

/// The workspace root, from the manifest directory of this crate.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

/// Every SVG kept under `docs/`.
fn figures() -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(&root().join("docs"), &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "svg") {
            found.push(path);
        }
    }
}

#[test]
fn every_figure_comes_out_as_marks() {
    for path in figures() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let name = path.display().to_string();
        let Some(drawing) = parse(&text) else {
            panic!("{name} came out with nothing on it");
        };
        assert!(
            drawing.width > 0 && drawing.height > 0,
            "{name} has no size"
        );
        assert!(
            drawing.items.len() > 2,
            "{name} came out with {} marks",
            drawing.items.len()
        );
    }
}

#[test]
fn every_mark_of_every_figure_stands_inside_it() {
    // A mark outside the drawing is a transform read the wrong way round,
    // and it is the failure that would be hardest to see in a page of a
    // thousand others. A little slack is allowed for a stroke that sits on
    // the edge and for a marker that overhangs the line it ends.
    for path in figures() {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(drawing) = parse(&text) else {
            continue;
        };
        let slack = drawing.width.max(drawing.height).wrapping_div(4);
        for item in &drawing.items {
            for (x, y) in points(item) {
                assert!(
                    x >= slack.saturating_neg()
                        && y >= slack.saturating_neg()
                        && x <= drawing.width.saturating_add(slack)
                        && y <= drawing.height.saturating_add(slack),
                    "{} draws at ({x}, {y}) outside {} by {}",
                    path.display(),
                    drawing.width,
                    drawing.height
                );
            }
        }
    }
}

#[test]
fn a_figure_keeps_its_shape_when_it_is_placed() {
    let Some(path) = figures().into_iter().next() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Some(drawing) = parse(&text) else { return };
    let width: Mils = 400_000;
    let height = drawing.height_at(width);
    let placed = drawing.placed(0, 0, width);
    assert_eq!(placed.len(), drawing.items.len());
    for item in &placed {
        for (x, y) in points(item) {
            assert!(x >= -100_000 && x <= width.saturating_add(100_000), "{x}");
            assert!(y >= -100_000 && y <= height.saturating_add(100_000), "{y}");
        }
    }
}

/// Every point one mark stands on.
fn points(item: &Item) -> Vec<(Mils, Mils)> {
    match item {
        Item::Text { x, y, .. } => vec![(*x, *y)],
        Item::Path { segments, .. } => segments
            .iter()
            .filter_map(|segment| match *segment {
                doc_pdf::page::Segment::Move { x, y }
                | doc_pdf::page::Segment::Line { x, y }
                | doc_pdf::page::Segment::Curve { x, y, .. } => Some((x, y)),
                doc_pdf::page::Segment::Close => None,
            })
            .collect(),
    }
}

#[test]
fn a_stroke_that_asks_for_an_arrowhead_gets_one() {
    // Figure 1 draws every link as a stroke with a marker at its end. The
    // marker is a filled triangle, and a triangle is the one shape in that
    // drawing that is filled and not stroked, so counting those counts the
    // arrowheads.
    let path = root().join("docs/ecma/img/figure-1.svg");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Some(drawing) = parse(&text) else {
        panic!("figure 1 came out with nothing on it");
    };
    let heads = drawing
        .items
        .iter()
        .filter(|item| {
            matches!(
                item,
                Item::Path {
                    fill: Some(_),
                    stroke: None,
                    ..
                }
            )
        })
        .count();
    // Ten links are drawn in the figure; the eleventh path of the file is
    // the arrowhead itself, which stands in the definitions.
    assert_eq!(heads, 10, "figure 1 came out with {heads} arrowheads");
}
