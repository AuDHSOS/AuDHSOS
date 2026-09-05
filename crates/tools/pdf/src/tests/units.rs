// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Lengths and how they are written.

use test_support::generators::range;
use test_support::property::check;

use crate::units::{Color, Mils, PageSize, number, pt};

#[test]
fn whole_points_have_no_fraction() {
    assert_eq!(number(pt(72)), "72");
    assert_eq!(number(0), "0");
    assert_eq!(number(pt(-10)), "-10");
}

#[test]
fn a_fraction_keeps_only_the_digits_it_needs() {
    assert_eq!(number(72_500), "72.5");
    assert_eq!(number(1), "0.001");
    assert_eq!(number(1_010), "1.01");
    assert_eq!(number(-500), "-0.5");
}

#[test]
fn the_page_sizes_are_the_ones_the_format_counts_in() {
    assert_eq!(PageSize::A4.width, pt(595));
    assert_eq!(PageSize::A4.height, pt(842));
    assert_eq!(PageSize::LETTER.width, pt(612));
}

#[test]
fn a_gray_has_three_equal_channels() {
    let [red, green, blue] = Color::gray(128).components();
    assert_eq!(red, green);
    assert_eq!(green, blue);
    assert_eq!(
        Color::WHITE.components(),
        ["1".to_owned(), "1".to_owned(), "1".to_owned()]
    );
    assert_eq!(
        Color::BLACK.components(),
        ["0".to_owned(), "0".to_owned(), "0".to_owned()]
    );
}

#[test]
fn a_written_number_reads_back_as_the_length_it_was() {
    check(
        "number round trip",
        &range::<i64>(-10_000_000..=10_000_000),
        |value: &Mils| {
            let text = number(*value);
            let back = parse(&text).ok_or_else(|| format!("`{text}` is not a number"))?;
            if back == *value {
                Ok(())
            } else {
                Err(format!("`{text}` reads back as {back}, not {value}"))
            }
        },
    );
}

#[test]
fn a_written_number_never_carries_a_trailing_zero() {
    check(
        "no trailing zero",
        &range::<i64>(-1_000_000..=1_000_000),
        |value: &Mils| {
            let text = number(*value);
            if text.contains('.') && text.ends_with('0') {
                return Err(format!("`{text}` ends in a zero after the point"));
            }
            Ok(())
        },
    );
}

/// Reads a written length back, in whole thousandths and without a
/// floating-point number: what the writer produced, taken apart the way it
/// was put together.
fn parse(text: &str) -> Option<Mils> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (digits, ""),
    };
    let mut mils = whole.parse::<Mils>().ok()?.checked_mul(1000)?;
    let mut scale = 100;
    for digit in fraction.chars() {
        let value = Mils::from(digit.to_digit(10)?);
        mils = mils.checked_add(value.checked_mul(scale)?)?;
        scale = scale.checked_div(10)?;
    }
    mils.checked_mul(sign)
}

#[test]
fn a_colour_is_three_channels_of_its_own() {
    let [red, green, blue] = Color::rgb(255, 0, 128).components();
    assert_eq!(red, "1");
    assert_eq!(green, "0");
    assert_ne!(blue, red);
    assert_ne!(blue, green);
}

#[test]
fn minus_nothing_is_nothing() {
    assert_eq!(number(-0), "0");
    assert_eq!(number(-1), "-0.001");
    assert_eq!(number(-1_500), "-1.5");
}

#[test]
fn a_length_of_a_whole_page_is_written_without_a_fraction() {
    assert_eq!(number(PageSize::LETTER.height), "792");
    assert_eq!(number(PageSize::LETTER.width), "612");
}
