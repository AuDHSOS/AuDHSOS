// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The syntax tree, as an arena.
//!
//! Nodes live in one vector and refer to each other by index rather than
//! by pointer. Three things follow, and all three are why it is built this
//! way: a tree is one allocation that grows rather than one per node, a
//! node is four words that a cache line holds several of, and a subtree is
//! an index, so nothing in the tree borrows anything else in it.
//!
//! Names and literals are not copied either: a node carries the span of
//! the text it was read from, and the text is the statement.

use alloc::vec::Vec;

use crate::token::Token;

/// Where something is in the statement it was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Span {
    /// Where it begins.
    pub start: usize,
    /// How many bytes it is.
    pub len: usize,
}

impl Span {
    /// The span of a token.
    #[must_use]
    pub const fn of(token: Token) -> Self {
        Span {
            start: token.start,
            len: token.len,
        }
    }

    /// The bytes, out of the statement they were read from.
    #[must_use]
    pub fn text<'a>(&self, sql: &'a [u8]) -> &'a [u8] {
        sql.get(self.start..self.start.saturating_add(self.len))
            .unwrap_or_default()
    }
}

/// One node of a tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExprId(u32);

/// A run of nodes: the arguments of a call, the list of an `IN`, the
/// branches of a `CASE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Range {
    /// Where the run begins in the arena's list of children.
    start: u32,
    /// How many nodes it holds.
    len: u32,
}

impl Range {
    /// How many nodes the run holds.
    #[must_use]
    pub fn len(self) -> usize {
        usize::try_from(self.len).unwrap_or(usize::MAX)
    }

    /// Whether the run holds none.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// What a value in a statement can be written as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Literal {
    /// `NULL`.
    Null,
    /// `CURRENT_TIME`, `CURRENT_DATE` or `CURRENT_TIMESTAMP`.
    CurrentTime(CurrentTime),
    /// A whole number, as it was written.
    Integer(Span),
    /// A number with a fraction or an exponent, as it was written.
    Float(Span),
    /// `'text'`, quotes included.
    Text(Span),
    /// `x'00'`, quotes included.
    Blob(Span),
}

/// Which of the three clock literals was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CurrentTime {
    /// `CURRENT_TIME`.
    Time,
    /// `CURRENT_DATE`.
    Date,
    /// `CURRENT_TIMESTAMP`.
    Timestamp,
}

/// An operator with one operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `NOT x`.
    Not,
    /// `-x`.
    Negate,
    /// `+x`, which SQLite keeps and does nothing with.
    Identity,
    /// `~x`.
    BitNot,
    /// `x ISNULL`, and `x IS NULL`.
    IsNull,
    /// `x NOTNULL`, and `x NOT NULL`.
    NotNull,
}

/// An operator with two operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    /// `OR`.
    Or,
    /// `AND`.
    And,
    /// `=` or `==`.
    Eq,
    /// `<>` or `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `IS`.
    Is,
    /// `IS NOT`.
    IsNot,
    /// `+`.
    Add,
    /// `-`.
    Subtract,
    /// `*`.
    Multiply,
    /// `/`.
    Divide,
    /// `%`.
    Modulo,
    /// `||`.
    Concat,
    /// `->`.
    Extract,
    /// `->>`.
    ExtractText,
    /// `&`.
    BitAnd,
    /// `|`.
    BitOr,
    /// `<<`.
    LShift,
    /// `>>`.
    RShift,
}

/// The pattern operators, which are functions wearing an operator's
/// clothes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LikeOp {
    /// `LIKE`.
    Like,
    /// `GLOB`.
    Glob,
    /// `REGEXP`.
    Regexp,
    /// `MATCH`.
    Match,
}

