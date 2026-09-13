// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The parser: tokens in, a tree out.
//!
//! Recursive descent, with the expression grammar as a precedence climb
//! over the table `src/parse.y` declares. That table is the specification
//! — `%left OR`, `%left AND`, `%right NOT`, and so on down to `%right
//! BITNOT` — and the binding powers below are it, in the order it gives
//! them.
//!
//! The walk is bounded. A statement can nest as deeply as it likes on the
//! page, and a parser that follows it down without counting is a parser a
//! file can overflow the stack of, so [`MAX_DEPTH`] is where it stops.

use alloc::vec::Vec;

use crate::ast::{Arena, BinaryOp, CurrentTime, ExprId, LikeOp, Literal, Node, Span, UnaryOp};
use crate::keyword::Keyword;
use crate::token::{Kind, Lexer, Token};

/// How tall an expression may grow, and how deeply the parser may nest.
///
/// Both are bounded, and they are not the same thing. The parser's own
/// recursion is what brackets and prefix operators drive; the height of
/// the tree is what a chain of left-associative operators drives, and
/// `a+a+a+…` builds a tree as tall as it is long without the parser
/// recursing once. SQLite bounds the second as well, in
/// `sqlite3ExprCheckHeight`, and for the same reason: whatever walks the
/// tree next has a stack.
pub const MAX_DEPTH: u32 = 200;

/// What the parser wanted where it stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Expected {
    /// An expression, or the start of one.
    Expression,
    /// A name.
    Name,
    /// A type name, in a `CAST`.
    Type,
    /// `)`.
    CloseParen,
    /// `(`.
    OpenParen,
    /// `AS`, in a `CAST`.
    As,
    /// `THEN`, in a `CASE`.
    Then,
    /// `END`, in a `CASE`.
    End,
    /// `AND`, in a `BETWEEN`.
    And,
    /// The end of the statement.
    Eof,
    /// Nothing: the statement nests deeper than the parser walks.
    Depth,
}

/// Where the parser stopped, and what it wanted there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Error {
    /// Where in the statement.
    pub at: usize,
    /// How many bytes the token there is.
    pub len: usize,
    /// What was wanted.
    pub expected: Expected,
}

/// The parser: a walk over the tokens of one statement, and the arena the
/// tree is built in.
#[derive(Clone, Debug)]
pub struct Parser<'a> {
    /// The statement.
    sql: &'a [u8],
    /// What is left of it.
    lexer: Lexer<'a>,
    /// The next two tokens, once they have been looked at.
    ahead: [Option<Token>; 2],
    /// The tree so far.
    arena: Arena,
    /// How deep the walk stands.
    depth: u32,
}

impl<'a> Parser<'a> {
    /// A parser over `sql` that has read nothing.
    #[must_use]
    pub fn new(sql: &'a [u8]) -> Self {
        let mut parser = Parser {
            sql,
            lexer: Lexer::new(sql),
            ahead: [None, None],
            arena: Arena::new(),
            depth: 0,
        };
        parser.ahead = [parser.read(), parser.read()];
        parser
    }

    /// The tree.
    #[must_use]
    pub const fn arena(&self) -> &Arena {
        &self.arena
    }

    /// The tree, taken out of the parser.
    #[must_use]
    pub fn into_arena(self) -> Arena {
        self.arena
    }

    /// Reads one expression, and nothing after it.
    ///
    /// # Errors
    ///
    /// Where the tokens are not an expression, or where something follows
    /// the expression.
    pub fn only_expression(&mut self) -> Result<ExprId, Error> {
        let expr = self.expression()?;
        match self.peek() {
            None => Ok(expr),
            Some(token) => Err(self.error(Some(token), Expected::Eof)),
        }
    }

    /// Reads one expression.
    ///
    /// # Errors
    ///
    /// Where the tokens are not an expression.
    pub fn expression(&mut self) -> Result<ExprId, Error> {
        self.binary(0)
    }

