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
}

impl Arena {
    /// An arena with nothing in it.
    #[must_use]
    pub const fn new() -> Self {
        Arena {
            nodes: Vec::new(),
            heights: Vec::new(),
            children: Vec::new(),
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
            Node::Literal(_) | Node::Column { .. } | Node::Variable(_) => {}
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
            Node::Cast { value, .. } | Node::Collate { value, .. } => under(value),
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
