// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Limits, Match, Regex, compile::Instruction};
use alloc::{vec, vec::Vec};
use audhsos_regex::syntax::newline;

/// Evidence for a completed bounded search. Work is not a linear-time claim.
#[derive(Clone, Debug)]
pub struct Report {
    /// First leftmost ordered match, if any.
    pub matched: Option<Match>,
    /// All charged work, including restarts and register copies.
    pub work: u64,
    /// Instructions executed, including repeated visits.
    pub state_visits: u64,
    /// Alternatives actually restored.
    pub backtracks: u64,
    /// Peak live saved alternatives across assertion contexts.
    pub peak_backtrack_frames: usize,
    /// Peak live nested assertions.
    pub peak_assertion_frames: usize,
}
struct Choice {
    pc: usize,
    position: usize,
    slots: Vec<Option<usize>>,
}
struct AssertionFrame {
    next: usize,
    position: usize,
    base: usize,
    positive: bool,
    slots: Vec<Option<usize>>,
}
struct Search<'a> {
    regex: &'a Regex,
    input: &'a [u16],
    limits: Limits,
    report: Report,
    choices: Vec<Choice>,
    assertions: Vec<AssertionFrame>,
    slots: Vec<Option<usize>>,
    pc: usize,
    position: usize,
}

impl Regex {
    /// Searches from `from`, or only at `from` when sticky. Every candidate
    /// start shares one work budget. Backtracking may take exponential work.
    ///
    /// # Errors
    /// Input, compiled-state, capture-cell, choice/assertion stack or work limits.
    /// Exhaustion is distinct from a successful report containing no match.
    pub fn find(
        &self,
        input: &[u16],
        from: usize,
        sticky: bool,
        limits: Limits,
    ) -> Result<Report, Error> {
        if input.len() > limits.core.input_units {
            return Err(Error::Limit {
                resource: "input units",
            });
        }
        if self.code.len() > limits.core.states {
            return Err(Error::Limit {
                resource: "match states",
            });
        }
        if self.slots > limits.core.capture_cells {
            return Err(Error::Limit {
                resource: "capture workspace",
            });
        }
        let report = Report {
            matched: None,
            work: 0,
            state_visits: 0,
            backtracks: 0,
            peak_backtrack_frames: 0,
            peak_assertion_frames: 0,
        };
        if from > input.len() {
            return Ok(report);
        }
        let mut search = Search {
            regex: self,
            input,
            limits,
            report,
            choices: Vec::new(),
            assertions: Vec::new(),
            slots: Vec::new(),
            pc: self.start,
            position: from,
        };
        search.charge(self.slots)?;
        search.slots = vec![None; self.slots];
        let end = if sticky { from } else { input.len() };
        for start in from..=end {
            search.charge(self.slots.saturating_add(1))?;
            search.slots.fill(None);
            search.choices.clear();
            search.assertions.clear();
            search.pc = self.start;
            search.position = start;
            if search.run()? {
                search.report.matched = Some(search.materialize_match()?);
                break;
            }
        }
        Ok(search.report)
    }
}
impl Search<'_> {
    fn charge(&mut self, n: usize) -> Result<(), Error> {
        self.report.work = self
            .report
            .work
            .checked_add(u64::try_from(n).unwrap_or(u64::MAX))
            .filter(|n| *n <= self.limits.core.work)
            .ok_or(Error::Limit {
                resource: "backtracking work",
            })?;
        Ok(())
    }
    fn save_slots(&mut self) -> Result<Vec<Option<usize>>, Error> {
        let frames = self
            .choices
            .len()
            .saturating_add(self.assertions.len())
            .saturating_add(2);
        if frames
            .checked_mul(self.regex.slots)
            .is_none_or(|n| n > self.limits.core.capture_cells)
        {
            return Err(Error::Limit {
                resource: "capture workspace",
            });
        }
        self.charge(self.regex.slots)?;
        Ok(self.slots.clone())
    }
    const fn unit_index(&self, backward: bool) -> Option<usize> {
        if backward {
            self.position.checked_sub(1)
        } else {
            Some(self.position)
        }
    }
    const fn advance(&mut self, backward: bool, next: usize) {
        self.position = if backward {
            self.position.saturating_sub(1)
        } else {
            self.position.saturating_add(1)
        };
        self.pc = next;
    }
    #[expect(
        clippy::too_many_lines,
        reason = "one iterative VM loop owns choice/assertion state without recursive matching"
    )]
    fn run(&mut self) -> Result<bool, Error> {
        loop {
            self.charge(1)?;
            self.report.state_visits = self.report.state_visits.saturating_add(1);
            let mut success = true;
            match self.regex.code.get(self.pc).ok_or(Error::InvalidProgram)? {
                Instruction::Match => {
                    if !self.assertions.is_empty() {
                        return Err(Error::InvalidProgram);
                    }
                    return Ok(true);
                }
                Instruction::LookEnd => {
                    let frame = self.assertions.pop().ok_or(Error::InvalidProgram)?;
                    self.choices.truncate(frame.base);
                    self.position = frame.position;
                    self.pc = frame.next;
                    if frame.positive {
                        // Progress registers belong to the enclosing repetition, not its assertion.
                        self.charge(self.regex.slots.saturating_sub(self.regex.captures))?;
                        self.slots
                            .get_mut(self.regex.captures..)
                            .ok_or(Error::InvalidProgram)?
                            .copy_from_slice(
                                frame
                                    .slots
                                    .get(self.regex.captures..)
                                    .ok_or(Error::InvalidProgram)?,
                            );
                    } else {
                        self.slots = frame.slots;
                        success = false;
                    }
                }
                Instruction::Unit(u, backward, next) => {
                    if self.unit_index(*backward).and_then(|i| self.input.get(i)) == Some(u) {
                        self.advance(*backward, *next);
                    } else {
                        success = false;
                    }
                }
                Instruction::Dot(backward, next) => {
                    if self
                        .unit_index(*backward)
                        .and_then(|i| self.input.get(i))
                        .is_some_and(|u| self.regex.options.dot_all || !newline(*u))
                    {
                        self.advance(*backward, *next);
                    } else {
                        success = false;
                    }
                }
                Instruction::Class(class, backward, next) => {
                    self.charge(
                        usize::try_from(
                            usize::BITS.saturating_sub(class.ranges.len().leading_zeros()),
                        )
                        .unwrap_or(usize::MAX)
                        .saturating_add(1),
                    )?;
                    if self
                        .unit_index(*backward)
                        .and_then(|i| self.input.get(i))
                        .is_some_and(|u| class.contains(*u))
                    {
                        self.advance(*backward, *next);
                    } else {
                        success = false;
                    }
                }
                Instruction::Assert(a, next) => {
                    if a.matches(self.input, self.position, self.regex.options.multiline) {
                        self.pc = *next;
                    } else {
                        success = false;
                    }
                }
                Instruction::Predicate(class, positive, next) => {
                    self.charge(class.ranges.len().saturating_add(1))?;
                    if self
                        .input
                        .get(self.position)
                        .is_some_and(|u| class.contains(*u))
                        == *positive
                    {
                        self.pc = *next;
                    } else {
                        success = false;
                    }
                }
                Instruction::Reference(group, backward, next) => {
                    let start = self.slots.get(group.saturating_mul(2)).copied().flatten();
                    let end = self
                        .slots
                        .get(group.saturating_mul(2).saturating_add(1))
                        .copied()
                        .flatten();
                    if let (Some(a), Some(b)) = (start, end) {
                        let length = b.checked_sub(a).ok_or(Error::InvalidProgram)?;
                        let other = if *backward {
                            self.position.checked_sub(length)
                        } else {
                            self.position
                                .checked_add(length)
                                .filter(|p| *p <= self.input.len())
                        };
                        if let Some(other) = other {
                            self.charge(length)?;
                            let low = self.position.min(other);
                            let high = self.position.max(other);
                            if self.input.get(a..b).ok_or(Error::InvalidProgram)?
                                == self.input.get(low..high).ok_or(Error::InvalidProgram)?
                            {
                                self.position = other;
                                self.pc = *next;
                            } else {
                                success = false;
                            }
                        } else {
                            success = false;
                        }
                    } else {
                        self.pc = *next;
                    }
                }
                Instruction::Split(first, second) => {
                    if self.choices.len() >= self.limits.backtrack_frames {
                        return Err(Error::Limit {
                            resource: "backtracking frames",
                        });
                    }
                    let slots = self.save_slots()?;
                    self.choices.push(Choice {
                        pc: *second,
                        position: self.position,
                        slots,
                    });
                    self.report.peak_backtrack_frames =
                        self.report.peak_backtrack_frames.max(self.choices.len());
                    self.pc = *first;
                }
                Instruction::Save(slot, next) => {
                    *self.slots.get_mut(*slot).ok_or(Error::InvalidProgram)? = Some(self.position);
                    self.pc = *next;
                }
                Instruction::Clear(range, next) => {
                    self.charge(range.len())?;
                    self.slots
                        .get_mut(range.clone())
                        .ok_or(Error::InvalidProgram)?
                        .fill(None);
                    self.pc = *next;
                }
                Instruction::Progress(slot, next) => {
                    if self.slots.get(*slot).copied().flatten() == Some(self.position) {
                        success = false;
                    } else {
                        self.pc = *next;
                    }
                }
                Instruction::Look {
                    entry,
                    next,
                    positive,
                } => {
                    if self.assertions.len() >= self.limits.assertion_frames {
                        return Err(Error::Limit {
                            resource: "assertion frames",
                        });
                    }
                    let slots = self.save_slots()?;
                    self.assertions.push(AssertionFrame {
                        next: *next,
                        position: self.position,
                        base: self.choices.len(),
                        positive: *positive,
                        slots,
                    });
                    self.report.peak_assertion_frames =
                        self.report.peak_assertion_frames.max(self.assertions.len());
                    self.pc = *entry;
                }
            }
            if !success && !self.backtrack()? {
                return Ok(false);
            }
        }
    }
    fn backtrack(&mut self) -> Result<bool, Error> {
        loop {
            self.charge(1)?;
            let boundary = self.assertions.last().map_or(0, |f| f.base);
            if self.choices.len() > boundary {
                let choice = self.choices.pop().ok_or(Error::InvalidProgram)?;
                self.pc = choice.pc;
                self.position = choice.position;
                self.slots = choice.slots;
                self.report.backtracks = self.report.backtracks.saturating_add(1);
                return Ok(true);
            }
            let Some(frame) = self.assertions.pop() else {
                return Ok(false);
            };
            self.pc = frame.next;
            self.position = frame.position;
            self.slots = frame.slots;
            if !frame.positive {
                return Ok(true);
            }
        }
    }
    fn materialize_match(&mut self) -> Result<Match, Error> {
        self.charge(self.regex.captures)?;
        let mut ranges = Vec::new();
        for pair in self
            .slots
            .get(..self.regex.captures)
            .ok_or(Error::InvalidProgram)?
            .as_chunks::<2>()
            .0
        {
            ranges.push(
                match (
                    pair.first().copied().flatten(),
                    pair.get(1).copied().flatten(),
                ) {
                    (Some(a), Some(b)) if a <= b && b <= self.input.len() => Some(a..b),
                    (None, None) => None,
                    _ => return Err(Error::InvalidProgram),
                },
            );
        }
        let range = ranges
            .first()
            .cloned()
            .flatten()
            .ok_or(Error::InvalidProgram)?;
        Ok(Match {
            range,
            captures: ranges.into_iter().skip(1).collect(),
        })
    }
}