/// One node of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// A constant.
    Literal(Literal),
    /// A column, with as much of `schema.table.column` as was written.
    Column {
        /// The schema, where one was named.
        schema: Option<Span>,
        /// The table, where one was named.
        table: Option<Span>,
        /// The column.
        column: Span,
    },
    /// `?`, `?1`, `:name`, `@name` or `$name`.
    Variable(Span),
    /// One operand.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// What it applies to.
        operand: ExprId,
    },
    /// Two operands.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// Left operand.
        left: ExprId,
        /// Right operand.
        right: ExprId,
    },
    /// `x BETWEEN low AND high`.
    Between {
        /// The value tested.
        value: ExprId,
        /// The lower bound.
        low: ExprId,
        /// The upper bound.
        high: ExprId,
        /// Whether `NOT` precedes it.
        negated: bool,
    },
    /// `x IN (a, b)`.
    InList {
        /// The value tested.
        value: ExprId,
        /// The list, which may be empty.
        list: Range,
        /// Whether `NOT` precedes it.
        negated: bool,
    },
    /// `x LIKE y ESCAPE z`, and the three operators that work like it.
    Like {
        /// Which of them.
        op: LikeOp,
        /// The value tested.
        value: ExprId,
        /// The pattern.
        pattern: ExprId,
        /// The escape, where one was written.
        escape: Option<ExprId>,
        /// Whether `NOT` precedes it.
        negated: bool,
    },
    /// `CAST(x AS type)`.
    Cast {
        /// What is converted.
        value: ExprId,
        /// The type name, as it was written.
        ty: Span,
    },
    /// `x COLLATE name`.
    Collate {
        /// What is collated.
        value: ExprId,
        /// The collation's name.
        name: Span,
    },
    /// `name(args)`, `count(*)`, `count(DISTINCT x)`.
    Call {
        /// The function's name.
        name: Span,
        /// Its arguments.
        args: Range,
        /// Whether `DISTINCT` precedes them.
        distinct: bool,
        /// Whether the argument list is `*`.
        star: bool,
    },
    /// `CASE operand WHEN a THEN b ... ELSE c END`, with the whens in
    /// pairs.
    Case {
        /// The value compared against, where one was written.
        operand: Option<ExprId>,
        /// The branches, two nodes each: the condition and the result.
        branches: Range,
        /// The `ELSE`, where one was written.
        otherwise: Option<ExprId>,
    },
    /// `(a, b, c)`, which is a row and not a parenthesis.
    Row(Range),
    /// `(SELECT ...)`, a statement used as a value.
    Subquery(SelectId),
    /// `EXISTS (SELECT ...)`.
    Exists(SelectId),
    /// `x IN (SELECT ...)`.
    InSelect {
        /// The value tested.
        value: ExprId,
        /// The statement it is looked for in.
        select: SelectId,
        /// Whether `NOT` precedes it.
        negated: bool,
    },
    /// `x IN table`, which is `IN (SELECT * FROM table)` written short.
    InTable {
        /// The value tested.
        value: ExprId,
        /// The schema, where one was named.
        schema: Option<Span>,
        /// The table.
        table: Span,
        /// Whether `NOT` precedes it.
        negated: bool,
    },
}

/// One statement in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectId(u32);

/// Whether a statement keeps every row or only the rows that differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Distinct {
    /// Neither word was written.
    #[default]
    Unspecified,
    /// `ALL`, which is what neither word means.
    All,
    /// `DISTINCT`.
    Distinct,
}

/// One thing a statement answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResultColumn {
    /// `*`.
    Star,
    /// `t.*`.
    TableStar(Span),
    /// An expression, with the name it is answered under.
    Expr {
        /// What is computed.
        expr: ExprId,
        /// The name after `AS`, or the one written without it.
        alias: Option<Span>,
    },
}

/// How one table of a `FROM` clause attaches to the ones before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Join {
    /// Whether the word `NATURAL` was written.
    pub natural: bool,
    /// Which join it is.
    pub kind: JoinKind,
    /// Whether it was written as a comma rather than as `JOIN`.
    pub comma: bool,
}

