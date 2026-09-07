// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pieces a drawing is read out of: numbers, transforms, colours,
//! stylesheets, and path data.

use doc_pdf::font::Font;
use doc_pdf::page::Segment;
use doc_pdf::units::Color;

use crate::css::Sheet;
use crate::matrix::{self, Matrix};
use crate::number::{self, ONE};
use crate::paint::{Anchor, State, color};
use crate::path;

#[test]
fn a_number_is_read_as_thousandths() {
    assert_eq!(number::value("1"), Some(1000));
    assert_eq!(number::value("120.5"), Some(120_500));
    assert_eq!(number::value("-3.21636"), Some(-3216));
    assert_eq!(number::value(" .5 "), Some(500));
    assert_eq!(number::value("+2"), Some(2000));
}

#[test]
fn a_number_may_carry_an_exponent() {
    assert_eq!(number::value("1e3"), Some(1_000_000));
    assert_eq!(number::value("1.5E2"), Some(150_000));
    assert_eq!(number::value("2e-1"), Some(200));
}

#[test]
fn a_length_in_pixels_is_the_user_unit_and_anything_else_is_refused() {
    assert_eq!(number::value("12px"), Some(12000));
    assert_eq!(number::value("12pt"), None);
    assert_eq!(number::value("wide"), None);
    assert_eq!(number::value(""), None);
}

#[test]
fn a_list_of_numbers_is_read_however_it_is_separated() {
    assert_eq!(number::list("0 0 65 215"), vec![0, 0, 65_000, 215_000]);
    assert_eq!(number::list("1,2 -3"), vec![1000, 2000, -3000]);
    assert_eq!(number::list("none"), Vec::<i64>::new());
}

#[test]
fn a_number_that_is_not_one_is_read_as_none() {
    assert_eq!(number::read("."), None);
    assert_eq!(number::read("-"), None);
    assert_eq!(number::read("x"), None);
}

#[test]
fn nothing_transforms_a_point_that_stays_where_it_is() {
    assert_eq!(Matrix::IDENTITY.apply(1000, 2000), (1000, 2000));
    assert_eq!(Matrix::IDENTITY.scale(), ONE);
}

#[test]
fn a_move_and_a_scale_do_what_they_say() {
    let moved = matrix::parse("translate(10 5)");
    assert_eq!(moved.apply(0, 0), (10_000, 5000));
    let scaled = matrix::parse("scale(2)");
    assert_eq!(scaled.apply(1000, 1000), (2000, 2000));
    assert_eq!(scaled.scale(), 2000);
    let one_way = matrix::parse("scale(2 3)");
    assert_eq!(one_way.apply(1000, 1000), (2000, 3000));
}

#[test]
fn transforms_are_applied_left_to_right() {
    // The move happens in the scaled space, which is what SVG says: the
    // point ends at twice the move, not at the move plus twice the point.
    let matrix = matrix::parse("scale(2) translate(10 0)");
    assert_eq!(matrix.apply(0, 0), (20_000, 0));
}

#[test]
fn a_matrix_is_its_six_numbers() {
    let matrix = matrix::parse("matrix(-3.21636 0 0 -2.22658 411.563 199.015)");
    assert_eq!(matrix.apply(0, 0), (411_563, 199_015));
    assert_eq!(matrix.apply(1000, 1000), (408_347, 196_789));
}

#[test]
fn a_quarter_turn_turns_a_point_a_quarter() {
    let turned = matrix::parse("rotate(90)");
    assert_eq!(turned.apply(1000, 0), (0, 1000));
    let half = matrix::parse("rotate(180)");
    assert_eq!(half.apply(1000, 0), (-1000, 0));
    let about = matrix::parse("rotate(90 1 0)");
    assert_eq!(about.apply(1000, 0), (1000, 0));
}

#[test]
fn a_transform_this_crate_does_not_know_changes_nothing() {
    assert_eq!(matrix::parse("skewX(20)"), Matrix::IDENTITY);
    assert_eq!(matrix::parse("translate()"), Matrix::IDENTITY);
    assert_eq!(matrix::parse("nonsense"), Matrix::IDENTITY);
}

#[test]
fn a_colour_is_read_by_word_by_digits_or_by_components() {
    assert_eq!(color("black"), Some(Color::BLACK));
    assert_eq!(color("#fff"), Some(Color::rgb(255, 255, 255)));
    assert_eq!(color("#1a2b3c"), Some(Color::rgb(26, 43, 60)));
    assert_eq!(color("rgb(1,2,3)"), Some(Color::rgb(1, 2, 3)));
    assert_eq!(color("WHITE"), Some(Color::rgb(255, 255, 255)));
    assert_eq!(color("chartreuse"), None);
    assert_eq!(color("#12345"), None);
}

