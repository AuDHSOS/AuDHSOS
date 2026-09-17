// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Feature, Glyph};
use crate::{
    FontError,
    cmap::Cmap,
    read::{add, sub},
    segment::grapheme_boundaries,
    unicode::{self, JoiningType as J, Script},
};

/// Scripts with complete horizontal shaping in this track.
#[must_use]
pub const fn supported(script: Script) -> bool {
    matches!(
        script,
        Script::Common
            | Script::Inherited
            | Script::Latin
            | Script::Greek
            | Script::Cyrillic
            | Script::Hebrew
            | Script::Arabic
            | Script::Han
            | Script::Hiragana
            | Script::Katakana
            | Script::Hangul
    )
}
/// OpenType script tag; refused scripts use DFLT simple placement.
#[must_use]
pub const fn script_tag(script: Script) -> [u8; 4] {
    match script {
        Script::Latin => *b"latn",
        Script::Greek => *b"grek",
        Script::Cyrillic => *b"cyrl",
        Script::Hebrew => *b"hebr",
        Script::Arabic => *b"arab",
        Script::Han => *b"hani",
        Script::Hiragana | Script::Katakana => *b"kana",
        Script::Hangul => *b"hang",
        _ => *b"DFLT",
    }
}
/// Ordered substitution stages for the supported scripts.
pub const SUBSTITUTION_FEATURES: &[Feature] = &[
    Feature {
        tag: *b"rvrn",
        value: 1,
    },
    Feature {
        tag: *b"ccmp",
        value: 1,
    },
    Feature {
        tag: *b"locl",
        value: 1,
    },
    Feature {
        tag: *b"rtla",
        value: 1,
    },
    Feature {
        tag: *b"rtlm",
        value: 1,
    },
    Feature {
        tag: *b"isol",
        value: 1,
    },
    Feature {
        tag: *b"fina",
        value: 1,
    },
    Feature {
        tag: *b"medi",
        value: 1,
    },
    Feature {
        tag: *b"init",
        value: 1,
    },
    Feature {
        tag: *b"ljmo",
        value: 1,
    },
    Feature {
        tag: *b"vjmo",
        value: 1,
    },
    Feature {
        tag: *b"tjmo",
        value: 1,
    },
    Feature {
        tag: *b"rlig",
        value: 1,
    },
    Feature {
        tag: *b"rclt",
        value: 1,
    },
    Feature {
        tag: *b"calt",
        value: 1,
    },
    Feature {
        tag: *b"liga",
        value: 1,
    },
    Feature {
        tag: *b"clig",
        value: 1,
    },
    Feature {
        tag: *b"mset",
        value: 1,
    },
];
/// Ordered horizontal positioning stages.
pub const POSITION_FEATURES: &[Feature] = &[
    Feature {
        tag: *b"curs",
        value: 1,
    },
    Feature {
        tag: *b"kern",
        value: 1,
    },
    Feature {
        tag: *b"mark",
        value: 1,
    },
    Feature {
        tag: *b"mkmk",
        value: 1,
    },
];

