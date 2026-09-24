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

impl ExprId {
    /// Where the node lies in the arena, which is what a walk holding
    /// one flag per node counts with.
    #[must_use]
    pub fn place(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

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
    /// `name(args)`, `count(*)`, `count(DISTINCT x)`, with the
    /// `FILTER (WHERE ...)` an aggregate may carry.
    Call {
        /// The function's name.
        name: Span,
        /// Its arguments.
        args: Range,
        /// Whether `DISTINCT` precedes them.
        distinct: bool,
        /// Whether the argument list is `*`.
        star: bool,
        /// The `FILTER (WHERE ...)`, which holds which rows an
        /// aggregate reads.
        filter: Option<ExprId>,
        /// The `ORDER BY` written inside the brackets, which an
        /// aggregate reads the rows of its group in the order of.
        ordered: Range,
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
    /// `name(args) FILTER (WHERE x) OVER window`, which reads the rows
    /// of a window rather than the one row the walk stands on.
    Over {
        /// The function's name.
        name: Span,
        /// Its arguments.
        args: Range,
        /// Whether `DISTINCT` precedes them.
        distinct: bool,
        /// Whether the argument list is `*`.
        star: bool,
        /// The `FILTER (WHERE ...)`, which holds which rows the
        /// function reads.
        filter: Option<ExprId>,
        /// The window it reads.
        window: WindowId,
    },
    /// `RAISE(IGNORE)` and `RAISE(action, message)`, which only a
    /// trigger's body may write.
    Raise {
        /// What it does to the statement that reached it.
        action: Raise,
        /// The message, which `RAISE(IGNORE)` writes none.
        message: Option<ExprId>,
    },
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

/// What is done where a constraint is broken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Conflict {
    /// Nothing was written, so the statement's own choice stands.
    #[default]
    Unspecified,
    /// `ROLLBACK`.
    Rollback,
    /// `ABORT`.
    Abort,
    /// `FAIL`.
    Fail,
    /// `IGNORE`.
    Ignore,
    /// `REPLACE`.
    Replace,
}

/// What a foreign key does to this row where the row it points at
/// changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Action {
    /// Nothing was written, which is `NO ACTION`.
    #[default]
    Unspecified,
    /// `SET NULL`.
    SetNull,
    /// `SET DEFAULT`.
    SetDefault,
    /// `CASCADE`.
    Cascade,
    /// `RESTRICT`.
    Restrict,
    /// `NO ACTION`.
    NoAction,
}

/// `REFERENCES table(columns)` and what follows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Foreign {
    /// The table pointed at.
    pub table: Span,
    /// Its columns, where any were named.
    pub columns: Range,
    /// What happens to this row when that one goes.
    pub on_delete: Action,
    /// What happens to this row when that one changes.
    pub on_update: Action,
    /// Whether the check waits until the transaction ends.
    pub deferred: bool,
}

/// What may follow a column's name and type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnConstraint {
    /// `PRIMARY KEY`.
    PrimaryKey {
        /// `ASC` or `DESC`, where either was written.
        order: Order,
        /// Whether `AUTOINCREMENT` follows.
        autoincrement: bool,
        /// The conflict clause.
        conflict: Conflict,
    },
    /// `NOT NULL`.
    NotNull(Conflict),
    /// `NULL`, which asks for nothing.
    Null(Conflict),
    /// `UNIQUE`.
    Unique(Conflict),
    /// `CHECK (expression)`.
    Check {
        /// What every row is held to.
        value: ExprId,
        /// The text of it, which `CHECK constraint failed:` writes
        /// where the constraint carries no name.
        text: Span,
    },
    /// `DEFAULT value`.
    Default {
        /// What the column falls back to.
        value: ExprId,
        /// The text it was written as, which is what `sqlite_schema`
        /// keeps and `PRAGMA table_info` answers with.
        text: Span,
    },
    /// `COLLATE name`.
    Collate(Span),
    /// `REFERENCES ...`.
    References(Foreign),
    /// `GENERATED ALWAYS AS (expression)`, and `AS (expression)` written
    /// short.
    Generated {
        /// What the column is computed from.
        value: ExprId,
        /// The word after it, where one was written: `STORED` or
        /// `VIRTUAL`.
        kind: Option<Span>,
    },
    /// `CONSTRAINT name`, which names whatever follows it.
    Named(Span),
}

/// One column of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColumnDef {
    /// The whole of it, from the name to the last word that follows it,
    /// which is the text a column takes out of the statement.
    pub written: Span,
    /// The name.
    pub name: Span,
    /// The declared type, where one was written.
    pub ty: Option<Span>,
    /// What follows it.
    pub constraints: Range,
}