#[test]
fn a_state_starts_black_and_unstroked() {
    let state = State::default();
    assert_eq!(state.fill, Some(Color::BLACK));
    assert_eq!(state.stroke, None);
    assert_eq!(state.font, Font::Regular);
    assert_eq!(state.anchor, Anchor::Start);
}

#[test]
fn a_declaration_changes_what_it_names_and_nothing_else() {
    let mut state = State::default();
    state.apply("fill", "none");
    state.apply("stroke", "#000");
    state.apply("stroke-width", "2");
    state.apply("stroke-dasharray", "2,2");
    state.apply("fill-rule", "evenodd");
    state.apply("text-anchor", "middle");
    state.apply("marker-end", "url(#arrowhead)");
    assert_eq!(state.fill, None);
    assert_eq!(state.stroke, Some(Color::BLACK));
    assert_eq!(state.width, 2000);
    assert_eq!(state.dashes, vec![2000, 2000]);
    assert!(state.even_odd);
    assert_eq!(state.anchor, Anchor::Middle);
    assert_eq!(state.marker_end.as_deref(), Some("arrowhead"));
    state.apply("stroke-dasharray", "none");
    assert!(state.dashes.is_empty());
    state.apply("text-anchor", "end");
    assert_eq!(state.anchor, Anchor::End);
}

#[test]
fn a_family_and_a_weight_choose_the_face_between_them() {
    let mut state = State::default();
    state.apply("font-family", "Courier New, monospace");
    assert_eq!(state.font, Font::Mono);
    state.apply("font-weight", "bold");
    assert_eq!(state.font, Font::MonoBold);
    state.apply("font-family", "Arial, sans-serif");
    assert_eq!(state.font, Font::Bold);
    state.apply("font-weight", "normal");
    assert_eq!(state.font, Font::Regular);
}

#[test]
fn a_size_may_be_a_share_of_the_one_it_inherits() {
    let mut state = State::default();
    state.apply("font-size", "20");
    assert_eq!(state.size, 20_000);
    state.apply("font-size", "60%");
    assert_eq!(state.size, 12_000);
    state.apply("font-size", "nonsense");
    assert_eq!(state.size, 12_000);
}

#[test]
fn a_paint_that_says_nothing_keeps_what_was_there() {
    let mut state = State::default();
    state.apply("fill", "inherit");
    assert_eq!(state.fill, Some(Color::BLACK));
    state.apply("fill", "not-a-colour");
    assert_eq!(state.fill, Some(Color::BLACK));
    state.apply("marker-end", "none");
    assert_eq!(state.marker_end, None);
}

#[test]
fn a_rule_reaches_what_its_selector_names() {
    let sheet = Sheet::parse(
        "text { font-family: Arial; } text.pn { font-family: monospace; } .box { fill: white; }",
    );
    assert_eq!(
        sheet.declarations("text", ""),
        vec![("font-family", "Arial")]
    );
    assert_eq!(
        sheet.declarations("text", "pn"),
        vec![("font-family", "Arial"), ("font-family", "monospace")]
    );
    assert_eq!(sheet.declarations("rect", "box"), vec![("fill", "white")]);
    assert!(sheet.declarations("path", "").is_empty());
}

#[test]
fn the_stronger_selector_comes_last_so_that_it_stands() {
    let sheet = Sheet::parse(".a { fill: red } rect { fill: blue } rect.a { fill: green }");
    let found: Vec<&str> = sheet
        .declarations("rect", "a")
        .iter()
        .map(|(_, value)| *value)
        .collect();
    assert_eq!(found, vec!["blue", "red", "green"]);
}

#[test]
fn a_comment_and_a_selector_this_crate_does_not_read_are_dropped() {
    let sheet = Sheet::parse(
        "/* a note */ g rect { fill: red } #id { fill: red } rect[x] { fill: red } .b { fill: white }",
    );
    assert_eq!(sheet.declarations("rect", "b"), vec![("fill", "white")]);
    assert!(Sheet::parse("rect { }").declarations("rect", "").is_empty());
    assert!(Sheet::parse("rect {").declarations("rect", "").is_empty());
}

#[test]
fn a_path_is_read_command_by_command() {
    assert_eq!(
        path::data("M 1 2 L 3 4 Z"),
        vec![
            Segment::Move { x: 1000, y: 2000 },
            Segment::Line { x: 3000, y: 4000 },
            Segment::Close,
        ]
    );
}

#[test]
fn a_lower_case_command_is_read_from_where_the_pen_stands() {
    assert_eq!(
        path::data("m1 1 l2 2 h1 v1"),
        vec![
            Segment::Move { x: 1000, y: 1000 },
            Segment::Line { x: 3000, y: 3000 },
            Segment::Line { x: 4000, y: 3000 },
            Segment::Line { x: 4000, y: 4000 },
        ]
    );
}

