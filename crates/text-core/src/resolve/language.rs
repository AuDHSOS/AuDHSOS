// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::TextError;

/// Explicit OpenType language selection, independent of host locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    tag: [u8; 4],
}
impl Default for Language {
    fn default() -> Self {
        Self::UND
    }
}
impl Language {
    /// Undetermined language: use the script's default language system.
    pub const UND: Self = Self { tag: *b"dflt" };
    /// Construct an explicitly registered OpenType language tag.
    /// # Errors
    /// Rejects nonprintable bytes.
    pub fn from_opentype(tag: [u8; 4]) -> Result<Self, TextError> {
        if tag.iter().any(|c| !(32..=126).contains(c)) {
            return Err(TextError::InvalidInput);
        }
        Ok(Self { tag })
    }
    /// Resolve a well-formed BCP-47 tag; unmapped tags use default `LangSys`.
    /// # Errors
    /// Rejects malformed syntax, repeated variants/extensions, or more than 255 bytes.
    /// Registry membership is not validated.
    pub fn parse(tag: &str) -> Result<Self, TextError> {
        let (first, script, region) = subtags(tag)?;
        let traditional = ["TW", "HK", "MO"]
            .iter()
            .any(|v| region.eq_ignore_ascii_case(v));
        let hong_kong = region.eq_ignore_ascii_case("HK") || region.eq_ignore_ascii_case("MO");
        let script = if script.eq_ignore_ascii_case("Hans") {
            Some(false)
        } else if script.eq_ignore_ascii_case("Hant") {
            Some(true)
        } else {
            None
        };
        if first.eq_ignore_ascii_case("zh") {
            return Ok(Self {
                tag: if script.unwrap_or(traditional) {
                    if hong_kong { *b"ZHH " } else { *b"ZHT " }
                } else {
                    *b"ZHS "
                },
            });
        }
        let mappings = [
            ("ja", *b"JAN "),
            ("ko", *b"KOR "),
            ("en", *b"ENG "),
            ("ar", *b"ARA "),
            ("he", *b"IWR "),
            ("el", *b"ELL "),
            ("ru", *b"RUS "),
            ("uk", *b"UKR "),
            ("sr", *b"SRB "),
            ("tr", *b"TRK "),
            ("de", *b"DEU "),
            ("fr", *b"FRA "),
            ("es", *b"ESP "),
            ("it", *b"ITA "),
            ("pt", *b"PTG "),
            ("fa", *b"FAR "),
            ("ur", *b"URD "),
            ("yi", *b"JII "),
            ("la", *b"LAT "),
        ];
        Ok(Self {
            tag: mappings
                .iter()
                .find(|(key, _)| first.eq_ignore_ascii_case(key))
                .map_or(*b"dflt", |(_, v)| *v),
        })
    }
    /// OpenType tag passed to script language selection.
    #[must_use]
    pub const fn tag(self) -> [u8; 4] {
        self.tag
    }
    pub(super) const fn regional_han(self) -> bool {
        matches!(&self.tag, b"JAN " | b"KOR " | b"ZHS " | b"ZHT " | b"ZHH ")
    }
}

fn subtags(tag: &str) -> Result<(&str, &str, &str), TextError> {
    if tag.len() > 255
        || tag
            .split('-')
            .any(|p| p.is_empty() || p.len() > 8 || !p.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(TextError::InvalidInput);
    }
    let irregular = [
        "en-GB-oed",
        "i-ami",
        "i-bnn",
        "i-default",
        "i-enochian",
        "i-hak",
        "i-klingon",
        "i-lux",
        "i-mingo",
        "i-navajo",
        "i-pwn",
        "i-tao",
        "i-tay",
        "i-tsu",
        "sgn-BE-FR",
        "sgn-BE-NL",
        "sgn-CH-DE",
    ];
    if irregular.iter().any(|v| tag.eq_ignore_ascii_case(v)) {
        return Ok(("und", "", ""));
    }
    let mut parts = tag.split('-').peekable();
    let mut language = parts.next().ok_or(TextError::InvalidInput)?;
    if language.eq_ignore_ascii_case("x") {
        return if parts.next().is_some() {
            Ok(("und", "", ""))
        } else {
            Err(TextError::InvalidInput)
        };
    }
    if !(2..=8).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(TextError::InvalidInput);
    }
    if language.len() <= 3 {
        for _ in 0..3 {
            if parts
                .peek()
                .is_some_and(|p| p.len() == 3 && p.bytes().all(|b| b.is_ascii_alphabetic()))
            {
                language = parts.next().ok_or(TextError::InvalidInput)?;
            } else {
                break;
            }
        }
    }
    let script = if parts
        .peek()
        .is_some_and(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic()))
    {
        parts.next().ok_or(TextError::InvalidInput)?
    } else {
        ""
    };
    let region = if parts.peek().is_some_and(|p| {
        (p.len() == 2 && p.bytes().all(|b| b.is_ascii_alphabetic()))
            || (p.len() == 3 && p.bytes().all(|b| b.is_ascii_digit()))
    }) {
        parts.next().ok_or(TextError::InvalidInput)?
    } else {
        ""
    };
    suffix(parts)?;
    Ok((language, script, region))
}

fn suffix(mut parts: core::iter::Peekable<core::str::Split<'_, char>>) -> Result<(), TextError> {
    let variants = parts.clone();
    let mut count = 0;
    while parts.peek().is_some_and(|p| {
        p.len() >= 5 || (p.len() == 4 && p.bytes().next().is_some_and(|b| b.is_ascii_digit()))
    }) {
        let variant = parts.next().ok_or(TextError::InvalidInput)?;
        if variants
            .clone()
            .take(count)
            .any(|v| v.eq_ignore_ascii_case(variant))
        {
            return Err(TextError::InvalidInput);
        }
        count = count.checked_add(1).ok_or(TextError::InvalidInput)?;
    }
    let mut extensions = 0_u64;
    while let Some(part) = parts.next() {
        if part.eq_ignore_ascii_case("x") {
            return if parts.next().is_some() {
                Ok(())
            } else {
                Err(TextError::InvalidInput)
            };
        }
        if part.len() != 1 {
            return Err(TextError::InvalidInput);
        }
        let byte = part
            .bytes()
            .next()
            .ok_or(TextError::InvalidInput)?
            .to_ascii_lowercase();
        let index = if byte.is_ascii_digit() {
            byte.checked_sub(b'0')
        } else {
            byte.checked_sub(b'a').and_then(|v| v.checked_add(10))
        }
        .ok_or(TextError::InvalidInput)?;
        let mask = 1_u64
            .checked_shl(u32::from(index))
            .ok_or(TextError::InvalidInput)?;
        if extensions & mask != 0 || parts.peek().is_none_or(|p| p.len() < 2) {
            return Err(TextError::InvalidInput);
        }
        extensions |= mask;
        while parts.peek().is_some_and(|p| p.len() >= 2) {
            parts.next();
        }
    }
    Ok(())
}