/// What may follow the columns of a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableConstraint {
    /// `PRIMARY KEY (columns)`.
    PrimaryKey {
        /// The columns, each with its order.
        columns: Range,
        /// Whether `AUTOINCREMENT` follows them.
        autoincrement: bool,
        /// The conflict clause.
        conflict: Conflict,
    },
    /// `UNIQUE (columns)`.
    Unique {
        /// The columns, each with its order.
        columns: Range,
        /// The conflict clause.
        conflict: Conflict,
    },
    /// `CHECK (expression)`.
    Check {
        /// What every row is held to.
        value: ExprId,
        /// The text of it, which `CHECK constraint failed:` writes
        /// where the constraint carries no name.
        text: Span,
    },
    /// `FOREIGN KEY (columns) REFERENCES ...`.
    ForeignKey {
        /// The columns of this table.
        columns: Range,
        /// What they point at.
        foreign: Foreign,
    },
    /// `CONSTRAINT name`, which names whatever follows it.
    Named(Span),
}

/// What a table is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableBody {
    /// Columns written out, with the constraints that follow them.
    Columns {
        /// The columns.
        columns: Range,
        /// The constraints of the table as a whole.
        constraints: Range,
    },
    /// `AS SELECT ...`, which takes its columns from the statement.
    Select(SelectId),
}

/// What may be written after a table's columns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TableOptions {
    /// Whether `WITHOUT ROWID` was written.
    pub without_rowid: bool,
    /// Whether `STRICT` was written.
    pub strict: bool,
}

/// `CREATE TABLE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreateTable {
    /// Whether `TEMP` or `TEMPORARY` was written.
    pub temporary: bool,
    /// Whether `IF NOT EXISTS` was written.
    pub if_not_exists: bool,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The name.
    pub name: Span,
    /// What it is made of.
    pub body: TableBody,
    /// What follows the columns.
    pub options: TableOptions,
    /// Where in the statement a column added later is written, which is
    /// `addColOffset`: the comma the constraints begin after, else the
    /// bracket that closes the columns.
    pub add_at: Option<usize>,
}

/// `ALTER TABLE [schema.]name ADD [COLUMN] <column>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AddColumn {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table.
    pub table: Span,
    /// The column, as the statement wrote it, which is what the schema
    /// text gains.
    pub written: Span,
    /// The column, read.
    pub column: ColumnDef,
}

/// `ALTER TABLE [schema.]name DROP [COLUMN] name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropColumn {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table.
    pub table: Span,
    /// The column it loses.
    pub column: Span,
}

/// `ALTER TABLE [schema.]name` with one constraint dropped or added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropConstraint {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table.
    pub table: Span,
    /// The constraint by name, or the column whose `NOT NULL` goes.
    pub which: Constrained,
}

/// Which constraint an `ALTER TABLE` drops, and which one it adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Constrained {
    /// `DROP CONSTRAINT name`.
    Named(Span),
    /// `ALTER [COLUMN] name DROP NOT NULL`.
    NotNull(Span),
    /// `ALTER [COLUMN] name SET NOT NULL [ON CONFLICT ...]`, with the
    /// column and the text of the constraint as the statement wrote it.
    SetNotNull(Span, Span),
    /// `ADD [CONSTRAINT name] CHECK (...) [ON CONFLICT ...]`, with the
    /// text of the constraint as the statement wrote it, the name it
    /// carries where it carries one, and what every row is held to.
    Add(Span, Option<Span>, ExprId),
}

/// `ALTER TABLE [schema.]name RENAME [COLUMN] name TO name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenameColumn {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table.
    pub table: Span,
    /// The column as it stands.
    pub column: Span,
    /// The name it takes.
    pub name: Span,
}

/// `ALTER TABLE [schema.]name RENAME TO name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenameTable {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table as it is named now.
    pub table: Span,
    /// The name it takes.
    pub name: Span,
}

/// `CREATE VIEW [IF NOT EXISTS] name [(columns)] AS select`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreateView {
    /// Whether `TEMP` or `TEMPORARY` was written.
    pub temporary: bool,
    /// Whether `IF NOT EXISTS` was written.
    pub if_not_exists: bool,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The name.
    pub name: Span,
    /// The names the view answers its columns under, where they were
    /// written.
    pub columns: Range,
    /// The statement it answers.
    pub select: SelectId,
}

/// `CREATE INDEX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreateIndex {
    /// Whether `UNIQUE` was written.
    pub unique: bool,
    /// Whether `IF NOT EXISTS` was written.
    pub if_not_exists: bool,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The name.
    pub name: Span,
    /// The table it is over.
    pub table: Span,
    /// The terms, each with its order.
    pub columns: Range,
    /// The `WHERE` clause of a partial index, where one was written.
    pub filter: Option<ExprId>,
}

