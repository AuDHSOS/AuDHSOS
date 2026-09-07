// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Colours, and the state an element draws under.
//!
//! Everything that can be inherited is in one struct, and an element makes
//! its own by copying its parent's and applying what it says for itself.
//! What it says comes from three places, in this order: the presentation
//! attributes it carries, the stylesheet rules that match it, and its own
//! `style` attribute. That is the order CSS gives them, and it is why a
//! `fill="black"` on an element loses to a `fill: white` in a rule.

use doc_pdf::font::Font;
use doc_pdf::units::Color;

use crate::Unit;
use crate::number::{self, ONE};

/// Where the text sits relative to the point it is placed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anchor {
    /// The point is the start of the text.
    Start,
    /// The point is the middle of it.
    Middle,
    /// The point is the end of it.
    End,
}

/// What an element draws with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    /// The fill, or nothing.
    pub(crate) fill: Option<Color>,
    /// Whether the fill counts crossings by the even-odd rule.
    pub(crate) even_odd: bool,
    /// The stroke, or nothing.
    pub(crate) stroke: Option<Color>,
    /// How wide the stroke is.
    pub(crate) width: Unit,
    /// The dashes, on and off in turn.
    pub(crate) dashes: Vec<Unit>,
    /// The face text is set in.
    pub(crate) font: Font,
    /// Whether the family asked for is a fixed-pitch one.
    pub(crate) mono: bool,
    /// Whether the weight asked for is bold.
    pub(crate) bold: bool,
    /// The size text is set at.
    pub(crate) size: Unit,
    /// Where the text sits.
    pub(crate) anchor: Anchor,
    /// The marker drawn at the end of a stroke, by the name it is under.
    pub(crate) marker_end: Option<String>,
}

impl Default for State {
    /// What SVG starts with: black fill, no stroke, and sixteen-unit text.
    fn default() -> Self {
        Self {
            fill: Some(Color::BLACK),
            even_odd: false,
            stroke: None,
            width: ONE,
            dashes: Vec::new(),
            font: Font::Regular,
            mono: false,
            bold: false,
            size: ONE.saturating_mul(16),
            anchor: Anchor::Start,
            marker_end: None,
        }
    }
}

impl State {
    /// The state with one declaration applied.
    pub(crate) fn apply(&mut self, property: &str, value: &str) {
        let value = value.trim();
        match property {
            "fill" => self.fill = paint(value, self.fill),
            "stroke" => self.stroke = paint(value, self.stroke),
            "stroke-width" => self.width = number::value(value).unwrap_or(self.width),
            "stroke-dasharray" => {
                self.dashes = if value == "none" {
                    Vec::new()
                } else {
                    number::list(value)
                };
            }
            "fill-rule" => self.even_odd = value == "evenodd",
            "font-size" => self.size = size(value, self.size),
            "font-family" => {
                self.mono = value.to_ascii_lowercase().contains("courier")
                    || value.to_ascii_lowercase().contains("mono");
                self.font = face(self.mono, self.bold);
            }
            "font-weight" => {
                self.bold = matches!(value, "bold" | "bolder" | "600" | "700" | "800" | "900");
                self.font = face(self.mono, self.bold);
            }
            "text-anchor" => {
                self.anchor = match value {
                    "middle" => Anchor::Middle,
                    "end" => Anchor::End,
                    _ => Anchor::Start,
                };
            }
            "marker-end" => self.marker_end = reference(value),
            _ => {}
        }
    }
}

/// The face a family and a weight ask for.
const fn face(mono: bool, bold: bool) -> Font {
    match (mono, bold) {
        (true, true) => Font::MonoBold,
        (true, false) => Font::Mono,
        (false, true) => Font::Bold,
        (false, false) => Font::Regular,
    }
}

/// A fill or a stroke: a colour, nothing, or what was there before.
fn paint(value: &str, current: Option<Color>) -> Option<Color> {
    match value {
        "none" | "transparent" => None,
        "inherit" | "currentColor" => current,
        other => color(other).or(current),
    }
}

/// A hundred whole units, which a percentage is measured against.
const HUNDRED: Unit = ONE.saturating_mul(100);

/// A font size, which may be given as a share of the one it inherits.
fn size(value: &str, inherited: Unit) -> Unit {
    if let Some(percent) = value.strip_suffix('%') {
        let share = number::value(percent).unwrap_or(HUNDRED);
        return inherited.saturating_mul(share).wrapping_div(HUNDRED);
    }
    number::value(value).unwrap_or(inherited)
}

/// The name inside `url(#name)`.
fn reference(value: &str) -> Option<String> {
    let inside = value.trim().strip_prefix("url(")?.strip_suffix(')')?;
    let name = inside.trim().trim_matches(['"', '\'']).strip_prefix('#')?;
    Some(name.to_owned())
}

/// The colours a drawing names by word.
const NAMED: [(&str, u8, u8, u8); 16] = [
    ("black", 0, 0, 0),
    ("white", 255, 255, 255),
    ("red", 255, 0, 0),
    ("green", 0, 128, 0),
    ("lime", 0, 255, 0),
    ("blue", 0, 0, 255),
    ("yellow", 255, 255, 0),
    ("cyan", 0, 255, 255),
    ("aqua", 0, 255, 255),
    ("magenta", 255, 0, 255),
    ("fuchsia", 255, 0, 255),
    ("gray", 128, 128, 128),
    ("grey", 128, 128, 128),
    ("silver", 192, 192, 192),
    ("maroon", 128, 0, 0),
    ("navy", 0, 0, 128),
];

/// A colour, by word, by hexadecimal, or by its three components.
pub(crate) fn color(text: &str) -> Option<Color> {
    let value = text.trim();
    if let Some(digits) = value.strip_prefix('#') {
        return hex(digits);
    }
    if let Some(components) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let values = number::list(components);
        let component = |index: usize| {
            values
                .get(index)
                .map(|value| value.wrapping_div(ONE))
                .and_then(|value| u8::try_from(value.clamp(0, 255)).ok())
        };
        return Some(Color::rgb(component(0)?, component(1)?, component(2)?));
    }
    let lower = value.to_ascii_lowercase();
    NAMED
        .iter()
        .find(|(name, _, _, _)| *name == lower)
        .map(|(_, red, green, blue)| Color::rgb(*red, *green, *blue))
}

/// A colour written as three or six hexadecimal digits.
fn hex(digits: &str) -> Option<Color> {
    let value = digits.trim();
    let pair = |at: usize| -> Option<u8> {
        let text = value.get(at..at.saturating_add(2))?;
        u8::from_str_radix(text, 16).ok()
    };
    let single = |at: usize| -> Option<u8> {
        let text = value.get(at..at.saturating_add(1))?;
        let digit = u8::from_str_radix(text, 16).ok()?;
        Some(digit.saturating_mul(17))
    };
    match value.len() {
        3 => Some(Color::rgb(single(0)?, single(1)?, single(2)?)),
        6 => Some(Color::rgb(pair(0)?, pair(2)?, pair(4)?)),
        _ => None,
    }
}