fn mapped(cmap: Cmap<'_>, code: u32) -> u16 {
    char::from_u32(code).map_or(0, |c| cmap.glyph_index(c))
}
fn push(out: &mut [Glyph], len: &mut usize, g: Glyph) -> Result<(), FontError> {
    *out.get_mut(*len).ok_or(FontError::BufferTooSmall)? = g;
    *len = add(*len, 1)?;
    Ok(())
}
fn decompose(
    code: u32,
    source: Glyph,
    out: &mut [Glyph],
    len: &mut usize,
    depth: usize,
) -> Result<(), FontError> {
    if depth == 32 {
        return Err(FontError::LimitExceeded);
    }
    if (0xac00..=0xd7a3).contains(&code) {
        let s = code.checked_sub(0xac00).ok_or(FontError::Overflow)?;
        let codes = [
            0x1100_u32.checked_add(s / 588).ok_or(FontError::Overflow)?,
            0x1161_u32
                .checked_add((s % 588) / 28)
                .ok_or(FontError::Overflow)?,
            0x11a7_u32.checked_add(s % 28).ok_or(FontError::Overflow)?,
        ];
        for code in codes.into_iter().take(if s % 28 == 0 { 2 } else { 3 }) {
            push(out, len, Glyph { code, ..source })?;
        }
    } else {
        let parts = unicode::canonical_decomposition(code);
        if parts.is_empty() {
            push(out, len, Glyph { code, ..source })?;
        } else {
            for part in parts {
                decompose(*part, source, out, len, add(depth, 1)?)?;
            }
        }
    }
    Ok(())
}
fn hangul(first: u32, second: u32) -> Option<u32> {
    if (0x1100..=0x1112).contains(&first) && (0x1161..=0x1175).contains(&second) {
        0xac00_u32
            .checked_add(first.checked_sub(0x1100)?.checked_mul(588)?)?
            .checked_add(second.checked_sub(0x1161)?.checked_mul(28)?)
    } else if (0xac00..=0xd7a3).contains(&first)
        && first.checked_sub(0xac00)?.is_multiple_of(28)
        && (0x11a8..=0x11c2).contains(&second)
    {
        first.checked_add(second.checked_sub(0x11a7)?)
    } else {
        None
    }
}
/// Normalize supported scripts, map glyphs, and assign joining forms and clusters.
/// Returns a glyph count; refused scripts retain simple scalar placement.
/// # Errors
/// Returns insufficient capacity or bounded normalization work errors.
pub fn prepare(
    text: &str,
    cmap: Cmap<'_>,
    script: Script,
    rtl: bool,
    out: &mut [Glyph],
) -> Result<usize, FontError> {
    let mut len = 0;
    let mut previous = 0;
    let full = supported(script);
    for end in grapheme_boundaries(text).skip(1) {
        let cluster = text.get(previous..end).ok_or(FontError::InvalidTable)?;
        for c in cluster.chars() {
            let source = Glyph {
                start: previous,
                end,
                ..Glyph::default()
            };
            if full {
                decompose(u32::from(c), source, out, &mut len, 0)?;
            } else {
                push(
                    out,
                    &mut len,
                    Glyph {
                        code: u32::from(c),
                        ..source
                    },
                )?;
            }
        }
        previous = end;
    }
    if full {
        len = normalize(cmap, out, len)?;
    }
    let glyphs = out.get_mut(..len).ok_or(FontError::BufferTooSmall)?;
    let mut prior: Option<usize> = None;
    for i in 0..len {
        let code = glyphs.get(i).ok_or(FontError::BufferTooSmall)?.code;
        let jt = unicode::joining_type(code);
        if jt == J::T {
            continue;
        }
        let mut form = 1;
        if let Some(p) = prior {
            let before = glyphs.get_mut(p).ok_or(FontError::BufferTooSmall)?;
            if matches!(unicode::joining_type(before.code), J::D | J::L | J::C)
                && matches!(jt, J::D | J::R | J::C)
            {
                before.form = if before.form == 8 { 4 } else { 2 };
                form = 8;
            }
        }
        glyphs.get_mut(i).ok_or(FontError::BufferTooSmall)?.form =
            if script == Script::Arabic { form } else { 0 };
        prior = Some(i);
    }
    for i in 0..len {
        let code = glyphs.get(i).ok_or(FontError::BufferTooSmall)?.code;
        let mut glyph = *glyphs.get(i).ok_or(FontError::BufferTooSmall)?;
        glyph.id = mapped(cmap, code);
        glyph.hidden = unicode::default_ignorable(code) == unicode::DefaultIgnorable::Yes
            && !matches!(code, 0x115f | 0x1160 | 0x3164 | 0xffa0);
        if rtl {
            let mirror = unicode::mirror(code);
            let id = mapped(cmap, mirror);
            if mirror != code && id != 0 {
                glyph.id = id;
            } else {
                glyph.mirror = mirror != code;
            }
        }
        if let Some(next) = glyphs.get(add(i, 1)?)
            && let (Some(c), Some(selector)) = (char::from_u32(code), char::from_u32(next.code))
            && let Some(id) = cmap.variation_glyph(c, selector)
        {
            glyph.id = id;
        }
        glyph.class = if matches!(
            unicode::general_category(code),
            unicode::GeneralCategory::Mn
                | unicode::GeneralCategory::Mc
                | unicode::GeneralCategory::Me
        ) {
            3
        } else {
            1
        };
        glyph.components = 1;
        glyph.jamo = match code {
            0x1100..=0x115f | 0xa960..=0xa97c => 1,
            0x1160..=0x11a7 | 0xd7b0..=0xd7c6 => 2,
            0x11a8..=0x11ff | 0xd7cb..=0xd7fb => 4,
            _ => 0,
        };
        *glyphs.get_mut(i).ok_or(FontError::BufferTooSmall)? = glyph;
    }
    Ok(len)
}

fn normalize(cmap: Cmap<'_>, out: &mut [Glyph], mut len: usize) -> Result<usize, FontError> {
    let mut work = 0;
    {
        for i in 1..len {
            let mut j = i;
            while j > 0 {
                work = add(work, 1)?;
                if work > 1_000_000 {
                    return Err(FontError::LimitExceeded);
                }
                let before = sub(j, 1)?;
                let g = *out.get(j).ok_or(FontError::BufferTooSmall)?;
                let p = *out.get(before).ok_or(FontError::BufferTooSmall)?;
                let c = mark_order(g.code);
                let pc = mark_order(p.code);
                if g.start != p.start || c == 0 || pc <= c {
                    break;
                }
                out.swap(before, j);
                j = before;
            }
        }
        let mut starter: Option<usize> = None;
        let mut last = 0;
        let mut i = 0;
        while i < len {
            let g = *out.get(i).ok_or(FontError::BufferTooSmall)?;
            let class = mark_order(g.code);
            let composed = if let Some(s) = starter {
                let p = *out.get(s).ok_or(FontError::BufferTooSmall)?;
                if p.start == g.start && (last == 0 || last < class) {
                    hangul(p.code, g.code)
                        .filter(|c| mapped(cmap, *c) != 0)
                        .or_else(|| unicode::composition(p.code, g.code, |c| mapped(cmap, c) != 0))
                        .map(|code| (s, code))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((s, code)) = composed {
                out.get_mut(s).ok_or(FontError::BufferTooSmall)?.code = code;
                out.copy_within(add(i, 1)?..len, i);
                len = sub(len, 1)?;
            } else {
                if class == 0 {
                    starter = Some(i);
                }
                last = class;
                i = add(i, 1)?;
            }
        }
    }

    Ok(len)
}

fn mark_order(code: u32) -> u8 {
    match unicode::combining_class(code) {
        10 => 22,
        11 => 15,
        12 => 16,
        13 => 17,
        14 => 23,
        15 => 18,
        16 => 19,
        17 => 20,
        18 => 21,
        19 => 14,
        20 => 24,
        21 => 12,
        22 => 25,
        23 => 13,
        24 => 10,
        25 => 11,
        27 => 28,
        28 => 29,
        29 => 30,
        30 => 31,
        31 => 32,
        32 => 33,
        33 => 27,
        other => other,
    }
}