/// `INSERT INTO name [(columns)] <select>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Insert {
    /// What `INSERT OR ...` says to do where a row is already there.
    pub conflict: Conflict,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table the rows go in.
    pub name: Span,
    /// The columns the rows are for, or an empty run for every column
    /// of the table in the order the table was created with.
    pub columns: Range,
    /// Where the rows come from, which is a `VALUES` or a `SELECT`.
    pub select: SelectId,
    /// Whether `DEFAULT VALUES` was written, which writes one row of
    /// what every column falls back to and reads no statement.
    pub defaults: bool,
    /// The `ON CONFLICT` clauses, in the order they were written.
    pub upserts: Range,
    /// The columns a `RETURNING` answers, or an empty run.
    pub returning: Range,
}

/// One `ON CONFLICT` clause of an `INSERT`, which is `sqlite3UpsertNew`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Upsert {
    /// The terms the clause names, written as the columns of an index
    /// are, or an empty run where it names none, which is the clause
    /// every conflict reaches.
    pub targets: Range,
    /// The `WHERE` that names a partial index, where one was written.
    pub over: Option<ExprId>,
    /// What the clause writes, or an empty run for `DO NOTHING`.
    pub sets: Range,
    /// Whether the clause writes a row at all, which tells `DO UPDATE`
    /// from `DO NOTHING`.
    pub writes: bool,
    /// The `WHERE` the write is held to, where one was written.
    pub filter: Option<ExprId>,
}

/// `DELETE FROM name [WHERE filter]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Delete {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table the rows come out of.
    pub name: Span,
    /// What the statement says about the index to use.
    pub indexed: Indexed,
    /// The `WHERE` clause, where one was written; a statement without
    /// one takes every row out.
    pub filter: Option<ExprId>,
    /// The columns a `RETURNING` answers, or an empty run.
    pub returning: Range,
}

/// One `column = value` of an `UPDATE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Set {
    /// The column written.
    pub column: Span,
    /// What it is written with.
    pub value: ExprId,
    /// Where the column stands in the row the value answers, where the
    /// statement wrote `SET (a, b) = value`, and nothing where it wrote
    /// one column and one value.
    pub at: Option<usize>,
}

/// `UPDATE name SET column = value, ... [WHERE filter]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Update {
    /// What `UPDATE OR ...` says to do where a constraint is broken.
    pub conflict: Conflict,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table whose rows change.
    pub name: Span,
    /// What the statement says about the index to use.
    pub indexed: Indexed,
    /// The columns written, each with what it is written with.
    pub sets: Range,
    /// The tables a `FROM` names, as the statement that answers their
    /// columns: `UPDATE t SET ... FROM x` reads the columns of `x` in
    /// its `SET` clauses and in its `WHERE`.
    pub from: Option<SelectId>,
    /// The `WHERE` clause, where one was written.
    pub filter: Option<ExprId>,
    /// The columns a `RETURNING` answers, or an empty run.
    pub returning: Range,
}

/// `ANALYZE [[schema.]name]`, which writes `sqlite_stat1` out of what
/// the tables and their indexes hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Analyze {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The table or the index it names, where it names one; every
    /// table of the schema otherwise.
    pub name: Option<Span>,
}

/// `REINDEX [[schema.]name]`, which writes the entries of an index
/// again out of the rows they belong to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reindex {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The collation, the table or the index it names, where it names
    /// one; every index of the schema otherwise.
    pub name: Option<Span>,
}

/// `PRAGMA [schema.]name [= value | (value)]`, which says how a
/// connection is configured or answers how it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pragma {
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// What it configures.
    pub name: Span,
    /// What it is set to, where the statement sets it.
    pub value: Option<Span>,
}

/// One statement that changes what a database holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    /// `INSERT` and `REPLACE`.
    Insert(Insert),
    /// `DELETE`.
    Delete(Delete),
    /// `UPDATE`.
    Update(Update),
}

/// One definition out of `sqlite_schema`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Definition {
    /// `CREATE TABLE`.
    Table(CreateTable),
    /// `CREATE INDEX`.
    Index(CreateIndex),
    /// `CREATE VIEW`.
    View(CreateView),
    /// `ALTER TABLE ... ADD COLUMN`.
    AddColumn(AddColumn),
    /// `ALTER TABLE ... RENAME TO`.
    Rename(RenameTable),
    /// `ALTER TABLE ... DROP COLUMN`.
    DropColumn(DropColumn),
    /// `ALTER TABLE ... RENAME COLUMN`.
    RenameColumn(RenameColumn),
    /// `ALTER TABLE ... DROP CONSTRAINT`, and `ALTER TABLE ... ALTER
    /// COLUMN ... DROP NOT NULL`.
    DropConstraint(DropConstraint),
    /// `CREATE TRIGGER`.
    Trigger(CreateTrigger),
    /// `DROP TABLE`, `DROP INDEX`, `DROP VIEW` and `DROP TRIGGER`.
    Drop(Drop),
    /// `VACUUM`.
    Vacuum(Vacuum),
    /// `ATTACH`.
    Attach(Attach),
    /// `DETACH`.
    Detach(Detach),
}

