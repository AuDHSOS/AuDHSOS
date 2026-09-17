// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Default Unicode 18.0.0 grapheme, word, and line boundaries.
mod grapheme;
mod word;
pub use grapheme::{GraphemeBoundaries, grapheme_boundaries};
pub use word::{WordBoundaries, word_boundaries};

mod line;
pub use line::{Break, LineBoundary, LineUnit, line_breaks};
