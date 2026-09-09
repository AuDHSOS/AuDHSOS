// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ordered Thompson simulation. One seen set is shared by all start positions
//! at each input offset. No input cursor is ever moved backward.

use crate::{
    Error, Limits, Regex,
    compile::Instruction,
    parse::{Assertion, newline},
};
use alloc::{vec, vec::Vec};
use core::ops::Range;

/// A leftmost, ordered match and its capturing-group ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// Whole-match range in UTF-16 code units.
    pub range: Range<usize>,
    /// Captures in opening-parenthesis order. Unmatched groups are `None`.
    pub captures: Vec<Option<Range<usize>>>,
}

/// Matching evidence, including operation counts suitable for complexity tests.
#[derive(Clone, Debug)]
pub struct Report {
    /// Match, or `None` when no candidate accepts.
    pub matched: Option<Match>,
    /// Number of unique state/position visits, including epsilon states.
    pub state_visits: u64,
    /// Charged work including capture-vector copying and class lookup bounds.
    pub work: u64,
}

#[derive(Clone)]
struct Thread {
    pc: usize,
    captures: Vec<Option<usize>>,
}

struct Search<'a> {
    regex: &'a Regex,
    input: &'a [u16],
    seen: Vec<bool>,
    pending: Vec<Thread>,
    visits: u64,
    work: u64,
    limit: u64,
}

impl Regex {
    /// Finds the first ordered match at or after `from`. `sticky` admits only
    /// the start position `from`; ordinary search adds one start per position
    /// to the same NFA simulation, never restarting the entire matcher.
    ///
    /// # Errors
    /// Resource exhaustion or an internal program invariant failure.
    pub fn find(
        &self,
        input: &[u16],
        from: usize,
        sticky: bool,
        limits: Limits,
    ) -> Result<Report, Error> {
        if input.len() > limits.input_units {
            return Err(Error::Limit {
                resource: "input units",
            });
        }
        let cells = self
            .code
            .len()
            .checked_mul(self.registers)
            .and_then(|n| n.checked_mul(6))
            .ok_or(Error::Limit {
                resource: "capture workspace",
            })?;
        if cells > limits.capture_cells {
            return Err(Error::Limit {
                resource: "capture workspace",
            });
        }
        if self.code.len() > limits.states {
            return Err(Error::Limit {
                resource: "match states",
            });
        }
        if from > input.len() {
            return Ok(Report {
                matched: None,
                state_visits: 0,
                work: 0,
            });
        }
        Search {
            regex: self,
            input,
            seen: vec![false; self.code.len()],
            pending: Vec::new(),
            visits: 0,
            work: 0,
            limit: limits.work,
        }
        .run(from, sticky)
    }
}