/// `ATTACH [DATABASE] file AS name [KEY key]`.
///
/// Both the file and the name are expressions, which
/// `cmd ::= ATTACH database_kw_opt expr AS expr key_opt` of
/// `research/sqlite/src/parse.y:1851` reads them as. The key is read and
/// nothing is done with it, which is what a library built without an
/// encryption extension does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Attach {
    /// The file the statement names.
    pub file: ExprId,
    /// The name the database answers to from the statement on.
    pub name: ExprId,
}

/// `DETACH [DATABASE] name`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Detach {
    /// The name an `ATTACH` gave.
    pub name: ExprId,
}

/// `VACUUM [schema] [INTO expr]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vacuum {
    /// The schema the statement names, and nothing where it names none.
    pub schema: Option<Span>,
    /// The file `INTO` names, and nothing where the statement writes the
    /// database it was run on.
    pub into: Option<ExprId>,
    /// The text of that expression, as the statement wrote it.
    pub text: Option<Span>,
}

/// What a `DROP` takes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dropped {
    /// `DROP TABLE`.
    Table,
    /// `DROP INDEX`.
    Index,
    /// `DROP VIEW`.
    View,
    /// `DROP TRIGGER`.
    Trigger,
}

/// When a trigger runs against the row it is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TriggerTime {
    /// `BEFORE`, which is also what a trigger with no time written
    /// runs at.
    #[default]
    Before,
    /// `AFTER`.
    After,
    /// `INSTEAD OF`, which only a view carries.
    InsteadOf,
}

/// What a trigger runs on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TriggerEvent {
    /// `DELETE`.
    #[default]
    Delete,
    /// `INSERT`.
    Insert,
    /// `UPDATE`, with the columns an `UPDATE OF` named.
    Update,
}

/// `CREATE TRIGGER name time event ON table [WHEN ...] BEGIN ... END`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CreateTrigger {
    /// Whether `TEMP` or `TEMPORARY` was written.
    pub temporary: bool,
    /// Whether `IF NOT EXISTS` was written.
    pub if_not_exists: bool,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The name.
    pub name: Span,
    /// When it runs.
    pub time: TriggerTime,
    /// What it runs on.
    pub event: TriggerEvent,
    /// The columns an `UPDATE OF` named, or an empty run.
    pub columns: Range,
    /// The schema in front of the table, where one was named.
    pub table_schema: Option<Span>,
    /// The table it is on.
    pub table: Span,
    /// The `WHEN`, where one was written.
    pub condition: Option<ExprId>,
    /// The statements between `BEGIN` and `END`.
    pub body: Range,
    /// The text from the name to the `END`, which is what
    /// `sqlite3FinishTrigger` writes after the words `CREATE TRIGGER`.
    pub written: Span,
}

/// What a `RAISE` does to the statement that reached it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Raise {
    /// `IGNORE`: the row the trigger stands on is passed over.
    Ignore,
    /// `ROLLBACK`: the transaction is undone.
    Rollback,
    /// `ABORT`: the statement is undone.
    Abort,
    /// `FAIL`: the statement stops where it stands.
    Fail,
}

/// One statement of a trigger's body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriggerStep {
    /// `INSERT`.
    Insert(Insert),
    /// `UPDATE`.
    Update(Update),
    /// `DELETE`.
    Delete(Delete),
    /// `SELECT`, which answers no row to anything and runs for what it
    /// reads.
    Select(SelectId),
}

/// `BEGIN`, `COMMIT` and `ROLLBACK`, which are what a connection
/// bounds a transaction with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transaction {
    /// `BEGIN [DEFERRED | IMMEDIATE | EXCLUSIVE] [TRANSACTION]`. The
    /// three words say when the connection takes its locks, which one
    /// writer of one file answers the same way.
    Begin,
    /// `COMMIT [TRANSACTION]`, and `END [TRANSACTION]`, which is the
    /// same statement under another word.
    Commit,
    /// `ROLLBACK [TRANSACTION]`.
    Rollback,
}

/// `SAVEPOINT`, `RELEASE` and `ROLLBACK TO`, which bound a part of a
/// transaction under a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Savepoint {
    /// `SAVEPOINT name`, which opens one and opens a transaction where
    /// the connection holds none.
    Open(Span),
    /// `RELEASE [SAVEPOINT] name`, which keeps what the savepoint wrote.
    Release(Span),
    /// `ROLLBACK [TRANSACTION] TO [SAVEPOINT] name`, which puts the file
    /// back to what the savepoint stands over and leaves it open.
    Back(Span),
}

/// `DROP TABLE [IF EXISTS] [schema.]name`, and the same for an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Drop {
    /// What it takes away.
    pub kind: Dropped,
    /// Whether `IF EXISTS` was written.
    pub if_exists: bool,
    /// The schema, where one was named.
    pub schema: Option<Span>,
    /// The name.
    pub name: Span,
}

