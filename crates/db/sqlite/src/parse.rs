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
    Action, Arena, BinaryOp, Bound, Change, ColumnConstraint, ColumnDef, Compound, Conflict,
    CreateIndex, CreateTable, Cte, CurrentTime, Definition, Delete, Distinct, Exclude, ExprId,
    Foreign, Frame, Frames, Indexed, Insert, Join, JoinKind, LikeOp, Limit, Literal, Materialized,
    NamedWindow, Node, Nulls, Order, OrderTerm, Range, ResultColumn, Select, SelectId, Set, Source,
    SourceKind, Span, TableBody, TableConstraint, TableOptions, UnaryOp, Update, Window, WindowId,
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
    /// `AS`, in a `CAST` or an `ATTACH`.
    As,
    /// `THEN`, in a `CASE`.
    Then,
    /// `BEGIN`, before the body of a trigger.
    Body,
    /// A statement of a trigger's body, which is an `INSERT`, an
    /// `UPDATE`, a `DELETE` or a `SELECT`.
    Step,
    /// `;`, after a statement of a trigger's body.
    Semi,
    /// `ROW`, after `FOR EACH`.
    Row,
    /// `OF`, after `INSTEAD`.
    Of,
    /// `END`, in a `CASE`.
    End,
    /// `AND`, in a `BETWEEN`.
    And,
    /// `FROM`, after a comma in a `USING`.
    From,
    /// `BY`, after `GROUP` or `ORDER`.
    By,
    /// `GROUP`, after `WITHIN`.
    Group,
    /// `ORDER`, after the bracket of a `WITHIN GROUP`.
    Order,
    /// Nothing: a `DISTINCT` on an ordered-set aggregate, which the
    /// span names the function of.
    OrderedDistinct,
    /// Nothing: a token the tokenizer read as no token at all, which
    /// the span names.
    Unrecognized,
    /// Nothing: a `NULLS FIRST` or a `NULLS LAST` written where an
    /// index takes its terms, the truth telling the two apart.
    ExplicitNulls(bool),
    /// A combination of words no join is written with, which is
    /// `unknown join type`.
    JoinType,
    /// Nothing: an `ORDER BY` or a `LIMIT` written on a core of a
    /// compound other than the last, which the word it carries names,
    /// the truth telling the `ORDER BY` from the `LIMIT`.
    BeforeCompound(bool, crate::ast::Compound),
    /// Nothing: the table an `UPDATE` changes named again in its
    /// `FROM`, which the span names.
    TargetInFrom,
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
    /// `CREATE`.
    Create,
    /// `INSERT` or `REPLACE`.
    Insert,
    /// `DELETE`.
    Delete,
    /// `UPDATE`.
    Update,
    /// The word `PRAGMA`.
    Pragma,
    /// The word `ANALYZE`.
    Analyze,
    /// The word `REINDEX`.
    Reindex,
    /// `SET`, after the table of an `UPDATE`.
    Set,
    /// `=`, after a column of a `SET`.
    Eq,
    /// `INTO`, after `INSERT`.
    Into,
    /// `TABLE` or `INDEX`, after `CREATE`.
    Table,
    /// `KEY`, after `PRIMARY` or `FOREIGN`.
    Key,
    /// `NULL`, after `NOT`.
    Null,
    /// `DROP`, after the column of an `ALTER TABLE ... ALTER COLUMN`.
    Drop,
    /// `NOT`, after the `DROP` of an `ALTER TABLE ... ALTER COLUMN`.
    Not,
    /// `CHECK`, after the name of an `ALTER TABLE ... ADD CONSTRAINT`.
    Constraint,
    /// `WITHOUT ROWID` or `STRICT`, after a table's columns.
    TableOption,
    /// A way of resolving a conflict, after `ON CONFLICT`.
    Conflict,
    /// What a foreign key does, after `ON DELETE` or `ON UPDATE`.
    Action,
    /// `ON`, in a `CREATE INDEX`.
    On,
    /// `DO`, in an `ON CONFLICT`.
    Do,
    /// `BEGIN`, `COMMIT`, `END` or `ROLLBACK`.
    Transaction,
    /// `SAVEPOINT`, `RELEASE` or `ROLLBACK`, which a savepoint is named
    /// after.
    Savepoint,
    /// `TO`, after `ROLLBACK`.
    To,
    /// `VALUES`, after `DEFAULT` in an `INSERT`.
    Values,
    /// `ADD`, after the table of an `ALTER TABLE`.
    Add,
    /// `WHERE`, in the `FILTER` of a window function.
    Where,
    /// Where a frame begins or ends.
    Bound,
    /// What a frame excludes, after `EXCLUDE`.
    Exclude,
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

/// The parse of two that got furthest, which is the one the C library
/// reports: `sqlite3ErrorMsg` names the token the parser stopped at,
/// and the reading that took in most of the statement stopped last.
#[must_use]
pub const fn furthest(held: Option<Error>, other: Error) -> Error {
    match held {
        Some(held) if held.at >= other.at => held,
        _ => other,
    }
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
    /// One past the last byte of the last token read, which is where
    /// the text of an expression ends.
    end: usize,
    /// The tree so far.
    arena: Arena,
    /// How deep the walk stands.
    depth: u32,
    /// The kind of the token last taken, which three words are read by.
    last: Option<Kind>,
}

