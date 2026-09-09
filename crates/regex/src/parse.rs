// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Shared UTF-16 grammar from ECMA-262 22.2.1. Parsing is bounded; the selected
//! profile never changes the matcher of `audhsos-regex`. Backtracking constructs
//! are AST nodes only and require the independent `audhsos-regex-bt` compiler.

use crate::{Error, Limits};
use alloc::{boxed::Box, rc::Rc, vec::Vec};

#[derive(Clone, Debug)]
/// Normalized inclusive ranges and an optional complement.
pub struct Class {
    /// Sorted, merged inclusive code-unit ranges from the parser.
    pub ranges: Vec<(u16, u16)>,
    /// Match the complement of these ranges.
    pub negate: bool,
}

impl Class {
    /// Tests membership in normalized ranges, respecting negation.
    #[must_use]
    pub fn contains(&self, unit: u16) -> bool {
        let index = self.ranges.partition_point(|(start, _)| *start <= unit);
        let found = index
            .checked_sub(1)
            .and_then(|i| self.ranges.get(i))
            .is_some_and(|(_, end)| unit <= *end);
        found != self.negate
    }
    fn normalize(&mut self) {
        self.ranges.sort_unstable();
        let mut merged: Vec<(u16, u16)> = Vec::new();
        for (start, end) in self.ranges.drain(..) {
            if let Some(last) = merged.last_mut()
                && start <= last.1.saturating_add(1)
            {
                last.1 = last.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        self.ranges = merged;
    }
    fn positive_ranges(&self) -> Vec<(u16, u16)> {
        if !self.negate {
            return self.ranges.clone();
        }
        let mut result = Vec::new();
        let mut next = 0u32;
        for (start, end) in &self.ranges {
            if next < u32::from(*start) {
                result.push((u16::try_from(next).unwrap_or(0), start.saturating_sub(1)));
            }
            next = u32::from(*end).saturating_add(1);
        }
        if let Ok(next) = u16::try_from(next) {
            result.push((next, u16::MAX));
        }
        result
    }
}

#[derive(Clone, Copy, Debug)]
/// Zero-width assertion shared by both engines.
pub enum Assertion {
    /// Start of input, or of a line in multiline mode.
    Start,
    /// End of input, or of a line in multiline mode.
    End,
    /// Word boundary if true; non-boundary otherwise (ASCII word characters).
    Word(bool),
}

/// Bounded-depth syntax tree shared by independently selected compilers.
#[derive(Debug)]
pub enum Expr {
    /// Empty concatenation.
    Empty,
    /// Literal UTF-16 unit.
    Unit(u16),
    /// Any unit, subject to dot-all mode.
    Dot,
    /// Normalized character class.
    Class(Rc<Class>),
    /// Anchor or word boundary.
    Assert(Assertion),
    /// One-unit lookahead predicate supported by the automaton profile.
    Look {
        /// Predicate evaluated at the current offset.
        class: Rc<Class>,
        /// Require predicate success if true, failure otherwise.
        positive: bool,
    },
    /// Decimal capture reference, including forward references.
    Backreference(usize),
    /// General atomic assertion, available only in the backtracking profile.
    Lookaround {
        /// Asserted subexpression.
        body: Box<Expr>,
        /// Positive versus negative assertion.
        positive: bool,
        /// Evaluate the subexpression backwards for lookbehind.
        backward: bool,
    },
    /// Concatenation in source order.
    Sequence(Vec<Expr>),
    /// Alternatives in priority order.
    Alternative(Vec<Expr>),
    /// Numbered capturing group (group zero is implicit).
    Group(usize, Box<Expr>),
    /// Greedy or lazy bounded/unbounded repetition.
    Repeat {
        /// Repeated atom.
        body: Box<Expr>,
        /// Required repetitions.
        min: usize,
        /// Maximum repetitions; None is unbounded.
        max: Option<usize>,
        /// Prefer another iteration over the continuation.
        greedy: bool,
        /// Capture IDs cleared before each iteration.
        captures: core::ops::Range<usize>,
    },
}

impl Expr {
    fn nullable(&self) -> bool {
        match self {
            Self::Empty
            | Self::Assert(_)
            | Self::Look { .. }
            | Self::Lookaround { .. }
            | Self::Backreference(_) => true,
            Self::Unit(_) | Self::Dot | Self::Class(_) => false,
            Self::Sequence(items) => items.iter().all(Self::nullable),
            Self::Alternative(items) => items.iter().any(Self::nullable),
            Self::Group(_, body) => body.nullable(),
            Self::Repeat { body, min, .. } => *min == 0 || body.nullable(),
        }
    }
}

/// Explicit grammar selection. No matcher fallback is implied by this choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The existing non-backtracking subset; nullable quantifiers are rejected.
    Automaton,
    /// Adds decimal backreferences, general lookaround and nullable repetition.
    Backtracking,
}

/// Parses with the original automaton restrictions.
///
/// # Errors
/// Invalid syntax, unsupported syntax, or resource exhaustion.
pub fn parse(pattern: &[u16], limits: Limits) -> Result<(Expr, usize), Error> {
    parse_with_profile(pattern, limits, Profile::Automaton)
}
/// Parses a pattern with an explicitly selected syntax profile.
///
/// # Errors
/// Invalid syntax, unsupported syntax, or resource exhaustion. References to
/// non-existent groups are unsupported legacy escapes, never approximated.
pub fn parse_with_profile(
    pattern: &[u16],
    limits: Limits,
    profile: Profile,
) -> Result<(Expr, usize), Error> {
    if pattern.len() > limits.pattern_units {
        return Err(Error::Limit {
            resource: "pattern units",
        });
    }
    let mut parser = Parser {
        pattern,
        at: 0,
        depth: 0,
        groups: 0,
        ranges: 0,
        limits,
        profile,
        largest_reference: 0,
    };
    let expr = parser.disjunction()?;
    if parser.peek().is_some() {
        return Err(parser.syntax("unmatched closing parenthesis"));
    }
    if parser.largest_reference > parser.groups {
        return Err(parser.unsupported("legacy decimal or octal escape without a capture"));
    }
    Ok((expr, parser.groups))
}
struct Parser<'a> {
    pattern: &'a [u16],
    at: usize,
    depth: usize,
    groups: usize,
    ranges: usize,
    limits: Limits,
    profile: Profile,
    largest_reference: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.pattern.get(self.at).copied()
    }
    fn bump(&mut self) -> Option<u16> {
        let unit = self.peek()?;
        self.at = self.at.saturating_add(1);
        Some(unit)
    }
    fn eat(&mut self, unit: u16) -> bool {
        if self.peek() == Some(unit) {
            self.bump();
            true
        } else {
            false
        }
    }
    const fn syntax(&self, message: &'static str) -> Error {
        Error::Syntax {
            offset: self.at,
            message,
        }
    }
    const fn unsupported(&self, feature: &'static str) -> Error {
        Error::Unsupported {
            offset: self.at,
            feature,
        }
    }
    fn disjunction(&mut self) -> Result<Expr, Error> {
        if self.depth >= self.limits.depth.min(48) {
            return Err(Error::Limit {
                resource: "pattern nesting",
            });
        }
        self.depth = self.depth.saturating_add(1);
        let mut choices = Vec::new();
        loop {
            let mut sequence = Vec::new();
            while self.peek().is_some_and(|u| !matches!(u, 41 | 124)) {
                sequence.push(self.term()?);
            }
            choices.push(match sequence.len() {
                0 => Expr::Empty,
                1 => sequence.pop().ok_or(Error::InvalidProgram)?,
                _ => Expr::Sequence(sequence),
            });
            if !self.eat(124) {
                break;
            }
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(if choices.len() == 1 {
            choices.pop().ok_or(Error::InvalidProgram)?
        } else {
            Expr::Alternative(choices)
        })
    }
    fn term(&mut self) -> Result<Expr, Error> {
        let first_group = self.groups.saturating_add(1);
        let atom = self.atom()?;
        let quantifier = match self.peek() {
            Some(42) => {
                self.bump();
                Some((0, None))
            }
            Some(43) => {
                self.bump();
                Some((1, None))
            }
            Some(63) => {
                self.bump();
                Some((0, Some(1)))
            }
            Some(123)
                if self
                    .pattern
                    .get(self.at.saturating_add(1))
                    .is_some_and(|u| matches!(u, 48..=57)) =>
            {
                Some(self.counted()?)
            }
            _ => None,
        };
        let Some((min, max)) = quantifier else {
            return Ok(atom);
        };
        if matches!(
            atom,
            Expr::Assert(_) | Expr::Look { .. } | Expr::Lookaround { .. }
        ) {
            return Err(self.syntax("cannot quantify an assertion"));
        }
        let greedy = !self.eat(63);
        if matches!(self.peek(), Some(42 | 43 | 63 | 123)) {
            return Err(self.syntax("repeated quantifier"));
        }
        // ECMAScript suppresses empty repeat iterations differently from a
        // tagged epsilon loop. Refuse this gap rather than report wrong captures.
        if self.profile == Profile::Automaton && atom.nullable() {
            return Err(self.unsupported("repetition of a nullable expression"));
        }
        Ok(Expr::Repeat {
            body: Box::new(atom),
            min,
            max,
            greedy,
            captures: first_group..self.groups.saturating_add(1),
        })
    }
    fn counted(&mut self) -> Result<(usize, Option<usize>), Error> {
        self.bump();
        let min = self.decimal()?;
        let max = if self.eat(44) {
            if self.peek() == Some(125) {
                None
            } else {
                Some(self.decimal()?)
            }
        } else {
            Some(min)
        };
        if !self.eat(125) {
            return Err(self.syntax("unterminated repetition"));
        }
        if max.is_some_and(|n| n < min) {
            return Err(self.syntax("repetition maximum below minimum"));
        }
        Ok((min, max))
    }
    fn decimal(&mut self) -> Result<usize, Error> {
        let mut number = 0usize;
        let start = self.at;
        while let Some(unit @ 48..=57) = self.peek() {
            self.bump();
            number = number
                .checked_mul(10)
                .and_then(|n| n.checked_add(usize::from(unit.saturating_sub(48))))
                .filter(|n| *n <= self.limits.repetition)
                .ok_or(Error::Limit {
                    resource: "repetition expansion",
                })?;
        }
        if self.at == start {
            return Err(self.syntax("expected repetition digits"));
        }
        Ok(number)
    }
    fn atom(&mut self) -> Result<Expr, Error> {
        Ok(
            match self.bump().ok_or_else(|| self.syntax("expected atom"))? {
                94 => Expr::Assert(Assertion::Start),
                36 => Expr::Assert(Assertion::End),
                46 => Expr::Dot,
                92 => self.escape(false)?,
                91 => Expr::Class(Rc::new(self.class()?)),
                40 => {
                    let capture = if self.eat(63) {
                        if self.profile == Profile::Backtracking
                            && matches!(self.peek(), Some(61 | 33 | 60))
                        {
                            return self.general_lookaround();
                        }
                        if matches!(self.peek(), Some(61 | 33)) {
                            return self.lookahead();
                        }
                        if !self.eat(58) {
                            return Err(
                                self.unsupported("lookaround, named groups or inline modifiers")
                            );
                        }
                        None
                    } else {
                        if self.groups >= self.limits.captures {
                            return Err(Error::Limit {
                                resource: "captures",
                            });
                        }
                        self.groups = self.groups.saturating_add(1);
                        Some(self.groups)
                    };
                    let body = self.disjunction()?;
                    if !self.eat(41) {
                        return Err(self.syntax("unterminated group"));
                    }
                    if let Some(id) = capture {
                        Expr::Group(id, Box::new(body))
                    } else {
                        body
                    }
                }
                42 | 43 | 63 => return Err(self.syntax("quantifier without atom")),
                123 if self.peek().is_some_and(|u| matches!(u, 48..=57)) => {
                    return Err(self.syntax("quantifier without atom"));
                }
                93 => return Err(self.syntax("unescaped delimiter")),
                unit => Expr::Unit(unit),
            },
        )
    }
    fn escape(&mut self, in_class: bool) -> Result<Expr, Error> {
        let unit = self.bump().ok_or_else(|| self.syntax("trailing escape"))?;
        Ok(match unit {
            100 | 68 | 119 | 87 | 115 | 83 => Expr::Class(Rc::new(shorthand(unit))),
            98 if !in_class => Expr::Assert(Assertion::Word(true)),
            66 if !in_class => Expr::Assert(Assertion::Word(false)),
            98 => Expr::Unit(8),
            110 => Expr::Unit(10),
            114 => Expr::Unit(13),
            116 => Expr::Unit(9),
            118 => Expr::Unit(11),
            102 => Expr::Unit(12),
            120 => Expr::Unit(self.hex(2)?),
            117 => {
                if self.peek() == Some(123) {
                    return Err(self.unsupported("Unicode code-point escapes"));
                }
                Expr::Unit(self.hex(4)?)
            }
            99 => {
                let unit = self
                    .bump()
                    .filter(|u| matches!(u,65..=90|97..=122))
                    .ok_or_else(|| self.syntax("invalid control escape"))?;
                Expr::Unit(unit % 32)
            }
            48 if !self.peek().is_some_and(|u| matches!(u, 48..=57)) => Expr::Unit(0),
            49..=57 if !in_class && self.profile == Profile::Backtracking => {
                let mut group = usize::from(unit.saturating_sub(48));
                while let Some(unit @ 48..=57) = self.peek() {
                    self.bump();
                    group = group
                        .checked_mul(10)
                        .and_then(|n| n.checked_add(usize::from(unit.saturating_sub(48))))
                        .ok_or(Error::Limit {
                            resource: "backreference index",
                        })?;
                }
                self.largest_reference = self.largest_reference.max(group);
                Expr::Backreference(group)
            }
            48..=57 | 107 => return Err(self.unsupported("backreference or legacy octal escape")),
            112 | 80 => return Err(self.unsupported("Unicode property escape")),
            65..=90 | 97..=122 => return Err(self.unsupported("legacy identity escape")),
            unit => Expr::Unit(unit),
        })
    }

    fn lookahead(&mut self) -> Result<Expr, Error> {
        let positive = self.eat(61);
        if !positive {
            self.eat(33);
        }
        // A single character predicate can be checked at the current offset;
        // there is no secondary search, retry stack, or input cursor movement.
        let body = match self.peek() {
            Some(91) => {
                self.bump();
                Expr::Class(Rc::new(self.class()?))
            }
            Some(92) => {
                self.bump();
                self.escape(false)?
            }
            Some(46) => {
                self.bump();
                return Err(self.unsupported("dot lookahead"));
            }
            Some(unit) if !matches!(unit, 40 | 41 | 42 | 43 | 63 | 94 | 36 | 124 | 123) => {
                self.bump();
                Expr::Unit(unit)
            }
            _ => {
                return Err(self
                    .unsupported("lookahead requires a single noncapturing character predicate"));
            }
        };
        if !self.eat(41) {
            return Err(self.unsupported("multi-character or quantified lookahead"));
        }
        let class = match body {
            Expr::Unit(unit) => Rc::new(Class {
                ranges: alloc::vec![(unit, unit)],
                negate: false,
            }),
            Expr::Class(class) => class,
            _ => return Err(self.unsupported("nested assertion lookahead")),
        };
        Ok(Expr::Look { class, positive })
    }
    fn general_lookaround(&mut self) -> Result<Expr, Error> {
        let backward = self.eat(60);
        let positive = if self.eat(61) {
            true
        } else if self.eat(33) {
            false
        } else {
            return Err(self.unsupported("named capturing groups"));
        };
        let body = self.disjunction()?;
        if !self.eat(41) {
            return Err(self.syntax("unterminated lookaround"));
        }
        Ok(Expr::Lookaround {
            body: Box::new(body),
            positive,
            backward,
        })
    }
    fn hex(&mut self, count: usize) -> Result<u16, Error> {
        let mut number = 0u16;
        for _ in 0..count {
            let digit = self
                .bump()
                .and_then(|u| char::from_u32(u32::from(u)))
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.syntax("invalid hexadecimal escape"))?;
            number = number
                .checked_mul(16)
                .and_then(|n| n.checked_add(u16::try_from(digit).ok()?))
                .ok_or(Error::InvalidProgram)?;
        }
        Ok(number)
    }
    fn class(&mut self) -> Result<Class, Error> {
        let negate = self.eat(94);
        let mut ranges = Vec::new();
        while self.peek() != Some(93) {
            let first = self.class_atom()?;
            if self.peek() == Some(45) && self.pattern.get(self.at.saturating_add(1)) != Some(&93) {
                self.bump();
                let last = self.class_atom()?;
                let (Expr::Unit(a), Expr::Unit(b)) = (first, last) else {
                    return Err(self.unsupported("class escape as range endpoint"));
                };
                if a > b {
                    return Err(self.syntax("reversed class range"));
                }
                ranges.push((a, b));
            } else {
                match first {
                    Expr::Unit(u) => ranges.push((u, u)),
                    Expr::Class(c) => ranges.extend(c.positive_ranges()),
                    _ => return Err(Error::InvalidProgram),
                }
            }
            if ranges.len().saturating_add(self.ranges) > self.limits.ranges {
                return Err(Error::Limit {
                    resource: "character-class ranges",
                });
            }
        }
        self.bump();
        self.ranges = self.ranges.saturating_add(ranges.len());
        let mut class = Class { ranges, negate };
        class.normalize();
        Ok(class)
    }
    fn class_atom(&mut self) -> Result<Expr, Error> {
        match self
            .bump()
            .ok_or_else(|| self.syntax("unterminated character class"))?
        {
            92 => self.escape(true),
            unit => Ok(Expr::Unit(unit)),
        }
    }
}