/// The kinds of join the grammar spells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JoinKind {
    /// The first table, which joins nothing.
    #[default]
    None,
    /// `JOIN`, `INNER JOIN`, and a comma.
    Inner,
    /// `CROSS JOIN`.
    Cross,
    /// `LEFT JOIN` and `LEFT OUTER JOIN`.
    Left,
    /// `RIGHT JOIN` and `RIGHT OUTER JOIN`.
    Right,
    /// `FULL JOIN` and `FULL OUTER JOIN`.
    Full,
}

/// What a `FROM` clause draws rows from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    /// A table, with as much of `schema.table` as was written.
    Table {
        /// The schema, where one was named.
        schema: Option<Span>,
        /// The table.
        name: Span,
        /// What the statement says about the index to use.
        indexed: Indexed,
    },
    /// A table-valued function: `name(args)`.
    Function {
        /// The schema, where one was named.
        schema: Option<Span>,
        /// The function.
        name: Span,
        /// Its arguments.
        args: Range,
    },
    /// A statement in brackets.
    Select(SelectId),
}

/// What a table of a `FROM` clause says about indexes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Indexed {
    /// Nothing was said.
    #[default]
    Unspecified,
    /// `INDEXED BY name`.
    By(Span),
    /// `NOT INDEXED`.
    Not,
}

/// One table of a `FROM` clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Source {
    /// Where the rows come from.
    pub kind: SourceKind,
    /// The name it is known by in the statement.
    pub alias: Option<Span>,
    /// How it attaches to what came before it.
    pub join: Join,
    /// The `ON` condition, where one was written.
    pub on: Option<ExprId>,
    /// The names of a `USING` clause, where one was written.
    pub using: Range,
}

/// Which way a sort runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Order {
    /// Neither word was written.
    #[default]
    Unspecified,
    /// `ASC`.
    Ascending,
    /// `DESC`.
    Descending,
}

/// Where the nulls of a sort go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Nulls {
    /// Nothing was said.
    #[default]
    Unspecified,
    /// `NULLS FIRST`.
    First,
    /// `NULLS LAST`.
    Last,
}

/// One term of an `ORDER BY`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OrderTerm {
    /// What is sorted by.
    pub expr: ExprId,
    /// Which way.
    pub order: Order,
    /// Where the nulls go.
    pub nulls: Nulls,
}

/// A `LIMIT`, with the `OFFSET` that may follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Limit {
    /// How many rows.
    pub count: ExprId,
    /// How many to skip first.
    pub offset: Option<ExprId>,
}

/// How two statements are put together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compound {
    /// `UNION`.
    Union,
    /// `UNION ALL`.
    UnionAll,
    /// `EXCEPT`.
    Except,
    /// `INTERSECT`.
    Intersect,
}

/// What a `WITH` clause says about keeping a result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Materialized {
    /// Nothing was said.
    #[default]
    Unspecified,
    /// `MATERIALIZED`.
    Yes,
    /// `NOT MATERIALIZED`.
    No,
}

/// One table of a `WITH` clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cte {
    /// Its name.
    pub name: Span,
    /// The names of its columns, where they were written.
    pub columns: Range,
    /// What it answers.
    pub select: SelectId,
    /// What was said about keeping it.
    pub materialized: Materialized,
}

/// One `SELECT`, or one `VALUES`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Select {
    /// The tables of a `WITH` clause.
    pub ctes: Range,
    /// Whether `RECURSIVE` was written.
    pub recursive: bool,
    /// What was said about duplicate rows.
    pub distinct: Distinct,
    /// What the statement answers.
    pub columns: Range,
    /// Where the rows come from.
    pub from: Range,
    /// The `WHERE` clause.
    pub filter: Option<ExprId>,
    /// The `GROUP BY` terms.
    pub group: Range,
    /// The `HAVING` clause.
    pub having: Option<ExprId>,
    /// The rows of a `VALUES`, each of them a row node.
    pub values: Range,
    /// What this statement is put together with, and how.
    pub compound: Option<(Compound, SelectId)>,
    /// The `ORDER BY` terms.
    pub order: Range,
    /// The `LIMIT`.
    pub limit: Option<Limit>,
}

