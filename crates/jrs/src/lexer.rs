// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounded UTF-8 scanning; offsets always lie on character boundaries.
//! ECMA-262 12.2–12.4, 12.7, 12.9.3, 12.9.4 and 12.10.

use crate::{
    Error, Limits, Value,
    value::{line_terminator, radix_number, whitespace},
};
use alloc::{string::String, vec::Vec};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    Word(String),
    Literal(Value),
    Regex(String, String),
    Template {
        value: Value,
        tail: bool,
        head: bool,
    },
    Punct(&'static str),
    End,
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) kind: Kind,
    pub(crate) offset: usize,
    pub(crate) end: usize,
    pub(crate) newline: bool,
    pub(crate) use_strict: bool,
}

#[expect(
    clippy::too_many_lines,
    reason = "lexer context tracks regexp and template boundaries with exact token spans"
)]
pub(crate) fn lex(source: &str, limits: Limits) -> Result<Vec<Token>, Error> {
    if source.len() > limits.source_bytes {
        return Err(Error::Limit {
            resource: "source bytes",
        });
    }
    let mut lexer = Lexer {
        source,
        at: 0,
        limits,
    };
    let mut tokens = Vec::new();
    let mut expression = true;
    let mut control = false;
    let mut parentheses = Vec::new();
    let mut templates: Vec<usize> = Vec::new();
    loop {
        if tokens.len() >= limits.tokens {
            return Err(Error::Limit { resource: "tokens" });
        }
        let newline = lexer.skip()?;
        let offset = lexer.at;
        let resuming = lexer.peek() == Some('}') && templates.last() == Some(&0);
        let mut kind = if resuming {
            lexer.string('`')?
        } else if expression && lexer.peek() == Some('/') {
            lexer.regex()?
        } else {
            lexer.token()?
        };
        if let Kind::Template { tail, head, .. } = &mut kind {
            if resuming {
                *head = false;
                if *tail {
                    templates.pop();
                }
            } else if !*tail {
                if templates.len() >= limits.nesting.min(48) {
                    return Err(Error::Limit {
                        resource: "template nesting",
                    });
                }
                templates.push(0);
            }
        } else if let Some(depth) = templates.last_mut() {
            match kind {
                Kind::Punct("{") => *depth = depth.saturating_add(1),
                Kind::Punct("}") => *depth = depth.saturating_sub(1),
                Kind::End => return Err(lexer.error("unterminated template substitution")),
                _ => {}
            }
        }
        match &kind {
            Kind::Word(word) => {
                control = matches!(
                    word.as_str(),
                    "if" | "while" | "for" | "with" | "switch" | "catch"
                );
                expression = control
                    || matches!(
                        word.as_str(),
                        "return"
                            | "throw"
                            | "await"
                            | "case"
                            | "delete"
                            | "void"
                            | "typeof"
                            | "new"
                            | "in"
                            | "instanceof"
                            | "else"
                            | "do"
                    );
            }
            Kind::Punct("(") => {
                parentheses.push(control);
                control = false;
                expression = true;
            }
            Kind::Punct(")") => expression = parentheses.pop().unwrap_or(false),
            Kind::Template { tail, .. } => expression = !tail,
            Kind::Literal(_) | Kind::Regex(_, _) | Kind::Punct("]" | "++" | "--" | "}" | ".") => {
                expression = false;
            }
            _ => expression = true,
        }
        let end = kind == Kind::End;
        tokens.push(Token {
            use_strict: matches!(
                source.get(offset..lexer.at),
                Some("\"use strict\"" | "'use strict'")
            ),
            kind,
            offset,
            end: lexer.at,
            newline,
        });
        if end {
            return Ok(tokens);
        }
    }
}

struct Lexer<'a> {
    source: &'a str,
    at: usize,
    limits: Limits,
}