fn shorthand(unit: u16) -> Class {
    let ranges = match unit {
        100 | 68 => alloc::vec![(48, 57)],
        119 | 87 => alloc::vec![(48, 57), (65, 90), (95, 95), (97, 122)],
        _ => alloc::vec![
            (9, 13),
            (32, 32),
            (160, 160),
            (0x1680, 0x1680),
            (0x2000, 0x200a),
            (0x2028, 0x2029),
            (0x202f, 0x202f),
            (0x205f, 0x205f),
            (0x3000, 0x3000),
            (0xfeff, 0xfeff)
        ],
    };
    Class {
        ranges,
        negate: matches!(unit, 68 | 87 | 83),
    }
}

/// Whether a code unit belongs to the non-Unicode ECMAScript word class.
#[must_use]
pub const fn word(unit: u16) -> bool {
    matches!(unit,48..=57|65..=90|95|97..=122)
}
/// Whether a unit is an ECMAScript line terminator.
#[must_use]
pub const fn newline(unit: u16) -> bool {
    matches!(unit, 10 | 13 | 0x2028 | 0x2029)
}

impl Assertion {
    /// Evaluates an anchor/boundary at a code-unit offset.
    #[must_use]
    pub fn matches(self, input: &[u16], position: usize, multiline: bool) -> bool {
        let previous = position.checked_sub(1).and_then(|i| input.get(i)).copied();
        let next = input.get(position).copied();
        match self {
            Self::Start => position == 0 || (multiline && previous.is_some_and(newline)),
            Self::End => position == input.len() || (multiline && next.is_some_and(newline)),
            Self::Word(boundary) => {
                (previous.is_some_and(word) != next.is_some_and(word)) == boundary
            }
        }
    }
}