impl Search<'_> {
    fn charge(&mut self, amount: usize) -> Result<(), Error> {
        self.work = self
            .work
            .checked_add(u64::try_from(amount).unwrap_or(u64::MAX))
            .filter(|n| *n <= self.limit)
            .ok_or(Error::Limit {
                resource: "match work",
            })?;
        Ok(())
    }
    fn run(mut self, from: usize, sticky: bool) -> Result<Report, Error> {
        let mut current = Vec::new();
        let mut next = Vec::new();
        let mut candidate = None;
        for position in from..=self.input.len() {
            if candidate.is_none() && (!sticky || position == from) {
                self.charge(self.regex.registers)?;
                let thread = Thread {
                    pc: self.regex.start,
                    captures: vec![None; self.regex.registers],
                };
                self.expand(thread, position, &mut current)?;
            }
            // `current` is already ordered and epsilon-closed for this offset.
            self.seen.fill(false);
            next.clear();
            for thread in current.drain(..) {
                match self
                    .regex
                    .code
                    .get(thread.pc)
                    .ok_or(Error::InvalidProgram)?
                {
                    Instruction::Match => {
                        candidate = Some(thread.captures);
                        break;
                    }
                    Instruction::Unit(unit, target) => {
                        if self.input.get(position) == Some(unit) {
                            self.advance(thread, *target, position, &mut next)?;
                        }
                    }
                    Instruction::Dot(target) => {
                        if self
                            .input
                            .get(position)
                            .is_some_and(|u| self.regex.options.dot_all || !newline(*u))
                        {
                            self.advance(thread, *target, position, &mut next)?;
                        }
                    }
                    Instruction::Class(class, target) => {
                        // Binary search takes at most bit-width comparisons.
                        self.charge(
                            usize::try_from(
                                usize::BITS.saturating_sub(class.ranges.len().leading_zeros()),
                            )
                            .unwrap_or(usize::MAX)
                            .saturating_add(1),
                        )?;
                        if self.input.get(position).is_some_and(|u| class.contains(*u)) {
                            self.advance(thread, *target, position, &mut next)?;
                        }
                    }
                    _ => return Err(Error::InvalidProgram),
                }
            }
            if next.is_empty() && (candidate.is_some() || sticky) {
                break;
            }
            core::mem::swap(&mut current, &mut next);
        }
        let matched = candidate.map(|captures| to_match(&captures)).transpose()?;
        Ok(Report {
            matched,
            state_visits: self.visits,
            work: self.work,
        })
    }
    fn advance(
        &mut self,
        mut thread: Thread,
        target: usize,
        position: usize,
        out: &mut Vec<Thread>,
    ) -> Result<(), Error> {
        thread.pc = target;
        self.expand(thread, position.saturating_add(1), out)
    }
    fn expand(
        &mut self,
        thread: Thread,
        position: usize,
        out: &mut Vec<Thread>,
    ) -> Result<(), Error> {
        self.pending.push(thread);
        while let Some(mut thread) = self.pending.pop() {
            self.charge(1)?;
            let seen = self.seen.get_mut(thread.pc).ok_or(Error::InvalidProgram)?;
            if *seen {
                continue;
            }
            *seen = true;
            self.visits = self.visits.saturating_add(1);
            match self
                .regex
                .code
                .get(thread.pc)
                .ok_or(Error::InvalidProgram)?
            {
                Instruction::Split(first, second) => {
                    self.charge(thread.captures.len())?;
                    let mut other = thread.clone();
                    other.pc = *second;
                    self.pending.push(other);
                    thread.pc = *first;
                    self.pending.push(thread);
                }
                Instruction::Save(slot, target) => {
                    *thread
                        .captures
                        .get_mut(*slot)
                        .ok_or(Error::InvalidProgram)? = Some(position);
                    thread.pc = *target;
                    self.pending.push(thread);
                }
                Instruction::Clear(range, target) => {
                    self.charge(range.len())?;
                    thread
                        .captures
                        .get_mut(range.clone())
                        .ok_or(Error::InvalidProgram)?
                        .fill(None);
                    thread.pc = *target;
                    self.pending.push(thread);
                }
                Instruction::Assert(assertion, target) => {
                    if self.assertion(*assertion, position) {
                        thread.pc = *target;
                        self.pending.push(thread);
                    }
                }
                Instruction::Look(class, positive, target) => {
                    self.charge(
                        usize::try_from(
                            usize::BITS.saturating_sub(class.ranges.len().leading_zeros()),
                        )
                        .unwrap_or(usize::MAX)
                        .saturating_add(1),
                    )?;
                    if self
                        .input
                        .get(position)
                        .is_some_and(|unit| class.contains(*unit))
                        == *positive
                    {
                        thread.pc = *target;
                        self.pending.push(thread);
                    }
                }
                _ => out.push(thread),
            }
        }
        Ok(())
    }
    fn assertion(&self, assertion: Assertion, position: usize) -> bool {
        assertion.matches(self.input, position, self.regex.options.multiline)
    }
}

fn to_match(registers: &[Option<usize>]) -> Result<Match, Error> {
    let mut ranges = registers.as_chunks::<2>().0.iter().map(|pair| {
        match (
            pair.first().copied().flatten(),
            pair.get(1).copied().flatten(),
        ) {
            (Some(start), Some(end)) if start <= end => Ok(Some(start..end)),
            (None, None) => Ok(None),
            _ => Err(Error::InvalidProgram),
        }
    });
    let range = ranges
        .next()
        .ok_or(Error::InvalidProgram)??
        .ok_or(Error::InvalidProgram)?;
    Ok(Match {
        range,
        captures: ranges.collect::<Result<_, _>>()?,
    })
}
