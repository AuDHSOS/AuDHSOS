// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Numbers, in thousandths of a user unit.
//!
//! An SVG is written in decimals — `120.5`, `-3.21636`, `1e-3` — and
//! nothing here keeps one. A length is a whole number of thousandths of a
//! user unit, the same way a length in `doc-pdf` is a whole number of
//! thousandths of a point, so that two runs on two machines draw the same
//! figure. Three decimal places on a diagram that is scaled down to fit a
//! page is a thousandth of a millimetre; what is thrown away is not
//! visible and not worth a floating-point number.

use crate::Unit;

/// How many decimal places are kept.
const PLACES: u32 = 3;

/// The scale of a unit: a whole unit is this many thousandths.
pub(crate) const ONE: Unit = 1000;

/// Reads the number at the front of `text`, and says how many bytes it
/// took.
pub(crate) fn read(text: &str) -> Option<(Unit, usize)> {
    let mut at = 0usize;
    let mut digits = false;
    if let Some(rest) = text.get(at..)
        && (rest.starts_with('-') || rest.starts_with('+'))
    {
        at = at.saturating_add(1);
    }
    let start_integer = at;
    at = at.saturating_add(count_digits(text.get(at..).unwrap_or_default()));
    if at > start_integer {
        digits = true;
    }
    if text.get(at..).is_some_and(|rest| rest.starts_with('.')) {
        at = at.saturating_add(1);
        let fraction = count_digits(text.get(at..).unwrap_or_default());
        if fraction > 0 {
            digits = true;
        }
        at = at.saturating_add(fraction);
    }
    if !digits {
        return None;
    }
    let mantissa = text.get(..at)?;
    let mut end = at;
    let mut exponent = 0i32;
    if let Some(rest) = text.get(at..)
        && (rest.starts_with('e') || rest.starts_with('E'))
    {
        let mut after = at.saturating_add(1);
        if text
            .get(after..)
            .is_some_and(|rest| rest.starts_with('-') || rest.starts_with('+'))
        {
            after = after.saturating_add(1);
        }
        let power = count_digits(text.get(after..).unwrap_or_default());
        if power > 0 {
            let taken = after.saturating_add(power);
            exponent = text
                .get(at.saturating_add(1)..taken)
                .and_then(|value| value.trim_start_matches('+').parse().ok())
                .unwrap_or(0);
            end = taken;
        }
    }
    Some((scale(fixed(mantissa), exponent), end))
}

/// Reads a number that is the whole of `text`.
pub(crate) fn value(text: &str) -> Option<Unit> {
    let trimmed = text.trim();
    let (value, length) = read(trimmed)?;
    // A length may carry a unit this crate has no scale for. `px` is the
    // user unit itself, and the others — `pt`, `mm`, `em` — are refused
    // rather than guessed at.
    match trimmed.get(length..).unwrap_or_default().trim() {
        "" | "px" => Some(value),
        _ => None,
    }
}

/// Reads every number in `text`, whatever separates them.
pub(crate) fn list(text: &str) -> Vec<Unit> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(rest) = text.get(at..) {
        let skipped = rest.find(|character: char| {
            character.is_ascii_digit() || matches!(character, '-' | '+' | '.')
        });
        let Some(offset) = skipped else { break };
        at = at.saturating_add(offset);
        let Some((value, length)) = read(text.get(at..).unwrap_or_default()) else {
            at = at.saturating_add(1);
            continue;
        };
        out.push(value);
        at = at.saturating_add(length);
    }
    out
}

/// How many digits are at the front of `text`.
fn count_digits(text: &str) -> usize {
    text.find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len())
}

/// A decimal without an exponent, as thousandths.
fn fixed(text: &str) -> Unit {
    let negative = text.starts_with('-');
    let body = text.trim_start_matches(['-', '+']);
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let mut value: Unit = whole.parse().unwrap_or(0).min(Unit::MAX / ONE);
    value = value.saturating_mul(ONE);
    let mut place = ONE;
    for digit in fraction.chars().take(usize::try_from(PLACES).unwrap_or(3)) {
        place = place.wrapping_div(10);
        let Some(digit) = digit.to_digit(10) else {
            break;
        };
        value = value.saturating_add(Unit::from(digit).saturating_mul(place));
    }
    if negative {
        value.saturating_neg()
    } else {
        value
    }
}

/// Applies a power of ten.
fn scale(value: Unit, exponent: i32) -> Unit {
    let mut value = value;
    for _ in 0..exponent.abs().min(18) {
        if exponent > 0 {
            value = value.saturating_mul(10);
        } else {
            value = value.wrapping_div(10);
        }
    }
    value
}
