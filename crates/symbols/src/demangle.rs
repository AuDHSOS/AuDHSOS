// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The Rust symbol names, read back.
//!
//! A symbol table carries `_RNvNtCs1234_11kernel_core6memory8bring_up`; a
//! report wants `kernel_core::memory::bring_up`. This reads the `v0`
//! scheme of RFC 2603 and the legacy `_ZN` scheme, and writes the path.
//!
//! Invariants: nothing is allocated, so the name is written straight into
//! the formatter; a name the parser does not understand is written
//! unchanged, which is why the parse runs twice, once into a sink that
//! discards and once into the formatter; a backreference must point
//! backwards and the nesting is bounded, so no input makes the parser
//! loop or run out of stack.

use core::fmt;

/// How deep a path or a type may nest before the parser gives up.
const MAX_DEPTH: u32 = 64;

/// The prefix of a `v0` symbol.
const V0: &str = "_R";

/// The prefix of a legacy symbol.
const LEGACY: &str = "_ZN";

/// Generic arguments are dropped, because a panic report wants the path
/// and not the instantiation.
const ARGS: &str = "";

/// A symbol name that writes itself demangled.
#[derive(Clone, Copy, Debug)]
pub struct Demangled<'a> {
    name: &'a str,
}

/// The name of `symbol`, written demangled where it can be.
#[must_use]
pub const fn demangle(symbol: &str) -> Demangled<'_> {
    Demangled { name: symbol }
}

impl fmt::Display for Demangled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if write_demangled(self.name, &mut Discard).is_err() {
            return f.write_str(self.name);
        }
        let mut sink = Writer { inner: f };
        write_demangled(self.name, &mut sink).map_err(|_| fmt::Error)
    }
}

/// Somewhere the demangled name goes.
trait Sink {
    /// Appends `text`.
    fn put(&mut self, text: &str) -> Result<(), Failed>;

    /// Appends a number in decimal.
    fn put_number(&mut self, value: u64) -> Result<(), Failed>;
}

/// A sink that keeps nothing, for the pass that only asks whether the
/// name parses.
struct Discard;

impl Sink for Discard {
    fn put(&mut self, _text: &str) -> Result<(), Failed> {
        Ok(())
    }

    fn put_number(&mut self, _value: u64) -> Result<(), Failed> {
        Ok(())
    }
}

/// A sink that writes into a formatter.
struct Writer<'a, 'b> {
    inner: &'a mut fmt::Formatter<'b>,
}

impl Sink for Writer<'_, '_> {
    fn put(&mut self, text: &str) -> Result<(), Failed> {
        fmt::Write::write_str(self.inner, text).map_err(|_| Failed)
    }

    fn put_number(&mut self, value: u64) -> Result<(), Failed> {
        fmt::Write::write_fmt(self.inner, format_args!("{value}")).map_err(|_| Failed)
    }
}

/// The name is not one this module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Failed;

/// Writes the demangled `symbol` into `out`.
fn write_demangled(symbol: &str, out: &mut impl Sink) -> Result<(), Failed> {
    if let Some(rest) = symbol.strip_prefix(V0) {
        return Parser::new(rest).symbol(out);
    }
    if let Some(rest) = symbol.strip_prefix(LEGACY) {
        return legacy(rest, out);
    }
    Err(Failed)
}

/// Writes a legacy name: length-prefixed components until `E`, with the
/// trailing hash component left out.
fn legacy(mut rest: &str, out: &mut impl Sink) -> Result<(), Failed> {
    let mut components: [&str; 32] = [""; 32];
    let mut count = 0usize;
    loop {
        if rest.starts_with('E') {
            break;
        }
        let digits = rest.find(|c: char| !c.is_ascii_digit()).ok_or(Failed)?;
        if digits == 0 {
            return Err(Failed);
        }
        let len: usize = rest
            .get(..digits)
            .ok_or(Failed)?
            .parse()
            .map_err(|_| Failed)?;
        let after = rest.get(digits..).ok_or(Failed)?;
        let component = after.get(..len).ok_or(Failed)?;
        rest = after.get(len..).ok_or(Failed)?;
        let slot = components.get_mut(count).ok_or(Failed)?;
        *slot = component;
        count = count.saturating_add(1);
    }
    // The last component of a legacy name is the hash of the symbol.
    let kept = count.saturating_sub(1);
    if kept == 0 {
        return Err(Failed);
    }
    for (index, component) in components.iter().take(kept).enumerate() {
        if index > 0 {
            out.put("::")?;
        }
        out.put(component)?;
    }
    Ok(())
}