/// One statement in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SelectId(u32);

impl SelectId {
    /// Where the statement lies in the arena, which names a statement
    /// written inside a `FROM` that carries no alias.
    #[must_use]
    pub const fn place(self) -> u32 {
        self.0
    }
}

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
        /// The text of the expression as it stands in the statement,
        /// which is the name it is answered under where none is written
        /// and it is not a column.
        text: Span,
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

/// Which rows of a partition the frame of a window is measured in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Frame {
    /// `ROWS`: the frame is counted in rows.
    #[default]
    Rows,
    /// `RANGE`: the frame holds the rows whose order terms lie within
    /// the bound of the row's own.
    Range,
    /// `GROUPS`: the frame is counted in groups of rows that share
    /// their order terms.
    Groups,
}

/// Where a frame begins or ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bound {
    /// `UNBOUNDED PRECEDING`.
    #[default]
    UnboundedPreceding,
    /// `<expr> PRECEDING`.
    Preceding(ExprId),
    /// `CURRENT ROW`.
    CurrentRow,
    /// `<expr> FOLLOWING`.
    Following(ExprId),
    /// `UNBOUNDED FOLLOWING`.
    UnboundedFollowing,
}

/// Which rows of the frame a window function passes over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Exclude {
    /// `EXCLUDE NO OTHERS`, which is what writing none means.
    #[default]
    NoOthers,
    /// `EXCLUDE CURRENT ROW`.
    CurrentRow,
    /// `EXCLUDE GROUP`: the row and every row that shares its order
    /// terms.
    Group,
    /// `EXCLUDE TIES`: every row that shares its order terms but the
    /// row itself.
    Ties,
}

/// The frame of a window, which says which rows of the partition a
/// window function over it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Frames {
    /// What the bounds are measured in.
    pub kind: Frame,
    /// Where the frame begins.
    pub start: Bound,
    /// Where it ends.
    pub end: Bound,
    /// Which of its rows are passed over.
    pub exclude: Exclude,
}

/// One window in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u32);

/// A window: how the rows are partitioned and ordered, and which of
/// them a function over it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    /// The window this one is written on top of, where `OVER` names
    /// one before its own clauses.
    pub base: Option<Span>,
    /// Whether the window is the named one and nothing else, which
    /// `OVER name` writes and `OVER (name)` does not: the first takes
    /// the named window's frame and the second may write its own.
    pub named: bool,
    /// The `PARTITION BY` terms.
    pub partition: Range,
    /// The `ORDER BY` terms.
    pub order: Range,
    /// The frame, where the window wrote one.
    pub frame: Option<Frames>,
}

/// One window a `WINDOW` clause names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NamedWindow {
    /// What the clause calls it.
    pub name: Span,
    /// The window itself.
    pub window: WindowId,
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
    ///
    /// `RECURSIVE` is not kept: a term that reads its own name reads
    /// itself whether the word was written or not.
    pub ctes: Range,
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
    /// The windows a `WINDOW` clause names.
    pub windows: Range,
    /// The `HAVING` clause.
    pub having: Option<ExprId>,
    /// The rows of a `VALUES`, each of them a row node.
    pub values: Range,
    /// Whether the statement stands for the tables written inside
    /// brackets in a `FROM`, which is `SF_NestedFrom`: a column written
    /// with the name of one of those tables in front of it reaches
    /// that table through this statement.
    pub nested: bool,
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
    /// The `column = value` of the statements that write columns, in
    /// runs.
    sets: Vec<Set>,
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
    /// The statements of the trigger bodies, in runs.
    steps: Vec<TriggerStep>,
    /// The `ON CONFLICT` clauses of the statements, in runs.
    upserts: Vec<Upsert>,
    /// The columns of the tables, in runs.
    columns: Vec<ColumnDef>,
    /// What follows each column, in runs.
    column_constraints: Vec<ColumnConstraint>,
    /// What follows the columns of a table, in runs.
    table_constraints: Vec<TableConstraint>,
    /// Every window a statement writes.
    windows: Vec<Window>,
    /// The windows the `WINDOW` clauses name, in runs.
    named_windows: Vec<NamedWindow>,
}

impl Arena {
    /// An arena with nothing in it.
    #[must_use]
    pub const fn new() -> Self {
        Arena {
            steps: Vec::new(),
            upserts: Vec::new(),
            nodes: Vec::new(),
            heights: Vec::new(),
            children: Vec::new(),
            sets: Vec::new(),
            selects: Vec::new(),
            results: Vec::new(),
            sources: Vec::new(),
            orders: Vec::new(),
            names: Vec::new(),
            ctes: Vec::new(),
            columns: Vec::new(),
            windows: Vec::new(),
            named_windows: Vec::new(),
            column_constraints: Vec::new(),
            table_constraints: Vec::new(),
        }
    }

