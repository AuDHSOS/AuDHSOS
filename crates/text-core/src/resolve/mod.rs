// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ordered font resolution by grapheme, script, language, and bidi level.
mod language;
use crate::{
    Fixed, Font, TextError, bidi,
    glyf::Glyf,
    shape::{self, Glyph, LayoutTable},
    unicode::{self, Script},
    variation::{Axes, AxisValue},
};
pub use language::Language;

/// Requested font-chain role.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// Proportional interface text.
    #[default]
    Ui,
    /// Monospaced text.
    Mono,
}
/// Caller-owned ordered face chains and their explicit snapshot generation.
#[derive(Clone, Copy, Debug)]
pub struct FontSet<'s, 'f> {
    /// Interface font chain, highest priority first.
    pub ui: &'s [Font<'f>],
    /// Monospaced font chain, highest priority first.
    pub mono: &'s [Font<'f>],
    /// Caller-assigned snapshot identifier.
    pub generation: u64,
}
impl<'s, 'f> FontSet<'s, 'f> {
    /// Borrow the requested role chain.
    #[must_use]
    pub const fn chain(self, role: Role) -> &'s [Font<'f>] {
        match role {
            Role::Ui => self.ui,
            Role::Mono => self.mono,
        }
    }
}
/// All settings that may affect pure text geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextStyle {
    /// Ordered chain to search.
    pub role: Role,
    /// Positive em size in output units.
    pub size: Fixed,
    /// Explicit text language.
    pub lang: Language,
    /// Requested wght coordinate, 1 through 1000.
    pub weight: u16,
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            role: Role::Ui,
            size: Fixed::from_i32(16),
            lang: Language::UND,
            weight: 400,
        }
    }
}
impl TextStyle {
    pub(crate) fn validate(self) -> Result<(), TextError> {
        if self.size <= Fixed::ZERO || !(1..=1000).contains(&self.weight) {
            Err(TextError::InvalidInput)
        } else {
            Ok(())
        }
    }
}
/// One contiguous logical run; face indices address the requested role chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// Inclusive UTF-8 byte start.
    pub start: usize,
    /// Exclusive UTF-8 byte end.
    pub end: usize,
    /// Ordered face index in the role chain.
    pub face: usize,
    /// Snapshot identifier.
    pub generation: u64,
    /// Resolved Unicode script.
    pub script: Script,
    /// Explicit OpenType language.
    pub language: Language,
    /// Paragraph-resolved embedding level.
    pub level: u8,
    /// Style size divided by this face's units-per-em.
    pub scale: Fixed,
    /// Full shaping was explicitly refused for this script.
    pub simple: bool,
    /// The base face supplies glyph zero for uncovered characters.
    pub missing: bool,
    coordinates: [Fixed; 64],
    axes: usize,
}
impl Default for Run {
    fn default() -> Self {
        Self {
            start: 0,
            end: 0,
            face: 0,
            generation: 0,
            script: Script::Common,
            language: Language::UND,
            level: 0,
            scale: Fixed::ZERO,
            simple: false,
            missing: false,
            coordinates: [Fixed::ZERO; 64],
            axes: 0,
        }
    }
}
impl Run {
    /// Borrow the normalized instance coordinates in fvar order.
    #[must_use]
    pub fn coordinates(&self) -> &[Fixed] {
        self.coordinates.get(..self.axes).unwrap_or(&[])
    }
}