#[test]
fn a_second_set_of_arguments_after_a_move_is_a_line() {
    assert_eq!(
        path::data("M0 0 1 1 2 2"),
        vec![
            Segment::Move { x: 0, y: 0 },
            Segment::Line { x: 1000, y: 1000 },
            Segment::Line { x: 2000, y: 2000 },
        ]
    );
}

#[test]
fn a_curve_keeps_its_controls_and_a_smooth_one_mirrors_them() {
    let segments = path::data("M0 0 C1 1 2 2 3 3 S4 4 5 5");
    assert_eq!(
        segments.get(2),
        Some(&Segment::Curve {
            x1: 4000,
            y1: 4000,
            x2: 4000,
            y2: 4000,
            x: 5000,
            y: 5000,
        })
    );
}

#[test]
fn a_quadratic_curve_becomes_the_cubic_it_is() {
    assert_eq!(
        path::data("M0 0 Q3 3 6 0"),
        vec![
            Segment::Move { x: 0, y: 0 },
            Segment::Curve {
                x1: 2000,
                y1: 2000,
                x2: 4000,
                y2: 2000,
                x: 6000,
                y: 0,
            },
        ]
    );
    assert_eq!(path::data("M0 0 Q3 3 6 0 T12 0").len(), 3);
}

#[test]
fn an_arc_is_drawn_as_the_line_to_where_it_ends() {
    assert_eq!(
        path::data("M0 0 A5 5 0 0 1 10 10"),
        vec![
            Segment::Move { x: 0, y: 0 },
            Segment::Line {
                x: 10_000,
                y: 10_000
            },
        ]
    );
}

#[test]
fn a_path_of_nonsense_stops_where_the_nonsense_starts() {
    assert_eq!(path::data(""), Vec::new());
    assert_eq!(path::data("L"), Vec::new());
    assert_eq!(path::data("M0 0 L1"), vec![Segment::Move { x: 0, y: 0 }]);
}

#[test]
fn a_rectangle_is_four_corners_and_a_round_one_is_eight() {
    assert_eq!(path::rectangle(0, 0, 2000, 1000, 0, 0).len(), 5);
    assert_eq!(path::rectangle(0, 0, 2000, 1000, 500, 500).len(), 10);
    assert!(path::rectangle(0, 0, 0, 1000, 0, 0).is_empty());
}

#[test]
fn a_circle_is_four_curves_and_one_of_no_size_is_nothing() {
    assert_eq!(path::ellipse(0, 0, 1000, 1000).len(), 6);
    assert!(path::ellipse(0, 0, 0, 1000).is_empty());
}

#[test]
fn a_run_of_points_is_joined_and_closed_if_it_asks_to_be() {
    let points = [0, 0, 1000, 0, 1000, 1000];
    assert_eq!(path::polygon(&points, false).len(), 3);
    assert_eq!(path::polygon(&points, true).len(), 4);
    assert!(path::polygon(&[], true).is_empty());
    assert_eq!(path::polygon(&[0, 0, 1000], false).len(), 1);
}

#[test]
fn a_declaration_of_nothing_is_no_declaration() {
    let sheet = Sheet::parse("rect { : red; fill: ; stroke: black }");
    assert_eq!(sheet.declarations("rect", ""), vec![("stroke", "black")]);
}

#[test]
fn a_selector_of_nothing_and_a_selector_of_two_classes_are_dropped() {
    let sheet = Sheet::parse(", rect { fill: red } .a.b { fill: white }");
    assert_eq!(sheet.declarations("rect", "a b"), vec![("fill", "red")]);
}

#[test]
fn a_transform_whose_brackets_never_close_ends_where_it_stops() {
    assert_eq!(matrix::parse("translate(10 5"), Matrix::IDENTITY);
}

#[test]
fn an_exponent_of_nothing_is_not_an_exponent() {
    assert_eq!(number::value("1e"), None);
    assert_eq!(number::read("2e+"), Some((2000, 1)));
}

#[test]
fn a_list_steps_over_what_is_not_a_number() {
    assert_eq!(number::list("- . 3"), vec![3000]);
}

#[test]
fn a_shape_of_no_height_and_a_circle_of_no_second_radius_are_nothing() {
    assert!(path::rectangle(0, 0, 1000, 0, 0, 0).is_empty());
    assert!(path::ellipse(0, 0, 1000, 0).is_empty());
}

#[test]
fn a_rectangle_with_one_radius_and_not_the_other_has_square_corners() {
    assert_eq!(path::rectangle(0, 0, 2000, 1000, 500, 0).len(), 5);
}