/// A tree, and the nodes it is made of.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Arena {
    /// Every node.
    nodes: Vec<Node>,
    /// How tall the subtree under each node is, a leaf being one.
    heights: Vec<u32>,
    /// The children of the nodes that have a run of them.
    children: Vec<ExprId>,
    /// Every statement.
    selects: Vec<Select>,
    /// The result columns of the statements, in runs.
    results: Vec<ResultColumn>,
    /// The tables of the `FROM` clauses, in runs.
    sources: Vec<Source>,
    /// The terms of the `ORDER BY` clauses, in runs.
    orders: Vec<OrderTerm>,
    /// The names of the `USING` clauses and of the `WITH` columns, in
    /// runs.
    names: Vec<Span>,
    /// The tables of the `WITH` clauses, in runs.
    ctes: Vec<Cte>,
}

impl Arena {
    /// An arena with nothing in it.
    #[must_use]
    pub const fn new() -> Self {
        Arena {
            nodes: Vec::new(),
            heights: Vec::new(),
            children: Vec::new(),
            selects: Vec::new(),
            results: Vec::new(),
            sources: Vec::new(),
            orders: Vec::new(),
            names: Vec::new(),
            ctes: Vec::new(),
        }
    }

    /// How many nodes the arena holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether it holds none.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Adds a node and answers where it went.
    ///
    /// The height of the node is one more than the tallest of its
    /// children, which is what a caller bounds to keep a tree readable by
    /// anything that walks it.
    pub fn push(&mut self, node: Node) -> ExprId {
        let height = self.height_of(node);
        let at = u32::try_from(self.nodes.len()).unwrap_or(u32::MAX);
        self.nodes.push(node);
        self.heights.push(height);
        ExprId(at)
    }

    /// How tall the subtree under `id` is: one for a leaf.
    #[must_use]
    pub fn height(&self, id: ExprId) -> u32 {
        self.heights
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(0)
    }

    /// The height a node would have, from the children it names.
    fn height_of(&self, node: Node) -> u32 {
        let mut tallest = 0;
        let mut under = |id: ExprId| tallest = tallest.max(self.height(id));
        match node {
            // A leaf has no children, and a statement is a tree of its
            // own, walked and bounded on its own.
            Node::Literal(_)
            | Node::Column { .. }
            | Node::Variable(_)
            | Node::Subquery(_)
            | Node::Exists(_) => {}
            Node::Unary { operand, .. } => under(operand),
            Node::Binary { left, right, .. } => {
                under(left);
                under(right);
            }
            Node::Between {
                value, low, high, ..
            } => {
                under(value);
                under(low);
                under(high);
            }
            Node::InList { value, list, .. } => {
                under(value);
                for item in self.children(list) {
                    under(*item);
                }
            }
            Node::Like {
                value,
                pattern,
                escape,
                ..
            } => {
                under(value);
                under(pattern);
                if let Some(escape) = escape {
                    under(escape);
                }
            }
            Node::Call { args, .. } => {
                for arg in self.children(args) {
                    under(*arg);
                }
            }
            Node::Case {
                operand,
                branches,
                otherwise,
            } => {
                for id in operand.into_iter().chain(otherwise) {
                    under(id);
                }
                for branch in self.children(branches) {
                    under(*branch);
                }
            }
            Node::Row(items) => {
                for item in self.children(items) {
                    under(*item);
                }
            }
            Node::Cast { value, .. }
            | Node::Collate { value, .. }
            | Node::InSelect { value, .. }
            | Node::InTable { value, .. } => under(value),
        }
        tallest.saturating_add(1)
    }