impl<'a> Parser<'a> {
    /// A parser over `sql` that has read nothing.
    #[must_use]
    pub fn new(sql: &'a [u8]) -> Self {
        let mut parser = Parser {
            sql,
            lexer: Lexer::new(sql),
            ahead: [None, None, None],
            end: 0,
            arena: Arena::new(),
            depth: 0,
            last: None,
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
        let ctes = if self.eat_keyword(Keyword::With) {
            self.with_clause()?
        } else {
            Range::default()
        };
        let mut first = self.select_core()?;
        first.ctes = ctes;
        let mut cores = alloc::vec![first];
        let mut operators = Vec::new();
        while let Some(operator) = self.compound_operator() {
            operators.push(operator);
            cores.push(self.select_core()?);
        }
        // `VALUES` is a core and not a statement of its own, so nothing
        // of the whole may follow one: `oneselect` carries the
        // `ORDER BY` and the `LIMIT`, and `values` is the other rule.
        if cores.last().is_some_and(|core| !core.values.is_empty())
            && (self.at_keyword(Keyword::Order) || self.at_keyword(Keyword::Limit))
        {
            return Err(self.error(self.peek(), Expected::Eof));
        }
        let ordered = self.at_keyword(Keyword::Order);
        let order = self.order_by()?;
        let limit = self.limit()?;
        // `parserDoubleLinkSelect` refuses an `ORDER BY` or a `LIMIT`
        // written on a core other than the last one, naming the word
        // that joins that core to the one after it.
        // The loop above took every word that joins two cores, so a
        // word standing here is one an `ORDER BY` or a `LIMIT` stopped
        // the loop before.
        if let Some(operator) = self.compound_ahead() {
            return Err(self.error(self.peek(), Expected::BeforeCompound(ordered, operator)));
        }
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
    ///
    /// `RECURSIVE` is read and not kept, because a term that reads its
    /// own name reads itself whether the word was written or not.
    fn with_clause(&mut self) -> Result<Range, Error> {
        let _ = self.eat_keyword(Keyword::Recursive);
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
        Ok(self.arena.push_ctes(&ctes))
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
        let windows = self.window_clause()?;
        Ok(Select {
            distinct,
            columns,
            from,
            filter,
            group,
            windows,
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
        let start = self.peek().map_or(self.end, |token| token.start);
        let expr = self.expression()?;
        let text = Span {
            start,
            len: self.end.saturating_sub(start),
        };
        let alias = if self.eat_keyword(Keyword::As) {
            Some(self.name()?)
        } else {
            self.optional_name()
        };
        Ok(ResultColumn::Expr { expr, alias, text })
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
        // `sqlite3JoinType` reads up to three words before `JOIN`, each
        // a name of its own, and holds the combination they make.
        let mut join = Join::default();
        let mut mask = 0u32;
        let mut words = 0u32;
        let mut span: Option<Span> = None;
        while let Some(token) = self.peek().filter(|_| words < 3) {
            if !matches!(token.kind, Kind::Keyword(_) | Kind::Id) {
                break;
            }
            if token.kind == Kind::Keyword(Keyword::Join) {
                break;
            }
            self.bump();
            words = words.saturating_add(1);
            let held = Span::of(token);
            span = Some(span.map_or(held, |first| join_span(first, held)));
            mask |= join_mask(token);
            match token.kind {
                Kind::Keyword(Keyword::Natural) => join.natural = true,
                Kind::Keyword(Keyword::Left) => join.kind = JoinKind::Left,
                Kind::Keyword(Keyword::Right) => join.kind = JoinKind::Right,
                Kind::Keyword(Keyword::Full) => join.kind = JoinKind::Full,
                Kind::Keyword(Keyword::Cross) => join.kind = JoinKind::Cross,
                Kind::Keyword(Keyword::Inner) => join.kind = JoinKind::Inner,
                _ => {}
            }
        }
        // A word no join carries, and `INNER` beside `OUTER`, are the
        // two a combination is refused for.
        if mask & JOIN_OTHER != 0 || mask & (JOIN_INNER | JOIN_OUTER) == (JOIN_INNER | JOIN_OUTER) {
            let at = span.unwrap_or(Span { start: 0, len: 0 });
            return Err(Error {
                at: at.start,
                len: at.len,
                expected: Expected::JoinType,
            });
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
            let select = if self.at_select() {
                self.select()?
            } else {
                self.parenthesized_tables()?
            };
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

    /// The tables a `FROM` names inside brackets, which is `seltablist
    /// ::= stl_prefix LP seltablist RP` of `parse.y`: the tables and
    /// the joins between them stand for a statement that answers every
    /// column of them, so the brackets hold the joins together and the
    /// join written after the brackets is against what they answer.
    fn parenthesized_tables(&mut self) -> Result<SelectId, Error> {
        let from = self.tables()?;
        let columns = self.arena.push_results(&[ResultColumn::Star]);
        Ok(self.arena.push_select(Select {
            columns,
            from,
            nested: true,
            ..Select::default()
        }))
    }

    /// `INDEXED BY name` and `NOT INDEXED` after a name, which
    /// `qualified-table-name` of `research/sqlite/src/parse.y:760`
    /// writes and an `UPDATE` and a `DELETE` carry as well as a table of
    /// a `FROM`.
    fn indexed_name(&mut self) -> Result<Indexed, Error> {
        if self.eat_keyword(Keyword::Indexed) {
            self.expect_keyword(Keyword::By, Expected::By)?;
            return Ok(Indexed::By(self.name()?));
        }
        if self.at_keyword(Keyword::Not) && self.ahead(1).is_some_and(is_indexed) {
            self.bump();
            self.bump();
            return Ok(Indexed::Not);
        }
        Ok(Indexed::Unspecified)
    }

    /// `INDEXED BY name` and `NOT INDEXED`, which only a table may carry.
    fn indexed_by(&mut self, kind: SourceKind) -> Result<SourceKind, Error> {
        let indexed = match self.indexed_name()? {
            Indexed::Unspecified => return Ok(kind),
            held => held,
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
        self.sort_list()
    }

    /// `expr [ASC|DESC] [NULLS FIRST|LAST]`, comma separated, which is
    /// `sortlist`: the body of an `ORDER BY` and of an index.
    fn sort_list(&mut self) -> Result<Range, Error> {
        let mut terms = Vec::new();
        loop {
            let expr = self.expression()?;
            let order = self.sort_order();
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

    /// The same list, held to the terms an index takes: `NULLS FIRST`
    /// and `NULLS LAST` are written where a statement sorts rows and
    /// nowhere else, which `sqlite3HasExplicitNulls` refuses.
    fn indexed_list(&mut self) -> Result<Range, Error> {
        let terms = self.sort_list()?;
        let written = self
            .arena
            .orders(terms)
            .iter()
            .find_map(|term| match term.nulls {
                Nulls::First => Some(true),
                Nulls::Last => Some(false),
                Nulls::Unspecified => None,
            });
        match written {
            Some(first) => Err(self.error(self.peek(), Expected::ExplicitNulls(first))),
            None => Ok(terms),
        }
    }

    /// `ASC`, `DESC`, or neither.
    fn sort_order(&mut self) -> Order {
        if self.eat_keyword(Keyword::Asc) {
            Order::Ascending
        } else if self.eat_keyword(Keyword::Desc) {
            Order::Descending
        } else {
            Order::Unspecified
        }
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

    /// The compound operator the next tokens are, where they are one,
    /// read without taking them.
    fn compound_ahead(&self) -> Option<Compound> {
        if self.at_keyword(Keyword::Union) {
            if self
                .ahead(1)
                .is_some_and(|token| token.kind == Kind::Keyword(Keyword::All))
            {
                return Some(Compound::UnionAll);
            }
            return Some(Compound::Union);
        }
        if self.at_keyword(Keyword::Except) {
            return Some(Compound::Except);
        }
        if self.at_keyword(Keyword::Intersect) {
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

    /// One `CREATE TABLE` or `CREATE INDEX` and nothing after it, which
    /// is what a row of `sqlite_schema` holds.
    ///
    /// # Errors
    ///
    /// Where it is not one of the two, or something follows it.
    /// One statement that changes a database and nothing after it.
    ///
    /// # Errors
    ///
    /// Where the statement is not one this crate writes, and where
    /// anything but a semicolon follows it.
    pub fn only_change(&mut self) -> Result<Change, Error> {
        let change = self.change()?;
        self.eat(Kind::Semi);
        if let Some(token) = self.peek() {
            return Err(self.error(Some(token), Expected::Eof));
        }
        Ok(change)
    }

    /// One statement that changes a database, with the `WITH` clause
    /// that stands before it where one does: the tables that clause
    /// names belong to the statement the rows come from.
    fn change(&mut self) -> Result<Change, Error> {
        let ctes = if self.eat_keyword(Keyword::With) {
            self.with_clause()?
        } else {
            Range::default()
        };
        if self.at_keyword(Keyword::Update) {
            let mut statement = self.update()?;
            if !ctes.is_empty() {
                // A `WITH` before an `UPDATE` names tables its `FROM`
                // reads, which is the statement that clause is
                // answered as; a `WHERE` that names one is not
                // answered yet.
                let id = statement
                    .from
                    .ok_or_else(|| self.error(None, Expected::Select))?;
                // The statement was pushed just above, so the arena
                // holds it.
                let mut select = self
                    .arena
                    .select(id)
                    .ok_or(self.error(None, Expected::Select))?;
                select.ctes = ctes;
                statement.from = Some(self.arena.push_select(select));
            }
            return Ok(Change::Update(statement));
        }
        if self.at_keyword(Keyword::Delete) {
            let statement = self.delete()?;
            if !ctes.is_empty() {
                // A `WITH` before a `DELETE` names tables its `WHERE`
                // may read, which this crate does not answer yet.
                return Err(self.error(None, Expected::Select));
            }
            return Ok(Change::Delete(statement));
        }
        let mut statement = self.insert()?;
        if !ctes.is_empty() {
            let id = statement.select;
            let mut select = self
                .arena
                .select(id)
                .ok_or(self.error(None, Expected::Select))?;
            select.ctes = ctes;
            statement.select = self.arena.push_select(select);
        }
        Ok(Change::Insert(statement))
    }

    /// `INSERT INTO name [(columns)] <select>`, and `REPLACE INTO`,
    /// which is `INSERT OR REPLACE INTO`.
    fn insert(&mut self) -> Result<Insert, Error> {
        let conflict = if self.eat_keyword(Keyword::Replace) {
            Conflict::Replace
        } else {
            self.expect_keyword(Keyword::Insert, Expected::Insert)?;
            if self.eat_keyword(Keyword::Or) {
                self.or_conflict()?
            } else {
                Conflict::Unspecified
            }
        };
        self.expect_keyword(Keyword::Into, Expected::Into)?;
        let (schema, name) = self.qualified_name()?;
        let mut columns = Vec::new();
        if self.at(Kind::Lp) {
            self.bump();
            loop {
                columns.push(self.name()?);
                if !self.eat(Kind::Comma) {
                    break;
                }
            }
            self.expect(Kind::Rp, Expected::CloseParen)?;
        }
        let columns = self.arena.push_names(&columns);
        // `INSERT INTO t DEFAULT VALUES` reads no statement: it writes
        // one row of what every column falls back to.
        let defaults = self.eat_keyword(Keyword::Default);
        let select = if defaults {
            self.expect_keyword(Keyword::Values, Expected::Values)?;
            self.arena.push_select(crate::ast::Select::default())
        } else {
            self.select()?
        };
        let mut upserts = Vec::new();
        while self.eat_keyword(Keyword::On) {
            self.expect_keyword(Keyword::Conflict, Expected::Conflict)?;
            upserts.push(self.upsert()?);
        }
        let upserts = self.arena.push_upserts(&upserts);
        let returning = self.returning()?;
        Ok(Insert {
            conflict,
            schema,
            name,
            columns,
            select,
            defaults,
            upserts,
            returning,
        })
    }

    /// What follows `ON CONFLICT` in an `INSERT`, which is
    /// `sqlite3UpsertNew`: the columns the clause is for, and what it
    /// does where a row conflicts on them.
    fn upsert(&mut self) -> Result<crate::ast::Upsert, Error> {
        // A target is written as an indexed column, with the collation
        // and the order of one, which is what names the index.
        let mut targets = Range::default();
        let mut over = None;
        if self.eat(Kind::Lp) {
            targets = self.indexed_list()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            if self.eat_keyword(Keyword::Where) {
                over = Some(self.expression()?);
            }
        }
        self.expect_keyword(Keyword::Do, Expected::Do)?;
        if self.eat_keyword(Keyword::Nothing) {
            return Ok(crate::ast::Upsert {
                targets,
                over,
                sets: Range::default(),
                writes: false,
                filter: None,
            });
        }
        self.expect_keyword(Keyword::Update, Expected::Update)?;
        self.expect_keyword(Keyword::Set, Expected::Set)?;
        let mut sets = Vec::new();
        loop {
            let column = self.name()?;
            self.expect(Kind::Eq, Expected::Eq)?;
            let value = self.expression()?;
            sets.push(Set { column, value });
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        let sets = self.arena.push_sets(&sets);
        let filter = if self.eat_keyword(Keyword::Where) {
            Some(self.expression()?)
        } else {
            None
        };
        Ok(crate::ast::Upsert {
            targets,
            over,
            sets,
            writes: true,
            filter,
        })
    }

    /// `UPDATE name SET column = value, ... [WHERE filter]`.
    fn update(&mut self) -> Result<Update, Error> {
        self.expect_keyword(Keyword::Update, Expected::Update)?;
        let conflict = if self.eat_keyword(Keyword::Or) {
            self.or_conflict()?
        } else {
            Conflict::Unspecified
        };
        let (schema, name) = self.qualified_name()?;
        let indexed = self.indexed_name()?;
        self.expect_keyword(Keyword::Set, Expected::Set)?;
        let mut sets = Vec::new();
        loop {
            let column = self.name()?;
            self.expect(Kind::Eq, Expected::Eq)?;
            let value = self.expression()?;
            sets.push(Set { column, value });
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        let sets = self.arena.push_sets(&sets);
        // The tables of a `FROM` are answered as the statement
        // `SELECT * FROM ...` answers them, which is what
        // `sqlite3Update` builds them into.
        let from = if self.eat_keyword(Keyword::From) {
            let tables = self.tables()?;
            self.refused_target((schema, name), tables)?;
            let columns = self.arena.push_results(&[ResultColumn::Star]);
            Some(self.arena.push_select(Select {
                columns,
                from: tables,
                ..Select::default()
            }))
        } else {
            None
        };
        let filter = if self.eat_keyword(Keyword::Where) {
            Some(self.expression()?)
        } else {
            None
        };
        let returning = self.returning()?;
        Ok(Update {
            conflict,
            schema,
            name,
            indexed,
            sets,
            from,
            filter,
            returning,
        })
    }

    /// `DELETE FROM name [WHERE filter]`.
    fn delete(&mut self) -> Result<Delete, Error> {
        self.expect_keyword(Keyword::Delete, Expected::Delete)?;
        self.expect_keyword(Keyword::From, Expected::From)?;
        let (schema, name) = self.qualified_name()?;
        let indexed = self.indexed_name()?;
        let filter = if self.eat_keyword(Keyword::Where) {
            Some(self.expression()?)
        } else {
            None
        };
        let returning = self.returning()?;
        Ok(Delete {
            schema,
            name,
            indexed,
            filter,
            returning,
        })
    }

    /// The columns a `RETURNING` answers, or an empty run where the
    /// statement writes none, which is `sqlite3AddReturning`.
    fn returning(&mut self) -> Result<Range, Error> {
        if !self.eat_keyword(Keyword::Returning) {
            return Ok(Range::default());
        }
        self.result_columns()
    }

    /// What follows `INSERT OR`.
    fn or_conflict(&mut self) -> Result<Conflict, Error> {
        let Some(token) = self.peek() else {
            return Err(self.error(None, Expected::Conflict));
        };
        let conflict = match token.kind {
            Kind::Keyword(Keyword::Rollback) => Conflict::Rollback,
            Kind::Keyword(Keyword::Abort) => Conflict::Abort,
            Kind::Keyword(Keyword::Fail) => Conflict::Fail,
            Kind::Keyword(Keyword::Ignore) => Conflict::Ignore,
            Kind::Keyword(Keyword::Replace) => Conflict::Replace,
            _ => return Err(self.error(Some(token), Expected::Conflict)),
        };
        self.bump();
        Ok(conflict)
    }

    /// One `CREATE TABLE` or `CREATE INDEX` and nothing after it.
    ///
    /// # Errors
    ///
    /// Where the statement is not one of the two, and where anything
    /// but a semicolon follows it.
    pub fn only_definition(&mut self) -> Result<Definition, Error> {
        let definition = self.definition()?;
        self.eat(Kind::Semi);
        if let Some(token) = self.peek() {
            return Err(self.error(Some(token), Expected::Eof));
        }
        Ok(definition)
    }

    /// A statement that makes something, takes it away, or changes what
    /// it is made of.
    fn definition(&mut self) -> Result<Definition, Error> {
        if self.eat_keyword(Keyword::Drop) {
            return Ok(Definition::Drop(self.drop_statement()?));
        }
        if self.eat_keyword(Keyword::Alter) {
            return self.alter_table();
        }
        if self.eat_keyword(Keyword::Vacuum) {
            return Ok(Definition::Vacuum(self.vacuum()?));
        }
        if self.eat_keyword(Keyword::Attach) {
            return Ok(Definition::Attach(self.attach()?));
        }
        if self.eat_keyword(Keyword::Detach) {
            return Ok(Definition::Detach(self.detach()?));
        }
        self.expect_keyword(Keyword::Create, Expected::Create)?;
        let temporary = self.at_temporary();
        if temporary {
            self.bump();
        }
        if self.eat_keyword(Keyword::Table) {
            return Ok(Definition::Table(self.create_table(temporary)?));
        }
        if self.eat_keyword(Keyword::View) {
            return Ok(Definition::View(self.create_view(temporary)?));
        }
        if self.eat_keyword(Keyword::Trigger) {
            return Ok(Definition::Trigger(self.create_trigger(temporary)?));
        }
        let unique = self.eat_keyword(Keyword::Unique);
        self.expect_keyword(Keyword::Index, Expected::Table)?;
        Ok(Definition::Index(self.create_index(unique)?))
    }

    /// `ATTACH [DATABASE] file AS name [KEY key]`, with the word already
    /// read.
    ///
    /// The key is read and nothing is done with it, which is what a
    /// library built without an encryption extension does.
    fn attach(&mut self) -> Result<crate::ast::Attach, Error> {
        self.eat_keyword(Keyword::Database);
        let file = self.expression()?;
        self.expect_keyword(Keyword::As, Expected::As)?;
        let name = self.expression()?;
        if self.eat_keyword(Keyword::Key) {
            self.expression()?;
        }
        Ok(crate::ast::Attach { file, name })
    }

    /// `DETACH [DATABASE] name`, with the word already read.
    fn detach(&mut self) -> Result<crate::ast::Detach, Error> {
        self.eat_keyword(Keyword::Database);
        let name = self.expression()?;
        Ok(crate::ast::Detach { name })
    }

    /// `VACUUM [schema] [INTO expr]`, with the word already read.
    fn vacuum(&mut self) -> Result<crate::ast::Vacuum, Error> {
        // The schema is a name and `INTO` is a word, so the token after
        // `VACUUM` says which of the two stands there, and a statement
        // that ends after the word names neither.
        let named = self.peek().is_some_and(|token| match token.kind {
            Kind::Id | Kind::String => true,
            Kind::Keyword(keyword) => keyword != Keyword::Into && keyword.can_be_name(),
            _ => false,
        });
        let schema = named.then(|| self.name()).transpose()?;
        let into = self
            .eat_keyword(Keyword::Into)
            .then(|| self.expression())
            .transpose()?;
        Ok(crate::ast::Vacuum { schema, into })
    }

    /// Whether `TEMP` or `TEMPORARY` stands here as the word and not as
    /// a name, which the word after it decides.
    fn at_temporary(&self) -> bool {
        let temporary = matches!(
            self.peek().map(|token| token.kind),
            Some(Kind::Keyword(Keyword::Temp | Keyword::Temporary))
        );
        temporary
            && matches!(
                self.ahead(1).map(|token| token.kind),
                Some(Kind::Keyword(
                    Keyword::Table | Keyword::View | Keyword::Trigger
                ))
            )
    }

    /// `BEGIN`, `COMMIT`, `END` or `ROLLBACK`, each with the words
    /// SQLite lets stand beside it.
    ///
    /// `ROLLBACK TO` names a savepoint, which is a statement of its own
    /// and not this one, so it is refused here rather than read as a
    /// rollback of the whole transaction.
    fn transaction(&mut self) -> Result<crate::ast::Transaction, Error> {
        let read = if self.eat_keyword(Keyword::Begin) {
            for word in [Keyword::Deferred, Keyword::Immediate, Keyword::Exclusive] {
                if self.eat_keyword(word) {
                    break;
                }
            }
            crate::ast::Transaction::Begin
        } else if self.eat_keyword(Keyword::Commit) || self.eat_keyword(Keyword::End) {
            crate::ast::Transaction::Commit
        } else {
            self.expect_keyword(Keyword::Rollback, Expected::Transaction)?;
            crate::ast::Transaction::Rollback
        };
        // `trans_opt ::= TRANSACTION nm`: a name after the word names
        // nothing and is read past, which
        // `research/sqlite/src/parse.y:210` writes no code for.
        if self.eat_keyword(Keyword::Transaction) {
            let _ = self.name();
        }
        Ok(read)
    }

    /// `SAVEPOINT name`, `RELEASE [SAVEPOINT] name` and `ROLLBACK
    /// [TRANSACTION] TO [SAVEPOINT] name`, which is `sqlite3Savepoint`
    /// under the three words `savepoint_opcode` answers.
    fn savepoint(&mut self) -> Result<crate::ast::Savepoint, Error> {
        if self.eat_keyword(Keyword::Savepoint) {
            return Ok(crate::ast::Savepoint::Open(self.name()?));
        }
        if self.eat_keyword(Keyword::Release) {
            self.eat_keyword(Keyword::Savepoint);
            return Ok(crate::ast::Savepoint::Release(self.name()?));
        }
        self.expect_keyword(Keyword::Rollback, Expected::Savepoint)?;
        self.eat_keyword(Keyword::Transaction);
        self.expect_keyword(Keyword::To, Expected::To)?;
        self.eat_keyword(Keyword::Savepoint);
        Ok(crate::ast::Savepoint::Back(self.name()?))
    }

    /// What follows `ALTER TABLE`: the table, the word `ADD`, and the
    /// column it gains.
    ///
    /// The text of the column is what the schema statement gains, so it
    /// is kept as it was written, from the first byte of the column to
    /// the last byte of its last token. `sqlite3AlterFinishAddColumn`
    /// takes the trailing semicolon and the space before it off there,
    /// which a span that ends on a token never holds.
    /// What follows the `ADD` of an `ALTER TABLE` where it writes a
    /// constraint of the table's own, which is `ADD CONSTRAINT name
    /// CHECK (...)` and `ADD CHECK (...)`, and nothing where it writes
    /// a column.
    ///
    /// The text of the constraint is what the schema statement gains,
    /// so it is kept as it was written.
    fn added_constraint(&mut self) -> Result<Option<crate::ast::Constrained>, Error> {
        let start = self.peek().map_or(self.end, |token| token.start);
        let named = if self.eat_keyword(Keyword::Constraint) {
            let name = self.name()?;
            self.expect_keyword(Keyword::Check, Expected::Constraint)?;
            Some(name)
        } else if self.eat_keyword(Keyword::Check) {
            None
        } else {
            return Ok(None);
        };
        let value = self.check_body()?;
        let written = Span {
            start,
            len: self.end.saturating_sub(start),
        };
        Ok(Some(crate::ast::Constrained::Add(written, named, value)))
    }

    /// The body of a `CHECK` and the conflict clause after it, read for
    /// what they hold and not for what they answer.
    fn check_body(&mut self) -> Result<ExprId, Error> {
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let value = self.expression()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        self.conflict_clause()?;
        Ok(value)
    }

    fn alter_table(&mut self) -> Result<Definition, Error> {
        self.expect_keyword(Keyword::Table, Expected::Table)?;
        let (schema, table) = self.qualified_name()?;
        if self.eat_keyword(Keyword::Rename) {
            if self.eat_keyword(Keyword::To) {
                let name = self.name()?;
                return Ok(Definition::Rename(crate::ast::RenameTable {
                    schema,
                    table,
                    name,
                }));
            }
            self.eat_keyword(Keyword::Column);
            let column = self.name()?;
            self.expect_keyword(Keyword::To, Expected::To)?;
            let name = self.name()?;
            return Ok(Definition::RenameColumn(crate::ast::RenameColumn {
                schema,
                table,
                column,
                name,
            }));
        }
        if self.eat_keyword(Keyword::Drop) {
            if self.eat_keyword(Keyword::Constraint) {
                let name = self.name()?;
                return Ok(Definition::DropConstraint(crate::ast::DropConstraint {
                    schema,
                    table,
                    which: crate::ast::Constrained::Named(name),
                }));
            }
            self.eat_keyword(Keyword::Column);
            let column = self.name()?;
            return Ok(Definition::DropColumn(crate::ast::DropColumn {
                schema,
                table,
                column,
            }));
        }
        if self.eat_keyword(Keyword::Alter) {
            self.eat_keyword(Keyword::Column);
            let column = self.name()?;
            if self.eat_keyword(Keyword::Set) {
                let start = self.peek().map_or(self.end, |token| token.start);
                self.expect_keyword(Keyword::Not, Expected::Not)?;
                self.expect_keyword(Keyword::Null, Expected::Null)?;
                self.conflict_clause()?;
                let written = Span {
                    start,
                    len: self.end.saturating_sub(start),
                };
                return Ok(Definition::DropConstraint(crate::ast::DropConstraint {
                    schema,
                    table,
                    which: crate::ast::Constrained::SetNotNull(column, written),
                }));
            }
            self.expect_keyword(Keyword::Drop, Expected::Drop)?;
            self.expect_keyword(Keyword::Not, Expected::Not)?;
            self.expect_keyword(Keyword::Null, Expected::Null)?;
            return Ok(Definition::DropConstraint(crate::ast::DropConstraint {
                schema,
                table,
                which: crate::ast::Constrained::NotNull(column),
            }));
        }
        self.expect_keyword(Keyword::Add, Expected::Add)?;
        if let Some(which) = self.added_constraint()? {
            return Ok(Definition::DropConstraint(crate::ast::DropConstraint {
                schema,
                table,
                which,
            }));
        }
        self.eat_keyword(Keyword::Column);
        let start = self.peek().map_or(self.end, |token| token.start);
        let column = self.column_def()?;
        Ok(Definition::AddColumn(crate::ast::AddColumn {
            schema,
            table,
            written: Span {
                start,
                len: self.end.saturating_sub(start),
            },
            column,
        }))
    }

    /// What follows `CREATE VIEW`: the name, the names it answers its
    /// columns under where they were written, and the statement.
    fn create_view(&mut self, temporary: bool) -> Result<crate::ast::CreateView, Error> {
        let if_not_exists = self.if_not_exists()?;
        let (schema, name) = self.qualified_name()?;
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
            crate::ast::Range::default()
        };
        self.expect_keyword(Keyword::As, Expected::As)?;
        let select = self.select()?;
        Ok(crate::ast::CreateView {
            temporary,
            if_not_exists,
            schema,
            name,
            columns,
            select,
        })
    }

    /// `CREATE TRIGGER name [BEFORE|AFTER|INSTEAD OF] event ON table
    /// [FOR EACH ROW] [WHEN expr] BEGIN step; ... END`.
    fn create_trigger(&mut self, temporary: bool) -> Result<crate::ast::CreateTrigger, Error> {
        use crate::ast::{TriggerEvent, TriggerTime};
        let if_not_exists = self.if_not_exists()?;
        let start = self.peek().map_or(self.end, |token| token.start);
        let (schema, name) = self.qualified_name()?;
        let time = if self.eat_keyword(Keyword::Before) {
            TriggerTime::Before
        } else if self.eat_keyword(Keyword::After) {
            TriggerTime::After
        } else if self.eat_keyword(Keyword::Instead) {
            self.expect_keyword(Keyword::Of, Expected::Of)?;
            TriggerTime::InsteadOf
        } else {
            // A trigger with no time written runs before the row,
            // which is `trigger_time` reducing to `TK_BEFORE`.
            TriggerTime::Before
        };
        let mut columns = crate::ast::Range::default();
        let event = if self.eat_keyword(Keyword::Delete) {
            TriggerEvent::Delete
        } else if self.eat_keyword(Keyword::Insert) {
            TriggerEvent::Insert
        } else {
            self.expect_keyword(Keyword::Update, Expected::Update)?;
            if self.eat_keyword(Keyword::Of) {
                columns = self.name_list()?;
            }
            TriggerEvent::Update
        };
        self.expect_keyword(Keyword::On, Expected::On)?;
        let table = self.name()?;
        if self.eat_keyword(Keyword::For) {
            self.expect_keyword(Keyword::Each, Expected::Row)?;
            self.expect_keyword(Keyword::Row, Expected::Row)?;
        }
        let condition = if self.eat_keyword(Keyword::When) {
            Some(self.expression()?)
        } else {
            None
        };
        self.expect_keyword(Keyword::Begin, Expected::Body)?;
        let mut steps = Vec::new();
        loop {
            steps.push(self.trigger_step()?);
            self.expect(Kind::Semi, Expected::Semi)?;
            if self.eat_keyword(Keyword::End) {
                break;
            }
        }
        let body = self.arena.push_steps(&steps);
        let written = Span {
            start,
            len: self.end.saturating_sub(start),
        };
        Ok(crate::ast::CreateTrigger {
            temporary,
            if_not_exists,
            schema,
            name,
            time,
            event,
            columns,
            table,
            condition,
            body,
            written,
        })
    }

    /// `RAISE(IGNORE)` and `RAISE(action, message)`.
    fn raise(&mut self) -> Result<ExprId, Error> {
        use crate::ast::Raise;
        self.bump();
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let action = if self.eat_keyword(Keyword::Ignore) {
            Raise::Ignore
        } else if self.eat_keyword(Keyword::Rollback) {
            Raise::Rollback
        } else if self.eat_keyword(Keyword::Abort) {
            Raise::Abort
        } else {
            self.expect_keyword(Keyword::Fail, Expected::Action)?;
            Raise::Fail
        };
        let message = if action == Raise::Ignore {
            None
        } else {
            self.expect(Kind::Comma, Expected::Expression)?;
            Some(self.expression()?)
        };
        self.expect(Kind::Rp, Expected::CloseParen)?;
        self.node(Node::Raise { action, message })
    }

    /// One statement of a trigger's body.
    fn trigger_step(&mut self) -> Result<crate::ast::TriggerStep, Error> {
        use crate::ast::TriggerStep;
        if self.at_keyword(Keyword::Update) {
            return Ok(TriggerStep::Update(self.update()?));
        }
        if self.at_keyword(Keyword::Delete) {
            return Ok(TriggerStep::Delete(self.delete()?));
        }
        if self.at_keyword(Keyword::Insert) || self.at_keyword(Keyword::Replace) {
            return Ok(TriggerStep::Insert(self.insert()?));
        }
        if self.at_keyword(Keyword::Select)
            || self.at_keyword(Keyword::Values)
            || self.at_keyword(Keyword::With)
        {
            return Ok(TriggerStep::Select(self.select()?));
        }
        Err(self.error(self.peek(), Expected::Step))
    }

    /// A list of names in brackets.
    fn name_list(&mut self) -> Result<crate::ast::Range, Error> {
        let mut names = Vec::new();
        loop {
            names.push(self.name()?);
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(self.arena.push_names(&names))
    }

    /// What follows `DROP`: the word `TABLE`, `INDEX`, `VIEW` or
    /// `TRIGGER`, an optional `IF EXISTS`, and the name.
    fn drop_statement(&mut self) -> Result<crate::ast::Drop, Error> {
        let kind = if self.eat_keyword(Keyword::Table) {
            crate::ast::Dropped::Table
        } else if self.eat_keyword(Keyword::View) {
            crate::ast::Dropped::View
        } else if self.eat_keyword(Keyword::Trigger) {
            crate::ast::Dropped::Trigger
        } else {
            self.expect_keyword(Keyword::Index, Expected::Table)?;
            crate::ast::Dropped::Index
        };
        let if_exists = self.if_exists()?;
        let (schema, name) = self.qualified_name()?;
        Ok(crate::ast::Drop {
            kind,
            if_exists,
            schema,
            name,
        })
    }

    /// `IF EXISTS`.
    fn if_exists(&mut self) -> Result<bool, Error> {
        if !self.eat_keyword(Keyword::If) {
            return Ok(false);
        }
        self.expect_keyword(Keyword::Exists, Expected::Name)?;
        Ok(true)
    }

    /// `IF NOT EXISTS`.
    fn if_not_exists(&mut self) -> Result<bool, Error> {
        if !self.eat_keyword(Keyword::If) {
            return Ok(false);
        }
        self.expect_keyword(Keyword::Not, Expected::Name)?;
        self.expect_keyword(Keyword::Exists, Expected::Name)?;
        Ok(true)
    }

    /// `name` or `schema.name`.
    /// `PRAGMA [schema.]name [= value | (value)]`. The value is one
    /// word, one number or one string, which is every value the
    /// pragmas this crate answers take.
    fn pragma(&mut self) -> Result<crate::ast::Pragma, Error> {
        self.expect_keyword(Keyword::Pragma, Expected::Pragma)?;
        let (schema, name) = self.qualified_name()?;
        let value = if self.eat(Kind::Eq) {
            Some(self.pragma_value()?)
        } else if self.eat(Kind::Lp) {
            let value = self.pragma_value()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            Some(value)
        } else {
            None
        };
        Ok(crate::ast::Pragma {
            schema,
            name,
            value,
        })
    }

    /// What a `PRAGMA` is set to: a word, a number with an optional
    /// sign, or a string.
    fn pragma_value(&mut self) -> Result<Span, Error> {
        // A sign stands before a number, which is what the cache size
        // is written with.
        let sign = self.peek().filter(|token| token.kind == Kind::Minus);
        if sign.is_some() {
            self.bump();
        }
        let token = self.peek();
        let value = match token.map(|found| found.kind) {
            // `nmnum ::= plus_num | nm | ON | DELETE | DEFAULT`: three
            // words that are not names anywhere else name a setting
            // here, of which `DELETE` is a journal mode.
            Some(
                Kind::Integer
                | Kind::Float
                | Kind::Blob
                | Kind::Keyword(Keyword::On | Keyword::Delete | Keyword::Default),
            ) => {
                self.bump();
                Span::of(token.unwrap_or(EMPTY))
            }
            _ => self.name()?,
        };
        Ok(match sign {
            Some(sign) => Span {
                start: sign.start,
                len: value
                    .start
                    .saturating_add(value.len)
                    .saturating_sub(sign.start),
            },
            None => value,
        })
    }

    fn qualified_name(&mut self) -> Result<(Option<Span>, Span), Error> {
        let first = self.name()?;
        if self.eat(Kind::Dot) {
            return Ok((Some(first), self.name()?));
        }
        Ok((None, first))
    }

    /// What follows `CREATE TABLE`.
    fn create_table(&mut self, temporary: bool) -> Result<CreateTable, Error> {
        let if_not_exists = self.if_not_exists()?;
        let (schema, name) = self.qualified_name()?;
        if self.eat_keyword(Keyword::As) {
            let select = self.select()?;
            return Ok(CreateTable {
                temporary,
                if_not_exists,
                schema,
                name,
                body: TableBody::Select(select),
                options: TableOptions::default(),
                add_at: None,
            });
        }
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let (columns, constraints, add_at) = self.column_list()?;
        // A table with no constraint after its columns takes a column
        // added later in front of the bracket that closes them.
        let add_at = add_at.or_else(|| self.peek().map(|token| token.start));
        self.expect(Kind::Rp, Expected::CloseParen)?;
        let options = self.table_options()?;
        Ok(CreateTable {
            temporary,
            if_not_exists,
            schema,
            name,
            body: TableBody::Columns {
                columns,
                constraints,
            },
            options,
            add_at,
        })
    }

    /// The columns of a table, and the constraints that follow them.
    ///
    /// `add_at` is where a column added later is written, which is the
    /// comma the constraints begin after and nothing where the table
    /// has none; `sqlite3EndTable` takes the same token for
    /// `addColOffset`.
    fn column_list(&mut self) -> Result<(Range, Range, Option<usize>), Error> {
        let mut columns = Vec::new();
        let mut constraints = Vec::new();
        let mut add_at = None;
        loop {
            columns.push(self.column_def()?);
            let comma = self.peek().map(|token| token.start);
            if !self.eat(Kind::Comma) {
                break;
            }
            if self.at_table_constraint() {
                add_at = comma;
                // `tconscomma ::= COMMA. | .` — the comma between two
                // constraints may be left out, and one with nothing
                // after it is left where it stands so that the bracket
                // refuses it.
                loop {
                    constraints.push(self.table_constraint()?);
                    let comma = self.at(Kind::Comma);
                    if comma && !self.constraint_at(1) {
                        break;
                    }
                    if comma {
                        self.bump();
                    }
                    if !self.at_table_constraint() {
                        break;
                    }
                }
                break;
            }
        }
        let columns = self.arena.push_columns(&columns);
        Ok((
            columns,
            self.arena.push_table_constraints(&constraints),
            add_at,
        ))
    }

    /// Whether what stands here belongs to the table rather than to a
    /// column. None of the five words may be a name, so looking at one
    /// is enough.
    fn at_table_constraint(&self) -> bool {
        self.constraint_at(0)
    }

    /// Whether the token `at` tokens ahead begins a table constraint.
    fn constraint_at(&self, at: usize) -> bool {
        matches!(
            self.ahead(at).map(|token| token.kind),
            Some(Kind::Keyword(
                Keyword::Constraint
                    | Keyword::Primary
                    | Keyword::Unique
                    | Keyword::Check
                    | Keyword::Foreign
            ))
        )
    }

    /// One column: a name, a type where one is written, and whatever
    /// follows.
    fn column_def(&mut self) -> Result<ColumnDef, Error> {
        let start = self.peek().map_or(self.end, |token| token.start);
        let name = self.name()?;
        let ty = if self.at_type_word() {
            Some(self.type_name()?)
        } else {
            None
        };
        let mut constraints = Vec::new();
        while let Some(constraint) = self.column_constraint()? {
            constraints.push(constraint);
        }
        let constraints = self.arena.push_column_constraints(&constraints);
        Ok(ColumnDef {
            written: Span {
                start,
                len: self.end.saturating_sub(start),
            },
            name,
            ty,
            constraints,
        })
    }

    /// Whether a type name stands here.
    ///
    /// A type is a run of words, and every word a constraint begins with
    /// is a word that may not be a name — but for `GENERATED`, which is
    /// a type name unless `ALWAYS AS` follows it.
    fn at_type_word(&self) -> bool {
        let Some(token) = self.peek() else {
            return false;
        };
        match token.kind {
            Kind::Id | Kind::String => true,
            Kind::Keyword(Keyword::Generated) => !matches!(
                (
                    self.ahead(1).map(|token| token.kind),
                    self.ahead(2).map(|token| token.kind)
                ),
                (
                    Some(Kind::Keyword(Keyword::Always)),
                    Some(Kind::Keyword(Keyword::As))
                )
            ),
            Kind::Keyword(keyword) => keyword.can_be_name(),
            _ => false,
        }
    }

    /// One thing that follows a column, or nothing where the column is
    /// finished.
    fn column_constraint(&mut self) -> Result<Option<ColumnConstraint>, Error> {
        let Some(token) = self.peek() else {
            return Ok(None);
        };
        let Kind::Keyword(keyword) = token.kind else {
            return Ok(None);
        };
        Ok(Some(match keyword {
            Keyword::Constraint => {
                self.bump();
                ColumnConstraint::Named(self.name()?)
            }
            Keyword::Primary => {
                self.bump();
                self.expect_keyword(Keyword::Key, Expected::Key)?;
                let order = self.sort_order();
                let conflict = self.conflict_clause()?;
                let autoincrement = self.eat_keyword(Keyword::Autoincrement);
                ColumnConstraint::PrimaryKey {
                    order,
                    autoincrement,
                    conflict,
                }
            }
            Keyword::Not => {
                self.bump();
                if self.eat_keyword(Keyword::Deferrable) {
                    self.initially()?;
                    return Ok(Some(ColumnConstraint::Null(Conflict::Unspecified)));
                }
                self.expect_keyword(Keyword::Null, Expected::Null)?;
                ColumnConstraint::NotNull(self.conflict_clause()?)
            }
            Keyword::Null => {
                self.bump();
                ColumnConstraint::Null(self.conflict_clause()?)
            }
            Keyword::Unique => {
                self.bump();
                ColumnConstraint::Unique(self.conflict_clause()?)
            }
            Keyword::Check => {
                self.bump();
                let (value, text) = self.checked()?;
                ColumnConstraint::Check { value, text }
            }
            Keyword::Default => {
                self.bump();
                let (value, text) = self.default_value()?;
                ColumnConstraint::Default { value, text }
            }
            Keyword::Collate => {
                self.bump();
                ColumnConstraint::Collate(self.name()?)
            }
            Keyword::References => {
                self.bump();
                ColumnConstraint::References(self.foreign_key()?)
            }
            Keyword::Deferrable => {
                self.bump();
                self.initially()?;
                ColumnConstraint::Null(Conflict::Unspecified)
            }
            Keyword::Generated | Keyword::As => {
                self.bump();
                if keyword == Keyword::Generated {
                    self.expect_keyword(Keyword::Always, Expected::As)?;
                    self.expect_keyword(Keyword::As, Expected::As)?;
                }
                let value = self.parenthesized_expression()?;
                let kind = if self.at_name() {
                    Some(self.name()?)
                } else {
                    None
                };
                ColumnConstraint::Generated { value, kind }
            }
            _ => return Ok(None),
        }))
    }

    /// `( expression )` with the text between the brackets, which is
    /// what `CHECK constraint failed:` writes where the constraint
    /// carries no name.
    fn checked(&mut self) -> Result<(ExprId, Span), Error> {
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let start = self.peek().map_or(0, |token| token.start);
        let value = self.expression()?;
        let end = self.peek().map_or(start, |token| token.start);
        self.expect(Kind::Rp, Expected::CloseParen)?;
        Ok((
            value,
            Span {
                start,
                len: end.saturating_sub(start),
            },
        ))
    }

    /// `( expr )`, which is what a `CHECK` and a `DEFAULT` are written
    /// with.
    fn parenthesized_expression(&mut self) -> Result<ExprId, Error> {
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let expr = self.expression()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        Ok(expr)
    }

    /// What a column falls back to: a constant, a signed constant, an
    /// expression in brackets, or a bare word, which SQLite reads as
    /// text.
    fn default_value(&mut self) -> Result<(ExprId, Span), Error> {
        if self.at(Kind::Lp) {
            self.bump();
            let start = self.peek().map_or(0, |token| token.start);
            let value = self.expression()?;
            let end = self.peek().map_or(start, |token| token.start);
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return Ok((
                value,
                Span {
                    start,
                    len: end.saturating_sub(start),
                },
            ));
        }
        let from = self.peek().map_or(0, |token| token.start);
        let negative = if self.eat(Kind::Minus) {
            true
        } else {
            self.eat(Kind::Plus);
            false
        };
        let Some(token) = self.peek() else {
            return Err(self.error(None, Expected::Expression));
        };
        let span = Span::of(token);
        let literal = match token.kind {
            Kind::Integer => Literal::Integer(span),
            Kind::QNumber => self.separated(token)?,
            Kind::Float => Literal::Float(span),
            Kind::String | Kind::Id => Literal::Text(span),
            Kind::Blob => Literal::Blob(span),
            Kind::Keyword(Keyword::Null) => Literal::Null,
            Kind::Keyword(Keyword::CurrentTime) => Literal::CurrentTime(CurrentTime::Time),
            Kind::Keyword(Keyword::CurrentDate) => Literal::CurrentTime(CurrentTime::Date),
            Kind::Keyword(Keyword::CurrentTimestamp) => {
                Literal::CurrentTime(CurrentTime::Timestamp)
            }
            Kind::Keyword(keyword) if keyword.can_be_name() => Literal::Text(span),
            _ => return Err(self.error(Some(token), Expected::Expression)),
        };
        self.bump();
        let text = Span {
            start: from,
            len: span.start.saturating_add(span.len).saturating_sub(from),
        };
        let value = self.literal(literal)?;
        if negative {
            return Ok((self.unary(UnaryOp::Negate, value)?, text));
        }
        Ok((value, text))
    }

    /// `ON CONFLICT` and what to do.
    fn conflict_clause(&mut self) -> Result<Conflict, Error> {
        if !self.at_keyword(Keyword::On) {
            return Ok(Conflict::Unspecified);
        }
        if !matches!(
            self.ahead(1).map(|token| token.kind),
            Some(Kind::Keyword(Keyword::Conflict))
        ) {
            return Ok(Conflict::Unspecified);
        }
        self.bump();
        self.bump();
        let Some(token) = self.peek() else {
            return Err(self.error(None, Expected::Conflict));
        };
        let conflict = match token.kind {
            Kind::Keyword(Keyword::Rollback) => Conflict::Rollback,
            Kind::Keyword(Keyword::Abort) => Conflict::Abort,
            Kind::Keyword(Keyword::Fail) => Conflict::Fail,
            Kind::Keyword(Keyword::Ignore) => Conflict::Ignore,
            Kind::Keyword(Keyword::Replace) => Conflict::Replace,
            _ => return Err(self.error(Some(token), Expected::Conflict)),
        };
        self.bump();
        Ok(conflict)
    }

    /// `INITIALLY DEFERRED` or `INITIALLY IMMEDIATE`, and answers
    /// whether the check waits.
    fn initially(&mut self) -> Result<bool, Error> {
        if !self.eat_keyword(Keyword::Initially) {
            return Ok(false);
        }
        if self.eat_keyword(Keyword::Deferred) {
            return Ok(true);
        }
        self.expect_keyword(Keyword::Immediate, Expected::Action)?;
        Ok(false)
    }

    /// What follows `REFERENCES`.
    fn foreign_key(&mut self) -> Result<Foreign, Error> {
        let table = self.name()?;
        let columns = if self.eat(Kind::Lp) {
            let names = self.indexed_names()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            names
        } else {
            Range::default()
        };
        let mut on_delete = Action::Unspecified;
        let mut on_update = Action::Unspecified;
        loop {
            if self.eat_keyword(Keyword::Match) {
                self.name()?;
                continue;
            }
            if !self.at_keyword(Keyword::On) {
                break;
            }
            self.bump();
            if self.eat_keyword(Keyword::Insert) {
                self.reference_action()?;
                continue;
            }
            if self.eat_keyword(Keyword::Delete) {
                on_delete = self.reference_action()?;
                continue;
            }
            self.expect_keyword(Keyword::Update, Expected::Action)?;
            on_update = self.reference_action()?;
        }
        let mut deferred = false;
        if self.eat_keyword(Keyword::Deferrable) {
            deferred = self.initially()?;
        } else if self.at_keyword(Keyword::Not)
            && matches!(
                self.ahead(1).map(|token| token.kind),
                Some(Kind::Keyword(Keyword::Deferrable))
            )
        {
            self.bump();
            self.bump();
            self.initially()?;
        }
        Ok(Foreign {
            table,
            columns,
            on_delete,
            on_update,
            deferred,
        })
    }

    /// What a foreign key does to a row.
    fn reference_action(&mut self) -> Result<Action, Error> {
        if self.eat_keyword(Keyword::Set) {
            if self.eat_keyword(Keyword::Null) {
                return Ok(Action::SetNull);
            }
            self.expect_keyword(Keyword::Default, Expected::Action)?;
            return Ok(Action::SetDefault);
        }
        if self.eat_keyword(Keyword::Cascade) {
            return Ok(Action::Cascade);
        }
        if self.eat_keyword(Keyword::Restrict) {
            return Ok(Action::Restrict);
        }
        self.expect_keyword(Keyword::No, Expected::Action)?;
        self.expect_keyword(Keyword::Action, Expected::Action)?;
        Ok(Action::NoAction)
    }

    /// A list of names, each with the collation and the order the
    /// grammar allows after it and nothing reads.
    fn indexed_names(&mut self) -> Result<Range, Error> {
        let mut names = Vec::new();
        loop {
            names.push(self.name()?);
            if self.eat_keyword(Keyword::Collate) {
                self.name()?;
            }
            self.sort_order();
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(self.arena.push_names(&names))
    }

    /// One thing that follows the columns of a table.
    fn table_constraint(&mut self) -> Result<TableConstraint, Error> {
        if self.eat_keyword(Keyword::Constraint) {
            return Ok(TableConstraint::Named(self.name()?));
        }
        if self.eat_keyword(Keyword::Primary) {
            self.expect_keyword(Keyword::Key, Expected::Key)?;
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let columns = self.indexed_list()?;
            let autoincrement = self.eat_keyword(Keyword::Autoincrement);
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return Ok(TableConstraint::PrimaryKey {
                columns,
                autoincrement,
                conflict: self.conflict_clause()?,
            });
        }
        if self.eat_keyword(Keyword::Unique) {
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let columns = self.indexed_list()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            return Ok(TableConstraint::Unique {
                columns,
                conflict: self.conflict_clause()?,
            });
        }
        if self.eat_keyword(Keyword::Check) {
            let (value, text) = self.checked()?;
            self.conflict_clause()?;
            return Ok(TableConstraint::Check { value, text });
        }
        self.expect_keyword(Keyword::Foreign, Expected::Key)?;
        self.expect_keyword(Keyword::Key, Expected::Key)?;
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let columns = self.indexed_names()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        self.expect_keyword(Keyword::References, Expected::Name)?;
        Ok(TableConstraint::ForeignKey {
            columns,
            foreign: self.foreign_key()?,
        })
    }

    /// `WITHOUT ROWID` and `STRICT`, in either order and as many as are
    /// written.
    fn table_options(&mut self) -> Result<TableOptions, Error> {
        let mut options = TableOptions::default();
        loop {
            if self.eat_keyword(Keyword::Without) {
                let name = self.name()?;
                if !name.text(self.sql).eq_ignore_ascii_case(b"rowid") {
                    return Err(Error {
                        at: name.start,
                        len: name.len,
                        expected: Expected::TableOption,
                    });
                }
                options.without_rowid = true;
            } else if self.at_name() {
                let name = self.name()?;
                if !name.text(self.sql).eq_ignore_ascii_case(b"strict") {
                    return Err(Error {
                        at: name.start,
                        len: name.len,
                        expected: Expected::TableOption,
                    });
                }
                options.strict = true;
            } else {
                break;
            }
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(options)
    }

    /// What follows `CREATE [UNIQUE] INDEX`.
    fn create_index(&mut self, unique: bool) -> Result<CreateIndex, Error> {
        let if_not_exists = self.if_not_exists()?;
        let (schema, name) = self.qualified_name()?;
        self.expect_keyword(Keyword::On, Expected::On)?;
        let table = self.name()?;
        self.expect(Kind::Lp, Expected::OpenParen)?;
        let columns = self.indexed_list()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        let filter = if self.eat_keyword(Keyword::Where) {
            Some(self.expression()?)
        } else {
            None
        };
        Ok(CreateIndex {
            unique,
            if_not_exists,
            schema,
            name,
            table,
            columns,
            filter,
        })
    }

    /// Reads one expression, and nothing after it.
    ///
    /// # Errors
    ///
    /// Where the tokens are not an expression, or where something
    /// follows the expression.
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
            Kind::Keyword(Keyword::Raise) => self.raise(),
            Kind::Lp => self.parenthesized(),
            Kind::Integer => {
                self.bump();
                self.literal(Literal::Integer(Span::of(token)))
            }
            Kind::QNumber => {
                let held = self.separated(token)?;
                self.bump();
                self.literal(held)
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
            // `sqlite3RunParser` takes `#1` for a register of the
            // routine it writes and no statement may carry one, so the
            // token is read and then refused.
            Kind::Variable if Span::of(token).text(self.sql).first() == Some(&b'#') => {
                Err(self.error(Some(token), Expected::Expression))
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
        // `f(...) WITHIN GROUP (ORDER BY Y)` is `f(Y,...)`, which is
        // `sqlite3ExprAddFunctionOrderBy` putting the one term of the
        // clause in front of the arguments the call was written with.
        if self.eat_keyword(Keyword::Within) {
            // `sqlite3ExprAddFunctionOrderBy` refuses a `DISTINCT` on an
            // ordered-set aggregate, naming the function.
            if distinct {
                return Err(Error {
                    at: name.start,
                    len: name.len,
                    expected: Expected::OrderedDistinct,
                });
            }
            self.expect_keyword(Keyword::Group, Expected::Group)?;
            self.expect(Kind::Lp, Expected::OpenParen)?;
            self.expect_keyword(Keyword::Order, Expected::Order)?;
            self.expect_keyword(Keyword::By, Expected::By)?;
            let held = self.expression()?;
            // The term carries an order of its own, which the aggregate
            // reads the values in and this crate answers the same way
            // either way.
            let _ = self.eat_keyword(Keyword::Asc) || self.eat_keyword(Keyword::Desc);
            self.expect(Kind::Rp, Expected::CloseParen)?;
            args.insert(0, held);
        }
        let args = self.arena.push_children(&args);
        let filter = if self.eat_keyword(Keyword::Filter) {
            self.expect(Kind::Lp, Expected::OpenParen)?;
            self.expect_keyword(Keyword::Where, Expected::Where)?;
            let held = self.expression()?;
            self.expect(Kind::Rp, Expected::CloseParen)?;
            Some(held)
        } else {
            None
        };
        if self.eat_keyword(Keyword::Over) {
            let window = self.over()?;
            return self.node(Node::Over {
                name,
                args,
                distinct,
                star,
                filter,
                window,
            });
        }
        self.node(Node::Call {
            name,
            args,
            distinct,
            star,
            filter,
        })
    }

    /// What follows `OVER`: the name of a window a `WINDOW` clause
    /// defines, or a window written out in brackets.
    fn over(&mut self) -> Result<WindowId, Error> {
        if !self.at(Kind::Lp) {
            let base = self.name()?;
            return Ok(self.arena.push_window(Window {
                base: Some(base),
                named: true,
                partition: Range::default(),
                order: Range::default(),
                frame: None,
            }));
        }
        self.bump();
        self.window_definition()
    }

    /// The body of a window, the `(` of it already taken:
    /// `[name] [PARTITION BY exprs] [ORDER BY terms] [frame]`.
    fn window_definition(&mut self) -> Result<WindowId, Error> {
        // A window written in brackets may name one to build on, and
        // the name is told from `PARTITION` and `ORDER` by being
        // neither.
        let base = if self.at(Kind::Rp)
            || self.at_keyword(Keyword::Partition)
            || self.at_keyword(Keyword::Order)
            || self.at_keyword(Keyword::Rows)
            || self.at_keyword(Keyword::Range)
            || self.at_keyword(Keyword::Groups)
        {
            None
        } else {
            Some(self.name()?)
        };
        let partition = if self.eat_keyword(Keyword::Partition) {
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
        let order = self.order_by()?;
        let frame = self.frame()?;
        self.expect(Kind::Rp, Expected::CloseParen)?;
        Ok(self.arena.push_window(Window {
            base,
            named: false,
            partition,
            order,
            frame,
        }))
    }

    /// `(ROWS|RANGE|GROUPS) (BETWEEN bound AND bound | bound)
    /// [EXCLUDE ...]`, or none of it.
    fn frame(&mut self) -> Result<Option<Frames>, Error> {
        let kind = if self.eat_keyword(Keyword::Rows) {
            Frame::Rows
        } else if self.eat_keyword(Keyword::Range) {
            Frame::Range
        } else if self.eat_keyword(Keyword::Groups) {
            Frame::Groups
        } else {
            return Ok(None);
        };
        let (start, end) = if self.eat_keyword(Keyword::Between) {
            let start = self.bound(true)?;
            self.expect_keyword(Keyword::And, Expected::And)?;
            (start, self.bound(false)?)
        } else {
            // One bound written alone is where the frame begins, and
            // the frame ends at the row the walk stands on.
            (self.bound(true)?, Bound::CurrentRow)
        };
        // `sqlite3WindowAlloc` refuses a frame that ends before it
        // begins, which is `unsupported frame specification`.
        let backwards = matches!(
            (start, end),
            (Bound::CurrentRow, Bound::Preceding(_))
                | (Bound::Following(_), Bound::Preceding(_) | Bound::CurrentRow)
        );
        if backwards {
            return Err(self.error(self.peek(), Expected::Bound));
        }
        let exclude = if self.eat_keyword(Keyword::Exclude) {
            self.exclude()?
        } else {
            Exclude::NoOthers
        };
        Ok(Some(Frames {
            kind,
            start,
            end,
            exclude,
        }))
    }

    /// One end of a frame, `begins` saying whether it is the end the
    /// frame begins at.
    ///
    /// `frame_bound_s` takes `UNBOUNDED PRECEDING` and `frame_bound_e`
    /// takes `UNBOUNDED FOLLOWING`, so a frame that begins unbounded
    /// forwards or ends unbounded backwards is a refusal of the
    /// grammar.
    fn bound(&mut self, begins: bool) -> Result<Bound, Error> {
        if self.eat_keyword(Keyword::Unbounded) {
            if begins {
                self.expect_keyword(Keyword::Preceding, Expected::Bound)?;
                return Ok(Bound::UnboundedPreceding);
            }
            self.expect_keyword(Keyword::Following, Expected::Bound)?;
            return Ok(Bound::UnboundedFollowing);
        }
        if self.eat_keyword(Keyword::Current) {
            self.expect_keyword(Keyword::Row, Expected::Row)?;
            return Ok(Bound::CurrentRow);
        }
        let count = self.expression()?;
        if self.eat_keyword(Keyword::Preceding) {
            return Ok(Bound::Preceding(count));
        }
        self.expect_keyword(Keyword::Following, Expected::Bound)?;
        Ok(Bound::Following(count))
    }

    /// What follows `EXCLUDE`.
    fn exclude(&mut self) -> Result<Exclude, Error> {
        if self.eat_keyword(Keyword::No) {
            self.expect_keyword(Keyword::Others, Expected::Exclude)?;
            return Ok(Exclude::NoOthers);
        }
        if self.eat_keyword(Keyword::Current) {
            self.expect_keyword(Keyword::Row, Expected::Row)?;
            return Ok(Exclude::CurrentRow);
        }
        if self.eat_keyword(Keyword::Group) {
            return Ok(Exclude::Group);
        }
        self.expect_keyword(Keyword::Ties, Expected::Exclude)?;
        Ok(Exclude::Ties)
    }

    /// `WINDOW name AS (window), name AS (window)`, or none of it.
    fn window_clause(&mut self) -> Result<Range, Error> {
        if !self.eat_keyword(Keyword::Window) {
            return Ok(Range::default());
        }
        let mut windows = Vec::new();
        loop {
            let name = self.name()?;
            self.expect_keyword(Keyword::As, Expected::As)?;
            self.expect(Kind::Lp, Expected::OpenParen)?;
            let window = self.window_definition()?;
            windows.push(NamedWindow { name, window });
            if !self.eat(Kind::Comma) {
                break;
            }
        }
        Ok(self.arena.push_named_windows(&windows))
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
            // `idj ::= ID | INDEXED | JOIN_KW`: a word that names a
            // join, and the word `INDEXED`, are names where a name is
            // asked for outright. They are not names where one is only
            // allowed, which is why an alias reads them differently.
            Some(Kind::Keyword(keyword))
                if keyword.can_be_name()
                    || is_join_word(keyword)
                    || keyword == Keyword::Indexed =>
            {
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
    fn peek(&self) -> Option<Token> {
        self.ahead
            .first()
            .copied()
            .flatten()
            .map(|token| self.reclassified(token))
    }

    /// `WINDOW`, `OVER` and `FILTER` are keywords only where the words
    /// around them allow no other reading, which is
    /// `analyzeWindowKeyword`, `analyzeOverKeyword` and
    /// `analyzeFilterKeyword` of `tokenize.c`; anywhere else each of
    /// the three is a name.
    fn reclassified(&self, token: Token) -> Token {
        let Kind::Keyword(word @ (Keyword::Window | Keyword::Over | Keyword::Filter)) = token.kind
        else {
            return token;
        };
        let next = self.ahead.get(1).copied().flatten().map(|ahead| ahead.kind);
        let held = match word {
            // `WINDOW name AS` names a window, and the words are read
            // as they come out of the lexer, so a name that is itself a
            // keyword ends the clause.
            Keyword::Window => {
                next == Some(Kind::Id)
                    && self.ahead.get(2).copied().flatten().map(|ahead| ahead.kind)
                        == Some(Kind::Keyword(Keyword::As))
            }
            // `OVER` takes a window or its name, and both follow the
            // bracket of a call.
            Keyword::Over => {
                self.last == Some(Kind::Rp) && matches!(next, Some(Kind::Lp | Kind::Id))
            }
            _ => self.last == Some(Kind::Rp) && next == Some(Kind::Lp),
        };
        if held {
            token
        } else {
            Token {
                kind: Kind::Id,
                ..token
            }
        }
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
        self.end = token.map_or(self.end, |token| token.start.saturating_add(token.len));
        self.last = self
            .ahead
            .first()
            .copied()
            .flatten()
            .map(|ahead| ahead.kind);
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
    ///
    /// A token the tokenizer read as no token at all is refused as
    /// itself whatever was wanted there, which is the `TK_ILLEGAL` of
    /// `sqlite3RunParser`.
    fn error(&self, token: Option<Token>, expected: Expected) -> Error {
        let expected = match token {
            Some(found) if found.kind == Kind::Illegal => Expected::Unrecognized,
            _ => expected,
        };
        Error {
            at: token.map_or(self.sql.len(), |found| found.start),
            len: token.map_or(0, |found| found.len),
            expected,
        }
    }

    /// The table an `UPDATE` changes named again in its `FROM`, which
    /// `sqlite3Update` refuses because the clause would join the table
    /// with itself.
    ///
    /// Reading the sources costs O(n) in them.
    ///
    /// # Errors
    ///
    /// [`Expected::TargetInFrom`] where a source or its alias names the
    /// table the statement changes.
    fn refused_target(&self, target: (Option<Span>, Span), tables: Range) -> Result<(), Error> {
        let (schema, target) = target;
        let held = self.schema_named(schema);
        let wanted = crate::schema::dequote(target.text(self.sql));
        for source in self.arena.sources(tables) {
            let (named, under) = match source.kind {
                SourceKind::Table { schema, name, .. } => (Some(name), self.schema_named(schema)),
                _ => (None, held.clone()),
            };
            // A table of another schema is another object, whatever it
            // is named.
            if under != held {
                continue;
            }
            if let Some(span) = source.alias.or(named)
                && crate::schema::dequote(span.text(self.sql)).eq_ignore_ascii_case(&wanted)
            {
                return Err(Error {
                    at: span.start,
                    len: span.len,
                    expected: Expected::TargetInFrom,
                });
            }
        }
        Ok(())
    }

    /// The schema a name stands under, which is `main` where the
    /// statement wrote none.
    fn schema_named(&self, schema: Option<Span>) -> Vec<u8> {
        schema.map_or_else(
            || b"main".to_vec(),
            |span| {
                let mut name = crate::schema::dequote(span.text(self.sql));
                name.make_ascii_lowercase();
                name
            },
        )
    }

    /// A number written with digit separators, as the literal it is
    /// once they are taken out, which is `sqlite3DequoteNumber`: a
    /// separator lies between two digits, a `.` or an `e` makes the
    /// number a real, and a hex literal is a whole number whatever it
    /// holds.
    ///
    /// Reading the token costs O(n) in its bytes.
    ///
    /// # Errors
    ///
    /// [`Expected::Unrecognized`] where a separator lies anywhere else.
    fn separated(&self, token: Token) -> Result<Literal, Error> {
        let span = Span::of(token);
        let text = span.text(self.sql);
        let hex = text.first() == Some(&b'0') && matches!(text.get(1), Some(b'x' | b'X'));
        let digit = if hex {
            crate::token::is_hex
        } else {
            crate::token::is_digit
        };
        let mut real = false;
        for (at, byte) in text.iter().enumerate() {
            if *byte != b'_' {
                real = real || matches!(byte, b'e' | b'E' | b'.');
                continue;
            }
            let before = at.checked_sub(1).and_then(|at| text.get(at)).copied();
            let after = text.get(at.saturating_add(1)).copied();
            if !before.is_some_and(digit) || !after.is_some_and(digit) {
                return Err(Error {
                    at: token.start,
                    len: token.len,
                    expected: Expected::Unrecognized,
                });
            }
        }
        Ok(if real && !hex {
            Literal::Float(span)
        } else {
            Literal::Integer(span)
        })
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
/// `JT_NATURAL` of `sqlite3JoinType`.
const JOIN_NATURAL: u32 = 1;

/// `JT_LEFT`.
const JOIN_LEFT: u32 = 2;

/// `JT_RIGHT`.
const JOIN_RIGHT: u32 = 4;

/// `JT_OUTER`.
const JOIN_OUTER: u32 = 8;

/// `JT_INNER`.
const JOIN_INNER: u32 = 16;

/// `JT_CROSS`.
const JOIN_CROSS: u32 = 32;

/// `JT_ERROR`: a word no join is written with.
const JOIN_OTHER: u32 = 64;

/// The mask one word of a join type carries, which is the row of
/// `aKeyword` in `sqlite3JoinType` that names it.
const fn join_mask(token: Token) -> u32 {
    match token.kind {
        Kind::Keyword(Keyword::Natural) => JOIN_NATURAL,
        Kind::Keyword(Keyword::Left) => JOIN_LEFT | JOIN_OUTER,
        Kind::Keyword(Keyword::Outer) => JOIN_OUTER,
        Kind::Keyword(Keyword::Right) => JOIN_RIGHT | JOIN_OUTER,
        Kind::Keyword(Keyword::Full) => JOIN_LEFT | JOIN_RIGHT | JOIN_OUTER,
        Kind::Keyword(Keyword::Inner) => JOIN_INNER,
        Kind::Keyword(Keyword::Cross) => JOIN_INNER | JOIN_CROSS,
        // A word the tokenizer did not read as one of SQL's own is one
        // no join is written with.
        _ => JOIN_OTHER,
    }
}

/// The span that covers both words of a join type.
const fn join_span(first: Span, last: Span) -> Span {
    Span {
        start: first.start,
        len: last
            .start
            .saturating_add(last.len)
            .saturating_sub(first.start),
    }
}

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

/// Whether `sql` holds no statement, which a text of comments,
/// whitespace and semicolons alone does.
///
/// `sqlite3_prepare_v2` answers no statement for such a text, so
/// `sqlite3_exec` runs nothing and answers no row. Reading the text
/// costs O(n) in its bytes.
#[must_use]
pub fn blank(sql: &[u8]) -> bool {
    let mut parser = Parser::new(sql);
    while parser.eat(Kind::Semi) {}
    parser.peek().is_none()
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

/// Reads one `CREATE TABLE` or `CREATE INDEX` out of `sql`.
///
/// # Errors
///
/// Where the statement is not one of the two.
pub fn definition(sql: &[u8]) -> Result<(Arena, Definition), Error> {
    let mut parser = Parser::new(sql);
    let root = parser.only_definition()?;
    Ok((parser.into_arena(), root))
}

/// Reads one `PRAGMA` out of `sql`.
///
/// # Errors
///
/// Where the statement is not a `PRAGMA`, or where more is written
/// after it than a semicolon.
pub fn pragma(sql: &[u8]) -> Result<crate::ast::Pragma, Error> {
    let mut parser = Parser::new(sql);
    let read = parser.pragma()?;
    parser.eat(Kind::Semi);
    if let Some(token) = parser.peek() {
        return Err(parser.error(Some(token), Expected::Eof));
    }
    Ok(read)
}

/// Reads one `BEGIN`, `COMMIT` or `ROLLBACK` out of `sql`.
///
/// # Errors
///
/// Where the statement bounds no transaction.
pub fn transaction(sql: &[u8]) -> Result<crate::ast::Transaction, Error> {
    let mut parser = Parser::new(sql);
    let read = parser.transaction()?;
    parser.eat(Kind::Semi);
    if let Some(token) = parser.peek() {
        return Err(parser.error(Some(token), Expected::Eof));
    }
    Ok(read)
}

/// Reads one `SAVEPOINT`, `RELEASE` or `ROLLBACK TO` out of `sql`.
///
/// # Errors
///
/// Where the statement is none of the three, or where more is written
/// after it than a semicolon.
pub fn savepoint(sql: &[u8]) -> Result<crate::ast::Savepoint, Error> {
    let mut parser = Parser::new(sql);
    let read = parser.savepoint()?;
    parser.eat(Kind::Semi);
    if let Some(token) = parser.peek() {
        return Err(parser.error(Some(token), Expected::Eof));
    }
    Ok(read)
}

/// Reads one `ANALYZE` out of `sql`.
///
/// # Errors
///
/// Where the statement is not an `ANALYZE`, or where more is written
/// after it than a semicolon.
pub fn analyze(sql: &[u8]) -> Result<crate::ast::Analyze, Error> {
    let mut parser = Parser::new(sql);
    parser.expect_keyword(Keyword::Analyze, Expected::Analyze)?;
    let named = parser.peek().is_some_and(|token| token.kind != Kind::Semi);
    let (schema, name) = if named {
        let (schema, name) = parser.qualified_name()?;
        (schema, Some(name))
    } else {
        (None, None)
    };
    parser.eat(Kind::Semi);
    if let Some(token) = parser.peek() {
        return Err(parser.error(Some(token), Expected::Eof));
    }
    Ok(crate::ast::Analyze { schema, name })
}

/// Reads one `REINDEX` out of `sql`.
///
/// # Errors
///
/// Where the statement is not a `REINDEX`, or where more is written
/// after it than a semicolon.
pub fn reindex(sql: &[u8]) -> Result<crate::ast::Reindex, Error> {
    let mut parser = Parser::new(sql);
    parser.expect_keyword(Keyword::Reindex, Expected::Reindex)?;
    let named = parser.peek().is_some_and(|token| token.kind != Kind::Semi);
    let (schema, name) = if named {
        let (schema, name) = parser.qualified_name()?;
        (schema, Some(name))
    } else {
        (None, None)
    };
    parser.eat(Kind::Semi);
    if let Some(token) = parser.peek() {
        return Err(parser.error(Some(token), Expected::Eof));
    }
    Ok(crate::ast::Reindex { schema, name })
}

/// Reads one statement that changes a database out of `sql`.
///
/// # Errors
///
/// Where the statement is not one this crate writes.
pub fn change(sql: &[u8]) -> Result<(Arena, Change), Error> {
    let mut parser = Parser::new(sql);
    let root = parser.only_change()?;
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
