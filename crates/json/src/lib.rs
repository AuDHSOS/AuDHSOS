// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
extern crate alloc;
use alloc::{rc::Rc, string::String, vec::Vec};
use core::ops::Range;

/// Syntax or logical resource error; offsets are UTF-16 units, not UTF-8 bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Invalid JSON syntax at this code-unit offset.
    Syntax(usize),
    /// A configured logical resource limit was exceeded.
    Limit,
}
/// Independent parser limits.
#[derive(Clone, Copy)]
pub struct Limits {
    /// Maximum input code units.
    pub input: usize,
    /// Maximum total nodes in the flat arena.
    pub nodes: usize,
    /// Maximum container nesting (also hard-capped at 48).
    pub depth: usize,
    /// Maximum total decoded string/key units.
    pub strings: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            input: 1_048_576,
            nodes: 262_144,
            depth: 48,
            strings: 1_048_576,
        }
    }
}
/// Decoded JSON payload; container indices point to earlier arena nodes.
#[derive(Clone, Debug)]
pub enum Kind {
    /// JSON null.
    Null,
    /// JSON Boolean.
    Boolean(bool),
    /// Binary64 conversion, including overflow to infinity and signed zero.
    Number(f64),
    /// Decoded UTF-16 string.
    String(Rc<[u16]>),
    /// Array element indices in order.
    Array(Vec<usize>),
    /// Ordered names and child indices, including duplicates.
    Object(Vec<(Rc<[u16]>, usize)>),
}
/// One JSON value and its source range, excluding surrounding whitespace.
#[derive(Clone, Debug)]
pub struct Node {
    /// Decoded value.
    pub kind: Kind,
    /// Range of the exact lexical value.
    pub source: Range<usize>,
}
/// Postorder parse arena; the last node is the root.
pub struct Document {
    /// All parsed value nodes in postorder.
    pub nodes: Vec<Node>,
    /// Root node index.
    pub root: usize,
}
/// Parses exactly one JSON value, with no grammar extensions.
///
/// # Errors
/// Invalid grammar, or an input/depth/node/string/work limit.
pub fn parse(input: &[u16], limits: Limits, work: &mut u64) -> Result<Document, Error> {
    if input.len() > limits.input {
        return Err(Error::Limit);
    }
    let mut parser = Parser {
        input,
        at: 0,
        limits,
        nodes: Vec::new(),
        strings: 0,
        work,
    };
    let root = parser.value(0)?;
    parser.space()?;
    if parser.at != input.len() {
        return Err(Error::Syntax(parser.at));
    }
    Ok(Document {
        nodes: parser.nodes,
        root,
    })
}
struct Parser<'a, 'b> {
    input: &'a [u16],
    at: usize,
    limits: Limits,
    nodes: Vec<Node>,
    strings: usize,
    work: &'b mut u64,
}
impl Parser<'_, '_> {
    fn peek(&self) -> Option<u16> {
        self.input.get(self.at).copied()
    }
    fn bump(&mut self) -> Result<u16, Error> {
        let unit = self.peek().ok_or(Error::Syntax(self.at))?;
        charge(self.work, 1)?;
        self.at = self.at.saturating_add(1);
        Ok(unit)
    }
    fn eat(&mut self, u: u16) -> Result<bool, Error> {
        if self.peek() == Some(u) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn need(&mut self, u: u16) -> Result<(), Error> {
        if self.eat(u)? {
            Ok(())
        } else {
            Err(Error::Syntax(self.at))
        }
    }
    fn space(&mut self) -> Result<(), Error> {
        while matches!(self.peek(), Some(9 | 10 | 13 | 32)) {
            self.bump()?;
        }
        Ok(())
    }
    fn value(&mut self, depth: usize) -> Result<usize, Error> {
        if depth > self.limits.depth.min(48) || self.nodes.len() >= self.limits.nodes {
            return Err(Error::Limit);
        }
        self.space()?;
        let start = self.at;
        let kind = match self.peek() {
            Some(110) => {
                self.word("null")?;
                Kind::Null
            }
            Some(116) => {
                self.word("true")?;
                Kind::Boolean(true)
            }
            Some(102) => {
                self.word("false")?;
                Kind::Boolean(false)
            }
            Some(34) => Kind::String(self.string()?),
            Some(45 | 48..=57) => self.number()?,
            Some(91) => {
                self.bump()?;
                self.space()?;
                let mut items = Vec::new();
                if !self.eat(93)? {
                    loop {
                        items.push(self.value(depth.saturating_add(1))?);
                        self.space()?;
                        if self.eat(93)? {
                            break;
                        }
                        self.need(44)?;
                    }
                }
                Kind::Array(items)
            }
            Some(123) => {
                self.bump()?;
                self.space()?;
                let mut items = Vec::new();
                if !self.eat(125)? {
                    loop {
                        self.space()?;
                        self.need(34)?;
                        self.at = self.at.saturating_sub(1);
                        let name = self.string()?;
                        self.space()?;
                        self.need(58)?;
                        let value = self.value(depth.saturating_add(1))?;
                        items.push((name, value));
                        self.space()?;
                        if self.eat(125)? {
                            break;
                        }
                        self.need(44)?;
                    }
                }
                Kind::Object(items)
            }
            _ => return Err(Error::Syntax(self.at)),
        };
        if self.nodes.len() >= self.limits.nodes {
            return Err(Error::Limit);
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            kind,
            source: start..self.at,
        });
        Ok(index)
    }
    fn word(&mut self, word: &str) -> Result<(), Error> {
        for u in word.encode_utf16() {
            self.need(u)?;
        }
        Ok(())
    }
    fn number(&mut self) -> Result<Kind, Error> {
        let start = self.at;
        self.eat(45)?;
        if !self.eat(48)? {
            if !matches!(self.peek(), Some(49..=57)) {
                return Err(Error::Syntax(self.at));
            }
            self.digits()?;
        }
        if self.eat(46)? {
            self.digits()?;
        }
        if matches!(self.peek(), Some(69 | 101)) {
            self.bump()?;
            if matches!(self.peek(), Some(43 | 45)) {
                self.bump()?;
            }
            self.digits()?;
        }
        let text: String = self
            .input
            .get(start..self.at)
            .ok_or(Error::Syntax(start))?
            .iter()
            .map(|u| char::from_u32(u32::from(*u)).unwrap_or('\0'))
            .collect();
        Ok(Kind::Number(
            text.parse().map_err(|_| Error::Syntax(start))?,
        ))
    }
    fn digits(&mut self) -> Result<(), Error> {
        let start = self.at;
        while matches!(self.peek(), Some(48..=57)) {
            self.bump()?;
        }
        if self.at == start {
            Err(Error::Syntax(start))
        } else {
            Ok(())
        }
    }
    fn string(&mut self) -> Result<Rc<[u16]>, Error> {
        self.need(34)?;
        let mut units = Vec::new();
        loop {
            let unit = match self.bump()? {
                34 => break,
                92 => match self.bump()? {
                    34 => 34,
                    92 => 92,
                    47 => 47,
                    98 => 8,
                    102 => 12,
                    110 => 10,
                    114 => 13,
                    116 => 9,
                    117 => {
                        let mut n = 0u16;
                        for _ in 0..4 {
                            let u = self.bump()?;
                            let digit = match u {
                                48..=57 => u.saturating_sub(48),
                                65..=70 => u.saturating_sub(55),
                                97..=102 => u.saturating_sub(87),
                                _ => return Err(Error::Syntax(self.at.saturating_sub(1))),
                            };
                            n = n.wrapping_mul(16).wrapping_add(digit);
                        }
                        n
                    }
                    _ => return Err(Error::Syntax(self.at.saturating_sub(1))),
                },
                0..=31 => return Err(Error::Syntax(self.at.saturating_sub(1))),
                u => u,
            };
            self.strings = self
                .strings
                .checked_add(1)
                .filter(|n| *n <= self.limits.strings)
                .ok_or(Error::Limit)?;
            units.push(unit);
        }
        Ok(units.into())
    }
}
fn charge(work: &mut u64, n: usize) -> Result<(), Error> {
    *work = work
        .checked_sub(u64::try_from(n).unwrap_or(u64::MAX))
        .ok_or(Error::Limit)?;
    Ok(())
}
/// Appends units to a bounded output buffer and charges copied units.
///
/// # Errors
/// Output length or work quota exhausted before appending.
pub fn append(
    out: &mut Vec<u16>,
    units: &[u16],
    limit: usize,
    work: &mut u64,
) -> Result<(), Error> {
    if out.len().checked_add(units.len()).is_none_or(|n| n > limit) {
        return Err(Error::Limit);
    }
    charge(work, units.len())?;
    out.extend_from_slice(units);
    Ok(())
}
/// Appends a JSON-quoted UTF-16 string, escaping lone surrogates.
///
/// # Errors
/// Output/work quota exhausted. A partial prefix may already have been appended.
pub fn quote(out: &mut Vec<u16>, units: &[u16], limit: usize, work: &mut u64) -> Result<(), Error> {
    append(out, &[34], limit, work)?;
    for (index, u) in units.iter().copied().enumerate() {
        let escape = match u {
            34 => Some(34),
            92 => Some(92),
            8 => Some(98),
            9 => Some(116),
            10 => Some(110),
            12 => Some(102),
            13 => Some(114),
            _ => None,
        };
        if let Some(code) = escape {
            append(out, &[92, code], limit, work)?;
            continue;
        }
        let lone = match u {
            0xd800..=0xdbff => !units
                .get(index.saturating_add(1))
                .is_some_and(|v| (0xdc00..=0xdfff).contains(v)),
            0xdc00..=0xdfff => !index
                .checked_sub(1)
                .and_then(|i| units.get(i))
                .is_some_and(|v| (0xd800..=0xdbff).contains(v)),
            _ => false,
        };
        if u < 32 || lone {
            let mut text = [92, 117, 48, 48, 48, 48];
            for (i, shift) in [12, 8, 4, 0].iter().enumerate() {
                let digit = (u >> shift) & 15;
                *text.get_mut(i.saturating_add(2)).ok_or(Error::Limit)? = if digit < 10 {
                    48u16.saturating_add(digit)
                } else {
                    87u16.saturating_add(digit)
                };
            }
            append(out, &text, limit, work)?;
        } else {
            append(out, &[u], limit, work)?;
        }
    }
    append(out, &[34], limit, work)
}
#[cfg(test)]
mod tests;
