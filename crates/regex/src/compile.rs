// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Continuation-directed Thompson construction. Every edge is an instruction
//! index; recursion is confined to the bounded pattern tree, never matching.

use crate::{
    Error, Limits, Options,
    parse::{self, Assertion, Class, Expr},
};
use alloc::{rc::Rc, vec::Vec};

#[derive(Clone, Debug)]
pub(crate) enum Instruction {
    Unit(u16, usize),
    Dot(usize),
    Class(Rc<Class>, usize),
    Assert(Assertion, usize),
    Look(Rc<Class>, bool, usize),
    Split(usize, usize),
    Save(usize, usize),
    Clear(core::ops::Range<usize>, usize),
    Match,
}

/// Immutable compiled Thompson NFA over UTF-16 code units.
#[derive(Clone, Debug)]
pub struct Regex {
    pub(crate) code: Vec<Instruction>,
    pub(crate) start: usize,
    pub(crate) registers: usize,
    pub(crate) options: Options,
}

impl Regex {
    /// Compiles the supported BMP-pattern grammar without external code.
    ///
    /// # Errors
    /// Invalid syntax, an unsupported construct, or exhausted compile limits.
    pub fn compile(pattern: &[u16], options: Options, limits: Limits) -> Result<Self, Error> {
        let (tree, groups) = parse::parse(pattern, limits)?;
        let mut compiler = Compiler {
            code: Vec::new(),
            limits,
        };
        let end = compiler.emit(Instruction::Match)?;
        let end = compiler.emit(Instruction::Save(1, end))?;
        let body = compiler.expression(&tree, end)?;
        let start = compiler.emit(Instruction::Save(0, body))?;
        Ok(Self {
            code: compiler.code,
            start,
            registers: groups.saturating_add(1).saturating_mul(2),
            options,
        })
    }
    /// Number of compiled states, including epsilon/capture states.
    #[must_use]
    pub const fn state_count(&self) -> usize {
        self.code.len()
    }
    /// Number of explicit captures, excluding the whole match.
    #[must_use]
    pub const fn capture_count(&self) -> usize {
        self.registers.saturating_div(2).saturating_sub(1)
    }
}

struct Compiler {
    code: Vec<Instruction>,
    limits: Limits,
}
impl Compiler {
    fn emit(&mut self, instruction: Instruction) -> Result<usize, Error> {
        if self.code.len() >= self.limits.states {
            return Err(Error::Limit {
                resource: "compiled states",
            });
        }
        let at = self.code.len();
        self.code.push(instruction);
        Ok(at)
    }
    fn expression(&mut self, expr: &Expr, next: usize) -> Result<usize, Error> {
        match expr {
            Expr::Backreference(_) | Expr::Lookaround { .. } => Err(Error::Unsupported {
                offset: 0,
                feature: "backtracking syntax in automaton compiler",
            }),
            Expr::Empty => Ok(next),
            Expr::Unit(unit) => self.emit(Instruction::Unit(*unit, next)),
            Expr::Dot => self.emit(Instruction::Dot(next)),
            Expr::Class(class) => self.emit(Instruction::Class(class.clone(), next)),
            Expr::Assert(assertion) => self.emit(Instruction::Assert(*assertion, next)),
            Expr::Look { class, positive } => {
                self.emit(Instruction::Look(class.clone(), *positive, next))
            }
            Expr::Sequence(items) => {
                let mut next = next;
                for item in items.iter().rev() {
                    next = self.expression(item, next)?;
                }
                Ok(next)
            }
            Expr::Alternative(items) => {
                let mut choices = items.iter().rev();
                let last = choices.next().ok_or(Error::InvalidProgram)?;
                let mut start = self.expression(last, next)?;
                for item in choices {
                    let first = self.expression(item, next)?;
                    start = self.emit(Instruction::Split(first, start))?;
                }
                Ok(start)
            }
            Expr::Group(id, body) => {
                let end = self.emit(Instruction::Save(
                    id.saturating_mul(2).saturating_add(1),
                    next,
                ))?;
                let body = self.expression(body, end)?;
                self.emit(Instruction::Save(id.saturating_mul(2), body))
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
                    for _ in *min..*max {
                        let body = self.iteration(body, start, captures)?;
                        start = self.split(body, start, *greedy)?;
                    }
                } else {
                    let split = self.emit(Instruction::Split(0, 0))?;
                    let body = self.iteration(body, split, captures)?;
                    *self.code.get_mut(split).ok_or(Error::InvalidProgram)? = if *greedy {
                        Instruction::Split(body, next)
                    } else {
                        Instruction::Split(next, body)
                    };
                    start = split;
                }
                for _ in 0..*min {
                    start = self.iteration(body, start, captures)?;
                }
                Ok(start)
            }
        }
    }
    fn iteration(
        &mut self,
        body: &Expr,
        next: usize,
        captures: &core::ops::Range<usize>,
    ) -> Result<usize, Error> {
        let start = self.expression(body, next)?;
        if captures.is_empty() {
            Ok(start)
        } else {
            self.emit(Instruction::Clear(
                captures.start.saturating_mul(2)..captures.end.saturating_mul(2),
                start,
            ))
        }
    }
    fn split(&mut self, body: usize, next: usize, greedy: bool) -> Result<usize, Error> {
        self.emit(if greedy {
            Instruction::Split(body, next)
        } else {
            Instruction::Split(next, body)
        })
    }
}
