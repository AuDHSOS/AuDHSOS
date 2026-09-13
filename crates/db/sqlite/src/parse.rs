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

use crate::ast::{
    Arena, BinaryOp, Compound, Cte, CurrentTime, Distinct, ExprId, Indexed, Join, JoinKind, LikeOp,
    Limit, Literal, Materialized, Node, Nulls, Order, OrderTerm, Range, ResultColumn, Select,
    SelectId, Source, SourceKind, Span, UnaryOp,
};
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
    /// `FROM`, after a comma in a `USING`.
    From,
    /// `BY`, after `GROUP` or `ORDER`.
    By,
    /// `AS`, in a `WITH` clause.
    WithAs,
    /// `SELECT`, `VALUES` or `WITH`.
    Select,
    /// `JOIN`, after the words that describe one.
    Join,
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
    /// The next three tokens, once they have been looked at. Three is
    /// what `t.*` needs, which is the longest thing the grammar decides
    /// by looking rather than by reading.
    ahead: [Option<Token>; 3],
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
            ahead: [None, None, None],
            arena: Arena::new(),
            depth: 0,
        };
        parser.ahead = [parser.read(), parser.read(), parser.read()];
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

    /// Reads one statement, and nothing after it but a semicolon.
    ///
    /// # Errors
    ///
    /// Where the tokens are not a statement, or where something follows
    /// it.
    pub fn only_statement(&mut self) -> Result<SelectId, Error> {
        let select = self.select()?;
        self.eat(Kind::Semi);
        match self.peek() {
            None => Ok(select),
            Some(token) => Err(self.error(Some(token), Expected::Eof)),
        }
    }

    /// A statement: a `WITH` clause, one or more cores put together, and
    /// the `ORDER BY` and `LIMIT` that belong to the whole of it.
    ///
    /// The clauses of the whole are kept on the first core, which is what
    /// the statement is named by; the cores after it hang off its
    /// `compound`.
    ///
    /// # Errors
    ///
    /// Where the tokens are not a statement.
    pub fn select(&mut self) -> Result<SelectId, Error> {
        self.deeper()?;
        let (ctes, recursive) = if self.eat_keyword(Keyword::With) {
            self.with_clause()?
        } else {
            (Range::default(), false)
        };
        let mut first = self.select_core()?;
        first.ctes = ctes;
        first.recursive = recursive;
        let mut cores = alloc::vec![first];
        let mut operators = Vec::new();
        while let Some(operator) = self.compound_operator() {
            operators.push(operator);
            cores.push(self.select_core()?);
        }
        let order = self.order_by()?;
        let limit = self.limit()?;
        // The chain is built from the back, so that each core can name
        // the one after it. There is always a first core, which is the
        // statement, and the clauses of the whole belong to it.
        let mut last = cores.pop().unwrap_or_default();
        let mut at = cores.len();
        if at == 0 {
            last.order = order;
            last.limit = limit;
        }
        let mut chain = self.arena.push_select(last);
        while let Some(mut core) = cores.pop() {
            at = at.saturating_sub(1);
            core.compound = operators.get(at).copied().map(|operator| (operator, chain));
            if at == 0 {
                core.order = order;
                core.limit = limit;
            }
            chain = self.arena.push_select(core);
        }
        self.depth = self.depth.saturating_sub(1);
        Ok(chain)
    }

    /// `WITH [RECURSIVE] name [(columns)] AS [NOT] [MATERIALIZED] (select), ...`
    fn with_clause(&mut self) -> Result<(Range, bool), Error> {
        let recursive = self.eat_keyword(Keyword::Recursive);
        let mut ctes = Vec::new();
        loop {
            let name = self.name()?;
            let columns = if self.eat(Kind::Lp) {
                let mut names = Vec::new();
                loop {
                    names.push(self.name()?);
                    if !self.eat(Kind::Comma) {
                        break;
                    }
                }
                self.expect(Kind::Rp, Expected::CloseParen)?;
                self.arena.push_names(&names)
            } else {
                Range::default()
            };
            self.expect_keyword(Keyword::As, Expected::WithAs)?;
            let materialized = if self.eat_keyword(Keyword::Not) {
                self.expect_keyword(Keyword::Materialized, Expected::WithAs)?;
                Materialized::No
            } else if self.eat_keyword(Keyword::Materialized) {
                Materialized::Yes
            } else {
                Materialized::Unspecified
            };
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let select = self.select()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            ctes.push(Cte {
                name,
                columns,
                select,
                materialized,
            });
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok((self.arena.push_ctes(&ctes), recursive))
    }

    /// One `SELECT`, or one `VALUES`.
    fn select_core(&mut self) -> Result<Select, Error> {
        if self.eat_keyword(Keyword::Values) {
            return self.values();
        }
        self.expect_keyword(Keyword::Select, Expected::Select)?;
        let distinct = if self.eat_keyword(Keyword::Distinct) {
            Distinct::Distinct
        } else if self.eat_keyword(Keyword::All) {
            Distinct::All
        } else {
            Distinct::Unspecified
        };
        let columns = self.result_columns()?;
        let from = if self.eat_keyword(Keyword::From) {
            self.tables()?
        } else {
            Range::default()
        };
        let filter = if self.eat_keyword(Keyword::Where) {
            Some(self.expression()?)
        } else {
            None
        };
        let group = if self.eat_keyword(Keyword::Group) {
            self.expect_keyword(Keyword::By, Expected::By)?;
            let mut terms = Vec::new();
            loop {
                terms.push(self.expression()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
            self.arena.push_children(&terms)
        } else {
            Range::default()
        };
        let having = if self.eat_keyword(Keyword::Having) {
            Some(self.expression()?)
        } else {
            None
        };
        Ok(Select {
            distinct,
            columns,
            from,
            filter,
            group,
            having,
            ..Select::default()
        })
    }

    /// `VALUES (a, b), (c, d)`, where each row is a row node.
    fn values(&mut self) -> Result<Select, Error> {
        let mut rows = Vec::new();
        loop {
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let mut items = Vec::new();
            loop {
                items.push(self.expression()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
            self.expect(Kind::Rp, Expected::CloseParen)?;
            let children = self.arena.push_children(&items);
            rows.push(self.node(Node::Row(children))?);
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        let values = self.arena.push_children(&rows);
        Ok(Select {
            values,
            ..Select::default()
        })
    }

    /// What the statement answers: `*`, `t.*`, or an expression with the
    /// name it is answered under.
    fn result_columns(&mut self) -> Result<Range, Error> {
        let mut columns = Vec::new();
        loop {
            columns.push(self.result_column()?);
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(self.arena.push_results(&columns))
    }

    /// One result column.
    fn result_column(&mut self) -> Result<ResultColumn, Error> {
        if self.eat(Kind::Star) {
            return Ok(ResultColumn::Star);
        }
        // `t.*` is a result column and not an expression, so it is read
        // here and nowhere else.
        if let (Some(first), Some(second), Some(third)) =
            (self.peek(), self.ahead(1), self.ahead(2))
            && matches!(first.kind, Kind::Id | Kind::String)
            && second.kind == Kind::Dot
            && third.kind == Kind::Star
        {
            self.bump();
            self.bump();
            self.bump();
            return Ok(ResultColumn::TableStar(Span::of(first)));
        }
        let expr = self.expression()?;
        let alias = if self.eat_keyword(Keyword::As) {
            Some(self.name()?)
        } else {
            self.optional_name()
        };
        Ok(ResultColumn::Expr { expr, alias })
    }

    /// The tables of a `FROM` clause, with the joins between them.
    fn tables(&mut self) -> Result<Range, Error> {
        let mut sources = Vec::new();
        sources.push(self.source(Join::default())?);
        while let Some(join) = self.join_operator()? {
            sources.push(self.source(join)?);
        }
        Ok(self.arena.push_sources(&sources))
    }

    /// The operator between two tables, where there is one.
    fn join_operator(&mut self) -> Result<Option<Join>, Error> {
        if self.eat(Kind::Comma) {
            return Ok(Some(Join {
                natural: false,
                kind: JoinKind::Inner,
                comma: true,
            }));
        }
        if self.eat_keyword(Keyword::Join) {
            return Ok(Some(Join {
                natural: false,
                kind: JoinKind::Inner,
                comma: false,
            }));
        }
        let Some(token) = self.peek() else {
            return Ok(None);
        };
        let Kind::Keyword(first) = token.kind else {
            return Ok(None);
        };
        if !is_join_word(first) {
            return Ok(None);
        }
        // `NATURAL LEFT OUTER JOIN` and the shorter ways of writing it:
        // up to three words, and then `JOIN`.
        let mut join = Join::default();
        let mut words = 0u32;
        while words < 3 {
            let Some(token) = self.peek() else { break };
            let Kind::Keyword(word) = token.kind else {
                break;
            };
            if word == Keyword::Join {
                break;
            }
            if !is_join_word(word) {
                break;
            }
            self.bump();
            words = words.saturating_add(1);
            match word {
                Keyword::Natural => join.natural = true,
                Keyword::Left => join.kind = JoinKind::Left,
                Keyword::Right => join.kind = JoinKind::Right,
                Keyword::Full => join.kind = JoinKind::Full,
                Keyword::Cross => join.kind = JoinKind::Cross,
                Keyword::Inner => join.kind = JoinKind::Inner,
                _ => {}
            }
        }
        self.expect_keyword(Keyword::Join, Expected::Join)?;
        if join.kind == JoinKind::None {
            join.kind = JoinKind::Inner;
        }
        Ok(Some(join))
    }

    /// One table of a `FROM` clause, with its name, its alias, and the
    /// condition that joins it.
    fn source(&mut self, join: Join) -> Result<Source, Error> {
        let kind = if self.at(Kind::Lp) {
            self.bump();
            let select = self.select()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            SourceKind::Select(select)
        } else {
            let first = self.name()?;
            let (schema, name) = if self.eat(Kind::Dot) {
                (Some(first), self.name()?)
            } else {
                (None, first)
            };
            if self.at(Kind::Lp) {
                self.bump();
                let mut args = Vec::new();
                if !self.at(Kind::Rp) {
                    loop {
                        args.push(self.expression()?);
                        if !self.eat(Kind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Kind::Rp, Expected::CloseParen)?;
                let args = self.arena.push_children(&args);
                SourceKind::Function { schema, name, args }
            } else {
                SourceKind::Table {
                    schema,
                    name,
                    indexed: Indexed::Unspecified,
                }
            }
        };
        let alias = if self.eat_keyword(Keyword::As) {
            Some(self.name()?)
        } else {
            self.optional_name()
        };
        let kind = self.indexed_by(kind)?;
        let mut on = None;
        let mut using = Range::default();
        if self.eat_keyword(Keyword::On) {
            on = Some(self.expression()?);
        } else if self.eat_keyword(Keyword::Using) {
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let mut names = Vec::new();
            loop {
                names.push(self.name()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
            self.expect(Kind::Rp, Expected::CloseParen)?;
            using = self.arena.push_names(&names);
        }
        Ok(Source {
            kind,
            alias,
            join,
            on,
            using,
        })
    }

    /// `INDEXED BY name` and `NOT INDEXED`, which only a table may carry.
    fn indexed_by(&mut self, kind: SourceKind) -> Result<SourceKind, Error> {
        let indexed = if self.eat_keyword(Keyword::Indexed) {
            self.expect_keyword(Keyword::By, Expected::By)?;
            Indexed::By(self.name()?)
        } else if self.at_keyword(Keyword::Not) && self.ahead(1).is_some_and(is_indexed) {
            self.bump();
            self.bump();
            Indexed::Not
        } else {
            return Ok(kind);
        };
        match kind {
            SourceKind::Table { schema, name, .. } => Ok(SourceKind::Table {
                schema,
                name,
                indexed,
            }),
            // `INDEXED BY` after anything but a table is not a statement.
            SourceKind::Function { .. } | SourceKind::Select(_) => {
                Err(self.error(self.peek(), Expected::Eof))
            }
        }
    }

    /// `ORDER BY term, term`, where a term may say which way it sorts and
    /// where its nulls go.
    fn order_by(&mut self) -> Result<Range, Error> {
        if !self.eat_keyword(Keyword::Order) {
            return Ok(Range::default());
        }
        self.expect_keyword(Keyword::By, Expected::By)?;
        let mut terms = Vec::new();
        loop {
            let expr = self.expression()?;
            let order = if self.eat_keyword(Keyword::Asc) {
                Order::Ascending
            } else if self.eat_keyword(Keyword::Desc) {
                Order::Descending
            } else {
                Order::Unspecified
            };
            let nulls = if self.eat_keyword(Keyword::Nulls) {
                if self.eat_keyword(Keyword::First) {
                    Nulls::First
                } else {
                    self.expect_keyword(Keyword::Last, Expected::By)?;
                    Nulls::Last
                }
            } else {
                Nulls::Unspecified
            };
            terms.push(OrderTerm { expr, order, nulls });
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(self.arena.push_orders(&terms))
    }

    /// `LIMIT count`, `LIMIT count OFFSET skip`, and the older
    /// `LIMIT skip, count`.
    fn limit(&mut self) -> Result<Option<Limit>, Error> {
        if !self.eat_keyword(Keyword::Limit) {
            return Ok(None);
        }
        let first = self.expression()?;
        if self.eat_keyword(Keyword::Offset) {
            return Ok(Some(Limit {
                count: first,
                offset: Some(self.expression()?),
            }));
        }
        if self.eat(Kind::Comma) {
            // `LIMIT a, b` counts `b` rows after skipping `a`, which is
            // the other way round from `OFFSET`.
            let count = self.expression()?;
            return Ok(Some(Limit {
                count,
                offset: Some(first),
            }));
        }
        Ok(Some(Limit {
            count: first,
            offset: None,
        }))
    }

    /// `UNION`, `UNION ALL`, `EXCEPT` and `INTERSECT`.
    fn compound_operator(&mut self) -> Option<Compound> {
        if self.eat_keyword(Keyword::Union) {
            if self.eat_keyword(Keyword::All) {
                return Some(Compound::UnionAll);
            }
            return Some(Compound::Union);
        }
        if self.eat_keyword(Keyword::Except) {
            return Some(Compound::Except);
        }
        if self.eat_keyword(Keyword::Intersect) {
            return Some(Compound::Intersect);
        }
        None
    }

    /// The next token as a name, where it could be one. A name that is
    /// not there is not a refusal: an alias may be left out.
    fn optional_name(&mut self) -> Option<Span> {
        if !self.at_name() {
            return None;
        }
        self.bump().map(Span::of)
    }

    /// Whether the next token could be a name.
    fn at_name(&self) -> bool {
        match self.peek().map(|token| token.kind) {
            Some(Kind::Id | Kind::String) => true,
            Some(Kind::Keyword(keyword)) => keyword.can_be_name(),
            _ => false,
        }
    }

    /// The token `ahead` places on, once the two that are read have been
    /// looked at.
    fn ahead(&self, at: usize) -> Option<Token> {
        self.ahead.get(at).copied().flatten()
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

    /// `x IN (a, b)`, the empty list the grammar allows, `x IN (SELECT
    /// ...)`, and `x IN table`, which is the statement written short.
    fn in_list(&mut self, value: ExprId, negated: bool) -> Result<ExprId, Error> {
        if !self.at(Kind::Lp) {
            let first = self.name()?;
            let (schema, table) = if self.eat(Kind::Dot) {
                (Some(first), self.name()?)
            } else {
                (None, first)
            };
            return self.node(Node::InTable {
                value,
                schema,
                table,
                negated,
            });
        }
        self.bump();
        if self.at_select() {
            let select = self.select()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return self.node(Node::InSelect {
                value,
                select,
                negated,
            });
        }
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

    /// Whether a statement begins here.
    fn at_select(&self) -> bool {
        self.at_keyword(Keyword::Select)
            || self.at_keyword(Keyword::Values)
            || self.at_keyword(Keyword::With)
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
            Kind::Keyword(Keyword::Exists) => {
                self.bump();
                self.expect(Kind::Lp, Expected::OpenParen)?;
                let select = self.select()?;
                self.expect(Kind::Rp, Expected::CloseParen)?;
                self.node(Node::Exists(select))
            }
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
        if self.at_select() {
            let select = self.select()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return self.node(Node::Subquery(select));
        }
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
        self.ahead = [
            self.ahead.get(1).copied().flatten(),
            self.ahead.get(2).copied().flatten(),
            self.read(),
        ];
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

/// Whether a word is one of the seven that describe a join.
const fn is_join_word(keyword: Keyword) -> bool {
    matches!(
        keyword,
        Keyword::Cross
            | Keyword::Full
            | Keyword::Inner
            | Keyword::Left
            | Keyword::Natural
            | Keyword::Outer
            | Keyword::Right
    )
}

/// Whether a token is the word `INDEXED`.
fn is_indexed(token: Token) -> bool {
    token.kind == Kind::Keyword(Keyword::Indexed)
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

/// Reads one statement out of `sql` and answers the tree it built.
///
/// # Errors
///
/// Where the statement is not one `SELECT` or `VALUES`.
pub fn statement(sql: &[u8]) -> Result<(Arena, SelectId), Error> {
    let mut parser = Parser::new(sql);
    let root = parser.only_statement()?;
    Ok((parser.into_arena(), root))
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