/// A position in a `v0` symbol.
struct Parser<'a> {
    bytes: &'a str,
    at: usize,
    depth: u32,
}

impl<'a> Parser<'a> {
    /// A parser at the start of `bytes`, which is the symbol without its
    /// `_R` prefix.
    const fn new(bytes: &'a str) -> Self {
        Parser {
            bytes,
            at: 0,
            depth: 0,
        }
    }

    /// The whole symbol: an optional version, then the path.
    fn symbol(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        if self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            // A version this module does not know.
            return Err(Failed);
        }
        self.path(out)
    }

    /// The byte at the cursor.
    fn peek(&self) -> Option<u8> {
        self.bytes.as_bytes().get(self.at).copied()
    }

    /// Takes the byte at the cursor.
    fn next(&mut self) -> Result<u8, Failed> {
        let byte = self.peek().ok_or(Failed)?;
        self.at = self.at.saturating_add(1);
        Ok(byte)
    }

    /// Takes the byte at the cursor if it is `byte`.
    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.at = self.at.saturating_add(1);
            return true;
        }
        false
    }

    /// Runs `body` one level deeper, or gives up.
    fn nested<T>(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<T, Failed>,
    ) -> Result<T, Failed> {
        if self.depth >= MAX_DEPTH {
            return Err(Failed);
        }
        self.depth = self.depth.saturating_add(1);
        let outcome = body(self);
        self.depth = self.depth.saturating_sub(1);
        outcome
    }

    /// A base-62 number, which every count and every backreference uses.
    fn base62(&mut self) -> Result<usize, Failed> {
        let mut value = 0usize;
        loop {
            let byte = self.next()?;
            if byte == b'_' {
                return Ok(value);
            }
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte).wrapping_sub(u32::from(b'0')),
                b'a'..=b'z' => u32::from(byte)
                    .wrapping_sub(u32::from(b'a'))
                    .wrapping_add(10),
                b'A'..=b'Z' => u32::from(byte)
                    .wrapping_sub(u32::from(b'A'))
                    .wrapping_add(36),
                _ => return Err(Failed),
            };
            value = value
                .checked_mul(62)
                .and_then(|scaled| scaled.checked_add(usize::try_from(digit).ok()?))
                .ok_or(Failed)?;
        }
    }

    /// A base-62 number as it counts: the empty one is zero, every other
    /// is one more than it reads.
    fn counted(&mut self) -> Result<usize, Failed> {
        let start = self.at;
        let value = self.base62()?;
        if self.at.saturating_sub(start) == 1 {
            return Ok(0);
        }
        value.checked_add(1).ok_or(Failed)
    }

    /// A decimal number. A leading zero is the whole number, which is
    /// what tells the empty identifier of a closure from the digits of
    /// the identifier that follows it.
    fn decimal(&mut self) -> Result<usize, Failed> {
        let start = self.at;
        if self.peek() == Some(b'0') {
            self.at = self.at.saturating_add(1);
            return Ok(0);
        }
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.at = self.at.saturating_add(1);
        }
        if self.at == start {
            return Err(Failed);
        }
        self.bytes
            .get(start..self.at)
            .ok_or(Failed)?
            .parse()
            .map_err(|_| Failed)
    }

    /// The `s` disambiguator, if one is there.
    fn disambiguator(&mut self) -> Result<(), Failed> {
        if self.eat(b's') {
            self.counted()?;
        }
        Ok(())
    }

    /// An identifier: its bytes, without the length in front.
    fn identifier(&mut self) -> Result<&'a str, Failed> {
        self.disambiguator()?;
        self.undisambiguated()
    }

    /// An identifier without a disambiguator.
    fn undisambiguated(&mut self) -> Result<&'a str, Failed> {
        // A punycode identifier is written as it stands: this project has
        // no name outside ASCII, and a wrong guess would be worse than a
        // literal one.
        let _punycode = self.eat(b'u');
        let len = self.decimal()?;
        self.eat(b'_');
        let end = self.at.checked_add(len).ok_or(Failed)?;
        let text = self.bytes.get(self.at..end).ok_or(Failed)?;
        self.at = end;
        Ok(text)
    }

    /// A lifetime, which the output leaves out.
    fn lifetime(&mut self) -> Result<(), Failed> {
        self.counted().map(|_| ())
    }

    /// A backreference: where it points, checked to point backwards.
    fn backref(&mut self) -> Result<usize, Failed> {
        let here = self.at.saturating_sub(1);
        let target = self.counted()?;
        if target >= here {
            return Err(Failed);
        }
        Ok(target)
    }

    /// Runs `body` at `target` and comes back.
    fn at_backref<T>(
        &mut self,
        target: usize,
        body: impl FnOnce(&mut Self) -> Result<T, Failed>,
    ) -> Result<T, Failed> {
        let saved = self.at;
        self.at = target;
        let outcome = self.nested(body);
        self.at = saved;
        outcome
    }

    /// A path, written as `crate::module::name`.
    fn path(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        self.nested(|parser| parser.path_inner(out))
    }

    /// The body of [`Parser::path`].
    fn path_inner(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        match self.next()? {
            b'C' => {
                let name = self.identifier()?;
                out.put(name)
            }
            b'M' => {
                self.impl_path()?;
                self.kind(out)
            }
            b'X' => {
                self.impl_path()?;
                out.put("<")?;
                self.kind(out)?;
                out.put(" as ")?;
                self.path(out)?;
                out.put(">")
            }
            b'Y' => {
                out.put("<")?;
                self.kind(out)?;
                out.put(" as ")?;
                self.path(out)?;
                out.put(">")
            }
            b'N' => {
                let namespace = self.next()?;
                self.path(out)?;
                let name = self.identifier()?;
                out.put("::")?;
                if name.is_empty() {
                    return out.put(special(namespace));
                }
                out.put(name)
            }
            b'I' => {
                self.path(out)?;
                while !self.eat(b'E') {
                    self.generic_argument()?;
                }
                out.put(ARGS)
            }
            b'B' => {
                let target = self.backref()?;
                self.at_backref(target, |parser| parser.path_inner(out))
            }
            _ => Err(Failed),
        }
    }

    /// The path of an implementation, which the output leaves out.
    fn impl_path(&mut self) -> Result<(), Failed> {
        self.disambiguator()?;
        self.path(&mut Discard)
    }

    /// One generic argument, which the output leaves out.
    fn generic_argument(&mut self) -> Result<(), Failed> {
        match self.peek().ok_or(Failed)? {
            b'L' => {
                self.at = self.at.saturating_add(1);
                self.lifetime()
            }
            b'K' => {
                self.at = self.at.saturating_add(1);
                self.constant().map(|_| ())
            }
            _ => self.kind(&mut Discard),
        }
    }

    /// A type, written in a short form.
    fn kind(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        self.nested(|parser| parser.kind_inner(out))
    }

    /// The body of [`Parser::kind`].
    fn kind_inner(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        let byte = self.peek().ok_or(Failed)?;
        if let Some(name) = basic(byte) {
            self.at = self.at.saturating_add(1);
            return out.put(name);
        }
        match byte {
            b'A' => {
                self.at = self.at.saturating_add(1);
                out.put("[")?;
                self.kind(out)?;
                out.put("; ")?;
                match self.constant()? {
                    Some(length) => out.put_number(length)?,
                    None => out.put("_")?,
                }
                out.put("]")
            }
            b'S' => {
                self.at = self.at.saturating_add(1);
                out.put("[")?;
                self.kind(out)?;
                out.put("]")
            }
            b'P' => {
                self.at = self.at.saturating_add(1);
                out.put("*const ")?;
                self.kind(out)
            }
            b'O' => {
                self.at = self.at.saturating_add(1);
                out.put("*mut ")?;
                self.kind(out)
            }
            b'R' => {
                self.at = self.at.saturating_add(1);
                out.put("&")?;
                if self.eat(b'L') {
                    self.lifetime()?;
                }
                self.kind(out)
            }
            b'Q' => {
                self.at = self.at.saturating_add(1);
                out.put("&mut ")?;
                if self.eat(b'L') {
                    self.lifetime()?;
                }
                self.kind(out)
            }
            b'T' => {
                self.at = self.at.saturating_add(1);
                out.put("(")?;
                let mut first = true;
                while !self.eat(b'E') {
                    if !first {
                        out.put(", ")?;
                    }
                    first = false;
                    self.kind(out)?;
                }
                out.put(")")
            }
            b'F' => {
                self.at = self.at.saturating_add(1);
                self.function()?;
                out.put("fn(_)")
            }
            b'D' => {
                self.at = self.at.saturating_add(1);
                self.dynamic()?;
                out.put("dyn _")
            }
            b'B' => {
                self.at = self.at.saturating_add(1);
                let target = self.backref()?;
                self.at_backref(target, |parser| parser.kind_inner(out))
            }
            _ => self.path_inner_after_check(out),
        }
    }

    /// A type that is a path.
    fn path_inner_after_check(&mut self, out: &mut impl Sink) -> Result<(), Failed> {
        match self.peek().ok_or(Failed)? {
            b'C' | b'M' | b'X' | b'Y' | b'N' | b'I' => self.path_inner(out),
            _ => Err(Failed),
        }
    }

    /// A function signature, which the output leaves out.
    fn function(&mut self) -> Result<(), Failed> {
        if self.eat(b'G') {
            self.counted()?;
        }
        let _unsafe = self.eat(b'U');
        if self.eat(b'K') && !self.eat(b'C') {
            self.undisambiguated()?;
        }
        while !self.eat(b'E') {
            self.kind(&mut Discard)?;
        }
        self.kind(&mut Discard)
    }

    /// The bounds of a trait object, which the output leaves out.
    fn dynamic(&mut self) -> Result<(), Failed> {
        if self.eat(b'G') {
            self.counted()?;
        }
        while !self.eat(b'E') {
            self.path(&mut Discard)?;
            while self.eat(b'p') {
                self.undisambiguated()?;
                self.kind(&mut Discard)?;
            }
        }
        self.lifetime()
    }

    /// A constant generic argument, and its value when that is a
    /// non-negative number the output can write.
    fn constant(&mut self) -> Result<Option<u64>, Failed> {
        self.nested(|parser| {
            if parser.eat(b'B') {
                let target = parser.backref()?;
                return parser.at_backref(target, Parser::constant);
            }
            if parser.eat(b'p') {
                return Ok(None);
            }
            parser.kind(&mut Discard)?;
            let negative = parser.eat(b'n');
            let mut value: Option<u64> = Some(0);
            while !parser.eat(b'_') {
                let byte = parser.next()?;
                let digit = char::from(byte).to_digit(16).ok_or(Failed)?;
                value = value
                    .and_then(|so_far| so_far.checked_mul(16))
                    .and_then(|scaled| scaled.checked_add(u64::from(digit)));
            }
            Ok(if negative { None } else { value })
        })
    }
}

/// The name of a basic type, for the letters that name one. The letters
/// are the ones RFC 2603 assigns; `w`, `g`, `k`, `q`, and `r` name none.
const fn basic(byte: u8) -> Option<&'static str> {
    Some(match byte {
        b'a' => "i8",
        b'b' => "bool",
        b'c' => "char",
        b'd' => "f64",
        b'e' => "str",
        b'f' => "f32",
        b'h' => "u8",
        b'i' => "isize",
        b'j' => "usize",
        b'l' => "i32",
        b'm' => "u32",
        b'n' => "i128",
        b'o' => "u128",
        b'p' => "_",
        b's' => "i16",
        b't' => "u16",
        b'u' => "()",
        b'v' => "...",
        b'x' => "i64",
        b'y' => "u64",
        b'z' => "!",
        _ => return None,
    })
}

/// What a namespace whose identifier is empty is called.
const fn special(namespace: u8) -> &'static str {
    match namespace {
        b'C' => "{closure}",
        b'S' => "{shim}",
        _ => "{unnamed}",
    }
}