    /// Reads an expression whose operators bind at least as tightly as
    /// `least`.
    fn binary(&mut self, least: u8) -> Result<ExprId, Error> {
        self.deeper()?;
        let mut left = self.prefix()?;
        while let Some(token) = self.peek() {
            let Some((power, what)) = Self::infix_of(token) else {
                break;
            };
            if power <= least {
                break;
            }
            left = self.infix(left, what, power)?;
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(left)
    }

    /// What an operator does, once its binding power has decided that it
    /// is read here at all.
    const fn infix_of(token: Token) -> Option<(u8, Infix)> {
        Some(match token.kind {
            Kind::Keyword(Keyword::Or) => (1, Infix::Plain(BinaryOp::Or)),
            Kind::Keyword(Keyword::And) => (2, Infix::Plain(BinaryOp::And)),
            Kind::Eq => (4, Infix::Plain(BinaryOp::Eq)),
            Kind::Ne => (4, Infix::Plain(BinaryOp::Ne)),
            Kind::Keyword(Keyword::Is) => (4, Infix::Is),
            Kind::Keyword(Keyword::Isnull) => (4, Infix::IsNull),
            Kind::Keyword(Keyword::Notnull) => (4, Infix::NotNull),
            Kind::Keyword(Keyword::Between) => (4, Infix::Between),
            Kind::Keyword(Keyword::In) => (4, Infix::In),
            Kind::Keyword(Keyword::Like) => (4, Infix::Like(LikeOp::Like)),
            Kind::Keyword(Keyword::Glob) => (4, Infix::Like(LikeOp::Glob)),
            Kind::Keyword(Keyword::Regexp) => (4, Infix::Like(LikeOp::Regexp)),
            Kind::Keyword(Keyword::Match) => (4, Infix::Like(LikeOp::Match)),
            // `NOT` after an expression begins `NOT LIKE`, `NOT IN`,
            // `NOT BETWEEN`, `NOT GLOB`, `NOT MATCH`, `NOT REGEXP` and
            // `NOT NULL`, and binds where those do.
            Kind::Keyword(Keyword::Not) => (4, Infix::Not),
            Kind::Lt => (5, Infix::Plain(BinaryOp::Lt)),
            Kind::Le => (5, Infix::Plain(BinaryOp::Le)),
            Kind::Gt => (5, Infix::Plain(BinaryOp::Gt)),
            Kind::Ge => (5, Infix::Plain(BinaryOp::Ge)),
            Kind::BitAnd => (7, Infix::Plain(BinaryOp::BitAnd)),
            Kind::BitOr => (7, Infix::Plain(BinaryOp::BitOr)),
            Kind::LShift => (7, Infix::Plain(BinaryOp::LShift)),
            Kind::RShift => (7, Infix::Plain(BinaryOp::RShift)),
            Kind::Plus => (8, Infix::Plain(BinaryOp::Add)),
            Kind::Minus => (8, Infix::Plain(BinaryOp::Subtract)),
            Kind::Star => (9, Infix::Plain(BinaryOp::Multiply)),
            Kind::Slash => (9, Infix::Plain(BinaryOp::Divide)),
            Kind::Rem => (9, Infix::Plain(BinaryOp::Modulo)),
            Kind::Concat => (10, Infix::Plain(BinaryOp::Concat)),
            Kind::Ptr => (10, Infix::Plain(BinaryOp::Extract)),
            Kind::PtrPtr => (10, Infix::Plain(BinaryOp::ExtractText)),
            Kind::Keyword(Keyword::Collate) => (11, Infix::Collate),
            _ => return None,
        })
    }

    /// Reads the rest of an operator whose left operand is `left`.
    fn infix(&mut self, left: ExprId, what: Infix, power: u8) -> Result<ExprId, Error> {
        self.bump();
        match what {
            Infix::Plain(op) => {
                let right = self.binary(power)?;
                self.node(Node::Binary { op, left, right })
            }
            Infix::Not => self.negated(left),
            Infix::IsNull => self.unary(UnaryOp::IsNull, left),
            Infix::NotNull => self.unary(UnaryOp::NotNull, left),
            Infix::Collate => {
                let name = self.name()?;
                self.node(Node::Collate { value: left, name })
            }
            Infix::Between => self.between(left, false),
            Infix::In => self.in_list(left, false),
            Infix::Like(op) => self.like(left, op, false),
            Infix::Is => self.is(left, power),
        }
    }

    /// The operators `NOT` can precede, and `NOT NULL`.
    fn negated(&mut self, left: ExprId) -> Result<ExprId, Error> {
        let next = self.peek();
        match next.map(|token| token.kind) {
            Some(Kind::Keyword(Keyword::Null)) => {
                self.bump();
                self.unary(UnaryOp::NotNull, left)
            }
            Some(Kind::Keyword(Keyword::Between)) => {
                self.bump();
                self.between(left, true)
            }
            Some(Kind::Keyword(Keyword::In)) => {
                self.bump();
                self.in_list(left, true)
            }
            Some(Kind::Keyword(Keyword::Like)) => {
                self.bump();
                self.like(left, LikeOp::Like, true)
            }
            Some(Kind::Keyword(Keyword::Glob)) => {
                self.bump();
                self.like(left, LikeOp::Glob, true)
            }
            Some(Kind::Keyword(Keyword::Regexp)) => {
                self.bump();
                self.like(left, LikeOp::Regexp, true)
            }
            Some(Kind::Keyword(Keyword::Match)) => {
                self.bump();
                self.like(left, LikeOp::Match, true)
            }
            _ => {
                // `x NOT y` is not an operator; the grammar has no such
                // rule, so this is where the statement stops making sense.
                Err(self.error(next, Expected::Expression))
            }
        }
    }

    /// `IS`, `IS NOT`, `IS DISTINCT FROM` and `IS NOT DISTINCT FROM`.
    fn is(&mut self, left: ExprId, power: u8) -> Result<ExprId, Error> {
        let mut negated = false;
        if self.eat_keyword(Keyword::Not) {
            negated = true;
        }
        if self.eat_keyword(Keyword::Distinct) {
            self.expect_keyword(Keyword::From, Expected::Expression)?;
            // `IS DISTINCT FROM` is `IS NOT`, the other way round.
            negated = !negated;
        }
        let right = self.binary(power)?;
        let op = if negated {
            BinaryOp::IsNot
        } else {
            BinaryOp::Is
        };
        self.node(Node::Binary { op, left, right })
    }

    /// `x BETWEEN low AND high`, where the bounds bind tighter than `AND`.
    fn between(&mut self, value: ExprId, negated: bool) -> Result<ExprId, Error> {
        let low = self.binary(4)?;
        self.expect_keyword(Keyword::And, Expected::And)?;
        let high = self.binary(4)?;
        self.node(Node::Between {
            value,
            low,
            high,
            negated,
        })
    }

    /// `x IN (a, b)`, and the empty list the grammar allows.
    fn in_list(&mut self, value: ExprId, negated: bool) -> Result<ExprId, Error> {
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let mut items = Vec::new();
        if !self.at(Kind::Rp) {
            loop {
                items.push(self.expression()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
        }
        self.expect(Kind::Rp, Expected::CloseParen)?;
        let list = self.arena.push_children(&items);
        self.node(Node::InList {
            value,
            list,
            negated,
        })
    }

    /// `x LIKE pattern ESCAPE escape`, and the three that work like it.
    fn like(&mut self, value: ExprId, op: LikeOp, negated: bool) -> Result<ExprId, Error> {
        let pattern = self.binary(4)?;
        let escape = if self.eat_keyword(Keyword::Escape) {
            Some(self.binary(6)?)
        } else {
            None
        };
        self.node(Node::Like {
            op,
            value,
            pattern,
            escape,
            negated,
        })
    }

    /// An expression with nothing to its left: a constant, a name, a
    /// prefix operator, or something in brackets.
    fn prefix(&mut self) -> Result<ExprId, Error> {
        let Some(token) = self.peek() else {
            return Err(self.error(None, Expected::Expression));
        };
        match token.kind {
            Kind::Keyword(Keyword::Not) => {
                self.bump();
                let operand = self.binary(3)?;
                self.unary(UnaryOp::Not, operand)
            }
            Kind::BitNot => {
                self.bump();
                let operand = self.binary(11)?;
                self.unary(UnaryOp::BitNot, operand)
            }
            Kind::Minus => {
                self.bump();
                let operand = self.binary(11)?;
                self.unary(UnaryOp::Negate, operand)
            }
            Kind::Plus => {
                self.bump();
                let operand = self.binary(11)?;
                self.unary(UnaryOp::Identity, operand)
            }
            Kind::Keyword(Keyword::Cast) => self.cast(),
            Kind::Keyword(Keyword::Case) => self.case(),
            Kind::Lp => self.parenthesized(),
            Kind::Integer | Kind::QNumber => {
                self.bump();
                self.literal(Literal::Integer(Span::of(token)))
            }
            Kind::Float => {
                self.bump();
                self.literal(Literal::Float(Span::of(token)))
            }
            Kind::String => {
                self.bump();
                self.literal(Literal::Text(Span::of(token)))
            }
            Kind::Blob => {
                self.bump();
                self.literal(Literal::Blob(Span::of(token)))
            }
            Kind::Variable => {
                self.bump();
                self.node(Node::Variable(Span::of(token)))
            }
            Kind::Keyword(Keyword::Null) => {
                self.bump();
                self.literal(Literal::Null)
            }
            Kind::Keyword(Keyword::CurrentTime) => {
                self.bump();
                self.literal(Literal::CurrentTime(CurrentTime::Time))
            }
            Kind::Keyword(Keyword::CurrentDate) => {
                self.bump();
                self.literal(Literal::CurrentTime(CurrentTime::Date))
            }
            Kind::Keyword(Keyword::CurrentTimestamp) => {
                self.bump();
                self.literal(Literal::CurrentTime(CurrentTime::Timestamp))
            }
            _ => self.named(),
        }
    }

    /// A name: a column, or a call.
    fn named(&mut self) -> Result<ExprId, Error> {
        let first = self.name()?;
        if self.at(Kind::Lp) {
            return self.call(first);
        }
        if !self.eat(Kind::Dot) {
            return self.node(Node::Column {
                schema: None,
                table: None,
                column: first,
            });
        }
        let second = self.name()?;
        if !self.eat(Kind::Dot) {
            return self.node(Node::Column {
                schema: None,
                table: Some(first),
                column: second,
            });
        }
        let third = self.name()?;
        self.node(Node::Column {
            schema: Some(first),
            table: Some(second),
            column: third,
        })
    }

    /// `name(args)`, `name(*)` and `name(DISTINCT args)`.
    fn call(&mut self, name: Span) -> Result<ExprId, Error> {
        // The caller looked at the bracket before it called this, so it is
        // taken rather than asked for.
        self.bump();
        let mut star = false;
        let mut distinct = false;
        let mut args = Vec::new();
        if self.eat(Kind::Star) {
            star = true;
        } else if !self.at(Kind::Rp) {
            distinct = self.eat_keyword(Keyword::Distinct);
            if self.eat_keyword(Keyword::All) {
                // `ALL` is the default and means nothing.
            }
            loop {
                args.push(self.expression()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
        }
        self.expect(Kind::Rp, Expected::CloseParen)?;
        let args = self.arena.push_children(&args);
        self.node(Node::Call {
            name,
            args,
            distinct,
            star,
        })
    }

    /// `CAST(x AS type)`.
    fn cast(&mut self) -> Result<ExprId, Error> {
        self.bump();
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let value = self.expression()?;
        self.expect_keyword(Keyword::As, Expected::As)?;
        let ty = self.type_name()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        self.node(Node::Cast { value, ty })
    }

    /// `CASE x WHEN a THEN b ELSE c END`, with or without the `x` and the
    /// `ELSE`.
    fn case(&mut self) -> Result<ExprId, Error> {
        self.bump();
        let operand = if self.at_keyword(Keyword::When) {
            None
        } else {
            Some(self.expression()?)
        };
        let mut branches = Vec::new();
        while self.eat_keyword(Keyword::When) {
            let condition = self.expression()?;
            self.expect_keyword(Keyword::Then, Expected::Then)?;
            let result = self.expression()?;
            branches.push(condition);
            branches.push(result);
        }
        if branches.is_empty() {
            return Err(self.error(self.peek(), Expected::Expression));
        }
        let otherwise = if self.eat_keyword(Keyword::Else) {
            Some(self.expression()?)
        } else {
            None
        };
        self.expect_keyword(Keyword::End, Expected::End)?;
        let branches = self.arena.push_children(&branches);
        self.node(Node::Case {
            operand,
            branches,
            otherwise,
        })
    }

    /// `(x)`, which is the expression, and `(x, y)`, which is a row.
    fn parenthesized(&mut self) -> Result<ExprId, Error> {
        self.bump();
        let first = self.expression()?;
        if !self.at(Kind::Comma) {
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return Ok(first);
        }
        let mut items = Vec::new();
        items.push(first);
        while self.eat(Kind::Comma) {
            items.push(self.expression()?);
        }
        self.expect(Kind::Rp, Expected::CloseParen)?;
        let row = self.arena.push_children(&items);
        self.node(Node::Row(row))
    }

    /// A name: an identifier, a keyword that may be one, or a string,
    /// which SQLite allows where a name is expected.
    fn name(&mut self) -> Result<Span, Error> {
        let token = self.peek();
        match token.map(|found| found.kind) {
            Some(Kind::Id | Kind::String) => {
                self.bump();
                Ok(Span::of(token.unwrap_or(EMPTY)))
            }
            Some(Kind::Keyword(keyword)) if keyword.can_be_name() => {
                self.bump();
                Ok(Span::of(token.unwrap_or(EMPTY)))
            }
            _ => Err(self.error(token, Expected::Name)),
        }
    }

    /// The type in a `CAST`, which is one or more names and may carry
    /// lengths in brackets.
    fn type_name(&mut self) -> Result<Span, Error> {
        // `typetoken ::= .` — the grammar allows no type at all, and
        // `CAST('1' AS)` is a cast to nothing, which SQLite reads as a
        // cast that converts nothing.
        if let Some(token) = self.peek()
            && token.kind == Kind::Rp
        {
            return Ok(Span {
                start: token.start,
                len: 0,
            });
        }
        let first = self.name()?;
        let mut span = first;
        while let Some(token) = self.peek() {
            match token.kind {
                Kind::Id | Kind::String => {
                    self.bump();
                    span = join(span, Span::of(token));
                }
                Kind::Keyword(keyword) if keyword.can_be_name() => {
                    self.bump();
                    span = join(span, Span::of(token));
                }
                Kind::Lp => {
                    self.bump();
                    loop {
                        let Some(inner) = self.peek() else {
                            return Err(self.error(None, Expected::CloseParen));
                        };
                        match inner.kind {
                            Kind::Rp => break,
                            Kind::Integer
                            | Kind::Float
                            | Kind::Comma
                            | Kind::Plus
                            | Kind::Minus => {
                                self.bump();
                            }
                            _ => return Err(self.error(Some(inner), Expected::Type)),
                        }
                    }
                    // The loop above ended on the closing bracket.
                    let close = self.peek().unwrap_or(EMPTY);
                    self.bump();
                    span = join(span, Span::of(close));
                    break;
                }
                _ => break,
            }
        }
        Ok(span)
    }

    /// Adds a node for a constant.
    fn literal(&mut self, literal: Literal) -> Result<ExprId, Error> {
        self.node(Node::Literal(literal))
    }

    /// Adds a node, and refuses a tree that has grown too tall.
    fn node(&mut self, node: Node) -> Result<ExprId, Error> {
        let id = self.arena.push(node);
        if self.arena.height(id) > MAX_DEPTH {
            return Err(Error {
                at: self.peek().map_or(self.sql.len(), |token| token.start),
                len: 0,
                expected: Expected::Depth,
            });
        }
        Ok(id)
    }

    /// Adds a node for an operator with one operand.
    fn unary(&mut self, op: UnaryOp, operand: ExprId) -> Result<ExprId, Error> {
        self.node(Node::Unary { op, operand })
    }

    /// Counts one level of nesting, and refuses one too many.
    fn deeper(&mut self) -> Result<(), Error> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_DEPTH {
            return Err(Error {
                at: self.peek().map_or(self.sql.len(), |token| token.start),
                len: 0,
                expected: Expected::Depth,
            });
        }
        Ok(())
    }

    /// The next token that means something.
    const fn peek(&self) -> Option<Token> {
        self.ahead.first().copied().flatten()
    }

    /// Whether the next token is of this kind.
    fn at(&self, kind: Kind) -> bool {
        self.peek().is_some_and(|token| token.kind == kind)
    }

    /// Whether the next token is this keyword.
    fn at_keyword(&self, keyword: Keyword) -> bool {
        self.at(Kind::Keyword(keyword))
    }

    /// Takes the next token.
    fn bump(&mut self) -> Option<Token> {
        let token = self.peek();
        self.ahead = [self.ahead.get(1).copied().flatten(), self.read()];
        token
    }

    /// Takes the next token if it is of this kind.
    fn eat(&mut self, kind: Kind) -> bool {
        if self.at(kind) {
            self.bump();
            return true;
        }
        false
    }

    /// Takes the next token if it is this keyword.
    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        self.eat(Kind::Keyword(keyword))
    }

    /// Takes the next token, which has to be of this kind.
    fn expect(&mut self, kind: Kind, expected: Expected) -> Result<Token, Error> {
        if let Some(token) = self.peek()
            && token.kind == kind
        {
            self.bump();
            return Ok(token);
        }
        Err(self.error(self.peek(), expected))
    }

    /// Takes the next token, which has to be this keyword.
    fn expect_keyword(&mut self, keyword: Keyword, expected: Expected) -> Result<Token, Error> {
        self.expect(Kind::Keyword(keyword), expected)
    }

    /// The next token that is not whitespace or a comment.
    fn read(&mut self) -> Option<Token> {
        self.lexer.find(|token| !token.kind.is_trivia())
    }

    /// A refusal at `token`, or at the end of the statement.
    fn error(&self, token: Option<Token>, expected: Expected) -> Error {
        Error {
            at: token.map_or(self.sql.len(), |found| found.start),
            len: token.map_or(0, |found| found.len),
            expected,
        }
    }
}

/// What an infix operator turns out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Infix {
    /// An operator with a left and a right and nothing else.
    Plain(BinaryOp),
    /// `NOT`, which begins one of the negated forms.
    Not,
    /// `ISNULL`.
    IsNull,
    /// `NOTNULL`.
    NotNull,
    /// `COLLATE`.
    Collate,
    /// `BETWEEN`.
    Between,
    /// `IN`.
    In,
    /// `LIKE` and the three that work like it.
    Like(LikeOp),
    /// `IS`, and the three longer forms of it.
    Is,
}

/// A token that is nowhere and nothing, for the two places a peek that has
/// already been made cannot fail.
const EMPTY: Token = Token {
    kind: Kind::Illegal,
    start: 0,
    len: 0,
};

/// The span that covers both.
const fn join(left: Span, right: Span) -> Span {
    let end = right.start.saturating_add(right.len);
    Span {
        start: left.start,
        len: end.saturating_sub(left.start),
    }
}

/// Reads one expression out of `sql` and answers the tree it built.
///
/// # Errors
///
/// Where the statement is not one expression.
pub fn expression(sql: &[u8]) -> Result<(Arena, ExprId), Error> {
    let mut parser = Parser::new(sql);
    let root = parser.only_expression()?;
    Ok((parser.into_arena(), root))
}
