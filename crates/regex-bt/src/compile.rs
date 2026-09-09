// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
use crate::{Error, Limits, Options};
use alloc::{rc::Rc, vec::Vec};
use audhsos_regex::syntax::{self, Assertion, Class, Expr, Profile};
use core::ops::Range;

#[derive(Clone, Debug)]
pub(crate) enum Instruction {
    Unit(u16, bool, usize),
    Dot(bool, usize),
    Class(Rc<Class>, bool, usize),
    Assert(Assertion, usize),
    Predicate(Rc<Class>, bool, usize),
    Reference(usize, bool, usize),
    Split(usize, usize),
    Save(usize, usize),
    Clear(Range<usize>, usize),
    Progress(usize, usize),
    Look {
        entry: usize,
        next: usize,
        positive: bool,
    },
    LookEnd,
    Match,
}

/// Immutable backtracking bytecode. Construction never exposes raw instructions.
#[derive(Clone, Debug)]
pub struct Regex {
    pub(crate) code: Vec<Instruction>,
    pub(crate) start: usize,
    pub(crate) captures: usize,
    pub(crate) slots: usize,
    pub(crate) options: Options,
}
impl Regex {
    /// Compiles using the shared parser's explicit backtracking profile.
    ///
    /// # Errors
    /// Invalid/unsupported pattern or parser/compiled-storage quota exhaustion.
    pub fn compile(pattern: &[u16], options: Options, limits: Limits) -> Result<Self, Error> {
        let (tree, groups) =
            syntax::parse_with_profile(pattern, limits.core, Profile::Backtracking)?;
        let captures = groups.saturating_add(1).saturating_mul(2);
        if captures > limits.core.capture_cells {
            return Err(Error::Limit {
                resource: "capture registers",
            });
        }
        let mut c = Compiler {
            code: Vec::new(),
            limits,
            slots: captures,
            work: 0,
        };
        let end = c.emit(Instruction::Match)?;
        let end = c.emit(Instruction::Save(1, end))?;
        let body = c.expression(&tree, end, false)?;
        let start = c.emit(Instruction::Save(0, body))?;
        Ok(Self {
            code: c.code,
            start,
            captures,
            slots: c.slots,
            options,
        })
    }
    /// Number of bytecode instructions.
    #[must_use]
    pub const fn state_count(&self) -> usize {
        self.code.len()
    }
    /// Explicit capturing groups, excluding group zero.
    #[must_use]
    pub const fn capture_count(&self) -> usize {
        self.captures.saturating_div(2).saturating_sub(1)
    }
}
struct Compiler {
    code: Vec<Instruction>,
    limits: Limits,
    slots: usize,
    work: u64,
}
impl Compiler {
    fn emit(&mut self, op: Instruction) -> Result<usize, Error> {
        if self.code.len() >= self.limits.core.states {
            return Err(Error::Limit {
                resource: "compiled states",
            });
        }
        let index = self.code.len();
        self.code.push(op);
        Ok(index)
    }
    const fn slot(&mut self) -> Result<usize, Error> {
        if self.slots >= self.limits.core.capture_cells {
            return Err(Error::Limit {
                resource: "progress registers",
            });
        }
        let slot = self.slots;
        self.slots = self.slots.saturating_add(1);
        Ok(slot)
    }
    fn expression(&mut self, expr: &Expr, next: usize, backward: bool) -> Result<usize, Error> {
        self.charge()?;
        match expr {
            Expr::Empty => Ok(next),
            Expr::Unit(u) => self.emit(Instruction::Unit(*u, backward, next)),
            Expr::Dot => self.emit(Instruction::Dot(backward, next)),
            Expr::Class(c) => self.emit(Instruction::Class(c.clone(), backward, next)),
            Expr::Assert(a) => self.emit(Instruction::Assert(*a, next)),
            Expr::Look { class, positive } => {
                self.emit(Instruction::Predicate(class.clone(), *positive, next))
            }
            Expr::Backreference(id) => self.emit(Instruction::Reference(*id, backward, next)),
            Expr::Lookaround {
                body,
                positive,
                backward,
            } => {
                if !*backward && let Expr::Class(class) = body.as_ref() {
                    return self.emit(Instruction::Predicate(class.clone(), *positive, next));
                }
                let end = self.emit(Instruction::LookEnd)?;
                let entry = self.expression(body, end, *backward)?;
                self.emit(Instruction::Look {
                    entry,
                    next,
                    positive: *positive,
                })
            }
            Expr::Sequence(items) => {
                let mut start = next;
                if backward {
                    for item in items {
                        start = self.expression(item, start, true)?;
                    }
                } else {
                    for item in items.iter().rev() {
                        start = self.expression(item, start, false)?;
                    }
                }
                Ok(start)
            }
            Expr::Alternative(items) => {
                let mut iter = items.iter().rev();
                let mut start =
                    self.expression(iter.next().ok_or(Error::InvalidProgram)?, next, backward)?;
                for item in iter {
                    let first = self.expression(item, next, backward)?;
                    start = self.emit(Instruction::Split(first, start))?;
                }
                Ok(start)
            }
            Expr::Group(id, body) => {
                let a = id.saturating_mul(2);
                let b = a.saturating_add(1);
                let end = self.emit(Instruction::Save(if backward { a } else { b }, next))?;
                let start = self.expression(body, end, backward)?;
                self.emit(Instruction::Save(if backward { b } else { a }, start))
            }
            Expr::Repeat {
                body,
                min,
                max,
                greedy,
                captures,
            } => {
                let mut start = next;
                if let Some(max) = max {
                    if max > min {
                        let slot = self.slot()?;
                        for _ in *min..*max {
                            let progress = self.emit(Instruction::Progress(slot, start))?;
                            let iteration = self.iteration(body, progress, captures, backward)?;
                            let iteration = self.emit(Instruction::Save(slot, iteration))?;
                            start = self.split(iteration, start, *greedy)?;
                        }
                    }
                } else {
                    let slot = self.slot()?;
                    let branch = self.emit(Instruction::Match)?;
                    let progress = self.emit(Instruction::Progress(slot, branch))?;
                    let iteration = self.iteration(body, progress, captures, backward)?;
                    let iteration = self.emit(Instruction::Save(slot, iteration))?;
                    *self.code.get_mut(branch).ok_or(Error::InvalidProgram)? = if *greedy {
                        Instruction::Split(iteration, next)
                    } else {
                        Instruction::Split(next, iteration)
                    };
                    start = branch;
                }
                for _ in 0..*min {
                    start = self.iteration(body, start, captures, backward)?;
                }
                Ok(start)
            }
        }
    }
    fn iteration(
        &mut self,
        body: &Expr,
        next: usize,
        captures: &Range<usize>,
        backward: bool,
    ) -> Result<usize, Error> {
        self.charge()?;
        let start = self.expression(body, next, backward)?;
        if captures.is_empty() {
            Ok(start)
        } else {
            self.emit(Instruction::Clear(
                captures.start.saturating_mul(2)..captures.end.saturating_mul(2),
                start,
            ))
        }
    }
    fn split(&mut self, first: usize, last: usize, greedy: bool) -> Result<usize, Error> {
        self.emit(if greedy {
            Instruction::Split(first, last)
        } else {
            Instruction::Split(last, first)
        })
    }
    fn charge(&mut self) -> Result<(), Error> {
        self.work = self
            .work
            .checked_add(1)
            .filter(|n| *n <= self.limits.core.work)
            .ok_or(Error::Limit {
                resource: "compilation work",
            })?;
        Ok(())
    }
}
