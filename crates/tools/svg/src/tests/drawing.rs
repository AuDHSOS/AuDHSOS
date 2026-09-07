// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Whole drawings, read from the markup they are written in.

use doc_pdf::font::Font;
use doc_pdf::page::{Fill, Segment};
use doc_pdf::units::Color;

use crate::{Drawing, Item, parse};

/// The first path of a drawing.
fn first_path(drawing: &Drawing) -> Option<(&Vec<Segment>, Option<Fill>)> {
    drawing.items.iter().find_map(|item| match item {
        Item::Path { segments, fill, .. } => Some((segments, *fill)),
        Item::Text { .. } => None,
    })
}

/// The first line of text of a drawing.
fn first_text(drawing: &Drawing) -> Option<&Item> {
    drawing
        .items
        .iter()
        .find(|item| matches!(item, Item::Text { .. }))
}

/// Where a path starts.
fn start(drawing: &Drawing) -> Option<(i64, i64)> {
    match first_path(drawing)?.0.first() {
        Some(Segment::Move { x, y }) => Some((*x, *y)),
        _ => None,
    }
}

#[test]
fn what_is_not_a_drawing_is_not_read() {
    assert_eq!(parse(""), None);
    assert_eq!(parse("<p>text</p>"), None);
    assert_eq!(parse("<svg width=\"10\" height=\"10\"></svg>"), None);
}

#[test]
fn a_drawing_is_as_large_as_its_view_box_says() {
    let drawing = parse("<svg viewBox=\"0 0 65 215\"><rect width=\"10\" height=\"10\"/></svg>")
        .expect("a drawing");
    assert_eq!(drawing.width, 65_000);
    assert_eq!(drawing.height, 215_000);
}

#[test]
fn a_drawing_without_a_view_box_is_as_large_as_it_says_it_is() {
    let drawing = parse("<svg width=\"719\" height=\"354\"><rect width=\"1\" height=\"1\"/></svg>")
        .expect("a drawing");
    assert_eq!(drawing.width, 719_000);
    assert_eq!(drawing.height, 354_000);
}

#[test]
fn a_view_box_that_does_not_start_at_the_origin_is_moved_to_it() {
    let drawing = parse(
        "<svg viewBox=\"10 10 20 20\"><rect x=\"10\" y=\"10\" width=\"5\" height=\"5\"/></svg>",
    )
    .expect("a drawing");
    // The corner of the rectangle is the corner of the view box, so it
    // stands at the top left of the drawing: x nothing, y the full height.
    assert_eq!(start(&drawing), Some((0, 20_000)));
}

#[test]
fn the_drawing_is_turned_over_so_that_y_grows_upwards() {
    let drawing =
        parse("<svg viewBox=\"0 0 10 10\"><rect y=\"2\" width=\"4\" height=\"4\"/></svg>")
            .expect("a drawing");
    // The rectangle sits two units below the top in SVG, which is eight
    // units above the bottom once the drawing is turned over.
    assert_eq!(start(&drawing), Some((0, 8000)));
}

#[test]
fn a_shape_takes_the_colours_it_is_given() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><rect width=\"4\" height=\"4\" fill=\"#fff\" stroke=\"black\" stroke-width=\"2\"/></svg>",
    )
    .expect("a drawing");
    let Some(Item::Path { fill, stroke, .. }) = drawing.items.first() else {
        panic!("no path");
    };
    assert_eq!(
        *fill,
        Some(Fill {
            color: Color::rgb(255, 255, 255),
            even_odd: false,
        })
    );
    assert_eq!(stroke.as_ref().map(|stroke| stroke.width), Some(2000));
    assert_eq!(
        stroke.as_ref().map(|stroke| stroke.color),
        Some(Color::BLACK)
    );
}

#[test]
fn a_stylesheet_dresses_what_carries_no_colour_of_its_own() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><style>rect.box { fill: none; stroke: black }</style><rect class=\"box\" width=\"4\" height=\"4\"/></svg>",
    )
    .expect("a drawing");
    let Some(Item::Path { fill, stroke, .. }) = drawing.items.first() else {
        panic!("no path");
    };
    assert_eq!(*fill, None);
    assert!(stroke.is_some());
}

