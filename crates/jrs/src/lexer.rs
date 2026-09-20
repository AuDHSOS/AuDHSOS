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
    /// An `IdentifierName` of 12.7 a `\ UnicodeEscapeSequence` wrote whose
    /// `StringValue` is a reserved word of 12.7.2 or one of the words 13.1
    /// gives a meaning in place.
    ///
    /// 12.7.2 defines a reserved word as a literal sequence of source
    /// characters, so such a name is no keyword and stands only where an
    /// `IdentifierName` stands.
    EscapedWord(String),
    /// `PrivateIdentifier` of 12.7, whose name carries the `#`.
    Private(String),
    Literal(Value),
    /// The digits of a `BigInt` literal of 12.9.3, with the radix they stand in.
    BigInt(String, u32),
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
            Kind::Literal(_)
            | Kind::BigInt(_, _)
            | Kind::Regex(_, _)
            | Kind::Private(_)
            | Kind::Punct("]" | "++" | "--" | "}" | ".") => {
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
    const fn unsupported(feature: &'static str) -> Error {
        Error::Unsupported { feature }
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
        // 12.7: a `#` and the identifier behind it are one token.
        if ch == '#' && self.rest().chars().nth(1).is_some_and(identifier_start) {
            let start = self.at;
            self.bump();
            self.bump();
            while self.peek().is_some_and(identifier_part) {
                self.bump();
            }
            let word = self.source.get(start..self.at).unwrap_or_default();
            return Ok(Kind::Private(String::from(word)));
        }
        if identifier_start(ch) || (ch == '\\' && self.rest().chars().nth(1) == Some('u')) {
            return self.identifier();
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
        // 12.7: a `#` before a name this lexer does not take is the gap of
        // that name; a `#` before anything else is no name at all.
        let private_gap = ch == '#' && Self::names_a_gap(self.rest().get(1..).unwrap_or_default());
        if private_gap || ch == '\\' || !ch.is_ascii() {
            Err(Self::unsupported("Unicode identifiers"))
        } else {
            Err(self.error("invalid source character"))
        }
    }

    /// Whether the text behind a `#` is a name this lexer does not take,
    /// which 12.7 allows and the gap of Unicode identifiers covers.
    ///
    /// A `\u` escape names the code point it stands for, so an escape of a
    /// character that starts no identifier is no name at all.
    fn names_a_gap(rest: &str) -> bool {
        let mut chars = rest.chars();
        match chars.next() {
            Some('\\') => {}
            Some(ch) => return !ch.is_ascii(),
            None => return false,
        }
        if chars.next() != Some('u') {
            return false;
        }
        let tail: String = chars.collect();
        let digits = if let Some(braced) = tail.strip_prefix('{') {
            braced.split_once('}').map(|(digits, _)| digits)
        } else {
            tail.get(..4)
        };
        digits
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .and_then(char::from_u32)
            // A zero-width joiner is a part of a name and starts none, which
            // 12.7.1 tells apart without the tables the rest needs.
            .is_some_and(|ch| {
                (identifier_start(ch) || !ch.is_ascii())
                    && ch != '\u{200c}'
                    && ch != '\u{200d}'
            })
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
                if self.peek() == Some('n') {
                    self.bump();
                    self.number_end()?;
                    return Ok(Kind::BigInt(digits.replace('_', ""), radix));
                }
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
            return Err(Self::unsupported("legacy numeric literals"));
        }
        let mut integer = true;
        if self.peek() == Some('.') {
            integer = false;
            self.bump();
            text.push('.');
            text.push_str(&self.digits(10)?);
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            integer = false;
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
        if integer && self.peek() == Some('n') {
            self.bump();
            self.number_end()?;
            return Ok(Kind::BigInt(text.replace('_', ""), 10));
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

    /// `IdentifierName` of 12.7.1, whose characters a `\u` escape may name.
    ///
    /// 12.7.1 reads the escape as the character it stands for, so a reserved
    /// word written with one is that word and no name; this lexer answers the
    /// gap for it rather than the word.
    fn identifier(&mut self) -> Result<Kind, Error> {
        let mut text = String::new();
        let mut escaped = false;
        loop {
            let first = text.is_empty();
            match self.peek() {
                Some('\\') => {
                    self.bump();
                    if self.bump() != Some('u') {
                        return Err(self.error("invalid escape in an identifier"));
                    }
                    escaped = true;
                    let code = if self.peek() == Some('{') {
                        self.bump();
                        let mut code = 0u32;
                        while self.peek() != Some('}') {
                            code = code
                                .checked_mul(16)
                                .and_then(|value| value.checked_add(self.hex(1).ok()?))
                                .filter(|value| *value <= 0x10_ffff)
                                .ok_or_else(|| self.error("invalid Unicode escape"))?;
                        }
                        self.bump();
                        code
                    } else {
                        self.hex(4)?
                    };
                    let ch =
                        char::from_u32(code).ok_or_else(|| self.error("invalid Unicode escape"))?;
                    if !(if first {
                        identifier_start(ch)
                    } else {
                        identifier_part(ch)
                    }) {
                        return Err(self.error("an escape that is no part of a name"));
                    }
                    text.push(ch);
                }
                Some(ch) if (first && identifier_start(ch)) || (!first && identifier_part(ch)) => {
                    self.bump();
                    text.push(ch);
                }
                _ => break,
            }
        }
        if escaped {
            // 12.7.2: a reserved word an escape writes is that word, which
            // stands where no name may.
            // 12.7.2 and the contextual words of 13.1: an escape writes the
            // word, and the word stands where this lexer answers a name.
            if reserved_word(&text)
                || matches!(
                    text.as_str(),
                    "async"
                        | "get"
                        | "set"
                        | "of"
                        | "as"
                        | "from"
                        | "target"
                        | "accessor"
                        | "let"
                        | "static"
                        | "true"
                        | "false"
                        | "null"
                )
            {
                return Ok(Kind::EscapedWord(text));
            }
            return Ok(Kind::Word(text));
        }
        Ok(match text.as_str() {
            "true" => Kind::Literal(Value::Boolean(true)),
            "false" => Kind::Literal(Value::Boolean(false)),
            "null" => Kind::Literal(Value::Null),
            _ => Kind::Word(text),
        })
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
                    '0'..='9' => {
                        return Err(Self::unsupported("legacy numeric string escapes"));
                    }
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
/// Whether the text is one of the words 12.7.2 reserves, which no name is.
fn reserved_word(text: &str) -> bool {
    matches!(
        text,
        "await"
            | "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "implements"
            | "interface"
            | "package"
            | "private"
            | "protected"
            | "public"
    )
}

/// `IdentifierStartChar` of 12.7.1, which is `ID_Start` beside `$` and `_`.
///
/// Every character a Unicode letter is, is one; the categories `Nl` and
/// `Other_ID_Start` that `ID_Start` adds to them carry no character this
/// lexer answers for.
fn identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic()
        || matches!(ch, '_' | '$')
        || (!ch.is_ascii() && ch.is_alphabetic())
        // `Other_ID_Start`, which names six characters no category of letters
        // carries.
        || matches!(
            ch,
            '\u{1885}' | '\u{1886}' | '\u{2118}' | '\u{212e}' | '\u{309b}' | '\u{309c}'
        )
}

/// `IdentifierPartChar` of 12.7.1, which is `ID_Continue` beside `$`, the
/// zero-width non-joiner and the zero-width joiner.
fn identifier_part(ch: char) -> bool {
    identifier_start(ch)
        || ch.is_ascii_digit()
        || matches!(ch, '\u{200c}' | '\u{200d}')
        || (!ch.is_ascii() && ch.is_alphanumeric())
        // `Other_ID_Continue`, which names five more.
        || matches!(ch, '\u{b7}' | '\u{387}' | '\u{1369}'..='\u{1371}' | '\u{19da}')
}