impl Lexer<'_> {
    fn regex(&mut self) -> Result<Kind, Error> {
        self.bump();
        let start = self.at;
        let mut class = false;
        loop {
            let before = self.at;
            let ch = self
                .bump()
                .ok_or_else(|| self.error("unterminated regular expression literal"))?;
            if line_terminator(ch) {
                return Err(self.error("line terminator in regular expression"));
            }
            match ch {
                '\\' => {
                    let escaped = self
                        .bump()
                        .ok_or_else(|| self.error("unterminated regular expression escape"))?;
                    if line_terminator(escaped) {
                        return Err(self.error("line terminator in regular expression escape"));
                    }
                }
                '[' => class = true,
                ']' => class = false,
                '/' if !class => {
                    let pattern = String::from(self.source.get(start..before).unwrap_or_default());
                    let flags_start = self.at;
                    while self.peek().is_some_and(identifier_part) {
                        self.bump();
                    }
                    return Ok(Kind::Regex(
                        pattern,
                        String::from(self.source.get(flags_start..self.at).unwrap_or_default()),
                    ));
                }
                _ => {}
            }
        }
    }
    fn rest(&self) -> &str {
        self.source.get(self.at..).unwrap_or_default()
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.at = self.at.saturating_add(ch.len_utf8());
        Some(ch)
    }
    const fn error(&self, message: &'static str) -> Error {
        Error::Syntax {
            offset: self.at,
            message,
        }
    }
    fn skip(&mut self) -> Result<bool, Error> {
        let mut newline = false;
        loop {
            while let Some(ch) = self
                .peek()
                .filter(|c| whitespace(*c) || line_terminator(*c))
            {
                newline |= line_terminator(ch);
                self.bump();
            }
            if self.rest().starts_with("//") || (self.at == 0 && self.rest().starts_with("#!")) {
                while self.peek().is_some_and(|ch| !line_terminator(ch)) {
                    self.bump();
                }
            } else if self.rest().starts_with("/*") {
                self.at = self.at.saturating_add(2);
                while !self.rest().starts_with("*/") {
                    let ch = self
                        .bump()
                        .ok_or_else(|| self.error("unterminated comment"))?;
                    newline |= line_terminator(ch);
                }
                self.at = self.at.saturating_add(2);
            } else {
                return Ok(newline);
            }
        }
    }

    fn token(&mut self) -> Result<Kind, Error> {
        let Some(ch) = self.peek() else {
            return Ok(Kind::End);
        };
        if ch == '\'' || ch == '"' || ch == '`' {
            return self.string(ch);
        }
        if ch.is_ascii_digit()
            || (ch == '.'
                && self
                    .rest()
                    .chars()
                    .nth(1)
                    .is_some_and(|c| c.is_ascii_digit()))
        {
            return self.number();
        }
        if identifier_start(ch) {
            let start = self.at;
            self.bump();
            while self.peek().is_some_and(identifier_part) {
                self.bump();
            }
            let word = self.source.get(start..self.at).unwrap_or_default();
            return Ok(match word {
                "true" => Kind::Literal(Value::Boolean(true)),
                "false" => Kind::Literal(Value::Boolean(false)),
                "null" => Kind::Literal(Value::Null),
                _ => Kind::Word(String::from(word)),
            });
        }
        // Longest match. Unsupported operators remain whole tokens and cannot
        // accidentally be interpreted as a sequence of supported operators.
        for punct in [
            ">>>=", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "...", "=>",
            "==", "!=", "<=", ">=", "++", "--", "+=", "-=", "*=", "/=", "%=", "&&", "||", "??",
            "**", "<<", ">>", "&=", "|=", "^=", "?.", "+", "-", "*", "/", "%", "<", ">", "=", "!",
            "~", "&", "|", "^", "(", ")", "{", "}", "[", "]", ";", ",", ".", "?", ":",
        ] {
            if self.rest().starts_with(punct) {
                self.at = self.at.saturating_add(punct.len());
                return Ok(Kind::Punct(punct));
            }
        }
        Err(self.error("unsupported or invalid source character"))
    }

    fn digits(&mut self, radix: u32) -> Result<String, Error> {
        let mut text = String::new();
        while let Some(ch) = self.peek().filter(|c| c.is_digit(radix)) {
            text.push(ch);
            self.bump();
            if self.peek() == Some('_') {
                self.bump();
                if !self.peek().is_some_and(|c| c.is_digit(radix)) {
                    return Err(self.error("numeric separator must separate digits"));
                }
            }
        }
        Ok(text)
    }

    fn number(&mut self) -> Result<Kind, Error> {
        for (prefix, radix) in [
            ("0x", 16),
            ("0X", 16),
            ("0b", 2),
            ("0B", 2),
            ("0o", 8),
            ("0O", 8),
        ] {
            if self.rest().starts_with(prefix) {
                self.at = self.at.saturating_add(2);
                let digits = self.digits(radix)?;
                let value = radix_number(&digits, radix)
                    .ok_or_else(|| self.error("expected radix digits"))?;
                self.number_end()?;
                return Ok(Kind::Literal(Value::Number(value)));
            }
        }
        let start = self.at;
        let mut text = self.digits(10)?;
        if text.starts_with('0')
            && (text.len() > 1
                || self
                    .source
                    .get(start..self.at)
                    .is_some_and(|s| s.contains('_')))
        {
            return Err(self.error("legacy leading-zero numeric literals are not supported"));
        }
        if self.peek() == Some('.') {
            self.bump();
            text.push('.');
            text.push_str(&self.digits(10)?);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            text.push('e');
            if let Some(sign @ ('+' | '-')) = self.peek() {
                text.push(sign);
                self.bump();
            }
            let digits = self.digits(10)?;
            if digits.is_empty() {
                return Err(self.error("missing exponent digits"));
            }
            text.push_str(&digits);
        }
        self.number_end()?;
        text.parse::<f64>()
            .map(|n| Kind::Literal(Value::Number(n)))
            .map_err(|_| self.error("invalid numeric literal"))
    }

    fn number_end(&self) -> Result<(), Error> {
        if self.peek().is_some_and(|c| identifier_part(c) || c == '\\') {
            Err(self.error("invalid character after numeric literal"))
        } else {
            Ok(())
        }
    }

    fn hex(&mut self, count: usize) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..count {
            let digit = self
                .bump()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| self.error("invalid hexadecimal escape"))?;
            value = value
                .checked_mul(16)
                .and_then(|v| v.checked_add(digit))
                .ok_or_else(|| self.error("escape is out of range"))?;
        }
        Ok(value)
    }

    fn string(&mut self, quote: char) -> Result<Kind, Error> {
        self.bump();
        let mut units = Vec::new();
        loop {
            let mut ch = self
                .bump()
                .ok_or_else(|| self.error("unterminated string"))?;
            if ch == quote {
                let value = Value::String(units.into());
                return Ok(if quote == '`' {
                    Kind::Template {
                        value,
                        tail: true,
                        head: true,
                    }
                } else {
                    Kind::Literal(value)
                });
            }
            if quote == '`' && ch == '$' && self.peek() == Some('{') {
                self.bump();
                return Ok(Kind::Template {
                    value: Value::String(units.into()),
                    tail: false,
                    head: true,
                });
            }
            if quote == '`' && ch == '\r' {
                if self.peek() == Some('\n') {
                    self.bump();
                }
                ch = '\n';
            }
            if quote != '`' && matches!(ch, '\n' | '\r') {
                return Err(self.error("line break in string"));
            }
            if ch == '\\' {
                ch = self
                    .bump()
                    .ok_or_else(|| self.error("unterminated escape"))?;
                if line_terminator(ch) {
                    if ch == '\r' && self.peek() == Some('\n') {
                        self.bump();
                    }
                    continue;
                }
                ch = match ch {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'v' => '\u{b}',
                    '0' if !self.peek().is_some_and(|c| c.is_ascii_digit()) => '\0',
                    '0'..='9' => return Err(self.error("legacy numeric escape is not supported")),
                    'u' | 'x' => {
                        let code = if ch == 'u' && self.peek() == Some('{') {
                            self.bump();
                            let mut code = 0u32;
                            let mut count = 0usize;
                            while self.peek() != Some('}') {
                                code = code
                                    .checked_mul(16)
                                    .and_then(|v| v.checked_add(self.hex(1).ok()?))
                                    .filter(|v| *v <= 0x10_ffff)
                                    .ok_or_else(|| self.error("invalid Unicode escape"))?;
                                count = count.saturating_add(1);
                            }
                            if count == 0 {
                                return Err(self.error("empty Unicode escape"));
                            }
                            self.bump();
                            code
                        } else {
                            self.hex(if ch == 'u' { 4 } else { 2 })?
                        };
                        if let Ok(unit) = u16::try_from(code) {
                            units.push(unit);
                            self.check_string(units.len())?;
                            continue;
                        }
                        char::from_u32(code)
                            .ok_or_else(|| self.error("Unicode escape out of range"))?
                    }
                    other => other,
                };
            }
            let mut pair = [0; 2];
            units.extend_from_slice(ch.encode_utf16(&mut pair));
            self.check_string(units.len())?;
        }
    }

    const fn check_string(&self, count: usize) -> Result<(), Error> {
        if count > self.limits.string_units {
            Err(Error::Limit {
                resource: "string units",
            })
        } else {
            Ok(())
        }
    }
}

// Full Unicode ID_Start/ID_Continue and escaped identifiers are deliberately
// not approximated by Rust's alphabetic predicate. They are not supported yet.
const fn identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '_' | '$')
}
const fn identifier_part(ch: char) -> bool {
    identifier_start(ch) || ch.is_ascii_digit()
}