#[test]
fn a_style_attribute_stands_over_the_stylesheet_and_the_attributes() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><style>rect { fill: red }</style><rect fill=\"blue\" style=\"fill: #fff\" width=\"4\" height=\"4\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(
        first_path(&drawing)
            .and_then(|(_, fill)| fill)
            .map(|fill| fill.color),
        Some(Color::rgb(255, 255, 255))
    );
}

#[test]
fn a_group_hands_its_transform_and_its_colours_down() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><g transform=\"translate(2 0)\" fill=\"black\"><rect width=\"2\" height=\"2\"/></g></svg>",
    )
    .expect("a drawing");
    assert_eq!(start(&drawing), Some((2000, 10_000)));
}

#[test]
fn what_a_use_points_at_is_drawn_where_the_use_stands() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><defs><g id=\"box\"><rect width=\"2\" height=\"2\"/></g></defs><use xlink:href=\"#box\" x=\"5\" y=\"5\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 1);
    assert_eq!(start(&drawing), Some((5000, 15_000)));
}

#[test]
fn a_use_that_points_at_nothing_draws_nothing() {
    assert_eq!(
        parse("<svg viewBox=\"0 0 10 10\"><use href=\"#missing\"/><rect width=\"1\" height=\"1\"/></svg>")
            .map(|drawing| drawing.items.len()),
        Some(1)
    );
    assert_eq!(
        parse("<svg viewBox=\"0 0 10 10\"><use/><rect width=\"1\" height=\"1\"/></svg>")
            .map(|drawing| drawing.items.len()),
        Some(1)
    );
}

#[test]
fn a_use_that_points_at_a_shape_draws_that_shape() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><defs><rect id=\"r\" width=\"2\" height=\"2\"/></defs><use href=\"#r\" x=\"4\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(start(&drawing), Some((4000, 20_000)));
}

#[test]
fn what_carries_no_marks_carries_none() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><title>A drawing</title><desc>About it</desc><defs><rect width=\"9\" height=\"9\"/></defs><rect width=\"1\" height=\"1\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 1);
}

#[test]
fn a_line_of_text_stands_where_it_is_put() {
    let drawing = parse(
        "<svg viewBox=\"0 0 100 20\"><text x=\"10\" y=\"15\" font-size=\"8\">Hello</text></svg>",
    )
    .expect("a drawing");
    let Some(Item::Text {
        x,
        y,
        size,
        font,
        text,
        ..
    }) = first_text(&drawing)
    else {
        panic!("no text");
    };
    assert_eq!(*x, 10_000);
    assert_eq!(*y, 5000);
    assert_eq!(*size, 8000);
    assert_eq!(*font, Font::Regular);
    assert_eq!(text, "Hello");
}

#[test]
fn an_anchor_moves_the_text_back_by_what_it_measures() {
    let middle = parse(
        "<svg viewBox=\"0 0 100 20\"><text x=\"50\" y=\"10\" text-anchor=\"middle\">Hello</text></svg>",
    )
    .expect("a drawing");
    let end = parse(
        "<svg viewBox=\"0 0 100 20\"><text x=\"50\" y=\"10\" text-anchor=\"end\">Hello</text></svg>",
    )
    .expect("a drawing");
    let start_of = |drawing: &Drawing| match first_text(drawing) {
        Some(Item::Text { x, .. }) => *x,
        _ => 0,
    };
    assert!(start_of(&middle) < 50_000);
    assert!(start_of(&end) < start_of(&middle));
}

#[test]
fn text_of_nothing_and_text_of_no_colour_say_nothing() {
    assert_eq!(
        parse("<svg viewBox=\"0 0 10 10\"><text x=\"1\" y=\"1\">  </text><rect width=\"1\" height=\"1\"/></svg>")
            .map(|drawing| drawing.items.len()),
        Some(1)
    );
    assert_eq!(
        parse("<svg viewBox=\"0 0 10 10\"><text fill=\"none\">x</text><rect width=\"1\" height=\"1\"/></svg>")
            .map(|drawing| drawing.items.len()),
        Some(1)
    );
}

