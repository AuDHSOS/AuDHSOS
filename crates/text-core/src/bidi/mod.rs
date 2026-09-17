// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! UAX #9 revision 52: paragraph levels and per-line visual ordering.
mod explicit;
mod implicit;
mod reorder;
use crate::{
    TextError,
    unicode::{self as u, BidiClass as B},
};
pub use reorder::{LineInfo, reorder_line};

/// Paragraph direction policy, supplied by the caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    /// P2/P3 first strong character, defaulting to left-to-right.
    #[default]
    Auto,
    /// Paragraph level zero.
    LeftToRight,
    /// Paragraph level one.
    RightToLeft,
}
/// Caller-owned per-scalar bidi result and scratch record.
#[derive(Clone, Copy, Debug)]
pub struct Unit {
    byte: usize,
    code: u32,
    original: B,
    kind: B,
    embedding: u8,
    level: u8,
    base: u8,
    matching: Option<usize>,
    parent: Option<usize>,
    previous: Option<usize>,
    next: Option<usize>,
    sequence_next: Option<usize>,
    start: bool,
    incoming: bool,
    bracket: Option<usize>,
}
impl Default for Unit {
    fn default() -> Self {
        Self {
            byte: 0,
            code: 0,
            original: B::L,
            kind: B::L,
            embedding: 0,
            level: 0,
            base: 0,
            matching: None,
            parent: None,
            previous: None,
            next: None,
            sequence_next: None,
            start: false,
            incoming: false,
            bracket: None,
        }
    }
}
impl Unit {
    /// UTF-8 byte offset, or the logical index for class-only input.
    #[must_use]
    pub const fn byte(&self) -> usize {
        self.byte
    }
    /// Resolved paragraph level before per-line L1 resets; None for X9 controls.
    #[must_use]
    pub const fn level(&self) -> Option<u8> {
        if removed(self.original) {
            None
        } else {
            Some(self.level)
        }
    }
    /// Paragraph embedding level assigned by P2/P3 or the caller.
    #[must_use]
    pub const fn paragraph_level(&self) -> u8 {
        self.base
    }
    /// Code point after the L4 mirror mapping at an odd resolved level.
    #[must_use]
    pub fn mirrored(&self) -> u32 {
        if self.level & 1 != 0 {
            u::mirror(self.code)
        } else {
            self.code
        }
    }
}
/// Number of populated scalar records and paragraphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    /// Scalar count; populated prefix of the Unit buffer.
    pub count: usize,
    /// P1 paragraphs; empty text has zero paragraphs.
    pub paragraphs: usize,
}
/// Resolve paragraph levels using one Unit and one index per scalar.
/// # Errors
/// Returns capacity or arithmetic errors; errors invalidate both buffers.
pub fn resolve(
    text: &str,
    direction: Direction,
    units: &mut [Unit],
    sequence: &mut [usize],
) -> Result<Info, TextError> {
    let count = text.chars().count();
    let out = units.get_mut(..count).ok_or(TextError::BufferTooSmall)?;
    for (unit, (byte, ch)) in out.iter_mut().zip(text.char_indices()) {
        let code = u32::from(ch);
        let original = u::bidi_class(code);
        *unit = Unit {
            byte,
            code,
            original,
            kind: original,
            ..Unit::default()
        };
    }
    paragraphs(out, direction, sequence)
}
/// Resolve bidi classes without paired brackets, for abstract UAX #9 input.
/// # Errors
/// Returns capacity or arithmetic errors; errors invalidate both buffers.
pub fn resolve_classes(
    classes: &[B],
    direction: Direction,
    units: &mut [Unit],
    sequence: &mut [usize],
) -> Result<Info, TextError> {
    let out = units
        .get_mut(..classes.len())
        .ok_or(TextError::BufferTooSmall)?;
    for (i, (unit, original)) in out.iter_mut().zip(classes).enumerate() {
        *unit = Unit {
            byte: i,
            original: *original,
            kind: *original,
            ..Unit::default()
        };
    }
    paragraphs(out, direction, sequence)
}
fn paragraphs(
    units: &mut [Unit],
    direction: Direction,
    sequence: &mut [usize],
) -> Result<Info, TextError> {
    if sequence.len() < units.len() {
        return Err(TextError::BufferTooSmall);
    }
    let mut start = 0;
    let mut paragraphs = 0_usize;
    for end in 0..units.len() {
        if at(units, end)?.original == B::B || end.checked_add(1) == Some(units.len()) {
            let limit = add(end, 1)?;
            let paragraph = units
                .get_mut(start..limit)
                .ok_or(TextError::BufferTooSmall)?;
            explicit::matching(paragraph)?;
            let base = match direction {
                Direction::LeftToRight => 0,
                Direction::RightToLeft => 1,
                Direction::Auto => explicit::first_strong(paragraph, 0, paragraph.len())?,
            };
            explicit::levels(paragraph, base)?;
            sequences(paragraph, sequence, base)?;
            start = limit;
            paragraphs = add(paragraphs, 1)?;
        }
    }
    Ok(Info {
        count: units.len(),
        paragraphs,
    })
}
fn sequences(units: &mut [Unit], scratch: &mut [usize], base: u8) -> Result<(), TextError> {
    let mut previous = None;
    for i in 0..units.len() {
        if removed(at(units, i)?.original) {
            continue;
        }
        let before = previous.map(|p| at(units, p)).transpose()?.copied();
        let u = at_mut(units, i)?;
        u.previous = previous;
        u.start = before.is_none_or(|p| p.embedding != u.embedding);
        if let Some(p) = previous {
            at_mut(units, p)?.next = Some(i);
        }
        previous = Some(i);
    }
    for i in 0..units.len() {
        let u = *at(units, i)?;
        if removed(u.original) {
            continue;
        }
        let same = u
            .next
            .is_some_and(|n| units.get(n).is_some_and(|v| v.embedding == u.embedding));
        if same {
            at_mut(units, i)?.sequence_next = u.next;
        } else if isolate(u.original)
            && let Some(pdi) = u.matching
        {
            at_mut(units, i)?.sequence_next = Some(pdi);
            at_mut(units, pdi)?.incoming = true;
        }
    }
    for start in 0..units.len() {
        let first = *at(units, start)?;
        if !first.start || first.incoming {
            continue;
        }
        let mut len = 0;
        let mut next = Some(start);
        while let Some(i) = next {
            *scratch.get_mut(len).ok_or(TextError::BufferTooSmall)? = i;
            len = add(len, 1)?;
            if len > units.len() {
                return Err(TextError::Overflow);
            }
            next = at(units, i)?.sequence_next;
        }
        let seq = scratch.get(..len).ok_or(TextError::BufferTooSmall)?;
        let last = *at(units, *seq.last().ok_or(TextError::Overflow)?)?;
        let before = first
            .previous
            .map(|p| at(units, p).map(|u| u.embedding))
            .transpose()?
            .unwrap_or(base);
        let after = if isolate(last.original) {
            base
        } else {
            last.next
                .map(|p| at(units, p).map(|u| u.embedding))
                .transpose()?
                .unwrap_or(base)
        };
        implicit::resolve(
            units,
            seq,
            dir(first.embedding.max(before)),
            dir(last.embedding.max(after)),
            dir(first.embedding),
        )?;
    }
    Ok(())
}
const fn isolate(kind: B) -> bool {
    matches!(kind, B::Lri | B::Rli | B::Fsi)
}
const fn removed(kind: B) -> bool {
    matches!(kind, B::Lre | B::Rle | B::Lro | B::Rlo | B::Pdf | B::Bn)
}
const fn dir(level: u8) -> B {
    if level & 1 == 0 { B::L } else { B::R }
}
fn add(a: usize, b: usize) -> Result<usize, TextError> {
    a.checked_add(b).ok_or(TextError::Overflow)
}
fn sub(a: usize, b: usize) -> Result<usize, TextError> {
    a.checked_sub(b).ok_or(TextError::Overflow)
}
fn at(units: &[Unit], i: usize) -> Result<&Unit, TextError> {
    units.get(i).ok_or(TextError::BufferTooSmall)
}
fn at_mut(units: &mut [Unit], i: usize) -> Result<&mut Unit, TextError> {
    units.get_mut(i).ok_or(TextError::BufferTooSmall)
}