    /// How many nodes the arena holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Every node, with where it lies, in the order they were written.
    pub fn all(&self) -> impl Iterator<Item = (ExprId, Node)> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(at, node)| (ExprId(u32::try_from(at).unwrap_or(u32::MAX)), *node))
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
        self.under(node, |id| tallest = tallest.max(self.height(id)));
        tallest.saturating_add(1)
    }

    /// Calls `under` with every expression one node names.
    ///
    /// A statement under a node is a tree of its own, walked and bounded
    /// on its own, so nothing here descends into one.
    pub fn under(&self, node: Node, mut under: impl FnMut(ExprId)) {
        match node {
            Node::Raise { message, .. } => {
                if let Some(id) = message {
                    under(id);
                }
            }
            // The arguments of a window function and the `FILTER` that
            // holds which rows it reads are read against the row; the
            // window itself is a clause and not an expression.
            // A leaf names none.
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
            Node::Call {
                args,
                filter,
                ordered,
                ..
            } => {
                for arg in self.children(args) {
                    under(*arg);
                }
                if let Some(id) = filter {
                    under(id);
                }
                for term in self.orders(ordered) {
                    under(term.expr);
                }
            }
            Node::Over { args, filter, .. } => {
                for arg in self.children(args) {
                    under(*arg);
                }
                if let Some(id) = filter {
                    under(id);
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

    /// Where every column reference of the statement stands, in the
    /// order the parser wrote the nodes.
    ///
    /// Costs O(n) over the nodes of the statement.
    #[must_use]
    pub fn column_places(&self) -> Vec<ExprId> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node, Node::Column { .. }))
            .map(|(at, _)| ExprId(u32::try_from(at).unwrap_or(u32::MAX)))
            .collect()
    }

    /// The name and the arguments of every call of the statement, in
    /// the order the parser wrote the nodes.
    ///
    /// Costs O(n) over the nodes of the statement.
    #[must_use]
    pub fn calls(&self) -> Vec<(Span, Range)> {
        self.nodes
            .iter()
            .filter_map(|node| match *node {
                Node::Call { name, args, .. } => Some((name, args)),
                _ => None,
            })
            .collect()
    }

    /// The name every `COLLATE` of the statement carries, in the order
    /// the parser wrote the nodes.
    ///
    /// Costs O(n) over the nodes of the statement.
    #[must_use]
    pub fn collates(&self) -> Vec<Span> {
        self.nodes
            .iter()
            .filter_map(|node| match *node {
                Node::Collate { name, .. } => Some(name),
                _ => None,
            })
            .collect()
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

    /// Every statement the arena holds, in the order they were
    /// written.
    pub fn all_selects(&self) -> impl Iterator<Item = Select> {
        self.selects.iter().copied()
    }

    /// Every clause of a `SET` the arena holds, in the order they were
    /// written.
    pub fn all_sets(&self) -> impl Iterator<Item = Set> {
        self.sets.iter().copied()
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

    /// Every table of every `FROM` clause the arena holds, in the order
    /// they were read.
    pub fn all_sources(&self) -> impl Iterator<Item = Source> {
        self.sources.iter().copied()
    }

    /// Every table of every `WITH` clause the arena holds.
    pub fn all_ctes(&self) -> impl Iterator<Item = Cte> {
        self.ctes.iter().copied()
    }

    /// Every statement of every trigger body the arena holds.
    pub fn all_steps(&self) -> impl Iterator<Item = TriggerStep> {
        self.steps.iter().copied()
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

    /// Appends one window and answers where it went.
    pub fn push_window(&mut self, window: Window) -> WindowId {
        let at = u32::try_from(self.windows.len()).unwrap_or(u32::MAX);
        self.windows.push(window);
        WindowId(at)
    }

    /// The window `id` names.
    #[must_use]
    pub fn window(&self, id: WindowId) -> Option<Window> {
        self.windows
            .get(usize::try_from(id.0).unwrap_or(usize::MAX))
            .copied()
    }

    /// Appends a run of named windows and answers where it went.
    pub fn push_named_windows(&mut self, windows: &[NamedWindow]) -> Range {
        let start = u32::try_from(self.named_windows.len()).unwrap_or(u32::MAX);
        self.named_windows.extend_from_slice(windows);
        Range {
            start,
            len: u32::try_from(windows.len()).unwrap_or(u32::MAX),
        }
    }

    /// The named windows of a run.
    #[must_use]
    pub fn named_windows(&self, range: Range) -> &[NamedWindow] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.named_windows.get(start..end).unwrap_or_default()
    }

    /// Appends a run of columns and answers where it went.
    pub fn push_columns(&mut self, columns: &[ColumnDef]) -> Range {
        let start = u32::try_from(self.columns.len()).unwrap_or(u32::MAX);
        self.columns.extend_from_slice(columns);
        Range {
            start,
            len: u32::try_from(columns.len()).unwrap_or(u32::MAX),
        }
    }

    /// The columns of a run.
    #[must_use]
    pub fn columns(&self, range: Range) -> &[ColumnDef] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.columns.get(start..end).unwrap_or_default()
    }

    /// Appends a run of column constraints and answers where it went.
    pub fn push_column_constraints(&mut self, constraints: &[ColumnConstraint]) -> Range {
        let start = u32::try_from(self.column_constraints.len()).unwrap_or(u32::MAX);
        self.column_constraints.extend_from_slice(constraints);
        Range {
            start,
            len: u32::try_from(constraints.len()).unwrap_or(u32::MAX),
        }
    }

    /// The column constraints of a run.
    #[must_use]
    pub fn column_constraints(&self, range: Range) -> &[ColumnConstraint] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.column_constraints.get(start..end).unwrap_or_default()
    }

    /// Appends a run of table constraints and answers where it went.
    pub fn push_table_constraints(&mut self, constraints: &[TableConstraint]) -> Range {
        let start = u32::try_from(self.table_constraints.len()).unwrap_or(u32::MAX);
        self.table_constraints.extend_from_slice(constraints);
        Range {
            start,
            len: u32::try_from(constraints.len()).unwrap_or(u32::MAX),
        }
    }

    /// The table constraints of a run.
    #[must_use]
    pub fn table_constraints(&self, range: Range) -> &[TableConstraint] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.table_constraints.get(start..end).unwrap_or_default()
    }

    /// Adds a run of `WITH` tables.
    /// Keeps `sets` as a run and answers where it lies.
    pub fn push_sets(&mut self, sets: &[Set]) -> Range {
        let start = u32::try_from(self.sets.len()).unwrap_or(u32::MAX);
        self.sets.extend_from_slice(sets);
        Range {
            start,
            len: u32::try_from(sets.len()).unwrap_or(u32::MAX),
        }
    }

    /// The run of `column = value` at `range`.
    #[must_use]
    pub fn sets(&self, range: Range) -> &[Set] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.sets.get(start..end).unwrap_or_default()
    }

    /// Keeps `upserts` as a run and answers where it lies.
    pub fn push_upserts(&mut self, upserts: &[Upsert]) -> Range {
        let start = u32::try_from(self.upserts.len()).unwrap_or(u32::MAX);
        self.upserts.extend_from_slice(upserts);
        Range {
            start,
            len: u32::try_from(upserts.len()).unwrap_or(u32::MAX),
        }
    }

    /// The `ON CONFLICT` clauses of a run.
    #[must_use]
    pub fn upserts(&self, range: Range) -> &[Upsert] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.upserts.get(start..end).unwrap_or_default()
    }

    /// Keeps `ctes` as a run and answers where it lies.
    pub fn push_ctes(&mut self, ctes: &[Cte]) -> Range {
        let start = u32::try_from(self.ctes.len()).unwrap_or(u32::MAX);
        self.ctes.extend_from_slice(ctes);
        Range {
            start,
            len: u32::try_from(ctes.len()).unwrap_or(u32::MAX),
        }
    }

    /// Keeps the statements of one trigger body and answers where they
    /// are.
    pub fn push_steps(&mut self, steps: &[TriggerStep]) -> Range {
        let start = u32::try_from(self.steps.len()).unwrap_or(u32::MAX);
        self.steps.extend_from_slice(steps);
        Range {
            start,
            len: u32::try_from(steps.len()).unwrap_or(u32::MAX),
        }
    }

    /// The statements of a trigger body.
    #[must_use]
    pub fn steps(&self, range: Range) -> &[TriggerStep] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.steps.get(start..end).unwrap_or_default()
    }

    /// The `WITH` tables of a run.
    #[must_use]
    pub fn ctes(&self, range: Range) -> &[Cte] {
        let start = usize::try_from(range.start).unwrap_or(usize::MAX);
        let end = start.saturating_add(range.len());
        self.ctes.get(start..end).unwrap_or_default()
    }
}