    /// Adds a run of children and answers where it went.
    pub fn push_children(&mut self, children: &[ExprId]) -> Range {
        let start = u32::try_from(self.children.len()).unwrap_or(u32::MAX);
        self.children.extend_from_slice(children);
        Range {
            start,
            len: u32::try_from(children.len()).unwrap_or(u32::MAX),
        }
    }

    /// The node at `id`.
    #[must_use]
    pub fn node(&self, id: ExprId) -> Option<Node> {
        self.nodes
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .copied()
    }

    /// The children of a run.
    #[must_use]
    pub fn children(&self, range: Range) -> &[ExprId] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.children.get(start..end).unwrap_or_default()
    }
}

/// A run of one of the arena's side tables. Which table a run belongs to
/// is decided by what asks for it: the result columns of a statement are
/// read with [`Arena::results`], its tables with [`Arena::sources`], and so
/// on. One kind of run rather than six keeps the arena's shape plain.
impl Arena {
    /// Adds a statement and answers where it went.
    pub fn push_select(&mut self, select: Select) -> SelectId {
        let at = u32::try_from(self.selects.len()).unwrap_or(u32::MAX);
        self.selects.push(select);
        SelectId(at)
    }

    /// The statement at `id`.
    #[must_use]
    pub fn select(&self, id: SelectId) -> Option<Select> {
        self.selects
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .copied()
    }

    /// How many statements the arena holds.
    #[must_use]
    pub const fn selects(&self) -> usize {
        self.selects.len()
    }

    /// Adds a run of result columns.
    pub fn push_results(&mut self, columns: &[ResultColumn]) -> Range {
        let start = u32::try_from(self.results.len()).unwrap_or(u32::MAX);
        self.results.extend_from_slice(columns);
        Range {
            start,
            len: u32::try_from(columns.len()).unwrap_or(u32::MAX),
        }
    }

    /// The result columns of a run.
    #[must_use]
    pub fn results(&self, range: Range) -> &[ResultColumn] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.results.get(start..end).unwrap_or_default()
    }

    /// Adds a run of tables.
    pub fn push_sources(&mut self, sources: &[Source]) -> Range {
        let start = u32::try_from(self.sources.len()).unwrap_or(u32::MAX);
        self.sources.extend_from_slice(sources);
        Range {
            start,
            len: u32::try_from(sources.len()).unwrap_or(u32::MAX),
        }
    }

    /// The tables of a run.
    #[must_use]
    pub fn sources(&self, range: Range) -> &[Source] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.sources.get(start..end).unwrap_or_default()
    }

    /// Adds a run of sort terms.
    pub fn push_orders(&mut self, orders: &[OrderTerm]) -> Range {
        let start = u32::try_from(self.orders.len()).unwrap_or(u32::MAX);
        self.orders.extend_from_slice(orders);
        Range {
            start,
            len: u32::try_from(orders.len()).unwrap_or(u32::MAX),
        }
    }

    /// The sort terms of a run.
    #[must_use]
    pub fn orders(&self, range: Range) -> &[OrderTerm] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.orders.get(start..end).unwrap_or_default()
    }

    /// Adds a run of names.
    pub fn push_names(&mut self, names: &[Span]) -> Range {
        let start = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        self.names.extend_from_slice(names);
        Range {
            start,
            len: u32::try_from(names.len()).unwrap_or(u32::MAX),
        }
    }

    /// The names of a run.
    #[must_use]
    pub fn names(&self, range: Range) -> &[Span] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.names.get(start..end).unwrap_or_default()
    }

    /// Adds a run of `WITH` tables.
    pub fn push_ctes(&mut self, ctes: &[Cte]) -> Range {
        let start = u32::try_from(self.ctes.len()).unwrap_or(u32::MAX);
        self.ctes.extend_from_slice(ctes);
        Range {
            start,
            len: u32::try_from(ctes.len()).unwrap_or(u32::MAX),
        }
    }

    /// The `WITH` tables of a run.
    #[must_use]
    pub fn ctes(&self, range: Range) -> &[Cte] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.ctes.get(start..end).unwrap_or_default()
    }
}