#[test]
fn a_marker_is_drawn_at_the_end_of_the_line_it_belongs_to() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><defs><marker id=\"head\" viewBox=\"0 0 10 10\" refX=\"0\" refY=\"5\" markerUnits=\"userSpaceOnUse\" markerWidth=\"10\" markerHeight=\"10\"><path d=\"M0 0 L10 5 L0 10 z\"/></marker></defs><path d=\"M0 10 L10 10\" stroke=\"black\" marker-end=\"url(#head)\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 2, "{:?}", drawing.items);
    let Some(Item::Path { segments, fill, .. }) = drawing.items.get(1) else {
        panic!("no marker");
    };
    assert!(fill.is_some(), "an arrowhead is filled");
    // The line runs to the right and ends ten across at half the height.
    // The head is centred on that point — its reference is the middle of
    // its own box — so it begins five above the line and its tip lies ten
    // further along, in the direction the line was going.
    assert_eq!(
        segments.first(),
        Some(&Segment::Move {
            x: 10_000,
            y: 15_000
        })
    );
    assert_eq!(
        segments.get(1),
        Some(&Segment::Line {
            x: 20_000,
            y: 10_000
        })
    );
}

#[test]
fn a_marker_that_is_not_there_leaves_the_line_alone() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><path d=\"M0 0 L10 10\" stroke=\"black\" marker-end=\"url(#missing)\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 1);
}

#[test]
fn a_drawing_that_points_at_itself_does_not_run_for_ever() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><g id=\"loop\"><use href=\"#loop\"/><rect width=\"1\" height=\"1\"/></g></svg>",
    )
    .expect("a drawing");
    assert!(drawing.items.len() < 40, "{}", drawing.items.len());
}

#[test]
fn a_drawing_is_scaled_and_placed_where_it_is_asked_for() {
    let drawing = parse("<svg viewBox=\"0 0 10 20\"><rect width=\"10\" height=\"20\"/></svg>")
        .expect("a drawing");
    assert_eq!(drawing.height_at(100_000), 200_000);
    let items = drawing.placed(50_000, 25_000, 100_000);
    let Some(Item::Path { segments, .. }) = items.first() else {
        panic!("no path");
    };
    assert_eq!(
        segments.first(),
        Some(&Segment::Move {
            x: 50_000,
            y: 225_000
        })
    );
}

#[test]
fn a_placed_drawing_keeps_its_text_and_its_stroke() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><path d=\"M0 0 L10 10\" stroke=\"black\" stroke-width=\"1\" stroke-dasharray=\"2 2\"/><text x=\"1\" y=\"9\" font-size=\"2\">x</text></svg>",
    )
    .expect("a drawing");
    let items = drawing.placed(0, 0, 100_000);
    let stroke = items.iter().find_map(|item| match item {
        Item::Path { stroke, .. } => stroke.clone(),
        Item::Text { .. } => None,
    });
    assert_eq!(stroke.as_ref().map(|stroke| stroke.width), Some(10_000));
    assert_eq!(
        stroke.map(|stroke| stroke.dashes),
        Some(vec![20_000, 20_000])
    );
    let size = items.iter().find_map(|item| match item {
        Item::Text { size, .. } => Some(*size),
        Item::Path { .. } => None,
    });
    assert_eq!(size, Some(20_000));
}

#[test]
fn a_curve_is_scaled_at_every_point_it_bends_through() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><path d=\"M0 10 C0 0 10 0 10 10\" fill=\"black\"/></svg>",
    )
    .expect("a drawing");
    let items = drawing.placed(0, 0, 20_000);
    let Some(Item::Path { segments, .. }) = items.first() else {
        panic!("no path");
    };
    assert_eq!(
        segments.get(1),
        Some(&Segment::Curve {
            x1: 0,
            y1: 20_000,
            x2: 20_000,
            y2: 20_000,
            x: 20_000,
            y: 0,
        })
    );
}

