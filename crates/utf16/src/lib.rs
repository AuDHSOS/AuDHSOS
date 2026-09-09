// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate alloc;
use alloc::vec::Vec;

/// Exhausted work or pattern storage quota.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit;

/// A borrowed pattern with a bounded KMP failure table.
pub struct Pattern<'a> {
    units: &'a [u16],
    failure: Vec<usize>,
}
impl<'a> Pattern<'a> {
    /// Preprocesses the pattern, charging work and checking storage before allocation.
    ///
    /// # Errors
    /// Pattern length or work exceeds caller limits.
    pub fn new(units: &'a [u16], limit: usize, work: &mut u64) -> Result<Self, Limit> {
        if units.len() > limit {
            return Err(Limit);
        }
        charge(work, units.len())?;
        let mut failure = alloc::vec![0;units.len()];
        let mut matched = 0;
        for i in 1..units.len() {
            while matched > 0 {
                charge(work, 1)?;
                if units.get(i) == units.get(matched) {
                    break;
                }
                matched = *failure.get(matched.saturating_sub(1)).ok_or(Limit)?;
            }
            charge(work, 1)?;
            if units.get(i) == units.get(matched) {
                matched = matched.saturating_add(1);
            }
            *failure.get_mut(i).ok_or(Limit)? = matched;
        }
        Ok(Self { units, failure })
    }
    /// Finds the first match at or after `start` (clamped to the text length).
    ///
    /// # Errors
    /// Work quota exhaustion.
    pub fn find(&self, text: &[u16], start: usize, work: &mut u64) -> Result<Option<usize>, Limit> {
        let start = start.min(text.len());
        if self.units.is_empty() {
            return Ok(Some(start));
        }
        self.scan(text, start, text.len(), false, work)
    }
    /// Finds the last match starting at or before `start`, including overlaps.
    ///
    /// # Errors
    /// Work quota exhaustion.
    pub fn rfind(
        &self,
        text: &[u16],
        start: usize,
        work: &mut u64,
    ) -> Result<Option<usize>, Limit> {
        let start = start.min(text.len());
        if self.units.is_empty() {
            return Ok(Some(start));
        }
        self.scan(
            text,
            0,
            start.saturating_add(self.units.len()).min(text.len()),
            true,
            work,
        )
    }
    fn scan(
        &self,
        text: &[u16],
        start: usize,
        end: usize,
        last: bool,
        work: &mut u64,
    ) -> Result<Option<usize>, Limit> {
        if end.saturating_sub(start) < self.units.len() {
            return Ok(None);
        }
        let mut matched = 0usize;
        let mut found = None;
        for (i, unit) in text.get(start..end).ok_or(Limit)?.iter().enumerate() {
            while matched > 0 {
                charge(work, 1)?;
                if self.units.get(matched) == Some(unit) {
                    break;
                }
                matched = *self.failure.get(matched.saturating_sub(1)).ok_or(Limit)?;
            }
            charge(work, 1)?;
            if self.units.get(matched) == Some(unit) {
                matched = matched.saturating_add(1);
            }
            if matched == self.units.len() {
                found = Some(
                    start
                        .saturating_add(i)
                        .saturating_add(1)
                        .saturating_sub(matched),
                );
                if !last {
                    return Ok(found);
                }
                matched = *self.failure.get(matched.saturating_sub(1)).ok_or(Limit)?;
            }
        }
        Ok(found)
    }
}
fn charge(work: &mut u64, amount: usize) -> Result<(), Limit> {
    *work = work
        .checked_sub(u64::try_from(amount).unwrap_or(u64::MAX))
        .ok_or(Limit)?;
    Ok(())
}

/// Decoded code point, consumed code-unit count and unpaired-surrogate marker.
/// Returns `None` if `index` is outside the input. A trailing surrogate is never
/// paired backwards: indexing it directly returns that code unit.
#[must_use]
pub fn code_point_at(units: &[u16], index: usize) -> Option<(u32, usize, bool)> {
    let first = *units.get(index)?;
    if (0xd800..=0xdbff).contains(&first)
        && let Some(second) = units
            .get(index.saturating_add(1))
            .filter(|u| (0xdc00..=0xdfff).contains(*u))
    {
        let value = 0x10000u32
            .saturating_add(u32::from(first.saturating_sub(0xd800)) << 10)
            .saturating_add(u32::from(second.saturating_sub(0xdc00)));
        return Some((value, 2, false));
    }
    Some((u32::from(first), 1, (0xd800..=0xdfff).contains(&first)))
}
#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