/// Resolve already computed paragraph levels into caller-owned runs.
/// Glyph scratch must fit the largest normalized cluster.
/// # Errors
/// Returns malformed fonts, invalid style/levels, empty chains, or insufficient buffers.
pub fn resolve_into(
    set: &FontSet<'_, '_>,
    style: &TextStyle,
    text: &str,
    units: &[bidi::Unit],
    scratch: &mut [Glyph],
    out: &mut [Run],
) -> Result<usize, TextError> {
    style.validate()?;
    let chain = set.chain(style.role);
    if chain.is_empty() {
        return Err(TextError::NoFont);
    }
    if units.len() != text.chars().count()
        || !units
            .iter()
            .zip(text.char_indices())
            .all(|(u, (byte, _))| u.byte() == byte)
    {
        return Err(TextError::InvalidInput);
    }
    let mut count: usize = 0;
    let mut start = 0;
    let mut previous = text
        .chars()
        .map(|c| unicode::script(u32::from(c)))
        .find(|s| !neutral(*s))
        .unwrap_or(Script::Common);
    let mut scalar = 0;
    for end in crate::segment::grapheme_boundaries(text).skip(1) {
        let cluster = text.get(start..end).ok_or(TextError::InvalidInput)?;
        let script = cluster_script(text, start, end, previous)?;
        if !neutral(script) {
            previous = script;
        }
        let mut level = None;
        while let Some(unit) = units.get(scalar).filter(|unit| unit.byte() < end) {
            if level.is_none() {
                level = unit.level();
            }
            scalar = scalar.checked_add(1).ok_or(TextError::Overflow)?;
        }
        let level = level.unwrap_or_else(|| {
            units
                .get(scalar.saturating_sub(1))
                .map_or(0, bidi::Unit::paragraph_level)
        });
        let (face, missing) = select(chain, style.lang, cluster, script, level & 1 != 0, scratch)?;
        let simple = !shape::supported(script);
        if let Some(prior) = count.checked_sub(1).and_then(|i| out.get_mut(i))
            && prior.face == face
            && prior.script == script
            && prior.level == level
            && prior.missing == missing
            && prior.simple == simple
        {
            prior.end = end;
        } else {
            let font = chain.get(face).ok_or(TextError::NoFont)?;
            let metrics = font.metrics()?;
            let mut run = Run {
                start,
                end,
                face,
                generation: set.generation,
                script,
                language: style.lang,
                level,
                scale: style
                    .size
                    .mul_ratio(1, i64::from(metrics.head.units_per_em))?,
                simple,
                missing,
                ..Run::default()
            };
            run.axes = coordinates(font, style.weight, &mut run.coordinates)?;
            *out.get_mut(count).ok_or(TextError::BufferTooSmall)? = run;
            count = count.checked_add(1).ok_or(TextError::Overflow)?;
        }
        start = end;
    }
    Ok(count)
}
pub(crate) fn coordinates(
    font: &Font<'_>,
    weight: u16,
    out: &mut [Fixed],
) -> Result<usize, TextError> {
    let Some(fvar) = font.table(*b"fvar") else {
        return Ok(0);
    };
    let axes = Axes::parse(fvar.data, font.table(*b"avar").map(|t| t.data))?;
    let mut has_weight = false;
    for i in 0..axes.len() {
        has_weight |= axes.axis(i)?.tag == *b"wght";
    }
    let values = [AxisValue {
        tag: *b"wght",
        value: Fixed::from_i32(i32::from(weight)),
    }];
    Ok(axes.normalize(if has_weight { &values } else { &[] }, out)?)
}
const fn neutral(script: Script) -> bool {
    matches!(script, Script::Common | Script::Inherited | Script::Unknown)
}
fn cluster_script(
    text: &str,
    start: usize,
    end: usize,
    previous: Script,
) -> Result<Script, TextError> {
    let cluster = text.get(start..end).ok_or(TextError::InvalidInput)?;
    for c in cluster.chars() {
        let script = unicode::script(u32::from(c));
        if !neutral(script) {
            return Ok(script);
        }
    }
    let mut chosen = previous;
    for c in cluster.chars() {
        let extensions = unicode::script_extensions(u32::from(c));
        if !extensions.is_empty() && !extensions.contains(&chosen) {
            chosen = *extensions.first().ok_or(TextError::InvalidInput)?;
        }
    }
    Ok(chosen)
}
fn select(
    chain: &[Font<'_>],
    language: Language,
    text: &str,
    script: Script,
    rtl: bool,
    scratch: &mut [Glyph],
) -> Result<(usize, bool), TextError> {
    let regional = script == Script::Han && language.regional_han();
    for strict in [true, false] {
        if strict && !regional {
            continue;
        }
        for (i, font) in chain.iter().enumerate() {
            if bitmap_only(font) {
                continue;
            }
            if strict {
                let mut suitable = false;
                for (tag, substitution) in [(*b"GSUB", true), (*b"GPOS", false)] {
                    if let Some(table) = font.table(tag) {
                        suitable |= LayoutTable::parse(table.data, substitution)?
                            .supports_language(shape::script_tag(script), language.tag())?;
                    }
                }
                if !suitable {
                    continue;
                }
            }
            let cmap = font.cmap()?;
            let mut prior = None;
            let mut variations = true;
            for character in text.chars() {
                if matches!(u32::from(character),0xfe00..=0xfe0f|0xe0100..=0xe01ef) {
                    variations &=
                        prior.is_some_and(|base| cmap.variation_glyph(base, character).is_some());
                } else {
                    prior = Some(character);
                }
            }
            if !variations {
                continue;
            }
            let n = shape::prepare(text, cmap, script, rtl, scratch)?;
            if scratch
                .get(..n)
                .ok_or(TextError::BufferTooSmall)?
                .iter()
                .all(|g| g.id != 0 || g.hidden || text.chars().all(control))
            {
                return Ok((i, false));
            }
        }
    }
    // Nothing covered the cluster. The notdef comes from a face that can draw
    // one, which a face of a refused colour format cannot.
    let fallback = chain.iter().position(|font| !bitmap_only(font));
    Ok((fallback.unwrap_or(0), true))
}
/// Whether a face's only glyph data is a colour format this track refuses.
///
/// `CBDT`/`CBLC`, `sbix` and OpenType-SVG draw nothing here, so a face that
/// carries one of them and no outline covers no cluster: the chain moves to
/// its next layer rather than emitting a blank (D-176). An `sbix` face states
/// a `glyf` table whose every entry is empty, so the loca entries decide and
/// not the presence of the tag.
fn bitmap_only(font: &Font<'_>) -> bool {
    if font.table(*b"CFF ").is_some() || font.table(*b"CFF2").is_some() {
        return false;
    }
    if font.table(*b"CBDT").is_none()
        && font.table(*b"sbix").is_none()
        && font.table(*b"SVG ").is_none()
    {
        return false;
    }
    !Glyf::parse(font).is_ok_and(|glyf| glyf.any_outline().unwrap_or(true))
}

pub(crate) const fn control(c: char) -> bool {
    matches!(
        c,
        '\r' | '\n' | '\t' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}