#[test]
fn a_drawing_inside_another_document_is_still_found() {
    let drawing = parse("<html><body><p>text</p><svg viewBox=\"0 0 10 10\"><rect width=\"2\" height=\"2\"/></svg></body></html>")
        .expect("a drawing");
    assert_eq!(drawing.width, 10_000);
}

#[test]
fn a_view_box_that_says_too_little_is_not_a_view_box() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10\" width=\"40\" height=\"20\"><rect width=\"2\" height=\"2\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!((drawing.width, drawing.height), (40_000, 20_000));
    let empty = parse(
        "<svg viewBox=\"0 0 0 10\" width=\"40\" height=\"20\"><rect width=\"2\" height=\"2\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!((empty.width, empty.height), (40_000, 20_000));
    let flat = parse(
        "<svg viewBox=\"0 0 10 0\" width=\"40\" height=\"20\"><rect width=\"2\" height=\"2\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!((flat.width, flat.height), (40_000, 20_000));
}

#[test]
fn a_drawing_that_says_no_size_at_all_is_a_thousand_units_square() {
    let drawing = parse("<svg><rect width=\"2\" height=\"2\"/></svg>").expect("a drawing");
    assert_eq!((drawing.width, drawing.height), (1_000_000, 1_000_000));
}

#[test]
fn an_element_nobody_named_is_read_for_what_is_inside_it() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><foreignObject><rect width=\"2\" height=\"2\"/></foreignObject></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 1);
}

#[test]
fn a_style_attribute_of_nonsense_changes_nothing() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\"><rect width=\"2\" height=\"2\" style=\"nonsense; fill: #fff\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(
        first_path(&drawing)
            .and_then(|(_, fill)| fill)
            .map(|fill| fill.color),
        Some(Color::rgb(255, 255, 255))
    );
}

#[test]
fn text_outside_a_stylesheet_is_not_a_stylesheet() {
    let drawing = parse(
        "<svg viewBox=\"0 0 10 10\">rect { fill: none }<rect width=\"2\" height=\"2\"/></svg>",
    )
    .expect("a drawing");
    assert!(
        first_path(&drawing).and_then(|(_, fill)| fill).is_some(),
        "text that only looks like a rule was read as one"
    );
}

#[test]
fn a_marker_measured_in_stroke_widths_grows_with_the_stroke() {
    let drawing = |width: &str| {
        parse(&format!(
            "<svg viewBox=\"0 0 40 40\"><defs><marker id=\"h\" viewBox=\"0 0 10 10\" refX=\"0\" refY=\"5\" markerWidth=\"10\" markerHeight=\"10\"><path d=\"M0 0 L10 5 L0 10 z\"/></marker></defs><path d=\"M0 20 L10 20\" stroke=\"black\" stroke-width=\"{width}\" marker-end=\"url(#h)\"/></svg>"
        ))
        .expect("a drawing")
    };
    let tip = |drawing: &Drawing| match drawing.items.get(1) {
        Some(Item::Path { segments, .. }) => match segments.get(1) {
            Some(Segment::Line { x, .. }) => *x,
            _ => 0,
        },
        _ => 0,
    };
    assert!(
        tip(&drawing("2")) > tip(&drawing("1")),
        "a marker in stroke widths did not grow with the stroke"
    );
}

#[test]
fn a_marker_on_a_path_that_goes_nowhere_is_drawn_where_the_path_is() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><defs><marker id=\"h\" markerUnits=\"userSpaceOnUse\"><path d=\"M0 0 L2 1 L0 2 z\"/></marker></defs><path d=\"M5 5\" stroke=\"black\" marker-end=\"url(#h)\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 2, "{:?}", drawing.items);
}

#[test]
fn a_path_of_nothing_but_a_close_carries_no_marker() {
    let drawing = parse(
        "<svg viewBox=\"0 0 20 20\"><defs><marker id=\"h\"><path d=\"M0 0 L2 1 L0 2 z\"/></marker></defs><path d=\"Z\" stroke=\"black\" marker-end=\"url(#h)\"/><rect width=\"1\" height=\"1\"/></svg>",
    )
    .expect("a drawing");
    assert_eq!(drawing.items.len(), 2, "{:?}", drawing.items);
}