/// How two expressions compare, which is what `sqlite3ExprCompare` of
/// `research/sqlite/src/expr.c:6157` answers: the same expression, the
/// same but for a collation one side names, or another expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alike {
    /// The same expression.
    Same,
    /// The same but for a collation one side names and the other does
    /// not.
    Collated,
    /// Another expression.
    Other,
}

/// How the expression at `id` of one tree compares with the one at `at`
/// of another, each named by its tree, the place in it and the text it
/// points into.
///
/// A node that names a statement of its own compares as another
/// expression, because a statement is a tree this does not walk.
/// Comparing costs O(n) in the nodes of the smaller expression.
#[must_use]
pub fn alike(one: (&Arena, ExprId, &[u8]), other: (&Arena, ExprId, &[u8])) -> Alike {
    let (held, id, text) = one;
    let (beside, at, sql) = other;
    // A place no tree holds stands for no expression, which two of them
    // compare the same for.
    let node = held.node(id).unwrap_or(Node::Literal(Literal::Null));
    let against = beside.node(at).unwrap_or(Node::Literal(Literal::Null));
    // A `COLLATE` one side writes and the other does not leaves the
    // expressions the same but for that collation, which
    // `sqlite3ExprCompare` answers one for.
    if let Node::Collate { value, .. } = node
        && !matches!(against, Node::Collate { .. })
    {
        return collated(alike((held, value, text), other));
    }
    if let Node::Collate { value, .. } = against
        && !matches!(node, Node::Collate { .. })
    {
        return collated(alike(one, (beside, value, sql)));
    }
    // A node that names a statement of its own, a window or a `RAISE`
    // carries what this does not read, so it answers no shape and two
    // such nodes are other expressions whatever their children are.
    let (Some(one), Some(other)) = (shaped(&node, text), shaped(&against, sql)) else {
        return Alike::Other;
    };
    if one != other {
        return Alike::Other;
    }
    let mut ones = Vec::new();
    held.under(node, |id| ones.push(id));
    let mut others = Vec::new();
    beside.under(against, |at| others.push(at));
    if ones.len() != others.len() {
        return Alike::Other;
    }
    let mut answer = Alike::Same;
    for (id, at) in ones.into_iter().zip(others) {
        match alike((held, id, text), (beside, at, sql)) {
            Alike::Other => return Alike::Other,
            Alike::Collated => answer = Alike::Collated,
            Alike::Same => {}
        }
    }
    answer
}

/// The same comparison with a collation one side named, which is the
/// same expression at best.
const fn collated(held: Alike) -> Alike {
    match held {
        Alike::Other => Alike::Other,
        Alike::Same | Alike::Collated => Alike::Collated,
    }
}

/// The kind of one node and the text it carries, with no expression
/// under it: two nodes answer the same where they differ in their
/// children alone.
///
/// A name is read without its quotes and without its case, which is how
/// SQLite compares one. A node that names a statement of its own, a
/// window or a `RAISE` answers no shape at all, because this reads
/// neither. Building the answer costs O(n) in the bytes the node names.
fn shaped(node: &Node, sql: &[u8]) -> Option<Vec<u8>> {
    let named = |span: Span| {
        let mut held = crate::schema::dequote(span.text(sql));
        held.make_ascii_lowercase();
        held
    };
    let mut out = Vec::new();
    let mut tag = |word: &str| out.extend_from_slice(word.as_bytes());
    match *node {
        Node::Literal(literal) => {
            tag("literal ");
            match literal {
                Literal::Null => tag("null"),
                Literal::CurrentTime(which) => out.extend_from_slice(match which {
                    CurrentTime::Time => b"time",
                    CurrentTime::Date => b"date",
                    CurrentTime::Timestamp => b"timestamp",
                }),
                Literal::Integer(span) | Literal::Float(span) => {
                    out.extend_from_slice(span.text(sql));
                }
                Literal::Text(span) | Literal::Blob(span) => out.extend_from_slice(&named(span)),
            }
        }
        Node::Column {
            schema,
            table,
            column,
        } => {
            tag("column ");
            for span in schema.into_iter().chain(table) {
                out.extend_from_slice(&named(span));
                out.push(b'.');
            }
            out.extend_from_slice(&named(column));
        }
        Node::Variable(span) => {
            tag("variable ");
            out.extend_from_slice(span.text(sql));
        }
        Node::Unary { op, .. } => out.extend_from_slice(alloc::format!("unary {op:?}").as_bytes()),
        Node::Binary { op, .. } => {
            out.extend_from_slice(alloc::format!("binary {op:?}").as_bytes());
        }
        Node::Between { negated, .. } => {
            out.extend_from_slice(alloc::format!("between {negated}").as_bytes());
        }
        Node::InList { negated, .. } => {
            out.extend_from_slice(alloc::format!("in list {negated}").as_bytes());
        }
        Node::Like { op, negated, .. } => {
            out.extend_from_slice(alloc::format!("like {op:?} {negated}").as_bytes());
        }
        Node::Cast { ty, .. } => {
            tag("cast ");
            out.extend_from_slice(&named(ty));
        }
        Node::Collate { name, .. } => {
            tag("collate ");
            out.extend_from_slice(&named(name));
        }
        Node::Call {
            name,
            distinct,
            star,
            ..
        } => {
            out.extend_from_slice(alloc::format!("call {distinct} {star} ").as_bytes());
            out.extend_from_slice(&named(name));
        }
        Node::Case { .. } => tag("case"),
        Node::Row(_) => tag("row"),
        Node::Subquery(_)
        | Node::Exists(_)
        | Node::InSelect { .. }
        | Node::InTable { .. }
        | Node::Raise { .. }
        | Node::Over { .. } => return None,
    }
    Some(out)
}
