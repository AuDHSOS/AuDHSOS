// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A database that answers a statement.
//!
//! The file is read where it lies, its schema is read out of the
//! `CREATE` text `sqlite_schema` holds, and a statement is answered by
//! walking the sides of its `FROM` once: O(n) in the rows of one side
//! for a scan, the product of the rows for a join of several, O(n log n)
//! where an `ORDER BY` has to sort them, and O(n·g) where a `GROUP BY`
//! has to find which of `g` groups each row belongs to. Nothing is
//! compiled and no index is used yet, so the rows are right and the plan
//! is not — which is the order document 16 puts them in.
//!
//! A side of a `FROM` is a table of the schema, a statement written
//! inside the `FROM`, or a `WITH` term. Each carries a shape — one
//! name, one affinity and one written collation per column — so nothing
//! below asks which of the three it is reading.
//!
//! An expression uses a statement in four shapes — `(SELECT ...)` as a
//! value, `EXISTS`, `IN (SELECT ...)` and `IN table` — and each is
//! answered where it is read, against the row the walk stands on. A
//! statement that names a column no side of its own `FROM` answers
//! reads it from the row of the statement that encloses it, at O(n·m)
//! for `n` outer rows and `m` inner ones. Every shape of statement this
//! engine does not answer refuses by name rather than answering
//! something near it.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::agg::{self, Accumulator, Aggregate};
use crate::ast::{
    Arena, BinaryOp, Bound as Edge, Compound, Distinct, Exclude, ExprId, Frame, Frames, JoinKind,
    Literal, Node, Nulls, Order, OrderTerm, Range, ResultColumn, Select, SelectId, SourceKind,
    Span, UnaryOp, WindowId,
};
use crate::eval::{self, Used, arithmetic, evaluate_collated, evaluate_compared, evaluate_row};
use crate::header::Encoding;
use crate::image::Image;
use crate::parse;
use crate::record;
use crate::schema::{self, Generated, Table, dequote};
use crate::value::{
    Affinity, Collation, Value, apply_comparison, apply_numeric, compare, compare_affinity,
};
use crate::window::{self, Which};
use crate::{error, number};

/// The result code one refusal carries: the number and the name
/// `sqlite3_errcode` answers, and the ones `sqlite3_extended_errcode`
/// answers.
///
/// The numbers are the ones `research/sqlite/src/sqlite.h.in` gives and
/// the names the ones `sqlite3ErrName` of `research/sqlite/src/main.c`
/// writes. A refusal that carries no extended code of its own carries
/// the primary one twice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Code {
    /// The number `sqlite3_errcode` answers.
    pub number: i64,
    /// The name of that number.
    pub name: &'static [u8],
    /// The number `sqlite3_extended_errcode` answers.
    pub extended: i64,
    /// The name of that number.
    pub extended_name: &'static [u8],
}

impl Code {
    /// One code with no extended code of its own.
    const fn plain(number: i64, name: &'static [u8]) -> Self {
        Code {
            number,
            name,
            extended: number,
            extended_name: name,
        }
    }

    /// One `SQLITE_CONSTRAINT` with the extended code that says which
    /// constraint the row broke.
    const fn broke(extended: i64, extended_name: &'static [u8]) -> Self {
        Code {
            number: 19,
            name: b"SQLITE_CONSTRAINT",
            extended,
            extended_name,
        }
    }
}

/// What `sqlite3AlterFinishAddColumn` of `research/sqlite/src/alter.c:330`
/// refuses a column an `ALTER TABLE ... ADD COLUMN` writes for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    /// `PRIMARY KEY` on the column, whose index would hold the rows the
    /// table already has.
    PrimaryKey,
    /// `UNIQUE` on the column, the same.
    Unique,
    /// `NOT NULL` where the column falls back to nothing, the rows the
    /// table already has holding nothing for it.
    NotNull,
    /// A default the connection reads as no value of its own, which
    /// `sqlite3ValueFromExpr` answers nothing for.
    NonConstant,
    /// A view, which holds no row of its own.
    View,
}

impl Added {
    /// The words the refusal carries.
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Added::PrimaryKey => "Cannot add a PRIMARY KEY column",
            Added::Unique => "Cannot add a UNIQUE column",
            Added::NotNull => "Cannot add a NOT NULL column with default value NULL",
            Added::NonConstant => "Cannot add a column with non-constant default",
            Added::View => "Cannot add a column to a view",
        }
    }
}

/// Why a database could not answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file is not one this crate reads.
    Image(error::Error),
    /// The statement is not one this crate reads.
    Parse(parse::Error),
    /// A definition in the schema is not one this crate reads.
    Schema(schema::Error),
    /// An expression could not be answered.
    Eval(eval::Error),
    /// The function the connection was told refused the statement.
    Auth(crate::auth::Error),
    /// A table the statement names is not in the schema, by the name
    /// it was named under, which is empty where the walk asked for a
    /// side it had put there itself.
    NoTable(Vec<u8>),
    /// A `DROP` of a name the schema holds no such object under, with
    /// the word for what it makes and the name.
    NoObject(Vec<u8>, Vec<u8>),
    /// A statement the parser stopped in, with the token it stopped at,
    /// which is `near \"%T\": syntax error`.
    Syntax(Vec<u8>),
    /// A statement whose tokens ran out before it was whole, which is
    /// `incomplete input`.
    Incomplete,
    /// A `CREATE TRIGGER` whose time the thing it is over does not
    /// take, with the time, the word for what it is over, and its name.
    Timed(Vec<u8>, Vec<u8>, Vec<u8>),
    /// A `CREATE TRIGGER` over a table SQLite keeps for itself.
    SystemTrigger,
    /// An `ORDER BY` or a `GROUP BY` that counts to a column the answer
    /// does not have, with which term it is, the word for the clause,
    /// and how many columns the answer has.
    OrderRange(usize, Vec<u8>, usize),

    /// An aggregate in a `WHERE`, or one inside another aggregate,
    /// named as it was written.
    MisusedAggregate(Vec<u8>),
    /// An aggregate in a `GROUP BY`.
    GroupedAggregate,
    /// An aggregate in the `ORDER BY` of a statement that groups
    /// nothing, named as it was written.
    LooseAggregate(Vec<u8>),
    /// A name the statement answers under, standing for an aggregate
    /// and written inside another aggregate.
    MisusedAlias(Vec<u8>),
    /// `DISTINCT` before other than one argument of an aggregate.
    DistinctAggregate,
    /// `DISTINCT` before the arguments of a window function.
    DistinctWindow,
    /// A `*` written where the statement reads no table.
    NoTables,
    /// A `CREATE INDEX` over a table SQLite keeps for itself, named.
    NotIndexable(Vec<u8>),
    /// A `CREATE INDEX` over a view, which holds no row of its own.
    IndexedView,
    /// A `DROP INDEX` of an index a `UNIQUE` or a `PRIMARY KEY` made.
    ConstraintIndex,
    /// A `HAVING` on a statement that groups nothing.
    Having,
    /// Two sides of a compound that answer different numbers of columns.
    Compound(Vec<u8>),
    /// An `ORDER BY` term of a compound that names no column any of its
    /// cores answers, with which term it is.
    OrderMatch(usize),
    /// A `VALUES` whose rows are not all the same width.
    Values,
    /// A `NULLS FIRST` or a `NULLS LAST` written where an index takes
    /// its terms, the truth telling the two apart.
    ExplicitNulls(bool),
    /// A combination of words no join is written with.
    JoinType(Vec<u8>),
    /// An `ON` of an outer join that names a table read after it.
    Rightward,
    /// A trigger that carries a variable.
    TriggerVariable,
    /// A write of a trigger's body that names a schema.
    QualifiedInTrigger,
    /// An `UPDATE` or a `DELETE` of a trigger's body that names an
    /// index, which the flag says was written `NOT INDEXED`.
    IndexedInTrigger(bool),
    /// A column added to a table that points at a row of another and
    /// falls back to something.
    PointingDefault,
    /// A column an `ALTER TABLE ... ADD COLUMN` may not add, and which of
    /// the four it is.
    Added(Added),
    /// A `VACUUM ... INTO` whose file the client already holds bytes for,
    /// which `sqlite3RunVacuum` of `research/sqlite/src/vacuum.c:130`
    /// refuses rather than writing over.
    OutputExists,
    /// A `VACUUM ... INTO` whose expression answers other than text.
    NonTextFilename,
    /// A `DROP TABLE` over a view or a `DROP VIEW` over a table, with the
    /// word of what the name carries and the name.
    DropKind(bool, Vec<u8>),
    /// A word after a name of the column list of a `CREATE VIEW`, with
    /// that name.
    AfterViewColumn(Vec<u8>),
    /// A view whose column list is of another width than its statement,
    /// with the name, how many names it wrote and how many columns the
    /// statement answers.
    ViewWidth(Vec<u8>, usize, usize),
    /// A `RAISE` outside the body of a trigger.
    RaiseInTrigger,
    /// Two `WITH` terms of one statement under one name.
    DuplicateTerm(Vec<u8>),
    /// `WITH` terms that read each other.
    Circular(Vec<u8>),
    /// An `ORDER BY` or a `LIMIT` written on a core of a compound other
    /// than the last, with which clause it is and the word that joins
    /// that core to the one after it.
    BeforeCompound(bool, Compound),
    /// A `NATURAL` join with a condition written on it as well, which
    /// `sqlite3ProcessJoin` refuses.
    NaturalJoin,
    /// A `USING` that names a column one of the two tables does not
    /// have, named.
    JoinColumn(Vec<u8>),
    /// A `*` over two tables of one name, every column of which two
    /// tables would answer.
    Ambiguous,
    /// A computed column that names itself, or names a column no table
    /// has.
    Computed,
    /// A `WITH` term that writes more or fewer column names than its
    /// statement answers columns, with the name of the term, how many
    /// values its statement answers and how many names it wrote.
    Names(Vec<u8>, usize, usize),
    /// A statement used as a value, or looked in by an `IN`, that
    /// answers another number of columns than the place it stands in
    /// takes, with the two counts.
    Columns(usize, usize),
    /// A `BEGIN` on a connection that already has a transaction open,
    /// which `sqlite3BeginTransaction` refuses.
    Nested,
    /// A `CREATE` of a name the schema already holds, with the word for
    /// what holds it and that name, which `sqlite3StartTable` and
    /// `sqlite3CreateIndex` refuse unless the statement writes `IF NOT
    /// EXISTS`.
    Exists(Vec<u8>, Vec<u8>),
    /// A `CREATE` of an index whose name a table or a view holds, or of
    /// a table or a view whose name an index holds, with the word for
    /// what holds it and that name.
    AlreadyNamed(Vec<u8>, Vec<u8>),
    /// A `CREATE` of a name SQLite keeps for itself, which is one that
    /// begins `sqlite_`.
    Reserved(Vec<u8>),
    /// A statement that names a schema this connection does not hold.
    NoSchema(Vec<u8>),
    /// A `DETACH` of a name the connection holds no database under.
    NoDatabase(Vec<u8>),
    /// A `DETACH` of `main` or of `temp`, which no statement takes away.
    KeptDatabase(Vec<u8>),
    /// An `ATTACH` past the tenth database of a connection, which
    /// `SQLITE_MAX_ATTACHED` of `research/sqlite/src/sqliteLimit.h:179`
    /// holds it to.
    TooManyAttached,
    /// An `ATTACH` under a name the connection already holds a database
    /// under.
    DatabaseInUse(Vec<u8>),
    /// An `ATTACH` of a file name the opening function answered nothing
    /// for.
    NoDatabaseFile(Vec<u8>),
    /// An `ATTACH` of a file whose encoding is not the one of `main`.
    AttachEncoding,
    /// A `VACUUM` on a connection with a transaction open.
    VacuumInTransaction,
    /// A `PRAGMA synchronous = value` on a connection with a transaction
    /// open.
    SafetyInTransaction,
    /// A `PRAGMA encoding = value` naming no encoding the library holds,
    /// with the name it was written as.
    NoEncoding(Vec<u8>),
    /// A `COMMIT` or a `ROLLBACK` on a connection with no transaction
    /// open, the truth naming which of the two the statement was.
    NoTransaction(bool),
    /// An `ALTER TABLE ... RENAME TO` whose new name the schema already
    /// holds, with that name.
    Named(Vec<u8>),
    /// An `ALTER TABLE ... DROP COLUMN` of a column the table does not
    /// hold, with the name as it was written.
    NoSuchColumn(Vec<u8>),
    /// A blob handle over a view, with the name of the view.
    BlobView(Vec<u8>),
    /// A blob handle over a table written `WITHOUT ROWID`, with the name
    /// of the table.
    BlobKeyed(Vec<u8>),
    /// A blob handle that writes a column an index, the primary key or a
    /// foreign key holds, with which of the three holds it.
    BlobColumn(Vec<u8>),
    /// A blob handle over a value that is neither text nor bytes, with
    /// the name of the type it carries.
    BlobValue(Vec<u8>),
    /// A blob handle over a key no row of the table carries, with that
    /// key.
    NoRowid(i64),
    /// A read or a write of a blob handle that reaches past the value,
    /// which carries no message of its own.
    BlobRange,
    /// An `INSERT` that names a column the table does not hold, with the
    /// table and the column.
    NoNamedColumn(Vec<u8>, Vec<u8>),
    /// An `INSERT` that names columns and answers another number of
    /// values, with the values and the columns.
    ValueCount(usize, usize),
    /// An `INSERT` that names no column and answers another number of
    /// values than the table has columns, with the table, its columns
    /// and the values.
    ColumnCount(Vec<u8>, usize, usize),
    /// An `ALTER TABLE ... DROP COLUMN` of a column a key of the table
    /// is over, with the word of that key and the name of the column.
    KeyColumn(Vec<u8>, Vec<u8>),
    /// An `ALTER TABLE ... DROP COLUMN` of the one column a table has,
    /// with its name.
    LastColumn(Vec<u8>),
    /// A statement of the schema that no longer reads after a column
    /// was dropped: what it makes, its name, and what reading it
    /// refused.
    AfterDrop(Vec<u8>, Vec<u8>, alloc::string::String),
    /// A statement of the schema that no longer reads after a column
    /// was renamed: what it makes, its name, and what reading it
    /// refused.
    AfterRename(Vec<u8>, Vec<u8>, alloc::string::String),
    /// An `ALTER TABLE ... DROP CONSTRAINT` of a name the statement
    /// that made the table does not hold.
    NoConstraint(Vec<u8>),
    /// An `ALTER TABLE ... DROP CONSTRAINT` of a constraint that is
    /// neither a `CHECK` nor a `NOT NULL`, which are the two
    /// `alterDropConstraintFunc` cuts.
    KeptConstraint(Vec<u8>),
    /// An `ALTER TABLE` that writes a constraint the rows of the table
    /// do not hold to, which `sqlite3AlterSetNotNull` refuses.
    Constraint,
    /// An `ALTER TABLE ... ADD CONSTRAINT` of a name the statement that
    /// made the table already holds.
    HeldConstraint(Vec<u8>),
    /// A value written where the key of the table stands that is no
    /// whole number, which `sqlite3_column_int64` of the key refuses.
    Mismatch,
    /// An `ON CONFLICT` clause whose columns are the columns of no key
    /// of the table, which `sqlite3UpsertAnalyzeTarget` refuses.
    NoUpsertKey,
    /// An `ALTER TABLE` over a table SQLite keeps for itself, with the
    /// name of that table.
    NotAlterable(Vec<u8>),
    /// An `ALTER TABLE` over a view, with the name of the view.
    NotATable(Vec<u8>),
    /// A `DISTINCT` on an ordered-set aggregate, with the name of that
    /// aggregate.
    OrderedDistinct(Vec<u8>),
    /// A token the tokenizer read as no token at all, as it was
    /// written.
    Unrecognized(Vec<u8>),
    /// The table an `UPDATE` changes named again in its `FROM`, as it
    /// was written there.
    TargetInFrom(Vec<u8>),
    /// A table an `ALTER TABLE` left in a state the schema cannot be
    /// read from, with the table, the words for the kind of alter, and
    /// what reading the schema refused.
    AfterAlter(Vec<u8>, Vec<u8>, Vec<u8>),
    /// A value a column of a `STRICT` table may not hold, with the type
    /// of the value, the type of the column, the table and the column.
    StoredType(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>),
    /// An `INSERT`, an `UPDATE` or a `DELETE` over a view the schema
    /// carries no `INSTEAD OF` trigger of that event for, with the name
    /// of the view.
    ViewWrite(Vec<u8>),
    /// A `RELEASE` or a `ROLLBACK TO` that names a savepoint the
    /// connection does not hold open, with the name as it was written.
    NoSavepoint(Vec<u8>),
    /// A row that shares a key with one the table already holds, where
    /// the statement said to refuse it and undo what it wrote, with
    /// the columns the key is over as `table.column`.
    Unique(Vec<u8>),
    /// A row that holds nothing where a column refuses nothing, with
    /// that column as `table.column`.
    NotNull(Vec<u8>),
    /// A row a `CHECK` of the table does not hold for, with the name
    /// of the constraint or the text of the expression.
    Check(Vec<u8>),
    /// A `WITH` term that reads itself and answered more rows than
    /// `RECURSION_ROWS` allows.
    Recursion,
    /// A shape of statement this engine does not answer yet: a `WITH`
    /// term that reads itself under an operator other than `UNION`, a
    /// table-valued function.
    Unsupported,
    /// An `OVER` that names a window no `WINDOW` clause defines, with
    /// that name.
    NoWindowNamed(Vec<u8>),
    /// A window that writes again what the window it builds on wrote,
    /// with the words for what it wrote and the name of that window.
    Override(Vec<u8>, Vec<u8>),
    /// A scalar function written under an `OVER`, with its name.
    NotAWindow(Vec<u8>),
    /// A frame offset that is not a whole number of no sign, with
    /// whether it is the one the frame begins at and whether the frame
    /// counts in rows or in groups rather than in the values of its
    /// term.
    FrameOffset(bool, bool),
    /// An `ntile` whose count is not one or more.
    Tiles,
    /// An `nth_value` whose count is not one or more.
    Nth,
    /// A window written in a way `sqlite3WindowAlloc` refuses: a frame
    /// offset that is not a whole number of no sign, a `RANGE` offset
    /// where the window orders by other than one term, an `ntile` or an
    /// `nth_value` whose count is not one or more.
    Frame,
    /// An `OVER` naming a window no `WINDOW` clause defines, or one
    /// that carries a frame or terms the window over it also carries.
    Window,
    /// A row whose foreign key points at no row, or a row that a
    /// foreign key of another table points at and that the statement
    /// would take away or change.
    Foreign,
    /// A foreign key whose parent columns are not the primary key of
    /// the table it names and carry no unique index of their own,
    /// which `sqlite3FkLocateIndex` refuses as `foreign key mismatch`.
    ForeignMismatch(Vec<u8>, Vec<u8>),
    /// The function `sqlite3_commit_hook` told the connection answered
    /// true, which sends the transaction back.
    CommitHook,
    /// A `DROP TABLE` or a `DROP VIEW` over a name SQLite keeps for
    /// itself, which `tableMayNotBeDropped` of
    /// `research/sqlite/src/build.c:3476` refuses.
    NotDroppable(Vec<u8>),
}

impl Error {
    /// The words a compound is refused with, or nothing where the
    /// refusal is another.
    fn compounds(&self) -> Option<alloc::string::String> {
        let shown = |bytes: &[u8]| alloc::string::String::from_utf8_lossy(bytes).into_owned();
        Some(match self {
            Error::Compound(word) => alloc::format!(
                "SELECTs to the left and right of {} do not have the same number of result columns",
                shown(word)
            ),
            Error::OrderMatch(which) => alloc::format!(
                "{} ORDER BY term does not match any column in the result set",
                ordinal(*which)
            ),
            Error::Values => {
                alloc::string::String::from("all VALUES must have the same number of terms")
            }
            Error::BeforeCompound(ordered, operator) => alloc::format!(
                "{} clause should come after {} not before",
                if *ordered { "ORDER BY" } else { "LIMIT" },
                shown(&compound_named(*operator))
            ),
            _ => return None,
        })
    }

    /// The words an `ATTACH` or a `DETACH` is refused with, which
    /// `attachFunc` and `detachFunc` of `research/sqlite/src/attach.c`
    /// write, or nothing where the refusal is another.
    fn databases(&self) -> Option<alloc::string::String> {
        let shown = |bytes: &[u8]| alloc::string::String::from_utf8_lossy(bytes).into_owned();
        Some(match self {
            Error::NoDatabase(name) => alloc::format!("no such database: {}", shown(name)),
            Error::KeptDatabase(name) => {
                alloc::format!("cannot detach database {}", shown(name))
            }
            Error::TooManyAttached => {
                alloc::format!("too many attached databases - max {ATTACHED}")
            }
            Error::DatabaseInUse(name) => {
                alloc::format!("database {} is already in use", shown(name))
            }
            Error::NoDatabaseFile(name) => {
                alloc::format!("unable to open database: {}", shown(name))
            }
            Error::AttachEncoding => alloc::string::String::from(
                "attached databases must use the same text encoding as main database",
            ),
            _ => return None,
        })
    }

    /// The words a definition is refused with, from a name SQLite keeps
    /// for itself to a key that counts up where no key of the table does,
    /// or nothing where the refusal is another.
    fn defining(&self) -> Option<alloc::string::String> {
        let shown = |bytes: &[u8]| alloc::string::String::from_utf8_lossy(bytes).into_owned();
        Some(match self {
            Error::NotIndexable(name) => {
                alloc::format!("table {} may not be indexed", shown(name))
            }
            Error::NotDroppable(name) => {
                alloc::format!("table {} may not be dropped", shown(name))
            }
            Error::NaturalJoin => {
                alloc::string::String::from("a NATURAL join may not have an ON or USING clause")
            }
            Error::JoinColumn(name) => alloc::format!(
                "cannot join using column {} - column not present in both tables",
                shown(name)
            ),
            Error::Names(name, answered, written) => alloc::format!(
                "table {} has {answered} values for {written} columns",
                shown(name)
            ),
            Error::AfterViewColumn(name) => {
                alloc::format!("syntax error after column name \"{}\"", shown(name))
            }
            Error::ViewWidth(name, written, answered) => alloc::format!(
                "expected {written} columns for '{}' but got {answered}",
                shown(name)
            ),
            Error::RaiseInTrigger => {
                alloc::string::String::from("RAISE() may only be used within a trigger-program")
            }
            Error::IndexedView => alloc::string::String::from("views may not be indexed"),
            Error::ConstraintIndex => alloc::string::String::from(
                "index associated with UNIQUE or PRIMARY KEY constraint cannot be dropped",
            ),
            Error::Schema(schema::Error::DuplicateColumn(name)) => {
                alloc::format!("duplicate column name: {}", shown(name))
            }
            Error::Schema(schema::Error::ManyKeys(name)) => {
                alloc::format!("table \"{}\" has more than one primary key", shown(name))
            }
            Error::Schema(schema::Error::MissingKey(name)) => {
                alloc::format!("PRIMARY KEY missing on table {}", shown(name))
            }
            Error::Schema(schema::Error::NoSuchColumn(name)) => {
                alloc::format!("no such column: {}", shown(name))
            }
            Error::Schema(schema::Error::KeyExpression) => alloc::string::String::from(
                "expressions prohibited in PRIMARY KEY and UNIQUE constraints",
            ),
            Error::Schema(schema::Error::Autoincrement) => alloc::string::String::from(
                "AUTOINCREMENT is only allowed on an INTEGER PRIMARY KEY",
            ),
            Error::Schema(schema::Error::AutoincrementWithoutRowid) => {
                alloc::string::String::from("AUTOINCREMENT not allowed on WITHOUT ROWID tables")
            }
            _ => return None,
        })
    }

    /// The words an aggregate written where no group has been made is
    /// refused with, or nothing where the refusal is another.
    fn misused(&self) -> Option<alloc::string::String> {
        let shown = |bytes: &[u8]| alloc::string::String::from_utf8_lossy(bytes).into_owned();
        Some(match self {
            Error::MisusedAggregate(name) => {
                alloc::format!("misuse of aggregate function {}()", shown(name))
            }
            Error::NoTables => alloc::string::String::from("no tables specified"),
            Error::ExplicitNulls(first) => alloc::format!(
                "unsupported use of NULLS {}",
                if *first { "FIRST" } else { "LAST" }
            ),
            Error::JoinType(words) => {
                alloc::format!("unknown join type: {}", shown(words))
            }
            Error::Rightward => {
                alloc::string::String::from("ON clause references tables to its right")
            }
            Error::TriggerVariable => alloc::string::String::from("trigger cannot use variables"),
            Error::PointingDefault => alloc::string::String::from(
                "Cannot add a REFERENCES column with non-NULL default value",
            ),
            Error::Added(added) => alloc::string::String::from(added.words()),
            Error::DuplicateTerm(name) => {
                alloc::format!("duplicate WITH table name: {}", shown(name))
            }
            Error::Circular(name) => alloc::format!("circular reference: {}", shown(name)),
            Error::QualifiedInTrigger => alloc::string::String::from(concat!(
                "qualified table names are not allowed on ",
                "INSERT, UPDATE, and DELETE statements within triggers"
            )),
            Error::IndexedInTrigger(not) => alloc::format!(
                "the {} clause is not allowed on UPDATE or DELETE statements within triggers",
                if *not { "NOT INDEXED" } else { "INDEXED BY" }
            ),
            Error::Columns(answered, wanted) => {
                alloc::format!("sub-select returns {answered} columns - expected {wanted}")
            }
            Error::Image(crate::error::Error::Full) => {
                alloc::string::String::from("database or disk is full")
            }
            // `sqlite3ErrStr` writes one message for every byte of a
            // file the format forbids: the header says whether the file
            // is one at all, and everything under it says the file is
            // damaged.
            Error::Image(
                crate::error::Error::Magic
                | crate::error::Error::Truncated
                | crate::error::Error::PageSize(_)
                | crate::error::Error::Reserved(_)
                | crate::error::Error::Fractions
                | crate::error::Error::Encoding(_),
            ) => alloc::string::String::from("file is not a database"),
            Error::Image(_) => alloc::string::String::from("database disk image is malformed"),
            Error::NoSchema(name) => alloc::format!("unknown database {}", shown(name)),
            Error::OutputExists => alloc::string::String::from("output file already exists"),
            Error::NonTextFilename => alloc::string::String::from("non-text filename"),
            Error::DropKind(view, name) => alloc::format!(
                "use DROP {} to delete {} {}",
                if *view { "VIEW" } else { "TABLE" },
                if *view { "view" } else { "table" },
                shown(name)
            ),
            Error::VacuumInTransaction => {
                alloc::string::String::from("cannot VACUUM from within a transaction")
            }
            Error::SafetyInTransaction => {
                alloc::string::String::from("Safety level may not be changed inside a transaction")
            }
            Error::NoEncoding(name) => alloc::format!(
                "unsupported encoding: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Schema(schema::Error::IndexColumn(name)) => {
                alloc::format!("no such column: {}", shown(name))
            }
            Error::Schema(schema::Error::Likelihood) => alloc::string::String::from(
                "second argument to likelihood() must be a constant between 0.0 and 1.0",
            ),
            Error::Schema(schema::Error::NoCollation(name)) => {
                alloc::format!("no such collation sequence: {}", shown(name))
            }
            Error::Schema(schema::Error::ForeignColumn(name)) => alloc::format!(
                "unknown column \"{}\" in foreign key definition",
                shown(name)
            ),
            Error::Schema(schema::Error::ForeignWidth) => alloc::string::String::from(
                "number of columns in foreign key does not match the number of columns in the referenced table",
            ),
            Error::GroupedAggregate => alloc::string::String::from(
                "aggregate functions are not allowed in the GROUP BY clause",
            ),
            Error::LooseAggregate(name) => {
                alloc::format!("misuse of aggregate: {}()", shown(name))
            }
            Error::MisusedAlias(name) => {
                alloc::format!("misuse of aliased aggregate {}", shown(name))
            }
            Error::DistinctAggregate => {
                alloc::string::String::from("DISTINCT aggregates must have exactly one argument")
            }
            Error::DistinctWindow => {
                alloc::string::String::from("DISTINCT is not supported for window functions")
            }
            _ => return None,
        })
    }

    /// What an `ALTER TABLE` is refused with, or nothing where the
    /// refusal is another.
    /// The words the types of a `STRICT` table are refused with, and
    /// the words an `ALTER TABLE` that left the schema unreadable is
    /// named by.
    fn datatypes(&self) -> Option<alloc::string::String> {
        let shown = |bytes: &[u8]| alloc::string::String::from_utf8_lossy(bytes).into_owned();
        Some(match self {
            Error::Schema(schema::Error::MissingType(table, column)) => {
                alloc::format!("missing datatype for {}.{}", shown(table), shown(column))
            }
            Error::Schema(schema::Error::UnknownType(table, column, written)) => alloc::format!(
                "unknown datatype for {}.{}: \"{}\"",
                shown(table),
                shown(column),
                shown(written)
            ),
            Error::OrderRange(which, word, most) => alloc::format!(
                "{} {} BY term out of range - should be between 1 and {}",
                ordinal(*which),
                shown(word),
                most
            ),
            Error::NotAWindow(name) => {
                alloc::format!("{}() may not be used as a window function", shown(name))
            }
            Error::FrameOffset(starting, whole) => alloc::format!(
                "frame {} offset must be a non-negative {}",
                if *starting { "starting" } else { "ending" },
                if *whole { "integer" } else { "number" }
            ),
            Error::Tiles => {
                alloc::string::String::from("argument of ntile must be a positive integer")
            }
            Error::Nth => alloc::string::String::from(
                "second argument to nth_value must be a positive integer",
            ),
            Error::NoWindowNamed(name) => {
                alloc::format!("no such window: {}", shown(name))
            }
            Error::Override(what, name) => {
                alloc::format!("cannot override {} of window: {}", shown(what), shown(name))
            }
            Error::Unrecognized(token) => {
                alloc::format!("unrecognized token: \"{}\"", shown(token))
            }
            Error::TargetInFrom(name) => alloc::format!(
                "target object/alias may not appear in FROM clause: {}",
                shown(name)
            ),
            Error::OrderedDistinct(name) => alloc::format!(
                "DISTINCT not allowed on ordered-set aggregate {}()",
                shown(name)
            ),
            Error::AfterAlter(table, word, message) => alloc::format!(
                "error in table {} after {}: {}",
                shown(table),
                shown(word),
                shown(message)
            ),
            Error::StoredType(held, wanted, table, column) => alloc::format!(
                "cannot store {} value in {} column {}.{}",
                shown(held),
                shown(wanted),
                shown(table),
                shown(column)
            ),
            _ => return None,
        })
    }

    /// What a statement that names a table it may not reach is refused
    /// with, a blob handle among them, which `sqlite3_blob_open` of
    /// `research/sqlite/src/vdbeblob.c:74` writes, and nothing for every
    /// other refusal.
    fn opened(&self) -> Option<alloc::string::String> {
        let shown = |name: &[u8]| alloc::string::String::from_utf8_lossy(name).into_owned();
        Some(match self {
            Error::NoTable(name) => alloc::format!("no such table: {}", shown(name)),
            Error::ViewWrite(name) => {
                alloc::format!("cannot modify {} because it is a view", shown(name))
            }
            Error::BlobView(name) => alloc::format!("cannot open view: {}", shown(name)),
            Error::BlobKeyed(name) => {
                alloc::format!("cannot open table without rowid: {}", shown(name))
            }
            Error::BlobColumn(held) => {
                alloc::format!("cannot open {} column for writing", shown(held))
            }
            Error::BlobValue(kind) => alloc::format!("cannot open value of type {}", shown(kind)),
            Error::NoRowid(rowid) => alloc::format!("no such rowid: {rowid}"),
            Error::BlobRange => alloc::string::String::from("SQL logic error"),
            _ => return None,
        })
    }

    fn altered(&self) -> Option<alloc::string::String> {
        use alloc::string::ToString as _;
        Some(match self {
            Error::NoSuchColumn(name) => alloc::format!(
                "no such column: \"{}\"",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::KeyColumn(key, name) => alloc::format!(
                "cannot drop {} column: \"{}\"",
                alloc::string::String::from_utf8_lossy(key),
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::LastColumn(name) => alloc::format!(
                "cannot drop column \"{}\": no other columns exist",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::AfterDrop(kind, name, refused) => alloc::format!(
                "error in {} {} after drop column: {}",
                alloc::string::String::from_utf8_lossy(kind),
                alloc::string::String::from_utf8_lossy(name),
                refused
            ),
            Error::AfterRename(kind, name, refused) => alloc::format!(
                "error in {} {} after rename: {}",
                alloc::string::String::from_utf8_lossy(kind),
                alloc::string::String::from_utf8_lossy(name),
                refused
            ),
            Error::NoConstraint(name) => alloc::format!(
                "no such constraint: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::KeptConstraint(name) => alloc::format!(
                "constraint may not be dropped: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Constraint | Error::CommitHook => "constraint failed".to_string(),
            Error::HeldConstraint(name) => alloc::format!(
                "constraint {} already exists",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::NotAlterable(name) => alloc::format!(
                "table {} may not be altered",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::NotATable(name) => alloc::format!(
                "view {} may not be altered",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Named(name) => alloc::format!(
                "there is already another table or index with this name: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            _ => return None,
        })
    }

    /// The result code the refusal carries, which `sqlite3_errcode` and
    /// `sqlite3_extended_errcode` answer.
    ///
    /// Everything the C library answers `SQLITE_ERROR` for, which is
    /// every statement it could not read or run, carries that code, so
    /// the arms below are the refusals that carry another one.
    #[must_use]
    pub const fn code(&self) -> Code {
        match self {
            // `SQLITE_CONSTRAINT_*` of `sqlite.h.in`, each the primary
            // code and the constraint the row broke.
            Error::Unique(_) => Code::broke(2067, b"SQLITE_CONSTRAINT_UNIQUE"),
            Error::NotNull(_) => Code::broke(1299, b"SQLITE_CONSTRAINT_NOTNULL"),
            Error::Check(_) => Code::broke(275, b"SQLITE_CONSTRAINT_CHECK"),
            Error::Foreign | Error::ForeignMismatch(..) => {
                Code::broke(787, b"SQLITE_CONSTRAINT_FOREIGNKEY")
            }
            Error::StoredType(..) => Code::broke(3091, b"SQLITE_CONSTRAINT_DATATYPE"),
            Error::CommitHook => Code::broke(531, b"SQLITE_CONSTRAINT_COMMITHOOK"),
            // `OP_Halt` of `research/sqlite/src/vdbe.c:1337` carries the
            // code the trigger program named, which
            // `sqlite3ExprCodeTarget` sets to this one for a `RAISE` the
            // body of a trigger holds.
            Error::Eval(eval::Error::Raised(..)) => Code::broke(1811, b"SQLITE_CONSTRAINT_TRIGGER"),
            Error::Constraint | Error::HeldConstraint(_) => Code::plain(19, b"SQLITE_CONSTRAINT"),
            Error::Auth(_) => Code::plain(23, b"SQLITE_AUTH"),
            Error::Mismatch => Code::plain(20, b"SQLITE_MISMATCH"),
            Error::Image(crate::error::Error::Full) => Code::plain(13, b"SQLITE_FULL"),
            Error::Image(
                crate::error::Error::Magic
                | crate::error::Error::Truncated
                | crate::error::Error::PageSize(_)
                | crate::error::Error::Reserved(_)
                | crate::error::Error::Fractions
                | crate::error::Error::Encoding(_),
            ) => Code::plain(26, b"SQLITE_NOTADB"),
            Error::Image(_) => Code::plain(11, b"SQLITE_CORRUPT"),
            _ => Code::plain(1, b"SQLITE_ERROR"),
        }
    }

    /// The text the C library writes for this refusal, which is what a
    /// `catchsql` of SQLite's own test files compares.
    ///
    /// A refusal the C library writes a name into is written here
    /// without it, because this crate's refusals carry no names yet.
    #[must_use]
    pub fn message(&self) -> alloc::string::String {
        use alloc::string::ToString as _;
        if let Some(shown) = self
            .altered()
            .or_else(|| self.opened())
            .or_else(|| self.datatypes())
            .or_else(|| self.defining())
            .or_else(|| self.misused())
            .or_else(|| self.databases())
            .or_else(|| self.compounds())
        {
            return shown;
        }
        match self {
            Error::Syntax(token) => alloc::format!(
                "near \"{}\": syntax error",
                alloc::string::String::from_utf8_lossy(token)
            ),
            Error::Incomplete => "incomplete input".to_string(),
            Error::NoObject(kind, name) => alloc::format!(
                "no such {}: {}",
                alloc::string::String::from_utf8_lossy(kind),
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Timed(word, held, name) => alloc::format!(
                "cannot create {} trigger on {}: {}",
                alloc::string::String::from_utf8_lossy(word),
                alloc::string::String::from_utf8_lossy(held),
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::SystemTrigger => "cannot create trigger on system table".to_string(),
            Error::Foreign => "FOREIGN KEY constraint failed".to_string(),
            Error::ForeignMismatch(child, parent) => alloc::format!(
                "foreign key mismatch - \"{}\" referencing \"{}\"",
                alloc::string::String::from_utf8_lossy(child),
                alloc::string::String::from_utf8_lossy(parent)
            ),
            Error::Unique(columns) => alloc::format!(
                "UNIQUE constraint failed: {}",
                alloc::string::String::from_utf8_lossy(columns)
            ),
            Error::NotNull(column) => alloc::format!(
                "NOT NULL constraint failed: {}",
                alloc::string::String::from_utf8_lossy(column)
            ),
            Error::Check(shown) => alloc::format!(
                "CHECK constraint failed: {}",
                alloc::string::String::from_utf8_lossy(shown)
            ),
            Error::Exists(kind, name) => alloc::format!(
                "{} {} already exists",
                alloc::string::String::from_utf8_lossy(kind),
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::AlreadyNamed(kind, name) => alloc::format!(
                "there is already {} {} named {}",
                if kind == b"index" { "an" } else { "a" },
                alloc::string::String::from_utf8_lossy(kind),
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Reserved(name) => alloc::format!(
                "object name reserved for internal use: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Nested => "cannot start a transaction within a transaction".to_string(),
            Error::NoTransaction(rolling) => rolled(*rolling),
            Error::Mismatch => "datatype mismatch".to_string(),
            Error::NoNamedColumn(table, column) => alloc::format!(
                "table {} has no column named {}",
                alloc::string::String::from_utf8_lossy(table),
                alloc::string::String::from_utf8_lossy(column)
            ),
            Error::ValueCount(values, columns) => {
                alloc::format!("{values} values for {columns} columns")
            }
            Error::ColumnCount(table, columns, values) => alloc::format!(
                "table {} has {columns} columns but {values} values were supplied",
                alloc::string::String::from_utf8_lossy(table)
            ),
            Error::NoUpsertKey => {
                "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint".to_string()
            }
            Error::NoSavepoint(name) => alloc::format!(
                "no such savepoint: {}",
                alloc::string::String::from_utf8_lossy(name)
            ),
            Error::Schema(schema::Error::IndexDot) => {
                "the \".\" operator prohibited in index expressions".to_string()
            }
            Error::Recursion => "recursive aggregate queries not supported".to_string(),
            Error::Eval(error) => error.message(),
            Error::Auth(error) => error.message(),
            other => alloc::format!("{other:?}"),
        }
    }
}

/// A count with the two letters that name its place, which is `%r` of
/// `sqlite3_mprintf`: `1st`, `2nd`, `3rd` and `4th` onward, with the
/// teens taking `th`.
fn ordinal(count: usize) -> alloc::string::String {
    let word = match (count % 100, count % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    alloc::format!("{count}{word}")
}

impl From<error::Error> for Error {
    fn from(error: error::Error) -> Self {
        Error::Image(error)
    }
}

impl From<parse::Error> for Error {
    fn from(error: parse::Error) -> Self {
        Error::Parse(error)
    }
}

impl Error {
    /// This refusal with a parse written as the C library writes it:
    /// `near \"TOKEN\": syntax error` for a token the parser stopped
    /// at, and `incomplete input` where the tokens ran out first.
    ///
    /// Reading the token costs O(1).
    #[must_use]
    pub fn near(self, sql: &[u8]) -> Self {
        let Error::Parse(error) = self else {
            return self;
        };
        let held = sql.get(error.at..error.at.saturating_add(error.len));
        if error.expected == parse::Expected::OrderedDistinct {
            return Error::OrderedDistinct(held.unwrap_or_default().to_vec());
        }
        if error.expected == parse::Expected::Unrecognized {
            return Error::Unrecognized(held.unwrap_or_default().to_vec());
        }
        if let parse::Expected::ExplicitNulls(first) = error.expected {
            return Error::ExplicitNulls(first);
        }
        if error.expected == parse::Expected::JoinType {
            return Error::JoinType(held.unwrap_or_default().to_vec());
        }
        if let parse::Expected::BeforeCompound(ordered, operator) = error.expected {
            return Error::BeforeCompound(ordered, operator);
        }
        if error.expected == parse::Expected::TargetInFrom {
            return Error::TargetInFrom(held.unwrap_or_default().to_vec());
        }
        if error.expected == parse::Expected::AfterViewColumn {
            return Error::AfterViewColumn(held.unwrap_or_default().to_vec());
        }
        if error.expected == parse::Expected::RaiseInTrigger {
            return Error::RaiseInTrigger;
        }
        match held.filter(|token| !token.is_empty()) {
            Some(token) => Error::Syntax(token.to_vec()),
            None => Error::Incomplete,
        }
    }
}

impl From<crate::constraint::Error> for Error {
    fn from(error: crate::constraint::Error) -> Self {
        match error {
            crate::constraint::Error::NoSuch(name) => Error::NoConstraint(name),
            crate::constraint::Error::Kept(name) => Error::KeptConstraint(name),
        }
    }
}

impl From<schema::Error> for Error {
    fn from(error: schema::Error) -> Self {
        Error::Schema(error)
    }
}

impl From<eval::Error> for Error {
    fn from(error: eval::Error) -> Self {
        Error::Eval(error)
    }
}

impl From<crate::auth::Error> for Error {
    fn from(error: crate::auth::Error) -> Self {
        Error::Auth(error)
    }
}

/// One table of the schema, and where its rows are.
#[derive(Clone, Debug)]
struct Stored {
    /// The table as its statement describes it.
    table: Table,
    /// Which database of the connection it stands in, which is nought for
    /// the one the reader was opened over and one past the place of an
    /// attached one.
    place: usize,
    /// The page its tree begins at.
    root: u32,
    /// The `CREATE` text, which a computed column's expression points
    /// into.
    sql: Vec<u8>,
    /// The tree that text was parsed into.
    arena: Arena,
    /// Where each column's value stands in the record, which is not the
    /// column's own place where a column is computed and not stored or
    /// the table keeps its rows in the key's own tree.
    places: Vec<usize>,
    /// The indexes over it that this crate can walk.
    indexes: Vec<Kept>,
}

/// Raises where `INDEXED BY name` names an index the table does not
/// hold.
///
/// `sqlite3IndexedByLookup` of `research/sqlite/src/build.c:4600` reads
/// the name against the indexes of that table alone, so an index of
/// another table is one the table does not hold. Reading them costs
/// O(n) in their number.
///
/// # Errors
///
/// [`Error::NoObject`] names the index.
fn indexed_held(stored: &Stored, indexed: crate::ast::Indexed, sql: &[u8]) -> Result<(), Error> {
    let crate::ast::Indexed::By(span) = indexed else {
        return Ok(());
    };
    let name = dequote(span.text(sql));
    if stored
        .indexes
        .iter()
        .any(|kept| kept.index.name.eq_ignore_ascii_case(&name))
    {
        return Ok(());
    }
    Err(Error::NoObject(b"index".to_vec(), name))
}

/// One index of a table, and where its tree is.
#[derive(Clone, Debug)]
struct Kept {
    /// The index as its statement describes it.
    index: schema::Index,
    /// The page its tree begins at.
    root: u32,
    /// The `CREATE INDEX` text, which a place over an expression and a
    /// partial index's `WHERE` point into.
    sql: Vec<u8>,
    /// The tree that text was parsed into.
    arena: Arena,
}

/// One index of a table as a caller outside this module reads it: the
/// index, where its tree is, and what an expression it holds reads.
#[derive(Clone, Copy)]
pub struct Indexed<'a> {
    /// The index as its statement describes it.
    pub index: &'a schema::Index,
    /// The page its tree begins at.
    pub root: u32,
    /// The `CREATE INDEX` text.
    pub sql: &'a [u8],
    /// The tree that text was parsed into.
    pub arena: &'a Arena,
    /// The table the index is over, which an expression reads.
    pub table: &'a Table,
}

/// One column a statement answers, and what a comparison against it
/// does.
#[derive(Clone, Debug)]
struct Column {
    /// Its name, which is what a name written in the statement above
    /// it reaches it by.
    name: Vec<u8>,
    /// The name the statement answers it under, which the two pragmas
    /// [`Naming`] carries move away from its name.
    shown: Vec<u8>,
    /// The name of the side it came from, where the side it stands on
    /// answers the columns of several: a statement written inside a
    /// `FROM` for the tables inside brackets carries the name of each
    /// of them, so `t2.a` reaches the column `a` of `t2` through it.
    /// Empty where the side answers under a name of its own.
    from: Vec<u8>,
    /// Whether a `*` leaves the column out, which the column a `USING`
    /// or a `NATURAL` matched is left out by. A statement that stands
    /// for the tables inside brackets answers the column all the same,
    /// so that `t3.a` reaches it.
    hidden: bool,
    /// What is converted before it is compared.
    affinity: Affinity,
    /// How its text is compared, where anything was written about it.
    collation: Option<Collation>,
    /// Which classes the expression it came from may answer, which is
    /// what `sqlite3ExprDataType` reads off it: one bit for a number,
    /// one for text and one for a blob.
    datatype: u8,
    /// The type the schema declares for the column it came from, and
    /// nothing where it came from an expression, which is what
    /// `sqlite3_column_decltype` answers.
    declared: Vec<u8>,
}

/// The columns a side of a `FROM` answers.
///
/// A table of the schema, a statement written inside the `FROM` and a
/// `WITH` term answer the same question here, so nothing below this
/// asks which of the three it is reading.
#[derive(Clone, Debug, Default)]
struct Shape {
    /// The columns, in the order the side answers them.
    columns: Vec<Column>,
    /// Whether the side stands for the tables written inside brackets
    /// in a `FROM`, which answer under their own names through it.
    nested: bool,
    /// Whether a bare `rowid`, `oid` or `_rowid_` reaches the side.
    keyed: bool,
    /// The name the key is answered under, where a column of the side
    /// is another name for the rowid.
    key: Option<Vec<u8>>,
}

impl Shape {
    /// Where the column `name` stands, where the side has one.
    fn place(&self, name: &[u8]) -> Option<usize> {
        self.columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
    }

    /// Whether the side has a column of this name.
    fn has(&self, name: &[u8]) -> bool {
        self.place(name).is_some()
    }

    /// What a name reaches on this side: one of its columns, or its
    /// rowid where it keeps one.
    ///
    /// A column of the side's own is what a name reaches first, so a
    /// table with a column called `rowid` answers that column and not
    /// the key. The one name that reaches the key while naming a column
    /// is the column the key is another name for.
    fn reaches(&self, name: &[u8]) -> Option<Reached> {
        if let Some(place) = self.place(name) {
            let key = self
                .key
                .as_deref()
                .is_some_and(|key| key.eq_ignore_ascii_case(name));
            return Some(if key {
                Reached::Key
            } else {
                Reached::Column(place)
            });
        }
        // The three names the rowid answers to, which a statement inside
        // a `FROM` and a table that keeps its rows in the key's own tree
        // both refuse.
        let rowid = self.keyed
            && (name.eq_ignore_ascii_case(b"rowid")
                || name.eq_ignore_ascii_case(b"oid")
                || name.eq_ignore_ascii_case(b"_rowid_"));
        rowid.then_some(Reached::Key)
    }
}

/// One column the `FROM` of an `UPDATE` answers.
#[derive(Clone, Debug)]
pub struct Beside {
    /// The name it answers under.
    pub name: Vec<u8>,
    /// The table it came from, which a name written `t.a` matches.
    pub from: Vec<u8>,
    /// What is converted before it is compared.
    pub affinity: Affinity,
    /// How its text is compared.
    pub collation: Collation,
}

/// What a statement answered, and what a comparison against each of its
/// columns does.
#[derive(Clone, Debug, Default)]
struct Answered {
    /// The names and the rows.
    answer: Answer,
    /// The columns.
    shape: Shape,
}

/// What a statement is answered against: the `WITH` terms it may name,
/// and the row of the statement that encloses it.
#[derive(Clone, Copy)]
struct Scope<'a> {
    /// The `WITH` terms in scope, each already answered.
    terms: &'a [(Vec<u8>, Answered)],
    /// The row the enclosing statement stands on, which a correlated
    /// statement reads its columns from.
    outer: Option<&'a dyn eval::Row>,
    /// How many views the statement is being answered inside, which
    /// stops a view that names itself.
    views: u32,
}

/// How many views deep a statement is answered, which is what
/// `SQLITE_MAX_VIEW_DEPTH` bounds a view that names itself by.
const VIEW_DEPTH: u32 = 32;

/// How many databases an `ATTACH` may add to one connection, which is
/// `SQLITE_MAX_ATTACHED` of `research/sqlite/src/sqliteLimit.h:179`.
pub const ATTACHED: usize = 10;

/// How many rows a `WITH` term that reads itself may answer.
///
/// SQLite hands one row on as it answers it, so a term that does not
/// stop is stopped by the `LIMIT` of the statement that reads it. This
/// crate answers the rows of a term into memory before the statement
/// that reads them, which D2 records, so a term that does not stop is
/// stopped by this count instead.
const RECURSION_ROWS: usize = 100_000;

/// What a view puts in the place of a table: the columns it answers,
/// the rows its statement answered, and the name it is known by.
struct Viewed {
    /// The columns it answers.
    shape: Shape,
    /// The rows its statement answered.
    rows: Vec<Vec<Value>>,
    /// The name the view is known by.
    name: Vec<u8>,
    /// Which database of the connection the view stands in.
    place: usize,
}

/// What a statement written inside an expression is answered by.
///
/// A cursor carries one so that `(SELECT ...)`, `EXISTS` and `IN` are
/// answered where they are read and not before: an `ON` condition, a
/// `WHERE` and a `HAVING` each read one, and a `CASE` reads only the
/// branch it takes.
#[derive(Clone, Copy)]
struct Reach<'a> {
    /// The database the statement reads.
    database: &'a Database<'a>,
    /// The tree the statement was parsed into.
    arena: &'a Arena,
    /// The text that tree points into.
    sql: &'a [u8],
    /// The `WITH` terms in scope, and the row that encloses this one.
    scope: Scope<'a>,
}

/// Where a side of a `FROM` draws its rows.
enum Source<'a> {
    /// A table of the schema.
    Table(&'a Stored),
    /// The rows a statement answered, whether it was written inside the
    /// `FROM` or named by a `WITH`.
    Rows(Vec<Vec<Value>>),
}

/// One side of a `FROM` clause, and how it attaches to the ones before
/// it.
struct Side<'a> {
    /// The columns it answers.
    shape: Shape,
    /// Where its rows come from.
    source: Source<'a>,
    /// The name of the database the table it reads stands in, and
    /// nothing for a statement written inside the `FROM` and for a `WITH`
    /// term, which no schema names.
    schema: Vec<u8>,
    /// What the statement calls it: its alias, or the name of the table
    /// or the `WITH` term it reads, or nothing for a statement written
    /// inside the `FROM` with no alias.
    name: Vec<u8>,
    /// The name of the table or the view it reads, which an alias does
    /// not move, and nothing for a statement written inside the `FROM`.
    /// `PRAGMA full_column_names` names a column by this.
    table: Vec<u8>,
    /// Which join attaches it.
    kind: JoinKind,
    /// The `ON` condition written on it.
    on: Option<ExprId>,
    /// The columns a `USING` or a `NATURAL` matches it by, which are
    /// also the columns it does not answer a bare name with.
    using: Vec<Vec<u8>>,
    /// How the walk reads it.
    plan: Plan,
    /// The terms of the `WHERE` every side up to this one answers, read
    /// where this side's row is read rather than once per row of the
    /// sides after it.
    pushed: Vec<ExprId>,
}

/// What the column after the key of a plan is held between, which is
/// the range `sqlite3WhereBegin` writes the seek and the terminating
/// comparison of a loop from.
#[derive(Clone, Debug, Default)]
struct Bounds {
    /// The lowest value of that column, and whether an entry holding it
    /// is one the walk takes.
    low: Option<(Value, bool)>,
    /// The highest value of that column, and whether an entry holding it
    /// is one the walk takes.
    high: Option<(Value, bool)>,
}

impl Bounds {
    /// Whether neither end holds the column.
    const fn is_empty(&self) -> bool {
        self.low.is_none() && self.high.is_none()
    }
}

/// How a side's rows are read.
#[derive(Clone, Debug)]
enum Plan {
    /// The tree of the side, from the rowid the walk descends to up to
    /// the one it ends past. Both nothing is a scan of the whole tree.
    Rows(Option<i64>, Option<i64>),
    /// The rows an index names, which a `WHERE` holds to one key.
    Keyed {
        /// The page the index's tree begins at.
        root: u32,
        /// The values every entry the walk takes begins with.
        key: Vec<Value>,
        /// What each of them compares under, and after them what the
        /// column the bounds hold compares under.
        collations: Vec<Collation>,
        /// Where the rowid stands in an entry, which is after every
        /// column the index holds.
        rowid_at: usize,
        /// What the column after the key is held between.
        bounds: Bounds,
        /// Whether the index holds its entries from the largest value
        /// down, which a `CREATE INDEX` writing `DESC` makes it do on a
        /// file of schema format 4.
        backwards: bool,
    },
    /// The rows of every plan it holds, each row once, which is what
    /// an `OR` whose every branch names a key is read by. It is not the
    /// `UNION` of the grammar.
    Union(Vec<Plan>),
    /// The rows an index names, which an `ON` holds to the key a side
    /// already read answers, so the key is a value per row of that side
    /// and not one value for the statement.
    Joined {
        /// The page the index's tree begins at.
        root: u32,
        /// What the key is read from, once per row of the sides before
        /// this one.
        key: ExprId,
        /// What the key compares under.
        collation: Collation,
        /// What the column of the index converts a value under.
        affinity: Affinity,
        /// Where the rowid stands in an entry.
        rowid_at: usize,
    },
}

impl Side<'_> {
    /// The rowids the walk of the side's own tree is held to.
    const fn range(&self) -> (Option<i64>, Option<i64>) {
        match self.plan {
            Plan::Rows(first, last) => (first, last),
            Plan::Keyed { .. } | Plan::Joined { .. } | Plan::Union(_) => (None, None),
        }
    }

    /// Whether `named` is what the statement calls this side.
    fn named(&self, named: &[u8]) -> bool {
        !self.name.is_empty() && self.name.eq_ignore_ascii_case(named)
    }

    /// One column of this side as the statement above it answers the
    /// column: under this side's name where the side has one, and
    /// under the name it already carries where the side is the tables
    /// inside brackets, which answer under their own names.
    fn answered_as(&self, column: &Column) -> Column {
        let mut answered = column.clone();
        if !self.name.is_empty() && !self.shape.nested {
            answered.from.clone_from(&self.name);
        }
        answered
    }

    /// The name a `*` over this side answers one column under: the
    /// column's own name, with the name the statement calls this side
    /// by in front of it where `long` is set.
    fn shown_as(&self, column: &Column, long: bool) -> Vec<u8> {
        if !long {
            return column.name.clone();
        }
        let mut shown = self.called().to_vec();
        shown.push(b'.');
        shown.extend_from_slice(&column.name);
        shown
    }

    /// What a name in front of a column of this side reads as: what the
    /// statement calls the side, and the name of what it reads where it
    /// carries no name of its own.
    fn called(&self) -> &[u8] {
        if self.name.is_empty() {
            return &self.table;
        }
        &self.name
    }
}

/// What the name a result column is answered under is worked out from.
#[derive(Clone, Copy)]
struct ShownAs<'a> {
    /// The two pragmas the connection holds.
    naming: Naming,
    /// Whether an `AS` names the column.
    alias: bool,
    /// The name the statement reaches the column by.
    name: &'a [u8],
    /// The text the column was written as.
    text: Span,
    /// The table and the column of a column reference, and nothing
    /// where the result column is an expression.
    written: Option<(Option<Span>, Span)>,
}

/// The name one result column is answered under, which is
/// `sqlite3GenerateColumnNames`: an `AS` name is the name; a column
/// answers under its own name where either pragma is on, with the name
/// of its table in front of it where `full_column_names` is; everything
/// else answers under the text it was written as.
fn shown_name(sql: &[u8], sides: &[Side<'_>], shown: ShownAs<'_>) -> Vec<u8> {
    let Some((table, column)) = shown.written.filter(|_| !shown.alias) else {
        return shown.name.to_vec();
    };
    if shown.naming.full {
        let mut named = table_of(sides, table, column, sql);
        named.push(b'.');
        named.extend_from_slice(shown.name);
        return named;
    }
    if shown.naming.short {
        return shown.name.to_vec();
    }
    shown.text.text(sql).to_vec()
}

/// The name of the table one column reference reads, which is
/// `pTab->zName` of the resolved column: the table a name in front of
/// the column names, or the first side that holds the column.
fn table_of(sides: &[Side<'_>], table: Option<Span>, column: Span, sql: &[u8]) -> Vec<u8> {
    let named = table.map(|span| dequote(span.text(sql)));
    let name = dequote(column.text(sql));
    let found = match &named {
        Some(named) => sides.iter().find(|side| side.named(named)),
        None => sides.iter().find(|side| side.shape.has(&name)),
    };
    match found {
        Some(side) => side.table.clone(),
        None => named.unwrap_or_default(),
    }
}

/// The name `sqlite3SelectExpand` gives a statement written inside a
/// `FROM` that carries no alias, which `full_column_names` writes in
/// front of the columns it answers.
fn subquery_named(id: crate::ast::SelectId) -> Vec<u8> {
    let mut named = b"(subquery-".to_vec();
    named.extend_from_slice(&number::integer_text(i64::from(id.place())));
    named.push(b')');
    named
}

/// Whether a `*` leaves the column out: the column a `USING` or a
/// `NATURAL` matched, which the side that matched it hides, and the
/// column a statement standing for the tables inside brackets answers
/// under the name of the table it came from alone.
fn left_out(column: &Column, using: &[Vec<u8>]) -> bool {
    column.hidden
        || using
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&column.name))
}

/// The columns of a statement as a table holds them, with the names
/// made unique: a name a column before it carries takes `:1` after it,
/// `:2` for the one after that, and a name that already ends in a colon
/// and digits loses them before it takes its own.
///
/// Every name is read against the ones before it, so the walk is
/// O(columns²) in the width of the statement.
fn uniqued(mut shape: Shape) -> Shape {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for column in &mut shape.columns {
        let mut count: u32 = 0;
        while seen
            .iter()
            .any(|held| held.eq_ignore_ascii_case(&column.name))
        {
            count = count.saturating_add(1);
            let mut named = untailed(&column.name);
            named.push(b':');
            named.extend_from_slice(&number::integer_text(i64::from(count)));
            column.name = named;
        }
        seen.push(column.name.clone());
        column.shown.clone_from(&column.name);
    }
    shape
}

/// A name with a trailing colon and digits taken off, which is what
/// `sqlite3ColumnsFromExprList` reads back off a name it made unique.
fn untailed(name: &[u8]) -> Vec<u8> {
    let mut at = name.len();
    while at > 1
        && name
            .get(at.saturating_sub(1))
            .is_some_and(u8::is_ascii_digit)
    {
        at = at.saturating_sub(1);
    }
    if name.get(at.saturating_sub(1)) == Some(&b':') {
        return name
            .get(..at.saturating_sub(1))
            .unwrap_or_default()
            .to_vec();
    }
    name.to_vec()
}

/// The columns a table of the schema answers.
fn shape_of(table: &Table) -> Shape {
    Shape {
        nested: false,
        columns: table
            .columns
            .iter()
            .map(|column| Column {
                name: column.name.clone(),
                shown: column.name.clone(),
                from: Vec::new(),
                hidden: false,
                affinity: column.affinity,
                collation: Some(column.collation),
                datatype: classes_of(column.affinity),
                declared: column.declared.clone(),
            })
            .collect(),
        keyed: !table.without_rowid,
        key: table
            .rowid_alias
            .and_then(|at| table.columns.get(at))
            .map(|column| column.name.clone()),
    }
}

/// A database file, with its schema read.
#[derive(Clone, Debug)]
pub struct Database<'a> {
    /// The file.
    image: Image<'a>,
    /// The name the database at schema place nought answers to, which is
    /// `main` where no statement named another.
    named: Vec<u8>,
    /// The databases an `ATTACH` added, in the order they were attached,
    /// with the temp schema among them where the connection made one.
    attached: Vec<Attached<'a>>,
    /// The place of the temp schema, where the reader carries one.
    temp: Option<usize>,
    /// Whether a name is read in the database at place nought before the
    /// temp schema, which the connection that writes reads names in,
    /// because the statement named that database and the rows it writes
    /// are the ones of its table.
    writing: bool,
    /// Its tables.
    tables: Vec<Stored>,
    /// Its views.
    views: Vec<View>,
    /// Its triggers.
    triggers: Vec<Trigger>,
    /// What encoding its text is in.
    encoding: Encoding,
    /// Where `random` and `randomblob` take their bytes from.
    random: crate::random::Source,
    /// The moment `now` names, as the seconds since 1970, and nothing
    /// where the caller told the connection none.
    clock: Option<i64>,
    /// Whether `LIKE` tells the twenty-six letters apart, which
    /// `PRAGMA case_sensitive_like` on the connection that writes sets.
    sensitive: bool,
    /// The function `sqlite3_set_authorizer` told the connection, which
    /// every statement is read against.
    asking: Option<crate::auth::Asking>,
    /// The columns the function ignored a read of, as the name the
    /// statement knows the table by and the column, which the statement
    /// running now answers a null for.
    ///
    /// The list belongs to the statement and the statement is answered
    /// through a shared reference, so the cell is what carries it from
    /// the reading of the statement to the rows it answers.
    ignored: core::cell::RefCell<Vec<(Vec<u8>, Vec<u8>)>>,
    /// What the walks and the sorts of the statement running now have
    /// counted. The counts belong to the statement and the statement is
    /// answered through a shared reference, so the cell is what carries
    /// them out to the answer.
    stepped: core::cell::Cell<Stepped>,
    /// What the connection has written, which `changes()`,
    /// `total_changes()` and `last_insert_rowid()` answer.
    counted: crate::func::Counted,
    /// How the connection names the columns a statement answers.
    naming: Naming,
    /// The functions the application defined on the connection.
    defined: &'static [crate::func::Defined],
    /// The aggregates the application defined on the connection.
    grouped: &'static [crate::func::Grouped],
    /// The word the journal mode of the connection is written as,
    /// which `PRAGMA journal_mode` answers.
    journalled: &'static [u8],
    /// The collations the application defined on the connection, which
    /// the schema is read against and a `COLLATE` reaches.
    collating: &'static [crate::value::Collating],
}

/// One trigger of the schema: what it is on, and the statement that
/// made it.
#[derive(Clone, Debug)]
pub(crate) struct Trigger {
    /// The name, with its quotes taken off.
    pub(crate) name: Vec<u8>,
    /// Which database of the connection it stands in.
    pub(crate) place: usize,
    /// The table it is on, with its quotes taken off.
    pub(crate) table: Vec<u8>,
    /// The `CREATE TRIGGER` text, which the tree points into.
    pub(crate) sql: Vec<u8>,
    /// The tree that text was parsed into.
    pub(crate) arena: Arena,
    /// What the statement said.
    pub(crate) written: crate::ast::CreateTrigger,
}

/// One view of the schema: a name, the statement it answers, and the
/// names it answers its columns under where the definition wrote them.
#[derive(Clone, Debug)]
struct View {
    /// The name, with its quotes taken off.
    name: Vec<u8>,
    /// Which database of the connection it stands in.
    place: usize,
    /// The `CREATE VIEW` text, which the tree points into.
    sql: Vec<u8>,
    /// The tree that text was parsed into.
    arena: Arena,
    /// The statement it answers.
    select: crate::ast::SelectId,
    /// The names the definition wrote for its columns, where it wrote
    /// them.
    columns: Vec<Vec<u8>>,
}

/// The name a statement wrote for a table, with the schema in front of it
/// where it wrote one.
fn table_named(schema: Option<Span>, name: Span, sql: &[u8]) -> Named {
    let name = dequote(name.text(sql));
    let mut shown = Vec::new();
    if let Some(span) = schema {
        shown.extend_from_slice(&dequote(span.text(sql)));
        shown.push(b'.');
    }
    shown.extend_from_slice(&name);
    Named { name, shown }
}

/// One name a statement wrote for a table: the name itself, and the name
/// with the schema in front of it where the statement wrote one, which is
/// what `no such table:` writes.
struct Named {
    /// The name, with its quotes taken off.
    name: Vec<u8>,
    /// The same with the schema in front of it, where one was written.
    shown: Vec<u8>,
}

impl Named {
    /// One name no statement wrote a schema in front of.
    fn bare(name: &[u8]) -> Self {
        Named {
            name: name.to_vec(),
            shown: name.to_vec(),
        }
    }
}

/// One database an `ATTACH` added to the connection the reader stands
/// for: the name it answers to, and the file its trees stand in.
#[derive(Clone, Debug)]
struct Attached<'a> {
    /// The name the `ATTACH` gave, with its quotes taken off.
    name: Vec<u8>,
    /// The file.
    image: Image<'a>,
}

/// One row of a table as a statement that writes rows reads it: the
/// key that names the row, and the value of each column.
pub type Reading = (Vec<Value>, Vec<Value>);

/// How a statement names the columns it answers, which
/// `sqlite3GenerateColumnNames` reads two pragmas for.
///
/// A connection told nothing holds `short` on and `full` off, which is
/// what `SQLITE_ShortColNames` and `SQLITE_FullColNames` stand at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Naming {
    /// `PRAGMA short_column_names`: a column reference answers under
    /// the column's own name.
    pub short: bool,
    /// `PRAGMA full_column_names`: a column reference answers under
    /// the name of its table and the column's own name.
    pub full: bool,
}

impl Default for Naming {
    fn default() -> Self {
        Self {
            short: true,
            full: false,
        }
    }
}

/// What a statement answered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    /// The name of each column, as SQLite would name it.
    pub names: Vec<Vec<u8>>,
    /// The type the schema declares for each column, and nothing for a
    /// column that came from an expression, which is what
    /// `sqlite3_column_decltype` answers.
    pub declared: Vec<Vec<u8>>,
    /// The rows, each as many values as there are names.
    pub rows: Vec<Vec<Value>>,
    /// What the walks and the sorts of the statement counted.
    pub stepped: Stepped,
}

/// What one statement's walks and sorts counted, which
/// `sqlite3_stmt_status` answers for `SQLITE_STMTSTATUS_FULLSCAN_STEP`
/// and `SQLITE_STMTSTATUS_SORT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stepped {
    /// The steps the walks of a whole table and of a whole index took,
    /// which is one per row after the first of each walk, as `OP_Next`
    /// counts them.
    pub steps: u64,
    /// How many times the rows were sorted, which `OP_Sort` counts.
    pub sorts: u64,
}

impl<'a> Database<'a> {
    /// Opens `bytes` and reads its schema.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::read(Image::open(bytes)?, &[])
    }

    /// The same, with the collations the application defined on the
    /// connection that writes.
    ///
    /// The schema is read against them, because a column is declared
    /// with the collation its text compares under, so a connection that
    /// is not told of one reads no table that names it.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open_collating(
        bytes: &'a [u8],
        collating: &'static [crate::value::Collating],
    ) -> Result<Self, Error> {
        Self::read(Image::open(bytes)?, collating)
    }

    /// The same for a database whose newest pages are in its
    /// write-ahead log, which a reader must follow.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open_with_log(bytes: &'a [u8], log: &'a crate::wal::Wal<'a>) -> Result<Self, Error> {
        Self::read(Image::open_with_log(bytes, log)?, &[])
    }

    /// The same, with the collations the application defined on the
    /// connection that writes.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open_log_collating(
        bytes: &'a [u8],
        log: &'a crate::wal::Wal<'a>,
        collating: &'static [crate::value::Collating],
    ) -> Result<Self, Error> {
        Self::read(Image::open_with_log(bytes, log)?, collating)
    }

    /// The same for a database whose rollback journal is hot, which a
    /// reader must play back before it reads a row.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open_with_journal(
        bytes: &'a [u8],
        journal: &'a crate::journal::Journal<'a>,
    ) -> Result<Self, Error> {
        Self::read(Image::open_with_journal(bytes, journal)?, &[])
    }

    /// Reads the schema of an open file.
    fn read(
        image: Image<'a>,
        collating: &'static [crate::value::Collating],
    ) -> Result<Self, Error> {
        let encoding = image.header().encoding;
        let tables = read_tables(&image, 0, encoding, collating)?;
        let mut database = Database {
            image,
            named: b"main".to_vec(),
            attached: Vec::new(),
            temp: None,
            writing: false,
            tables,
            views: Vec::new(),
            triggers: Vec::new(),
            encoding,
            random: crate::random::Source::default(),
            clock: None,
            sensitive: false,
            asking: None,
            ignored: core::cell::RefCell::new(Vec::new()),
            stepped: core::cell::Cell::new(Stepped::default()),
            counted: crate::func::Counted::default(),
            naming: Naming::default(),
            defined: &[],
            grouped: &[],
            journalled: b"delete",
            collating,
        };
        database.read_indexes(&image, 0)?;
        database.read_views(&image, 0)?;
        database.read_triggers(&image, 0)?;
        Ok(database)
    }

    /// The same reader with the database at schema place nought named
    /// `name` rather than `main`.
    ///
    /// A connection whose statement writes an attached database reads that
    /// one at place nought and `main` beside it, so the name a statement
    /// writes in front of a table is the name the reader answers under.
    #[must_use]
    pub fn named_main(mut self, name: &[u8]) -> Self {
        self.named = name.to_vec();
        self
    }

    /// The same reader with the database `name` beside it, which an
    /// `ATTACH` added to the connection.
    ///
    /// The tables, views and triggers of that file are read into the same
    /// lists as the ones of `main` and carry the schema place of the
    /// file, so a bare name is answered out of `main` first and out of
    /// the attached databases in the order they were attached, which is
    /// `sqlite3FindTable` of `research/sqlite/src/build.c:373`.
    ///
    /// Reading the schema costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn attaching(mut self, name: &[u8], bytes: &'a [u8]) -> Result<Self, Error> {
        let image = Image::open(bytes)?;
        let place = self.attached.len().saturating_add(1);
        if is_temp(name) {
            self.temp = Some(place);
        }
        let tables = read_tables(&image, place, self.encoding, self.collating)?;
        self.tables.extend(tables);
        self.attached.push(Attached {
            name: name.to_vec(),
            image,
        });
        self.read_indexes(&image, place)?;
        self.read_views(&image, place)?;
        self.read_triggers(&image, place)?;
        Ok(self)
    }

    /// The file the tree of `stored` stands in, which is the one the
    /// reader was opened over for schema place nought and an attached one
    /// for every other place.
    fn imaged(&self, place: usize) -> Image<'a> {
        match place.checked_sub(1).and_then(|at| self.attached.get(at)) {
            Some(held) => held.image,
            None => self.image,
        }
    }

    /// The name of the database at schema place nought, which is the one a
    /// statement of the connection writes.
    #[must_use]
    pub fn main_named(&self) -> &[u8] {
        &self.named
    }

    /// The name of the database at `place`, which is `main` for the one
    /// the reader was opened over.
    fn named_place(&self, place: usize) -> Vec<u8> {
        match place.checked_sub(1).and_then(|at| self.attached.get(at)) {
            Some(held) => held.name.clone(),
            None => self.named.clone(),
        }
    }

    /// The name of the database that holds the table, the view, the index
    /// or the trigger named `name`, and nothing where no database of the
    /// connection holds one.
    ///
    /// `sqlite3LocateTable` of `research/sqlite/src/build.c:408` reads
    /// the databases in turn, so a bare name names the first one that
    /// holds it. Reading them costs O(n) in the names they hold.
    #[must_use]
    pub fn holding(&self, name: &[u8]) -> Option<Vec<u8>> {
        self.holding_at(name).map(|place| self.named_place(place))
    }

    /// The schema place of the database that holds the table, the view,
    /// the index or the trigger named `name`.
    fn holding_at(&self, name: &[u8]) -> Option<usize> {
        if let Some(stored) = self.find(name) {
            return Some(stored.place);
        }
        self.searching().into_iter().find(|place| {
            self.views
                .iter()
                .any(|view| view.place == *place && view.name.eq_ignore_ascii_case(name))
                || self
                    .triggers
                    .iter()
                    .any(|held| held.place == *place && held.name.eq_ignore_ascii_case(name))
                || self.tables.iter().any(|stored| {
                    stored.place == *place
                        && stored
                            .indexes
                            .iter()
                            .any(|kept| kept.index.name.eq_ignore_ascii_case(name))
                })
        })
    }

    /// The table of `name` in the database at `place`, and in every
    /// database of the connection where the statement named none.
    fn located(&self, place: Option<usize>, name: &[u8]) -> Option<&Stored> {
        match place {
            Some(place) => self.find_in(place, name),
            None => self.find(name),
        }
    }

    /// The table of `name` in the database at `place`.
    fn find_in(&self, place: usize, name: &[u8]) -> Option<&Stored> {
        let name = if schema_named(name) {
            SCHEMA_TABLE
        } else {
            name
        };
        self.tables
            .iter()
            .find(|stored| stored.place == place && stored.table.name.eq_ignore_ascii_case(name))
    }

    /// The schema place of the database named `schema`, and nothing where
    /// the connection holds none under that name.
    fn placed(&self, schema: &[u8]) -> Option<usize> {
        if self.named.eq_ignore_ascii_case(schema) {
            return Some(0);
        }
        let at = self
            .attached
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(schema))?;
        Some(at.saturating_add(1))
    }

    /// The same database, with `random` and `randomblob` answering the
    /// bytes `seed` draws.
    ///
    /// SQLite seeds `sqlite3_randomness` from the operating system,
    /// which this crate has none of, so the caller says where the bytes
    /// come from and a database that is not told answers the bytes
    /// nought draws.
    #[must_use]
    pub const fn seeded(mut self, seed: u64) -> Self {
        self.random = crate::random::Source::new(seed);
        self
    }

    /// The same database, with `now` naming the moment `seconds` since
    /// 1970 says.
    ///
    /// SQLite reads the clock of the operating system, which this crate
    /// has none of, so the caller says what the clock says and a
    /// database that is not told refuses a statement naming `now`,
    /// `CURRENT_TIME`, `CURRENT_DATE` or `CURRENT_TIMESTAMP`.
    #[must_use]
    pub const fn clocked(mut self, seconds: i64) -> Self {
        self.clock = Some(seconds);
        self
    }

    /// The same database, read against the function
    /// `sqlite3_set_authorizer` told the connection.
    ///
    /// Every statement is read before it answers a row: the function is
    /// asked once per action, the statement is refused where the
    /// function denies, and a column read the function ignores answers a
    /// null.
    #[must_use]
    pub const fn asked(mut self, asking: crate::auth::Asking) -> Self {
        self.asking = Some(asking);
        self
    }

    /// The same database, with `LIKE` telling the twenty-six letters
    /// apart where `sensitive` says so.
    ///
    /// `PRAGMA case_sensitive_like` registers the `like` function again
    /// on one connection, so a database that is not told answers what
    /// `sqlite3RegisterLikeFunctions` registers at open: `LIKE` that
    /// folds the letters.
    #[must_use]
    pub const fn sensitively(mut self, sensitive: bool) -> Self {
        self.sensitive = sensitive;
        self
    }

    /// The same database, read as the connection that writes reads it
    /// where `writing` says so: a name is then read in the database at
    /// schema place nought before the temp schema.
    #[must_use]
    pub const fn writing(mut self, writing: bool) -> Self {
        self.writing = writing;
        self
    }

    /// The same database, with `changes()`, `total_changes()` and
    /// `last_insert_rowid()` answering what the connection that writes
    /// has written.
    ///
    /// The three are properties of a connection and not of a file, so a
    /// database that is not told answers noughts.
    #[must_use]
    pub const fn counting(mut self, counted: crate::func::Counted) -> Self {
        self.counted = counted;
        self
    }

    /// The same database, with the aggregates the application defined on
    /// the connection that writes.
    ///
    /// The aggregates belong to a connection and not to a file, so a
    /// database that is not told holds none of them.
    #[must_use]
    pub const fn grouping(mut self, grouped: &'static [crate::func::Grouped]) -> Self {
        self.grouped = grouped;
        self
    }

    /// The same database, with the functions the application defined on
    /// the connection that writes.
    ///
    /// The functions belong to a connection and not to a file, so a
    /// database that is not told holds none of them.
    #[must_use]
    pub const fn defining(mut self, defined: &'static [crate::func::Defined]) -> Self {
        self.defined = defined;
        self
    }

    /// The same database, answering `PRAGMA journal_mode` with the mode
    /// of the connection that writes.
    ///
    /// The mode belongs to a connection and not to a file, unless the
    /// file is in write-ahead logging, so a database that is not told
    /// answers the mode a connection told nothing is in.
    #[must_use]
    pub const fn journalling(mut self, journalled: &'static [u8]) -> Self {
        self.journalled = journalled;
        self
    }

    /// The same database, naming the columns a statement answers as the
    /// connection that writes was told to name them.
    ///
    /// The two pragmas belong to a connection and not to a file, so a
    /// database that is not told names them the way a connection told
    /// nothing does.
    #[must_use]
    pub const fn naming(mut self, naming: Naming) -> Self {
        self.naming = naming;
        self
    }

    /// Reads the views of the schema.
    ///
    /// A view names a statement rather than a tree, so it carries no
    /// root page; one whose statement this crate cannot read is passed
    /// over, and a statement that names it then answers no table.
    fn read_views(&mut self, image: &Image<'a>, place: usize) -> Result<(), Error> {
        let mut views = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(image, &row.payload, &mut payload)?;
            let record = record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(record::Value::Text(bytes)) => crate::value::decoded(bytes, self.encoding),
                    _ => Vec::new(),
                })
            };
            if text(0)? != b"view" {
                continue;
            }
            let sql = text(4)?;
            let Ok((arena, definition)) = parse::definition(&sql) else {
                continue;
            };
            let crate::ast::Definition::View(written) = definition else {
                continue;
            };
            let columns = arena
                .names(written.columns)
                .iter()
                .map(|span| schema::dequote(span.text(&sql)))
                .collect();
            views.push(View {
                name: schema::dequote(written.name.text(&sql)),
                place,
                select: written.select,
                sql,
                arena,
                columns,
            });
        }
        self.views.extend(views);
        Ok(())
    }

    /// Reads every trigger of the schema, keeping the statement that
    /// made each one.
    ///
    /// A trigger this crate cannot read is passed over, so a database
    /// that holds one is read for everything else it holds.
    fn read_triggers(&mut self, image: &Image<'a>, place: usize) -> Result<(), Error> {
        let mut triggers = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(image, &row.payload, &mut payload)?;
            let record = record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(record::Value::Text(bytes)) => crate::value::decoded(bytes, self.encoding),
                    _ => Vec::new(),
                })
            };
            if text(0)? != b"trigger" {
                continue;
            }
            let sql = text(4)?;
            let Ok((arena, definition)) = parse::definition(&sql) else {
                continue;
            };
            let crate::ast::Definition::Trigger(written) = definition else {
                continue;
            };
            triggers.push(Trigger {
                name: schema::dequote(written.name.text(&sql)),
                place,
                table: schema::dequote(written.table.text(&sql)),
                sql,
                arena,
                written,
            });
        }
        self.triggers.extend(triggers);
        Ok(())
    }

    /// The triggers of `table` that run on `event` at `time`, in the
    /// order they run: `sqlite3FinishTrigger` puts each in front of the
    /// ones before it, so the one made last runs first.
    pub(crate) fn triggers_on(
        &self,
        table: &[u8],
        event: crate::ast::TriggerEvent,
        time: crate::ast::TriggerTime,
    ) -> Vec<&Trigger> {
        self.triggers
            .iter()
            .rev()
            .filter(|trigger| {
                trigger.table.eq_ignore_ascii_case(table)
                    && trigger.written.event == event
                    && trigger.written.time == time
            })
            .collect()
    }

    /// The trigger of `name` this database holds.
    pub(crate) fn trigger(&self, name: &[u8]) -> Option<&Trigger> {
        self.triggers
            .iter()
            .find(|trigger| trigger.name.eq_ignore_ascii_case(name))
    }

    /// The trigger of `name` the database at schema place nought holds.
    ///
    /// `sqlite3BeginTrigger` of `research/sqlite/src/trigger.c:150` reads
    /// the name in the schema the statement named, so a trigger of the
    /// temp schema and one of `main` may carry the same name.
    pub(crate) fn held_trigger(&self, name: &[u8]) -> Option<&Trigger> {
        self.triggers
            .iter()
            .find(|trigger| trigger.place == 0 && trigger.name.eq_ignore_ascii_case(name))
    }

    /// The rows a view answers, with the shape they carry and the name
    /// the view is known by.
    ///
    /// The statement of a view is answered where the view is named, so
    /// a view over a table reads what the table holds now. A view that
    /// names itself, directly or through another, is stopped by the
    /// count of views the statement is already inside.
    fn viewed(
        &self,
        place: Option<usize>,
        named: &Named,
        scope: Scope<'_>,
    ) -> Result<Viewed, Error> {
        let view = self
            .views
            .iter()
            .find(|view| {
                place.is_none_or(|place| view.place == place)
                    && view.name.eq_ignore_ascii_case(&named.name)
            })
            .ok_or_else(|| Error::NoTable(named.shown.clone()))?;
        if scope.views >= VIEW_DEPTH {
            return Err(Error::Unsupported);
        }
        let inner = Scope {
            terms: &[],
            outer: None,
            views: scope.views.saturating_add(1),
        };
        let mut answered = self.statement(&view.arena, view.select, &view.sql, inner)?;
        // `sqlite3ViewGetColumnNames` refuses a column list of another
        // width than the statement answers.
        let written = view.columns.len();
        if written != 0 && written != answered.shape.columns.len() {
            return Err(Error::ViewWidth(
                view.name.clone(),
                written,
                answered.shape.columns.len(),
            ));
        }
        // `CREATE VIEW v(a,b) AS ...` answers its columns under the
        // names the definition wrote, and under the statement's own
        // where it wrote none.
        for (column, name) in answered.shape.columns.iter_mut().zip(&view.columns) {
            column.name.clone_from(name);
        }
        Ok(Viewed {
            shape: answered.shape,
            rows: answered.answer.rows,
            name: view.name.clone(),
            place: view.place,
        })
    }

    /// A view as a table of the columns it answers, with the rows it
    /// holds now, which is what an `INSTEAD OF` trigger reads `old` and
    /// `new` out of.
    ///
    /// The statement of the view is answered here, so the cost is what
    /// that statement costs.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the schema holds no view of that name,
    /// and whatever the statement of the view refuses.
    pub fn viewing(&self, name: &[u8]) -> Result<(crate::schema::Table, Vec<Vec<Value>>), Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        let viewed = self.viewed(None, &Named::bare(name), scope)?;
        let columns: Vec<Vec<u8>> = viewed
            .shape
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        Ok((
            crate::schema::Table::viewed(viewed.name, &columns),
            viewed.rows,
        ))
    }

    /// The columns and the rows the `FROM` of an `UPDATE` answers.
    ///
    /// The statement is answered once, so the cost is what that
    /// statement costs.
    ///
    /// # Errors
    ///
    /// Whatever the statement of the clause refuses.
    pub fn joined(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
    ) -> Result<(Vec<Beside>, Vec<Vec<Value>>), Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        let answered = self.statement(arena, id, sql, scope)?;
        let columns = answered
            .shape
            .columns
            .iter()
            .map(|column| Beside {
                name: column.name.clone(),
                from: column.from.clone(),
                affinity: column.affinity,
                collation: column.collation.unwrap_or(self.collation()),
            })
            .collect();
        Ok((columns, answered.answer.rows))
    }

    /// The columns of a row that are computed, filled in from the
    /// columns the statement wrote, which is
    /// `sqlite3ComputeGeneratedColumns`.
    ///
    /// One expression may name another computed column, so the cost is
    /// O(k²) evaluations for `k` of them.
    ///
    /// # Errors
    ///
    /// Whatever the expression of a computed column refuses.
    pub fn compute_row(&self, name: &[u8], values: &mut [Value]) -> Result<(), Error> {
        let Some(stored) = self.find(name) else {
            return Ok(());
        };
        let computed: Vec<usize> = stored
            .table
            .columns
            .iter()
            .enumerate()
            .filter(|(_, column)| column.generated != Generated::Never)
            .map(|(at, _)| at)
            .collect();
        if computed.is_empty() {
            return Ok(());
        }
        let mut held: Vec<Option<Value>> = values.iter().cloned().map(Some).collect();
        for at in computed {
            for slot in held.iter_mut().skip(at).take(1) {
                *slot = None;
            }
        }
        // A column whose expression this row cannot answer keeps the
        // value it was given, because a statement writes down only the
        // columns computed once and the read of a column computed where
        // it is read refuses on its own.
        let held = match compute(stored, &mut held, self.encoding, Collation::Binary) {
            Ok(()) => held,
            Err(_) => return Ok(()),
        };
        for (slot, value) in values.iter_mut().zip(held) {
            *slot = value.unwrap_or(Value::Null);
        }
        Ok(())
    }

    /// The statement the table of `name` was written with, and where a
    /// column added later goes in it.
    #[must_use]
    pub fn written_as(&self, name: &[u8]) -> Option<(&[u8], Option<usize>)> {
        self.find(name)
            .map(|stored| (stored.sql.as_slice(), stored.table.add_at))
    }

    /// The view of `name`, where the schema holds one.
    #[must_use]
    pub fn view(&self, name: &[u8]) -> Option<&[u8]> {
        self.views
            .iter()
            .find(|view| view.name.eq_ignore_ascii_case(name))
            .map(|view| view.sql.as_slice())
    }

    /// Reads the indexes of the schema, once every table is read.
    ///
    /// An index names its table, and a schema may name the index before
    /// the table where it was written that way, so the tables are read
    /// first and the indexes against them. An index this crate cannot
    /// walk is passed over rather than refused: the statement over it
    /// scans, which is slower and just as right.
    fn read_indexes(&mut self, image: &Image<'a>, place: usize) -> Result<(), Error> {
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(image, &row.payload, &mut payload)?;
            let record = record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(record::Value::Text(bytes)) => crate::value::decoded(bytes, self.encoding),
                    _ => Vec::new(),
                })
            };
            if text(0)? != b"index" {
                continue;
            }
            let Some(record::Value::Int(root)) = record.value(3)? else {
                continue;
            };
            let sql = text(4)?;
            if sql.is_empty() {
                // An index a `UNIQUE` or a `PRIMARY KEY` made carries no
                // statement, and its columns are the constraint's: the
                // name says which of them it is.
                let name = text(1)?;
                let over = text(2)?;
                let Some(stored) = self.tables.iter_mut().find(|stored| {
                    stored.place == place && stored.table.name.eq_ignore_ascii_case(&over)
                }) else {
                    continue;
                };
                let index = (0..stored.table.keys.len())
                    .filter_map(|at| schema::own_index(&stored.table, at))
                    .find(|index| index.name.eq_ignore_ascii_case(&name));
                if let Some(index) = index {
                    stored.indexes.push(Kept {
                        sql: Vec::new(),
                        arena: Arena::default(),
                        index,
                        root: u32::try_from(root).unwrap_or(0),
                    });
                }
                continue;
            }
            let (arena, definition) = parse::definition(&sql)?;
            let crate::ast::Definition::Index(written) = definition else {
                continue;
            };
            let over = dequote(written.table.text(&sql));
            let Some(stored) = self.tables.iter_mut().find(|stored| {
                stored.place == place && stored.table.name.eq_ignore_ascii_case(&over)
            }) else {
                continue;
            };
            let Ok(index) = schema::index(&arena, &written, &sql, &stored.table, self.collating)
            else {
                continue;
            };
            stored.indexes.push(Kept {
                index,
                root: u32::try_from(root).unwrap_or(0),
                sql,
                arena,
            });
        }
        Ok(())
    }

    /// The tables of the schema, in the order the file holds them.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        // The schema's own table is read out of the schema rather than
        // written in it, so it is not one of the tables the file holds;
        // a statement that names it reaches it through the lookup.
        self.tables
            .iter()
            .map(|stored| &stored.table)
            .filter(|table| !schema_named(&table.name))
    }

    /// The file the database is read out of.
    #[must_use]
    pub const fn image(&self) -> &Image<'a> {
        &self.image
    }

    /// What encoding the file keeps its text in.
    #[must_use]
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// The schema format the file is written in, which is 4 where a
    /// `CREATE INDEX` writing `DESC` holds its entries backwards and
    /// below it where `sqlite3CreateIndex` ignores the word.
    #[must_use]
    pub const fn schema_format(&self) -> u32 {
        self.image.header().schema_format
    }

    /// The table of `name` and the page its tree begins at, where the
    /// database holds one.
    #[must_use]
    pub fn table(&self, name: &[u8]) -> Option<(&Table, u32)> {
        self.find(name).map(|stored| (&stored.table, stored.root))
    }

    /// The index of `name`, with the page its tree begins on.
    #[must_use]
    pub fn index(&self, name: &[u8]) -> Option<(&schema::Index, u32)> {
        self.tables.iter().find_map(|stored| {
            stored
                .indexes
                .iter()
                .find(|kept| kept.index.name.eq_ignore_ascii_case(name))
                .map(|kept| (&kept.index, kept.root))
        })
    }

    /// Whether the index of `name` is one a `UNIQUE` or a `PRIMARY KEY`
    /// made, which carries no statement of its own.
    #[must_use]
    pub fn constrained(&self, name: &[u8]) -> bool {
        self.tables.iter().any(|stored| {
            stored
                .indexes
                .iter()
                .any(|kept| kept.index.name.eq_ignore_ascii_case(name) && kept.sql.is_empty())
        })
    }

    /// The index of `name` as a caller that reads its expressions needs
    /// it.
    #[must_use]
    pub fn indexed(&self, name: &[u8]) -> Option<Indexed<'_>> {
        self.tables.iter().find_map(|stored| {
            stored
                .indexes
                .iter()
                .find(|kept| kept.index.name.eq_ignore_ascii_case(name))
                .map(|kept| Indexed {
                    index: &kept.index,
                    root: kept.root,
                    sql: &kept.sql,
                    arena: &kept.arena,
                    table: &stored.table,
                })
        })
    }

    /// The indexes over the table of `name` this crate holds, each with
    /// the page its tree begins at and the columns it is over.
    ///
    /// An index this crate cannot walk is not one it holds, so a table
    /// may have more indexes on disk than this answers; `has_others`
    /// says so.
    #[must_use]
    pub fn indexes(&self, name: &[u8]) -> Vec<Indexed<'_>> {
        self.find(name)
            .map(|stored| {
                stored
                    .indexes
                    .iter()
                    .map(|kept| Indexed {
                        index: &kept.index,
                        root: kept.root,
                        sql: &kept.sql,
                        arena: &kept.arena,
                        table: &stored.table,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What each column of the table of `name` falls back to, which is
    /// what a row that names no value for a column holds and what a row
    /// written before the column was added answers.
    ///
    /// One column with no `DEFAULT` falls back to nothing.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the expression could not answer.
    pub fn defaults(&self, name: &[u8]) -> Result<Vec<Value>, Error> {
        let Some(stored) = self
            .tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
        else {
            return Ok(Vec::new());
        };
        let clock = self.clock.map(crate::date::julian_of);
        let mut out = Vec::with_capacity(stored.table.columns.len());
        for column in &stored.table.columns {
            out.push(match column.falls_back {
                Some(id) => eval::evaluate(&stored.arena, id, &stored.sql, clock)?,
                None => Value::Null,
            });
        }
        Ok(out)
    }

    /// The name of the first `CHECK` of the table that does not hold
    /// for a row, or nothing where every one of them holds.
    ///
    /// `sqlite3ExprIfFalse`: a `CHECK` holds where it answers anything
    /// but false, so a row that answers nothing holds. Reading one row
    /// costs what the expressions of the table cost.
    ///
    /// # Errors
    ///
    /// [`Error`] names what an expression could not answer.
    pub fn refused_check(
        &self,
        name: &[u8],
        row: &dyn eval::Row,
    ) -> Result<Option<Vec<u8>>, Error> {
        let held = self
            .tables
            .iter()
            .filter(|stored| stored.table.name.eq_ignore_ascii_case(name));
        for stored in held {
            for check in &stored.table.checks {
                let value =
                    crate::eval::evaluate_row(&stored.arena, check.value, &stored.sql, row)?;
                if value != Value::Null && !value.truth(false) {
                    return Ok(Some(check.shown.clone()));
                }
            }
        }
        Ok(None)
    }

    /// Every row of the table of `name`, each with its key and the
    /// values of its columns, which is what a statement that takes rows
    /// out walks to find the rows it takes.
    ///
    /// One walk is O(n) in the rows of the table.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the database holds no such table,
    /// [`Error::Unsupported`] where the table keeps its rows in the
    /// key's own tree, and whatever reading a row of it refuses.
    pub fn rows_of(&self, name: &[u8]) -> Result<Vec<(i64, Vec<Value>)>, Error> {
        // The name is one the schema holds wherever this is reached
        // from, so the refusal carries no name to write into a message.
        let stored = self.find(name).ok_or(Error::NoTable(Vec::new()))?;
        if stored.table.without_rowid {
            return Err(Error::Unsupported);
        }
        let mut out = Vec::new();
        let mut payload = Vec::new();
        let image = self.imaged(stored.place);
        for step in self.walk(stored, (None, None)) {
            let (rowid, held) = step?;
            let rowid = rowid.ok_or(Error::Unsupported)?;
            read_payload(&image, &held, &mut payload)?;
            let values = values_of(
                &payload,
                stored,
                Some(rowid),
                self.encoding,
                self.collation(),
            )?;
            out.push((rowid, values));
        }
        Ok(out)
    }

    /// Every row of a table with the key that names it: the rowid
    /// where the table has one, and the columns of the `PRIMARY KEY`
    /// where the table keeps its rows in the key's own tree.
    ///
    /// One walk is O(n) in the rows of the table.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the database holds no such table, and
    /// whatever reading a row of it refuses.
    pub fn held_rows_of(&self, name: &[u8]) -> Result<Vec<Reading>, Error> {
        let stored = self.find(name).ok_or(Error::NoTable(Vec::new()))?;
        if stored.table.without_rowid {
            return Ok(self
                .keyed_rows_of(name)?
                .into_iter()
                .map(|values| (schema::key_of(&stored.table, &values), values))
                .collect());
        }
        Ok(self
            .rows_of(name)?
            .into_iter()
            .map(|(rowid, values)| (alloc::vec![Value::Int(rowid)], values))
            .collect())
    }

    /// The rows of a table that keeps its rows in the key's own tree,
    /// each with the columns in the order the table declares them.
    ///
    /// One walk is O(n) in the rows of the table.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the database holds no such table, and
    /// whatever reading a row of it refuses.
    pub fn keyed_rows_of(&self, name: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        // The name is one the schema holds wherever this is reached
        // from, so the refusal carries no name to write into a message.
        let stored = self.find(name).ok_or(Error::NoTable(Vec::new()))?;
        let mut out = Vec::new();
        let mut payload = Vec::new();
        let collation = self.collation();
        let image = self.imaged(stored.place);
        for step in self.walk(stored, (None, None)) {
            let (_, held) = step?;
            read_payload(&image, &held, &mut payload)?;
            out.push(values_of(&payload, stored, None, self.encoding, collation)?);
        }
        Ok(out)
    }

    /// The rows of a statement already read, which is what a statement
    /// that puts rows in a table answers its rows from.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    /// The rows a statement answers against a row that stands outside
    /// it, which is the row a trigger's body reads as `new` and `old`.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    pub fn rows_under(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
        outer: Option<&dyn eval::Row>,
    ) -> Result<Answer, Error> {
        let scope = Scope {
            terms: &[],
            outer,
            views: 0,
        };
        Ok(self.statement(arena, id, sql, scope)?.answer)
    }

    /// The rows a statement answers, with the affinity each of its
    /// columns compares under, which is what `CREATE TABLE ... AS`
    /// writes the column types from.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    pub(crate) fn answered(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
    ) -> Result<(Answer, Vec<Affinity>), Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        let answered = self.statement(arena, id, sql, scope)?;
        let affinities = answered
            .shape
            .columns
            .iter()
            .map(|column| column.affinity)
            .collect();
        Ok((answered.answer, affinities))
    }

    /// What a comparison uses where nothing writes a collation, which
    /// is `BINARY` over the encoding the file keeps its text in.
    const fn collation(&self) -> Collation {
        binary_of(self.encoding)
    }

    /// The table `name` names, with the three names of the schema's own
    /// table read as the one it is held under.
    fn find(&self, name: &[u8]) -> Option<&Stored> {
        // `sqlite3FindTable` of `research/sqlite/src/build.c:386` reads
        // `sqlite_schema` out of `main` and `sqlite_temp_schema` out of
        // the temp schema, whatever else a database holds under either
        // name.
        if temp_named(name) {
            return self
                .temp
                .and_then(|place| self.find_in(place, SCHEMA_TABLE));
        }
        if schema_named(name) {
            return self.find_in(0, SCHEMA_TABLE);
        }
        self.searching()
            .into_iter()
            .find_map(|place| self.find_in(place, name))
    }

    /// The schema places in the order `sqlite3FindTable` of
    /// `research/sqlite/src/build.c:373` reads them: the temp schema,
    /// then `main`, then the attached databases in the order they were
    /// attached.
    ///
    /// The connection that writes reads the database the statement named
    /// first, because `sqlite3TwoPartName` of
    /// `research/sqlite/src/build.c:596` names one database and the rows
    /// the statement writes are the ones of the table that database
    /// holds.
    ///
    /// Reading them costs O(n) in their number.
    fn searching(&self) -> Vec<usize> {
        let (first, second) = if self.writing {
            (Some(0), self.temp)
        } else {
            (self.temp, Some(0))
        };
        let mut out: Vec<usize> = first.into_iter().chain(second).collect();
        for place in 1..=self.attached.len() {
            if Some(place) != self.temp {
                out.push(place);
            }
        }
        out
    }

    /// What `sql` answers.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    pub fn query(&self, sql: &[u8]) -> Result<Answer, Error> {
        self.stepped.set(Stepped::default());
        let mut answer = self.queried(sql).map_err(|error| error.near(sql))?;
        answer.stepped = self.stepped.get();
        Ok(answer)
    }

    /// Counts `steps` more steps of a walk.
    fn step(&self, steps: u64) {
        let mut held = self.stepped.get();
        held.steps = held.steps.saturating_add(steps);
        self.stepped.set(held);
    }

    /// Counts one more sort.
    fn sort(&self) {
        let mut held = self.stepped.get();
        held.sorts = held.sorts.saturating_add(1);
        self.stepped.set(held);
    }

    /// One statement answered, with a parse answered as the parser
    /// wrote it.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the statement could not answer.
    fn queried(&self, sql: &[u8]) -> Result<Answer, Error> {
        // A text of comments alone holds no statement and answers no
        // row, which is what `sqlite3_exec` runs for one.
        if parse::blank(sql) {
            return Ok(Answer::default());
        }
        if let Ok(asked) = parse::pragma(sql) {
            return self.pragma(&asked, sql);
        }
        let (arena, root) = parse::statement(sql)?;
        eval::rows_placed(&arena)?;
        crate::schema::collations(&arena, sql, self.collating)?;
        crate::schema::likelihoods(&arena, sql)?;
        // A statement the function ignored answers no row, which is
        // `sqlite3Select` writing no code for it.
        if self.authorize(&arena, root, sql)? == crate::auth::Answer::Ignore {
            return Ok(Answer::default());
        }
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        Ok(self.statement(&arena, root, sql, scope)?.answer)
    }

    /// Whether the function the connection was told ignored a read of
    /// the column `column` of the side the statement knows as `side`.
    fn is_ignored(&self, side: &[u8], column: &[u8]) -> bool {
        self.ignored.borrow().iter().any(|(table, name)| {
            table.eq_ignore_ascii_case(side) && name.eq_ignore_ascii_case(column)
        })
    }

    /// The columns of one side the function the connection was told
    /// ignored a read of, written as nulls, which is `pExpr->op =
    /// TK_NULL` of `sqlite3AuthRead`.
    ///
    /// A connection told no function leaves the row as it is, so a row
    /// costs what it did.
    fn nulled(&self, side: &Side<'_>, values: &mut [Value]) {
        let ignored = self.ignored.borrow();
        if ignored.is_empty() {
            return;
        }
        for (at, column) in side.shape.columns.iter().enumerate() {
            let held = ignored.iter().any(|(table, name)| {
                table.eq_ignore_ascii_case(&side.name) && name.eq_ignore_ascii_case(&column.name)
            });
            if held {
                for slot in values.iter_mut().skip(at).take(1) {
                    *slot = Value::Null;
                }
            }
        }
    }

    /// The statement read against the function the connection was told,
    /// which leaves the columns a read it ignored in [`Database::ignored`].
    ///
    /// A connection told no function reads nothing, so a statement costs
    /// what it did.
    ///
    /// # Errors
    ///
    /// [`Error::Auth`] names what the function refused.
    fn authorize(
        &self,
        arena: &Arena,
        root: SelectId,
        sql: &[u8],
    ) -> Result<crate::auth::Answer, Error> {
        self.ignored.borrow_mut().clear();
        let Some(asking) = self.asking else {
            return Ok(crate::auth::Answer::Ok);
        };
        let mut authorizer = crate::auth::Authorizer::new(asking, self);
        authorizer.functions(arena, sql)?;
        let answered = authorizer.select(arena, root, sql)?;
        *self.ignored.borrow_mut() = authorizer.ignored();
        Ok(answered)
    }

    /// What a `PRAGMA` answers out of the header, which is one row of
    /// one column named after the pragma, and no row at all for a
    /// pragma the file does not hold.
    ///
    /// A `PRAGMA` that sets something answers nothing here, because a
    /// file being read is not being configured.
    fn pragma(&self, asked: &crate::ast::Pragma, sql: &[u8]) -> Result<Answer, Error> {
        let name = crate::schema::dequote(asked.name.text(sql));
        let setting = crate::pragma::of_name(&name).ok_or(Error::Unsupported)?;
        // `PRAGMA integrity_check` and `PRAGMA quick_check` answer a
        // row per problem the file holds, and a number after them says
        // how many problems to answer at most, which this crate bounds
        // on its own.
        let quick = match setting {
            crate::pragma::Setting::Integrity => Some(false),
            crate::pragma::Setting::Quick => Some(true),
            _ => None,
        };
        let columns = setting.columns(&name, asked.value.is_some());
        let shaped = |rows: Vec<Vec<Value>>| Answer {
            declared: alloc::vec![Vec::new(); columns.len()],
            names: columns.clone(),
            rows,
            stepped: Stepped::default(),
        };
        if let Some(quick) = quick {
            let asked = crate::check::checking(asked.value.map(|value| value.text(sql)));
            if let crate::check::Checking::Table(wanted) = &asked
                && !crate::check::holds_object(self, wanted)
            {
                return Err(Error::NoTable(wanted.clone()));
            }
            let mut left = crate::check::allowed(&asked);
            let mut found = crate::check::integrity(self, quick, (&asked, &self.named), &mut left)?;
            if found.is_empty() {
                found.push(b"ok".to_vec());
            }
            let rows = found
                .into_iter()
                .map(|text| alloc::vec![Value::Text(text)])
                .collect();
            return Ok(shaped(rows));
        }
        // The pragmas that answer the schema read the tables, the indexes
        // and the collations of this database, which D-330 records.
        if let Some(rows) = self.schema_rows(setting, asked, sql) {
            return Ok(shaped(rows));
        }
        // A file being read is not being configured, and a pragma the
        // file does not hold has no answer to read out of it.
        if asked.value.is_some() {
            return Err(Error::Unsupported);
        }
        // A pragma the connection keeps a value for is answered out of
        // what a connection told nothing answers, because a database
        // read here was told nothing.
        let value = match setting {
            crate::pragma::Setting::Held(at) => crate::pragma::HELD
                .get(at)
                .map(|keeps| crate::pragma::kept(at, keeps.fallback)),
            // The mode belongs to the connection that writes, which a
            // database read here was told, unless the file itself is in
            // write-ahead logging, which its header says.
            crate::pragma::Setting::JournalMode => {
                let logged = self.image.header().write_version == 2;
                let word = if logged { b"wal" } else { self.journalled };
                Some(Value::Text(word.to_vec()))
            }
            other => other.read(self.image.header()),
        }
        .ok_or(Error::Unsupported)?;
        Ok(shaped(alloc::vec![alloc::vec![value]]))
    }

    /// The rows one pragma of the schema answers, and nothing where the
    /// pragma is another.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn schema_rows(
        &self,
        setting: crate::pragma::Setting,
        asked: &crate::ast::Pragma,
        sql: &[u8],
    ) -> Option<Vec<Vec<Value>>> {
        use crate::pragma::Setting;
        if setting == Setting::CollationList {
            return Some(crate::change::listed_collations(self.collating));
        }
        let named = asked
            .value
            .map(|value| crate::schema::dequote(value.text(sql)))
            .unwrap_or_default();
        Some(match setting {
            Setting::TableInfo => crate::change::columns_of(self, &named, false),
            Setting::TableXinfo => crate::change::columns_of(self, &named, true),
            Setting::IndexInfo => crate::change::places_of(self, &named, false),
            Setting::IndexXinfo => crate::change::places_of(self, &named, true),
            Setting::IndexList => crate::change::listed_indexes(self, &named),
            _ => return None,
        })
    }

    /// A statement: one core, or several put together.
    ///
    /// A compound is answered core by core and then merged left to
    /// right, which is the shape `multiSelect` compiles: every operator
    /// but `UNION ALL` answers its rows sorted and each of them once.
    /// Its `ORDER BY` and its `LIMIT` belong to the whole of it; a
    /// statement of one core sorts and limits its own rows, because a
    /// term of that `ORDER BY` may be an expression over the row and the
    /// row is gone by the time the answer is put together.
    fn statement(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
        scope: Scope<'_>,
    ) -> Result<Answered, Error> {
        let first = arena.select(id).ok_or(Error::Unsupported)?;
        let held = self.terms(arena, &first, sql, scope)?;
        let scope = Scope {
            terms: &held,
            outer: scope.outer,
            views: scope.views,
        };
        if first.compound.is_none() {
            return self.core(arena, id, sql, true, scope);
        }
        let mut answers: Vec<Answered> = Vec::new();
        let mut cores: Vec<Vec<Vec<u8>>> = Vec::new();
        let mut operators: Vec<Compound> = Vec::new();
        let mut at = Some(id);
        // Every core answers as many columns as the first, and the
        // refusal names the word that joins the one that does not to
        // the core before it.
        let mut joined: Option<Compound> = None;
        while let Some(id) = at {
            let core = arena.select(id).ok_or(Error::Unsupported)?;
            let mine = self.core(arena, id, sql, false, scope)?;
            if answers
                .first()
                .is_some_and(|first: &Answered| first.answer.names.len() != mine.answer.names.len())
            {
                return Err(unjoined(joined, !core.values.is_empty()));
            }
            cores.push(named_of(&mine.shape));
            answers.push(mine);
            at = core.compound.map(|(operator, next)| {
                operators.push(operator);
                joined = Some(operator);
                next
            });
        }
        let mut rest = answers.into_iter();
        let Answered {
            mut answer,
            mut shape,
        } = rest.next().ok_or(Error::Unsupported)?;
        let others: Vec<Answered> = rest.collect();
        // A column compares under the collation of the first core that
        // writes one, which is `multiSelectCollSeq`.
        for other in &others {
            for (column, mine) in shape.columns.iter_mut().zip(&other.shape.columns) {
                column.collation = column.collation.or(mine.collation);
            }
        }
        compounded(&mut shape, &others);
        let collations = self.collations(&shape);
        for (operator, right) in operators.into_iter().zip(others) {
            combine(operator, &mut answer, right.answer, &collations);
        }
        let keys = matched(arena, &first, sql, &cores, &collations, self.collating)?;
        if !keys.is_empty() {
            self.sort();
            sort_by_keys(&mut answer.rows, &keys);
        }
        let reach = Reach {
            database: self,
            arena,
            sql,
            scope,
        };
        let cursor = Cursor::new(self.collation(), self.encoding, reach);
        limit(arena, &first, sql, &mut answer.rows, &cursor)?;
        Ok(Answered { answer, shape })
    }

    /// The collation each column of a shape compares under, which is
    /// what it was written with where it was written with one.
    fn collations(&self, shape: &Shape) -> Vec<Collation> {
        shape
            .columns
            .iter()
            .map(|column| column.collation.unwrap_or(self.collation()))
            .collect()
    }

    /// One core of a statement. `whole` asks for the `ORDER BY` and the
    /// `LIMIT`, which belong to a core only where it is the statement.
    fn core(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
        whole: bool,
        scope: Scope<'_>,
    ) -> Result<Answered, Error> {
        let select = arena.select(id).ok_or(Error::Unsupported)?;
        let reach = Reach {
            database: self,
            arena,
            sql,
            scope,
        };
        if !select.values.is_empty() {
            let cursor = Cursor::new(self.collation(), self.encoding, reach);
            return listed(arena, &select, sql, &cursor);
        }
        let mut sides = self.sides(arena, &select, sql, scope)?;
        planned(
            arena,
            select.filter,
            sql,
            &mut sides,
            self.sensitive,
            self.schema_format(),
        );
        let shape = shape(arena, &select, sql, &sides, self.naming, self.collating)?;
        let collations = self.collations(&shape);
        let names: Vec<Vec<u8>> = shape
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let shown: Vec<Vec<u8>> = shape
            .columns
            .iter()
            .map(|column| column.shown.clone())
            .collect();
        let keys = if whole {
            keys(arena, &select, sql, &names, &collations, self.collating)?
        } else {
            Vec::new()
        };
        subqueries(arena, &select)?;
        let calls = aggregates(arena, &select, sql, &sides, self.grouped)?;
        let overs = overs(arena, &select, sql, self.grouped)?;
        // An `ORDER BY` the walk already answers in needs no sort, which
        // is what `sqlite3WhereIsOrdered` answers for the loops
        // `sqlite3WhereBegin` chose. Only the walk of one table answers
        // an order: a join answers the product, a group answers one row
        // per group, and a window reads the rows it was given.
        let walked = overs.is_empty()
            && calls.is_empty()
            && select.group.is_empty()
            && in_order(arena, &select, sql, &mut sides, self.schema_format());
        let mut rows: Vec<Sorted> = Vec::new();
        if !overs.is_empty() {
            // A window function reads the rows a statement has already
            // filtered and grouped, so the rows are kept and the window
            // answered over them before the statement answers.
            let mut kept = self.kept(arena, &select, sql, &sides, &calls, reach)?;
            for at in overed(arena, &select, sql, &mut kept, &overs)? {
                let cursor = kept.get(at).ok_or(Error::NoTable(Vec::new()))?;
                rows.push(sorted(arena, &select, sql, cursor, &keys)?);
            }
        } else if calls.is_empty() && select.group.is_empty() {
            self.scan(&sides, arena, sql, reach, &mut |cursor: &Cursor<'_>| {
                if keep(arena, select.filter, sql, cursor)? {
                    rows.push(sorted(arena, &select, sql, cursor, &keys)?);
                }
                Ok(())
            })?;
        } else {
            for group in self.groups(arena, &select, sql, &sides, &calls, reach)? {
                rows.push(sorted(arena, &select, sql, &group, &keys)?);
            }
        }
        if !keys.is_empty() && !walked {
            self.sort();
            rows.sort_by(|left, right| order_of(&left.keys, &right.keys, &keys));
        }
        let mut rows: Vec<Vec<Value>> = rows.into_iter().map(|row| row.values).collect();
        if select.distinct == Distinct::Distinct {
            distinct(&mut rows, &collations);
        }
        if whole {
            let cursor = Cursor::new(self.collation(), self.encoding, reach);
            limit(arena, &select, sql, &mut rows, &cursor)?;
        }
        Ok(Answered {
            answer: Answer {
                names: shown,
                declared: shape
                    .columns
                    .iter()
                    .map(|column| column.declared.clone())
                    .collect(),
                rows,
                stepped: Stepped::default(),
            },
            shape,
        })
    }

    /// The sides of a `FROM` clause, and how each attaches to the ones
    /// before it.
    ///
    /// A statement written inside the `FROM`, and a `WITH` term one of
    /// them names, are answered here and their rows kept: SQLite has no
    /// `LATERAL`, so neither can read the row the walk stands on and
    /// neither is answered twice.
    fn sides<'s>(
        &'s self,
        arena: &Arena,
        select: &Select,
        sql: &'s [u8],
        scope: Scope<'_>,
    ) -> Result<Vec<Side<'s>>, Error> {
        let mut out: Vec<Side<'s>> = Vec::new();
        for source in arena.sources(select.from) {
            // A name is matched with its quotes off, which is
            // `sqlite3Dequote` over every identifier the parser keeps.
            let alias = source.alias.map(|span| dequote(span.text(sql)));
            let (shape, from, name, held) = match source.kind {
                SourceKind::Table {
                    schema,
                    name,
                    indexed,
                } => {
                    let named = table_named(schema, name, sql);
                    // A schema in front of the name says which database
                    // of the connection holds the table, which
                    // `sqlite3FindTable` of
                    // `research/sqlite/src/build.c:343` reads as a place
                    // of `db->aDb`.
                    let place = match schema {
                        Some(span) => match self.placed(&dequote(span.text(sql))) {
                            Some(place) => Some(place),
                            None => return Err(Error::NoTable(named.shown)),
                        },
                        None => None,
                    };
                    // A `WITH` term is reached by its bare name; a name
                    // with a schema in front of it is a table.
                    let found = match place {
                        Some(_) => None,
                        None => scope
                            .terms
                            .iter()
                            .find(|(term, _)| term.eq_ignore_ascii_case(&named.name)),
                    };
                    if let Some((term, answered)) = found {
                        (
                            answered.shape.clone(),
                            Source::Rows(answered.answer.rows.clone()),
                            term.clone(),
                            Vec::new(),
                        )
                    } else if let Some(stored) = self.located(place, &named.name) {
                        indexed_held(stored, indexed, sql)?;
                        (
                            shape_of(&stored.table),
                            Source::Table(stored),
                            stored.table.name.clone(),
                            self.named_place(stored.place),
                        )
                    } else {
                        // A view names a statement, so the rows are the
                        // ones that statement answers, which is what
                        // `sqlite3SelectExpand` puts in its place.
                        let viewed = self.viewed(place, &named, scope)?;
                        let held = self.named_place(viewed.place);
                        (viewed.shape, Source::Rows(viewed.rows), viewed.name, held)
                    }
                }
                SourceKind::Select(id) => {
                    let answered = self.statement(arena, id, sql, scope)?;
                    (
                        answered.shape,
                        Source::Rows(answered.answer.rows),
                        Vec::new(),
                        Vec::new(),
                    )
                }
                SourceKind::Function { .. } => return Err(Error::Unsupported),
            };
            // A statement that stands as a table has the names of its
            // columns made unique, which `sqlite3ColumnsFromExprList`
            // does and which the tables inside brackets are not, they
            // answering under the names of the tables themselves.
            let shape = if shape.nested { shape } else { uniqued(shape) };
            let written: Vec<Vec<u8>> = arena
                .names(source.using)
                .iter()
                .map(|span| dequote(span.text(sql)))
                .collect();
            let using = if source.join.natural {
                if source.on.is_some() || !written.is_empty() {
                    return Err(Error::NaturalJoin);
                }
                // A `NATURAL` join matches by every name both sides
                // hold, in the order the side read last holds them.
                shape
                    .columns
                    .iter()
                    .filter(|column| holds(&out, &column.name))
                    .map(|column| column.name.clone())
                    .collect()
            } else {
                for name in &written {
                    if !holds(&out, name) || !shape.has(name) {
                        return Err(Error::JoinColumn(name.clone()));
                    }
                }
                written
            };
            // `sqlite3SelectExpand` names a statement written inside
            // the `FROM` that carries no alias after the statement's
            // own place, which is what stands in front of its columns
            // where `full_column_names` is on.
            let table = match source.kind {
                SourceKind::Select(id) => subquery_named(id),
                _ => name.clone(),
            };
            out.push(Side {
                shape,
                source: from,
                schema: held,
                table,
                name: alias.unwrap_or(name),
                kind: source.join.kind,
                on: source.on,
                using,
                plan: Plan::Rows(None, None),
                pushed: Vec::new(),
            });
        }
        rightward(arena, sql, &out)?;
        Ok(out)
    }

    /// What one statement written inside an expression answers for a
    /// row: `(SELECT ...)` as a value, `EXISTS`, `IN (SELECT ...)` and
    /// `IN table`.
    ///
    /// The statement is answered here and not before, so a correlated
    /// one is answered once per row of the statement that encloses it:
    /// O(n·m) for `n` outer rows and `m` inner ones.
    /// The same, for a row of a statement that writes, which stands
    /// inside no statement of its own.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the statement could not answer.
    pub(crate) fn subquery(
        &self,
        arena: &Arena,
        sql: &[u8],
        used: Used,
        row: &dyn eval::Row,
    ) -> Result<Value, Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        self.answer(arena, sql, used, scope, row)
    }

    /// The first row a statement written inside an expression answers,
    /// for a row of a statement that writes.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the statement could not answer.
    pub(crate) fn subquery_items(
        &self,
        arena: &Arena,
        sql: &[u8],
        select: SelectId,
        row: &dyn eval::Row,
    ) -> Result<Vec<(Value, Affinity, Option<Collation>)>, Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        self.answer_items(arena, sql, select, scope, row)
    }

    fn answer(
        &self,
        arena: &Arena,
        sql: &[u8],
        used: Used,
        scope: Scope<'_>,
        row: &dyn eval::Row,
    ) -> Result<Value, Error> {
        let inner = Scope {
            terms: scope.terms,
            outer: Some(row),
            views: scope.views,
        };
        match used {
            Used::Exists(select) => {
                let answered = self.statement(arena, select, sql, inner)?;
                Ok(Value::Int(i64::from(!answered.answer.rows.is_empty())))
            }
            Used::In(value, select, negated) => {
                let answered = self.statement(arena, select, sql, inner)?;
                contained(
                    arena,
                    value,
                    sql,
                    row,
                    &answered.answer.rows,
                    &answered.shape.columns,
                    negated,
                )
            }
            Used::InTable(value, schema, table, negated) => {
                let called = dequote(table.text(sql));
                let place = match schema {
                    Some(span) => match self.placed(&dequote(span.text(sql))) {
                        Some(place) => Some(place),
                        None => return Err(Error::NoTable(called)),
                    },
                    None => None,
                };
                let stored = self
                    .located(place, &called)
                    .ok_or_else(|| Error::NoTable(called.clone()))?;
                let side = Side {
                    shape: shape_of(&stored.table),
                    source: Source::Table(stored),
                    schema: self.named_place(stored.place),
                    table: called.clone(),
                    name: Vec::new(),
                    kind: JoinKind::Inner,
                    on: None,
                    using: Vec::new(),
                    plan: Plan::Rows(None, None),
                    pushed: Vec::new(),
                };
                let columns = side.shape.columns.clone();
                let mut rows = Vec::new();
                for step in self.scanned(&side, stored) {
                    rows.push(step?.1);
                }
                contained(arena, value, sql, row, &rows, &columns, negated)
            }
        }
    }

    /// The first row the statement `select` answers, each value with
    /// the affinity and the collation of the column it stands in,
    /// which is what a row compared against `(SELECT a, b)` compares
    /// against.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    fn answer_items(
        &self,
        arena: &Arena,
        sql: &[u8],
        select: SelectId,
        scope: Scope<'_>,
        row: &dyn eval::Row,
    ) -> Result<Vec<(Value, Affinity, Option<Collation>)>, Error> {
        let inner = Scope {
            terms: scope.terms,
            outer: Some(row),
            views: scope.views,
        };
        let answered = self.statement(arena, select, sql, inner)?;
        let first = answered.answer.rows.first();
        let mut out = Vec::new();
        for (at, column) in answered.shape.columns.iter().enumerate() {
            let value = first
                .and_then(|held| held.get(at))
                .cloned()
                .unwrap_or(Value::Null);
            out.push((value, column.affinity, column.collation));
        }
        Ok(out)
    }

    /// The `WITH` terms of a statement, each answered once.
    fn terms(
        &self,
        arena: &Arena,
        select: &Select,
        sql: &[u8],
        scope: Scope<'_>,
    ) -> Result<Vec<(Vec<u8>, Answered)>, Error> {
        let mut out = scope.terms.to_vec();
        let mut waiting: Vec<(Vec<u8>, &crate::ast::Cte)> = Vec::new();
        for cte in arena.ctes(select.ctes) {
            let name = dequote(cte.name.text(sql));
            // `sqlite3WithAdd` holds the names of one `WITH` apart.
            if waiting
                .iter()
                .any(|(held, _)| held.eq_ignore_ascii_case(&name))
            {
                return Err(Error::DuplicateTerm(name));
            }
            waiting.push((name, cte));
        }
        // A term reads the terms beside it, whichever was written
        // first, so the one that reads no other is answered first and
        // the ones that read it follow. A turn that answers none of
        // what is left leaves only terms that read each other.
        while !waiting.is_empty() {
            let held = waiting.len();
            let mut later = Vec::new();
            for (name, cte) in waiting {
                let mine = Scope {
                    terms: &out,
                    outer: scope.outer,
                    views: scope.views,
                };
                match self.term(arena, cte, &name, sql, mine) {
                    Ok(answered) => out.push((name, answered)),
                    // A term the statement has no answer for yet is one
                    // this turn passes over; every other refusal is the
                    // term's own.
                    Err(Error::NoTable(wanted)) if holds_term(arena, select, sql, &wanted) => {
                        later.push((name, cte));
                    }
                    Err(error) => return Err(error),
                }
            }
            if later.len() == held {
                // A term the statement never reads is left unanswered,
                // which is what the C library does by answering a term
                // where it is read; only a circle the statement reaches
                // refuses it.
                return match circling(arena, select, sql, &later) {
                    Some(name) => Err(Error::Circular(name)),
                    None => Ok(out),
                };
            }
            waiting = later;
        }
        Ok(out)
    }

    /// One `WITH` term answered.
    ///
    /// `RECURSIVE` says nothing: a term that reads its own name reads
    /// itself whether the word was written or not, which is what
    /// `sqlite3WithPush` decides by the name alone.
    ///
    /// # Errors
    ///
    /// Whatever the statement of the term is refused with.
    fn term(
        &self,
        arena: &Arena,
        cte: &crate::ast::Cte,
        name: &[u8],
        sql: &[u8],
        scope: Scope<'_>,
    ) -> Result<Answered, Error> {
        let mut answered = match self.recursive(arena, cte, name, sql, scope)? {
            Some(answered) => answered,
            None => self.statement(arena, cte.select, sql, scope)?,
        };
        renamed(arena, cte.columns, (sql, name), &mut answered)?;
        Ok(answered)
    }

    /// A `WITH` term that reads itself, which is
    /// `generateWithRecursiveQuery`: the cores before the one that
    /// reads the term answer the rows the walk starts from, and each
    /// row the walk takes off the front is what the cores after it
    /// read, so the rows those answer stand behind it. The walk ends
    /// where the rows do.
    ///
    /// The term answers nothing where no core of it reads the name,
    /// which is a `WITH` written `RECURSIVE` whose terms read nothing
    /// of their own. A term of `n` rows costs what its recursive cores
    /// cost, `n` times.
    fn recursive(
        &self,
        arena: &Arena,
        cte: &crate::ast::Cte,
        name: &[u8],
        sql: &[u8],
        scope: Scope<'_>,
    ) -> Result<Option<Answered>, Error> {
        let links = chain(arena, cte.select);
        let Some(first) = links
            .iter()
            .position(|link| reads(arena, &link.core, sql, name))
        else {
            return Ok(None);
        };
        // `sqlite3SelectNew` refuses a term whose first core reads it,
        // because the walk would then start from nothing.
        let Some(split) = before(&links, first) else {
            return Err(Error::Unsupported);
        };
        // `multiSelect` refuses a recursive core under an operator that
        // is neither `UNION` nor `UNION ALL`.
        if links.iter().skip(first).any(|link| {
            link.operator
                .is_some_and(|operator| !matches!(operator, Compound::Union | Compound::UnionAll))
        }) || !matches!(split, Compound::Union | Compound::UnionAll)
        {
            return Err(Error::Unsupported);
        }
        let mut answered = self.started(arena, &links, first, sql, scope)?;
        renamed(arena, cte.columns, (sql, name), &mut answered)?;
        let collations = self.collations(&answered.shape);
        let width = answered.answer.names.len();
        let mut rows: Vec<Vec<Value>> = Vec::new();
        for row in core::mem::take(&mut answered.answer.rows) {
            add(&mut rows, row, split == Compound::Union, &collations);
        }
        // `generateWithRecursiveQuery`: the rows of the term wait in a
        // queue, the `ORDER BY` of the term says which of them is taken
        // next, and the `LIMIT` says how many are taken at all. A row
        // the walk takes is answered and then read by the cores that
        // read the term.
        let head = arena.select(cte.select).ok_or(Error::Unsupported)?;
        let named = named_of(&answered.shape);
        let order = matched(arena, &head, sql, &[named], &collations, self.collating)?;
        let reach = Reach {
            database: self,
            arena,
            sql,
            scope,
        };
        let cursor = Cursor::new(self.collation(), self.encoding, reach);
        let (skip, most) = bounds(arena, &head, sql, &cursor)?;
        let mut taken: Vec<bool> = Vec::new();
        let mut out: Vec<Vec<Value>> = Vec::new();
        let mut passed = 0_usize;
        let mut next = 0_usize;
        loop {
            // A term that wrote no `ORDER BY` takes its rows in the
            // order they were written, so the walk steps through the
            // queue rather than reading it whole for the smallest.
            let found = if order.is_empty() {
                rows.get(next).cloned().map(|row| (next, row))
            } else {
                taken.resize(rows.len(), false);
                waiting(&rows, &taken, &order)
                    .and_then(|at| rows.get(at).cloned().map(|row| (at, row)))
            };
            let Some((at, row)) = found else {
                break;
            };
            next = at.saturating_add(1);
            for slot in taken.iter_mut().skip(at).take(1) {
                *slot = true;
            }
            if passed < skip {
                passed = passed.saturating_add(1);
            } else {
                out.push(row.clone());
                if most.is_some_and(|most| out.len() >= most) {
                    break;
                }
            }
            let mut terms = scope.terms.to_vec();
            terms.push((
                name.to_vec(),
                Answered {
                    answer: Answer {
                        declared: answered.answer.declared.clone(),
                        names: answered.answer.names.clone(),
                        rows: alloc::vec![row],
                        stepped: Stepped::default(),
                    },
                    shape: answered.shape.clone(),
                },
            ));
            let mine = Scope {
                terms: &terms,
                outer: scope.outer,
                views: scope.views,
            };
            for (step, link) in links.iter().enumerate().skip(first) {
                let answer = self.core(arena, link.id, sql, false, mine)?;
                if answer.answer.names.len() != width {
                    return Err(unjoined(before(&links, step), false));
                }
                let once = before(&links, step) == Some(Compound::Union);
                for new in answer.answer.rows {
                    add(&mut rows, new, once, &collations);
                }
                if rows.len() > RECURSION_ROWS {
                    return Err(Error::Recursion);
                }
            }
        }
        answered.answer.rows = out;
        Ok(Some(answered))
    }

    /// The rows a recursive term begins with, which are the cores
    /// before the first one that reads the term, combined.
    ///
    /// Answering them costs what those cores cost.
    fn started(
        &self,
        arena: &Arena,
        links: &[Link],
        first: usize,
        sql: &[u8],
        scope: Scope<'_>,
    ) -> Result<Answered, Error> {
        let mut answered = Answered::default();
        for (at, link) in links.get(..first).unwrap_or_default().iter().enumerate() {
            let mine = self.core(arena, link.id, sql, false, scope)?;
            let Some(operator) = before(links, at) else {
                answered = mine;
                continue;
            };
            if answered.answer.names.len() != mine.answer.names.len() {
                return Err(unjoined(Some(operator), false));
            }
            let collations = self.collations(&answered.shape);
            combine(operator, &mut answered.answer, mine.answer, &collations);
        }
        Ok(answered)
    }

    /// Walks the rows a statement reads, once, and hands each to `each`.
    ///
    /// One side costs O(n); a join of `k` sides is the loops nested,
    /// which is the product of their rows — no index is used yet, so the
    /// rows are right and the plan is not.
    fn scan<'b>(
        &self,
        sides: &'b [Side<'b>],
        arena: &Arena,
        sql: &[u8],
        reach: Reach<'b>,
        each: &mut dyn FnMut(&Cursor<'b>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut cursor = Cursor::new(self.collation(), self.encoding, reach);
        if sides.is_empty() {
            // A statement with no `FROM` reads one row of nothing.
            return each(&cursor);
        }
        let mut kept: Vec<Vec<i64>> = alloc::vec![Vec::new(); sides.len()];
        self.nest(sides, 0, &mut cursor, arena, sql, each, &mut kept, None)?;
        // Then the rows a `RIGHT` or a `FULL` join keeps that the nest
        // matched nothing to, with the levels before them empty. They
        // come after every row the nest answered, which is the order
        // `sqlite3WhereRightJoinLoop` walks them in.
        for (at, side) in sides.iter().enumerate() {
            if !matches!(side.kind, JoinKind::Right | JoinKind::Full) {
                continue;
            }
            cursor.held.clear();
            for before in sides.iter().take(at) {
                cursor.held.push(Held::empty(
                    &before.shape,
                    &before.schema,
                    &before.name,
                    &before.using,
                ));
            }
            // The walk marks what it matches into the same list, so a
            // row a `RIGHT` join at an earlier level already answered
            // counts as matched here, which is the row set
            // `sqlite3WhereRightJoinLoop` keeps for the whole statement.
            let matched = kept.get(at).map_or(&[][..], Vec::as_slice).to_vec();
            self.nest(
                sides,
                at,
                &mut cursor,
                arena,
                sql,
                each,
                &mut kept,
                Some(&matched),
            )?;
            cursor.held.clear();
        }
        Ok(())
    }

    /// The rows of one side of a `FROM`, each with the rowid where the
    /// side is a table that keeps one.
    ///
    /// `cursor` holds the rows of the sides read before this one, which
    /// is what a plan reads its key from where an `ON` names one.
    fn feed<'f>(&self, side: &'f Side<'f>, cursor: &Cursor<'_>) -> Feed<'a, 'f> {
        let stored = match &side.source {
            Source::Rows(rows) => return Feed::Rows(rows.iter()),
            Source::Table(stored) => stored,
        };
        if let Plan::Union(plans) = &side.plan {
            let feeds: Option<Vec<Feed<'a, 'f>>> = plans
                .iter()
                .map(|plan| self.branch(stored, plan, cursor))
                .collect();
            return match feeds {
                Some(feeds) => Feed::Union(Box::new(Union {
                    feeds,
                    at: 0,
                    seen: Vec::new(),
                })),
                None => self.scanned(side, stored),
            };
        }
        if let Some(held) = sought(&side.plan, cursor)
            && let Some(feed) = self.keyed(stored, held)
        {
            return feed;
        }
        // A plan that names an index and could not be walked falls back
        // to the whole tree, which answers the same rows.
        self.scanned(side, stored)
    }

    /// The walk one branch of an `OR` is read by: the table's own tree
    /// where the branch names a rowid range, and the index the branch
    /// names a key of otherwise.
    fn branch<'f>(
        &self,
        stored: &'f Stored,
        plan: &Plan,
        cursor: &Cursor<'_>,
    ) -> Option<Feed<'a, 'f>> {
        if let Plan::Rows(first, last) = plan {
            return Some(self.ranged(stored, (*first, *last)));
        }
        sought(plan, cursor).and_then(|held| self.keyed(stored, held))
    }

    /// The walk of one index, held to the key the plan names, and
    /// nothing where the entries cannot be compared against that key.
    ///
    /// An entry this walk cannot read whole is one it cannot compare, so
    /// the descent gives up: a caller that answers no walk reads the
    /// table's own tree instead, which answers the same rows and only
    /// the cost is not the same.
    fn keyed<'f>(&self, stored: &'f Stored, held: Seek) -> Option<Feed<'a, 'f>> {
        let Seek {
            root,
            key,
            collations,
            rowid_at,
            bounds,
            backwards,
        } = held;
        let image = self.imaged(stored.place);
        // The descent stands on the first entry the walk takes, which is
        // the first one the end the entries begin at does not reach
        // before, and the entry after it where that end leaves it out.
        // An index that holds its entries backwards begins at the high
        // end of the bounds and one that holds them forwards at the low
        // end.
        let mut descent = key.clone();
        let mut inside = true;
        let end = if backwards { &bounds.high } else { &bounds.low };
        if let Some((value, held)) = end {
            descent.push(value.clone());
            inside = *held;
        }
        let mut scratch = Vec::new();
        let walk = image
            .entries_from(root, &mut |entry| {
                let order = order_of_entry(
                    &image,
                    entry,
                    &descent,
                    &collations,
                    self.encoding,
                    &mut scratch,
                )
                .map_err(|_| crate::error::Error::Overrun)?;
                Ok(is_before(order, backwards, inside))
            })
            .ok()?;
        Some(Feed::Keyed(Box::new(Sought {
            image,
            stored,
            walk,
            key,
            collations,
            rowid_at,
            bounds,
            backwards,
            encoding: self.encoding,
            collation: self.collation(),
            payload: Vec::new(),
        })))
    }

    /// The rows of one table read out of its own tree, which is what a
    /// side with no plan and a side whose plan could not be walked both
    /// answer.
    fn scanned<'f>(&self, side: &'f Side<'f>, stored: &'f Stored) -> Feed<'a, 'f> {
        self.ranged(stored, side.range())
    }

    /// The rows of one table between two rowids, read out of its own
    /// tree.
    fn ranged<'f>(&self, stored: &'f Stored, range: (Option<i64>, Option<i64>)) -> Feed<'a, 'f> {
        Feed::Tree(Box::new(Tree {
            image: self.imaged(stored.place),
            stored,
            walk: self.walk(stored, range),
            encoding: self.encoding,
            collation: self.collation(),
            payload: Vec::new(),
        }))
    }

    /// The rows of a table, whichever kind of tree holds them, each with
    /// the rowid where the table has one.
    fn walk(&self, stored: &Stored, range: (Option<i64>, Option<i64>)) -> Walk<'a> {
        let image = self.imaged(stored.place);
        if stored.table.without_rowid {
            Walk::Index(image.entries(stored.root))
        } else {
            Walk::Table(image.rows_between(stored.root, range.0, range.1))
        }
    }

    /// One level of the nest: every row of `sides[at]` against what the
    /// levels above it hold.
    ///
    /// `spare` names the rows of this level to pass over, and says that
    /// the levels before it are empty: the join condition is not read,
    /// and no row is kept for a `LEFT`. Finding whether a row was
    /// matched is a walk of what was: O(n·m) for `n` rows against `m`
    /// matches.
    #[expect(
        clippy::too_many_arguments,
        reason = "the tables, where the walk stands, what it holds, the statement, what it has matched, and what it passes over"
    )]
    fn nest<'b>(
        &self,
        sides: &'b [Side<'b>],
        at: usize,
        cursor: &mut Cursor<'b>,
        arena: &Arena,
        sql: &[u8],
        each: &mut dyn FnMut(&Cursor<'b>) -> Result<(), Error>,
        kept: &mut [Vec<i64>],
        spare: Option<&[i64]>,
    ) -> Result<(), Error> {
        let Some(side) = sides.get(at) else {
            return each(cursor);
        };
        let deeper = at.saturating_add(1);
        let mut any = false;
        let mut ordinal = 0i64;
        for step in self.feed(side, cursor) {
            let (rowid, mut values) = step?;
            self.nulled(side, &mut values);
            let at_row = ordinal;
            ordinal = ordinal.saturating_add(1);
            if spare.is_some_and(|skip| skip.contains(&at_row)) {
                continue;
            }
            cursor.held.push(Held {
                shape: &side.shape,
                schema: &side.schema,
                name: &side.name,
                using: &side.using,
                rowid,
                values,
            });
            let attached = match spare {
                Some(_) => true,
                None => attached(arena, side, cursor, sql)?,
            };
            if attached {
                any = true;
                mark(kept, at, at_row);
                // A term of the `WHERE` this level answers holds for
                // every row the levels after it read, so a row it
                // passes over is one no row of the statement holds.
                // The term is read again where the statement is
                // answered, which is what makes this a cost and not an
                // answer.
                if all_hold(arena, &side.pushed, sql, cursor)? {
                    self.nest(sides, deeper, cursor, arena, sql, each, kept, None)?;
                }
            }
            cursor.held.pop();
        }
        // `wherecode.c` counts a step for each row of a loop after the
        // first, and only for a loop no term holds to a key.
        if whole_walk(side) {
            self.step(u64::try_from(ordinal.saturating_sub(1)).unwrap_or(0));
        }
        if spare.is_none() && !any && matches!(side.kind, JoinKind::Left | JoinKind::Full) {
            // A `LEFT JOIN` answers the row on the left once with
            // nothing on the right where nothing on the right matched.
            cursor.held.push(Held::empty(
                &side.shape,
                &side.schema,
                &side.name,
                &side.using,
            ));
            self.nest(sides, deeper, cursor, arena, sql, each, kept, None)?;
            cursor.held.pop();
        }
        Ok(())
    }

    /// Every row a statement answers over, kept so that a window
    /// function can read them all: one cursor per group where the
    /// statement groups, and one per row where it does not.
    fn kept<'b>(
        &self,
        arena: &Arena,
        select: &Select,
        sql: &[u8],
        sides: &'b [Side<'b>],
        calls: &[Call],
        reach: Reach<'b>,
    ) -> Result<Vec<Cursor<'b>>, Error> {
        if !calls.is_empty() || !select.group.is_empty() {
            return self.groups(arena, select, sql, sides, calls, reach);
        }
        let mut out = Vec::new();
        self.scan(sides, arena, sql, reach, &mut |cursor: &Cursor<'b>| {
            if keep(arena, select.filter, sql, cursor)? {
                out.push(Cursor {
                    held: cursor.held.clone(),
                    collation: cursor.collation,
                    encoding: cursor.encoding,
                    aggregates: cursor.aggregates.clone(),
                    reach,
                });
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The groups a statement that aggregates answers: one cursor each,
    /// in the order the `GROUP BY` terms collate in, which is the order
    /// the sorter of `src/select.c` puts them in.
    fn groups<'b>(
        &self,
        arena: &Arena,
        select: &Select,
        sql: &[u8],
        sides: &'b [Side<'b>],
        calls: &[Call],
        reach: Reach<'b>,
    ) -> Result<Vec<Cursor<'b>>, Error> {
        let terms = grouping(arena, select, sql, sides)?;
        let mut groups: Vec<Group<'b>> = Vec::new();
        self.scan(sides, arena, sql, reach, &mut |cursor: &Cursor<'b>| {
            if !keep(arena, select.filter, sql, cursor)? {
                return Ok(());
            }
            let mut key = Vec::new();
            for term in &terms {
                key.push(grouped(*term, arena, sql, cursor)?);
            }
            let found = groups.iter().position(|group| alike(&group.key, &key));
            let group = if let Some(at) = found {
                groups.get_mut(at)
            } else {
                groups.push(Group::new(key, calls));
                groups.last_mut()
            };
            // A place read out of the list, or the end of a list just
            // pushed to: neither is ever nothing.
            group.map_or(Ok(()), |group| group.step(arena, sql, cursor, calls))
        })?;
        if terms.is_empty() && groups.is_empty() {
            // A statement that aggregates over no group answers one row
            // whatever the rows were, which is what `SELECT count(*)`
            // over an empty table is.
            groups.push(Group::new(Vec::new(), calls));
        }
        groups.sort_by(|left, right| order_of_keys(&left.key, &right.key));
        let mut out = Vec::new();
        for group in &groups {
            let mut answers = Vec::new();
            for (call, accumulator) in calls.iter().zip(&group.accumulators) {
                answers.push((call.id, accumulator.finish()?));
            }
            // A group that kept no row answers `NULL` for every column
            // of it, which is the accumulator never loaded.
            let held = group.magnet.clone().unwrap_or_else(|| {
                sides
                    .iter()
                    .map(|side| Held::empty(&side.shape, &side.schema, &side.name, &side.using))
                    .collect()
            });
            let cursor = Cursor {
                held,
                collation: self.collation(),
                encoding: self.encoding,
                aggregates: answers,
                reach,
            };
            if keep(arena, select.having, sql, &cursor)? {
                out.push(cursor);
            }
        }
        Ok(out)
    }
}

/// A `VALUES`, which answers its rows and names them by their place.
fn listed(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    outer: &dyn eval::Row,
) -> Result<Answered, Error> {
    let mut rows = Vec::new();
    for row in arena.children(select.values) {
        // The parser builds each row of a `VALUES` as a row node, so
        // what the node names is what the row holds.
        let mut items = Vec::new();
        arena
            .node(*row)
            .into_iter()
            .for_each(|node| arena.under(node, |item| items.push(item)));
        let mut values = Vec::new();
        for item in items {
            values.push(evaluate_row(arena, item, sql, outer)?);
        }
        if rows
            .first()
            .is_some_and(|first: &Vec<Value>| first.len() != values.len())
        {
            return Err(Error::Values);
        }
        rows.push(values);
    }
    let width = rows.first().map_or(0, Vec::len);
    let mut names = Vec::new();
    let mut columns = Vec::new();
    for at in 1..=width {
        let mut name = b"column".to_vec();
        name.extend_from_slice(&number::integer_text(i64::try_from(at).unwrap_or(0)));
        columns.push(Column {
            name: name.clone(),
            shown: name.clone(),
            from: Vec::new(),
            hidden: false,
            affinity: Affinity::None,
            collation: None,
            datatype: NUMBER,
            declared: Vec::new(),
        });
        names.push(name);
    }
    Ok(Answered {
        answer: Answer {
            declared: alloc::vec![Vec::new(); names.len()],
            names,
            rows,
            stepped: Stepped::default(),
        },
        shape: Shape {
            columns,
            nested: false,
            keyed: false,
            key: None,
        },
    })
}

/// Plans how each side of a `FROM` is read, out of the terms the
/// `WHERE` holds.
///
/// Only the terms a top-level `AND` spine holds are read, because a term
/// under an `OR` or a `NOT` does not have to be true of every row the
/// statement answers. Only the `WHERE` is read and never an `ON`,
/// because an `ON` decides which rows of an outer join match and not
/// which rows the statement answers. A row a plan leaves out is a row
/// one of those terms refuses, so every plan answers what a scan
/// answers.
fn planned(
    arena: &Arena,
    filter: Option<ExprId>,
    sql: &[u8],
    sides: &mut [Side<'_>],
    sensitive: bool,
    format: u32,
) {
    let terms = filter.map_or_else(Vec::new, |filter| {
        terms_of(arena, filter, sql, sides, sensitive)
    });
    // A rowid range is what a table's own tree is walked by, and it
    // answers a row where an index answers only where the row is, so it
    // is taken first and never given up for an index.
    let mut ranges = alloc::vec![(None, None); sides.len()];
    for term in &terms {
        let (Reached::Key, Value::Int(bound)) = (term.reached, &term.value) else {
            continue;
        };
        for range in ranges.iter_mut().skip(term.at).take(1) {
            narrow(&mut range.0, &mut range.1, term.op, *bound);
        }
    }
    let mut plans: Vec<Plan> = Vec::new();
    for ((at, side), range) in sides.iter().enumerate().zip(&ranges) {
        // An index over a table that keeps its rows in the key's own
        // tree ends its entries with that key and not with a rowid, and
        // a side already held to a rowid range is read by that range.
        let keyed = match &side.source {
            Source::Table(stored) if !stored.table.without_rowid && *range == (None, None) => {
                // A key the statement writes out is read once; a key a
                // side already read answers is read per row of that
                // side, so the first is taken where both are there. An
                // `OR` is read last, because one index costs less than
                // one walk per branch.
                plan_of(&terms, at, stored, format)
                    .or_else(|| {
                        filter.and_then(|filter| {
                            union_of(
                                arena,
                                filter,
                                sql,
                                sides,
                                at,
                                stored,
                                Settled { sensitive, format },
                            )
                        })
                    })
                    .or_else(|| joined(arena, sql, sides, at, stored))
            }
            Source::Table(_) | Source::Rows(_) => None,
        };
        plans.push(keyed.unwrap_or(Plan::Rows(range.0, range.1)));
    }
    for (side, plan) in sides.iter_mut().zip(plans) {
        side.plan = plan;
    }
    let Some(filter) = filter else {
        return;
    };
    // A `RIGHT` or a `FULL` join keeps the rows of its own side that
    // the walk matched nothing to, and the walk marks a row as matched
    // where it reads the levels under it, so a term read above such a
    // level would leave rows unmatched that the join matched.
    let outer = sides
        .iter()
        .rposition(|side| matches!(side.kind, JoinKind::Right | JoinKind::Full))
        .unwrap_or(0);
    // `sqlite3WhereSplit` puts every term of the `AND` spine on the
    // level where the sides it names are all read, so a term is read
    // once per row of that level and not once per row of the product.
    let mut spine = alloc::vec![filter];
    while let Some(id) = spine.pop() {
        if let Some(Node::Binary {
            op: BinaryOp::And,
            left,
            right,
        }) = arena.node(id)
        {
            spine.push(left);
            spine.push(right);
            continue;
        }
        let Some(at) = answerable(arena, id, sql, sides) else {
            continue;
        };
        // A term the levels above such a join answer is read on that
        // level instead, where every row it marks is marked already.
        let at = at.max(outer);
        for side in sides.iter_mut().skip(at).take(1) {
            side.pushed.push(id);
        }
    }
}

/// The last side a term names a column of, where every part of the
/// term is one this walk answers from the sides alone.
///
/// A term that names a column no side answers, a column two sides
/// answer, a statement of its own, a bound value or a function is
/// answered where the whole statement is: a function may answer
/// differently each time it is read, and the term is read again there.
///
/// Walking one term costs O(n) in its nodes.
fn answerable(arena: &Arena, id: ExprId, sql: &[u8], sides: &[Side<'_>]) -> Option<usize> {
    let mut deepest = 0;
    let mut stack = alloc::vec![id];
    while let Some(id) = stack.pop() {
        let node = arena.node(id)?;
        match node {
            Node::Column { .. } => {
                let (at, _) = reached(arena, id, sql, sides)?;
                deepest = deepest.max(at);
            }
            Node::Literal(_)
            | Node::Unary { .. }
            | Node::Binary { .. }
            | Node::Between { .. }
            | Node::InList { .. }
            | Node::Like { .. }
            | Node::Cast { .. }
            | Node::Collate { .. }
            | Node::Case { .. }
            | Node::Row(_) => {}
            Node::Variable(_)
            | Node::Call { .. }
            | Node::Over { .. }
            | Node::Subquery(_)
            | Node::Exists(_)
            | Node::InSelect { .. }
            | Node::InTable { .. }
            | Node::Raise { .. } => return None,
        }
        arena.under(node, |child| stack.push(child));
    }
    Some(deepest)
}

/// What one side's column is compared against, out of one term.
struct Bound {
    /// Which side.
    at: usize,
    /// Which of its columns, or its rowid.
    reached: Reached,
    /// Which way the comparison runs, with the column on the left.
    op: BinaryOp,
    /// What the column is compared against.
    value: Value,
    /// What the term asks of the column, where the term was read out of
    /// a `LIKE` or a `GLOB` rather than written.
    needs: Option<Pattern>,
}

/// What a bound read out of a pattern asks of the column it holds.
#[derive(Clone, Copy)]
struct Pattern {
    /// The collation the pattern matches under, which the column has to
    /// compare under for the bound to hold.
    collation: Collation,
    /// Whether the prefix of the pattern, or the value one past it,
    /// reads as a number. A column of any affinity but text converts
    /// such a value to a number and holds it before every text, so the
    /// bound would reach other rows than the pattern matches.
    numeric: bool,
}

/// What a name in a `WHERE` reaches.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reached {
    /// The rowid of the side.
    Key,
    /// The column of the side at this place.
    Column(usize),
}

/// Every term a top-level `AND` spine holds that compares a column of
/// one side against a value no row is needed to read.
fn terms_of(
    arena: &Arena,
    filter: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    sensitive: bool,
) -> Vec<Bound> {
    let mut out = Vec::new();
    let mut spine = alloc::vec![filter];
    while let Some(id) = spine.pop() {
        if let Some(Node::Like {
            op,
            value,
            pattern,
            escape,
            negated,
        }) = arena.node(id)
        {
            if !negated && escape.is_none() {
                out.extend(liked(arena, op, value, pattern, sql, sides, sensitive));
            }
            continue;
        }
        // `sqlite3ExprCodeBetween` reads `x BETWEEN a AND b` as
        // `x >= a AND x <= b`, and a term of each holds the walk.
        if let Some(Node::Between {
            value,
            low,
            high,
            negated,
        }) = arena.node(id)
        {
            if !negated {
                out.extend(bounded(arena, value, low, BinaryOp::Ge, sql, sides));
                out.extend(bounded(arena, value, high, BinaryOp::Le, sql, sides));
            }
            continue;
        }
        let Some(Node::Binary { op, left, right }) = arena.node(id) else {
            continue;
        };
        if op == BinaryOp::And {
            spine.push(left);
            spine.push(right);
            continue;
        }
        if !matches!(
            op,
            BinaryOp::Eq | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            continue;
        }
        // `a < 5` and `5 > a` say the same thing about `a`, so the
        // operator turns over with the operands.
        for (op, column, value) in [(op, left, right), (flipped(op), right, left)] {
            let Some((at, reached)) = reached(arena, column, sql, sides) else {
                continue;
            };
            // `sqlite3BinaryCompareCollSeq`: a `COLLATE` on either side
            // says what the comparison compares under, which need not be
            // what the column compares under, and an index over the
            // column then holds its entries in another order than the
            // term asks about. A `COLLATE` over the column itself is
            // already no term, because `reached` reads a column and not
            // a node above one.
            if matches!(arena.node(value), Some(Node::Collate { .. })) {
                continue;
            }
            // A term that names a column is a term about the row, and
            // the row is what is being planned for, so only a value the
            // walk needs no row to read is one it can be held to.
            let Ok(value) = evaluate_row(arena, value, sql, &eval::NoRow(None)) else {
                continue;
            };
            out.push(Bound {
                at,
                reached,
                op,
                value,
                needs: None,
            });
        }
    }
    out
}

/// The term one end of a `BETWEEN` holds the column at, and nothing
/// where the value is no column of a side or the end is no value the
/// walk can read without a row.
///
/// Reading one end costs what the expression it is read from costs.
fn bounded(
    arena: &Arena,
    value: ExprId,
    end: ExprId,
    op: BinaryOp,
    sql: &[u8],
    sides: &[Side<'_>],
) -> Option<Bound> {
    let (at, reached) = reached(arena, value, sql, sides)?;
    let held = evaluate_row(arena, end, sql, &eval::NoRow(None)).ok()?;
    Some(Bound {
        at,
        reached,
        op,
        value: held,
        needs: None,
    })
}

/// The two bounds a `LIKE` or a `GLOB` over a column holds that column
/// between, where the pattern begins with characters no wildcard stands
/// among.
///
/// This is `isLikeOrGlob` of `research/sqlite/src/whereexpr.c` and the
/// terms `exprAnalyze` writes from it: `x LIKE 'abc%'` holds `x` between
/// `'ABC'` and `'abd'`, the one in capitals and the other in small
/// letters so that the bounds hold a blob as well. The pattern is still
/// read for every row the bounds answer, so bounds that answer more rows
/// than the pattern matches answer the rows the statement answers.
///
/// Reading the pattern costs O(n) in its bytes.
fn liked(
    arena: &Arena,
    op: crate::ast::LikeOp,
    value: ExprId,
    pattern: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    sensitive: bool,
) -> Vec<Bound> {
    let wildcards: &[u8] = match op {
        crate::ast::LikeOp::Like => b"%_",
        crate::ast::LikeOp::Glob => b"*?[",
        crate::ast::LikeOp::Regexp | crate::ast::LikeOp::Match => return Vec::new(),
    };
    // `sqlite3IsLikeFunction`: `GLOB` tells the letters apart and `LIKE`
    // does not, except on a connection `PRAGMA case_sensitive_like` set.
    let nocase = op == crate::ast::LikeOp::Like && !sensitive;
    let Some((at, reached)) = reached(arena, value, sql, sides) else {
        return Vec::new();
    };
    let Ok(Value::Text(held)) = evaluate_row(arena, pattern, sql, &eval::NoRow(None)) else {
        return Vec::new();
    };
    // Only the bytes below 128 are read as a prefix, because the byte
    // after one above 127 is part of the same character and incrementing
    // the one would write over the other.
    let count = held
        .iter()
        .position(|byte| *byte >= 0x80 || wildcards.contains(byte))
        .unwrap_or(held.len());
    let Some(prefix) = held.get(..count).filter(|held| !held.is_empty()) else {
        return Vec::new();
    };
    let low: Vec<u8> = prefix
        .iter()
        .map(|byte| {
            if nocase {
                byte.to_ascii_uppercase()
            } else {
                *byte
            }
        })
        .collect();
    let mut high: Vec<u8> = prefix
        .iter()
        .map(|byte| {
            if nocase {
                byte.to_ascii_lowercase()
            } else {
                *byte
            }
        })
        .collect();
    // The last character of the prefix is incremented, which is what
    // makes the high end one past every value the prefix begins.
    for last in high.iter_mut().rev().take(1) {
        *last = last.saturating_add(1);
    }
    // A blob the pattern matches stands after every text, so the high
    // end is a blob: the walk then reaches the text the prefix begins
    // and the blobs it begins both, which is what the two passes of the
    // loop `sqlite3WhereBegin` writes for `WHERE_LIKEOPT` reach.
    // `isLikeOrGlob` refuses a prefix that reads as a number, and a
    // lone minus with it, where the column is not one of text affinity,
    // because the column converts such a value to a number.
    let mut bumped = prefix.to_vec();
    for last in bumped.iter_mut().rev().take(1) {
        *last = last.saturating_add(1);
    }
    let needs = Pattern {
        collation: if nocase {
            Collation::NoCase
        } else {
            Collation::Binary
        },
        numeric: prefix == b"-" || is_number(prefix) || is_number(&bumped),
    };
    alloc::vec![
        Bound {
            at,
            reached,
            op: BinaryOp::Ge,
            value: Value::Text(low),
            needs: Some(needs),
        },
        Bound {
            at,
            reached,
            op: BinaryOp::Lt,
            value: Value::Blob(high),
            needs: Some(needs),
        },
    ]
}

/// Whether the bytes read as a number, which is what `sqlite3AtoF`
/// answers above nought for.
fn is_number(bytes: &[u8]) -> bool {
    let read = crate::number::real(bytes);
    read.number() && read.complete()
}

/// Which side an expression names a column of, and which column.
fn reached(arena: &Arena, id: ExprId, sql: &[u8], sides: &[Side<'_>]) -> Option<(usize, Reached)> {
    let Node::Column {
        schema,
        table,
        column,
    } = arena.node(id)?
    else {
        return None;
    };
    let held = schema.map(|span| dequote(span.text(sql)));
    let named = table.map(|span| dequote(span.text(sql)));
    let column = &dequote(column.text(sql));
    let mut found = None;
    for (at, side) in sides.iter().enumerate() {
        // A schema in front of the table names the database the side
        // reads, which a side that reads no table has none of.
        if held
            .as_deref()
            .is_some_and(|held| !side.schema.eq_ignore_ascii_case(held))
        {
            continue;
        }
        if named.as_deref().is_some_and(|named| !side.named(named)) {
            continue;
        }
        let Some(reached) = side.shape.reaches(column) else {
            continue;
        };
        if found.is_some() {
            // A name two sides answer is ambiguous, and the statement
            // refuses it later; nothing is planned for it here.
            return None;
        }
        found = Some((at, reached));
    }
    found
}

/// The index the terms reach on one side, where they reach one.
///
/// The key is what the terms hold the leading columns of the index to
/// with `=`, and the bounds are what a term holds the column after them
/// between, which is the loop `sqlite3WhereBegin` writes for
/// `WHERE_COLUMN_EQ` and `WHERE_COLUMN_RANGE`. Reading the terms costs
/// O(i*c*t) in the indexes, their columns and the terms.
fn plan_of(terms: &[Bound], at: usize, stored: &Stored, format: u32) -> Option<Plan> {
    for kept in &stored.indexes {
        // A partial index answers fewer entries than the table has rows,
        // so a statement planned against one would read fewer rows than
        // it must.
        if kept.index.filter.is_some() {
            continue;
        }
        let mut key = Vec::new();
        let mut collations = Vec::new();
        let mut bounds = Bounds::default();
        // An index holds every place it has in one direction or reaches
        // no key, because the entries of a place held the other way run
        // over again for each value of the place before it.
        let backwards = held_backwards(kept.index.columns.first(), format);
        for held in &kept.index.columns {
            // An index held in another order than the terms ask about,
            // a place over an expression, and a place under another
            // collation than the column compares under each answer
            // entries in an order no term names.
            let found = held.place().and_then(|place| {
                stored
                    .table
                    .columns
                    .get(place)
                    .map(|column| (place, column))
            });
            let Some((place, column)) = found else {
                break;
            };
            if held_backwards(Some(held), format) != backwards || held.collation != column.collation
            {
                break;
            }
            // A column of real affinity holds a whole number as a whole
            // number, which `OP_RealAffinity` reads back as a real, so
            // an entry compares against a bound as the row does not:
            // 3175546974276630385 stands in an entry as itself and is
            // read out of the row as 3175546974276630528.
            if column.affinity == Affinity::Real {
                break;
            }
            let reached = Reached::Column(place);
            let named = |op: BinaryOp| {
                terms
                    .iter()
                    .find(|term| {
                        term.at == at
                            && term.op == op
                            && term.reached == reached
                            // An index holds no entry a comparison
                            // against `NULL` reaches, which is what
                            // `NULL = NULL` answering nothing means.
                            && term.value != Value::Null
                            // A bound read out of a pattern holds only
                            // where the column compares under the
                            // collation the pattern matches under, and
                            // only where the column holds text: a
                            // column that converts its values to
                            // numbers holds them before every text.
                            && term.needs.is_none_or(|wanted| {
                                wanted.collation == column.collation
                                    && (column.affinity == Affinity::Text || !wanted.numeric)
                            })
                    })
                    .map(|term| {
                        // An entry holds what the table's affinity left
                        // of the value, so the bound is what that
                        // affinity leaves of it. A conversion that
                        // answers another number, which a real affinity
                        // over a large whole number does, is one the
                        // bound is widened for.
                        let mut value = term.value.clone();
                        crate::value::apply(&mut value, column.affinity);
                        let exact = compare(&value, &term.value, Collation::Binary)
                            == core::cmp::Ordering::Equal;
                        (value, exact)
                    })
            };
            // A key that is not the value the term names would reach
            // other entries than the term is true of, so the index is
            // left alone.
            if let Some((value, exact)) = named(BinaryOp::Eq) {
                if !exact {
                    break;
                }
                key.push(value);
                collations.push(held.collation);
                continue;
            }
            // The column after the key is the last one a term reaches,
            // because the entries of the columns after it run over
            // again for each value of this one.
            let widened = |held: Option<(Value, bool)>, inside: bool| {
                held.map(|(value, exact)| (value, inside || !exact))
            };
            bounds.low =
                widened(named(BinaryOp::Ge), true).or_else(|| widened(named(BinaryOp::Gt), false));
            bounds.high =
                widened(named(BinaryOp::Le), true).or_else(|| widened(named(BinaryOp::Lt), false));
            if !bounds.is_empty() {
                collations.push(held.collation);
            }
            break;
        }
        if key.is_empty() && bounds.is_empty() {
            continue;
        }
        return Some(Plan::Keyed {
            root: kept.root,
            key,
            collations,
            rowid_at: kept.index.columns.len(),
            bounds,
            backwards,
        });
    }
    None
}

/// Whether the walk of this side reads a whole table or a whole index,
/// which is the loop `wherecode.c` counts the steps of: a loop no term
/// holds to a key.
const fn whole_walk(side: &Side<'_>) -> bool {
    matches!(side.source, Source::Table(_))
        && match &side.plan {
            Plan::Rows(None, None) => true,
            Plan::Keyed { key, bounds, .. } => key.is_empty() && bounds.is_empty(),
            Plan::Rows(_, _) | Plan::Joined { .. } | Plan::Union(_) => false,
        }
}

/// What the `ORDER BY` of a statement over one table asks of the walk.
enum Ordering {
    /// The walk answers another order, so the rows are sorted.
    Sorted,
    /// The walk as the plan stands answers the order.
    Walked,
    /// The walk of an index answers the order, read by this plan.
    Index(Plan),
}

/// Whether the walk of the one side answers the rows in the order the
/// `ORDER BY` asks for, reading that side by an index where one index
/// holds its entries in that order.
///
/// This is what `sqlite3WhereIsOrdered` answers: the terms of the
/// `ORDER BY` a loop already answers need no sort. Finding the index
/// costs O(n*m) in the indexes of the table and the terms.
fn in_order(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &mut [Side<'_>],
    format: u32,
) -> bool {
    let plan = match ordering(arena, select, sql, sides, format) {
        Ordering::Sorted => return false,
        Ordering::Walked => None,
        Ordering::Index(plan) => Some(plan),
    };
    if let Some(plan) = plan {
        for side in sides.iter_mut().take(1) {
            side.plan = plan.clone();
        }
    }
    true
}

/// What the walk of the one side answers of the `ORDER BY`.
fn ordering(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
    format: u32,
) -> Ordering {
    let terms = arena.orders(select.order);
    let [side] = sides else {
        return Ordering::Sorted;
    };
    let Source::Table(stored) = &side.source else {
        return Ordering::Sorted;
    };
    if terms.is_empty() || stored.table.without_rowid {
        return Ordering::Sorted;
    }
    let mut places = Vec::new();
    // An index holds its entries in one direction, so terms running in
    // two ask about an order no index holds.
    let backwards = terms
        .first()
        .is_some_and(|term| term.order == crate::ast::Order::Descending);
    for term in terms {
        // A term whose nulls are put where the order does not put them
        // asks about another order again, and a `COLLATE` over the
        // column is no column, which `reached` answers nothing for.
        if (term.order == crate::ast::Order::Descending) != backwards
            || term.nulls != crate::ast::Nulls::Unspecified
        {
            return Ordering::Sorted;
        }
        match reached(arena, term.expr, sql, sides) {
            Some((_, Reached::Key)) => places.push(None),
            Some((_, Reached::Column(place))) => places.push(Some(place)),
            None => return Ordering::Sorted,
        }
    }
    // A rowid stands once in the table, so no term after one decides
    // anything, and the table's own tree holds its rows in that order.
    if places.first() == Some(&None) {
        return if !backwards && matches!(side.plan, Plan::Rows(_, _)) {
            Ordering::Walked
        } else {
            Ordering::Sorted
        };
    }
    // An entry of an index ends with the rowid, so a last term naming
    // the rowid is the order the index holds its entries in already.
    if places.last() == Some(&None) {
        places.pop();
    }
    let Some(wanted) = places.into_iter().collect::<Option<Vec<usize>>>() else {
        return Ordering::Sorted;
    };
    // A side already held to a key by an index answers its entries in
    // that index's order, so the key is kept where that order is the one
    // the terms name.
    if let Plan::Keyed { root, key, .. } = &side.plan {
        return if suffixed(stored, *root, key.len(), &wanted, backwards, format) {
            Ordering::Walked
        } else {
            Ordering::Sorted
        };
    }
    if !matches!(side.plan, Plan::Rows(None, None)) {
        return Ordering::Sorted;
    }
    match walked(stored, &wanted, backwards, format) {
        Some(plan) => Ordering::Index(plan),
        None => Ordering::Sorted,
    }
}

/// Whether the walk of the index whose tree begins at `root`, held to
/// `held` many values of its key, answers its rows in the order the
/// columns at `wanted` name.
///
/// The first `held` columns of the index stand at one value for the
/// whole walk, so a term naming one of them decides nothing and the
/// terms after it name the columns from there on.
///
/// Costs O(n*m) in the columns of the index and the terms.
fn suffixed(
    stored: &Stored,
    root: u32,
    held: usize,
    wanted: &[usize],
    backwards: bool,
    format: u32,
) -> bool {
    stored
        .indexes
        .iter()
        .find(|kept| kept.root == root)
        .is_some_and(|kept| {
            let constant: Vec<usize> = kept
                .index
                .columns
                .iter()
                .take(held)
                .filter_map(crate::schema::Keyed::place)
                .collect();
            let mut at = held;
            for place in wanted {
                if constant.contains(place) {
                    continue;
                }
                if !holds_column(stored, &kept.index, at, *place, backwards, format) {
                    return false;
                }
                at = at.saturating_add(1);
            }
            true
        })
}

/// Whether the column of `index` at `at` holds the table column at
/// `place` in the order a term over that column asks about.
///
/// A place over an expression holds a value no column names, a place
/// held the other way answers its entries in the other order, and a
/// place under another collation than the column compares under answers
/// them in another order again. An index every place of which is held
/// backwards answers a `DESC` order read from its first entry on, which
/// is what a `CREATE INDEX` writing `DESC` is for.
fn holds_column(
    stored: &Stored,
    index: &crate::schema::Index,
    at: usize,
    place: usize,
    backwards: bool,
    format: u32,
) -> bool {
    index.columns.get(at).is_some_and(|held| {
        held.place() == Some(place)
            && held_backwards(Some(held), format) == backwards
            && stored
                .table
                .columns
                .get(place)
                .is_some_and(|column| column.collation == held.collation)
    })
}

/// The plan that walks an index of `stored` whole, where one index holds
/// its entries in the order the columns at `wanted` name.
///
/// The key of the plan is empty, which is the walk of every entry from
/// the first: `order_of_entry` compares nothing and answers that every
/// entry begins with the key.
///
/// Costs O(n*m) in the indexes and the columns named.
fn walked(stored: &Stored, wanted: &[usize], backwards: bool, format: u32) -> Option<Plan> {
    for kept in &stored.indexes {
        // A partial index answers fewer entries than the table has rows,
        // so it holds its entries in no order over the columns.
        if kept.index.filter.is_some() || kept.index.columns.len() < wanted.len() {
            continue;
        }
        let held = wanted
            .iter()
            .enumerate()
            .all(|(at, place)| holds_column(stored, &kept.index, at, *place, backwards, format));
        if held {
            return Some(Plan::Keyed {
                root: kept.root,
                key: Vec::new(),
                collations: Vec::new(),
                rowid_at: kept.index.columns.len(),
                bounds: Bounds::default(),
                backwards,
            });
        }
    }
    None
}

/// Whether the place of an index is held from the largest value down,
/// which `sqlite3CreateIndex` honors on a file of schema format 4 and
/// ignores on one below.
fn held_backwards(held: Option<&crate::schema::Keyed>, format: u32) -> bool {
    format >= 4 && held.is_some_and(|held| held.order == crate::ast::Order::Descending)
}

/// What the connection and the file settle for a plan.
#[derive(Clone, Copy)]
struct Settled {
    /// Whether `LIKE` compares its two sides case sensitively, which
    /// `PRAGMA case_sensitive_like` sets.
    sensitive: bool,
    /// The schema format of the file, which decides whether a place of
    /// an index written `DESC` is held backwards.
    format: u32,
}

/// The plans a top-level `OR` of the `WHERE` names, where every branch
/// of it names a key of an index over this side.
///
/// This is the multi-index `OR` optimization of `src/where.c`, which
/// `whereOrInsert` builds: the branches are walked one after another and
/// a row is answered once. A branch that names no key leaves the side
/// scanned, because such a branch holds any row and a walk that left it
/// out would answer fewer rows than the statement asks for.
///
/// Finding the branches costs O(n) in the nodes of the `WHERE`.
fn union_of(
    arena: &Arena,
    filter: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    at: usize,
    stored: &Stored,
    settled: Settled,
) -> Option<Plan> {
    let mut spine = alloc::vec![filter];
    while let Some(id) = spine.pop() {
        match arena.node(id) {
            Some(Node::Binary {
                op: BinaryOp::And,
                left,
                right,
            }) => {
                spine.push(left);
                spine.push(right);
            }
            Some(Node::Binary {
                op: BinaryOp::Or, ..
            }) => {
                if let Some(plan) = ored(arena, id, sql, sides, at, stored, settled) {
                    return Some(plan);
                }
            }
            _ => {}
        }
    }
    None
}

/// The plans the branches of the `OR` at `id` name, in the order the
/// `WHERE` writes them, and nothing where a branch names no key.
///
/// Reading one branch costs what [`plan_of`] costs over its terms.
fn ored(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    at: usize,
    stored: &Stored,
    settled: Settled,
) -> Option<Plan> {
    let mut plans = Vec::new();
    let mut spine = alloc::vec![id];
    while let Some(held) = spine.pop() {
        if let Some(Node::Binary {
            op: BinaryOp::Or,
            left,
            right,
        }) = arena.node(held)
        {
            spine.push(right);
            spine.push(left);
            continue;
        }
        let terms = terms_of(arena, held, sql, sides, settled.sensitive);
        // A branch that names the rowid is read out of the table's own
        // tree, which is the `OP_SeekRowid` loop `sqlite3WhereBegin`
        // writes for `WHERE_IPK`.
        let plan = plan_of(&terms, at, stored, settled.format).or_else(|| ranged_of(&terms, at))?;
        plans.push(plan);
    }
    Some(Plan::Union(plans))
}

/// The walk of the table's own tree the terms hold to a range of its
/// rowids, and nothing where no term names the rowid.
///
/// Reading the terms costs O(n) in them.
fn ranged_of(terms: &[Bound], at: usize) -> Option<Plan> {
    let mut range = (None, None);
    for term in terms {
        let Value::Int(bound) = term.value else {
            continue;
        };
        if term.at != at || term.reached != Reached::Key {
            continue;
        }
        narrow(&mut range.0, &mut range.1, term.op, bound);
    }
    if range == (None, None) {
        return None;
    }
    Some(Plan::Rows(range.0, range.1))
}

/// What one plan holds a walk of an index to.
struct Seek {
    /// The page the index's tree begins at.
    root: u32,
    /// The values every entry the walk takes begins with.
    key: Vec<Value>,
    /// What each of them compares under, and after them what the column
    /// the bounds hold compares under.
    collations: Vec<Collation>,
    /// Where the rowid stands in an entry.
    rowid_at: usize,
    /// What the column after the key is held between.
    bounds: Bounds,
    /// Whether the index holds its entries backwards.
    backwards: bool,
}

/// What `plan` holds a walk of an index to.
///
/// A plan that names no index answers nothing, and so does one whose
/// key a row of the sides before it does not answer, which is what
/// makes the walk fall back to the whole tree.
///
/// Reading one key costs what the expression it is read from costs.
fn sought(plan: &Plan, cursor: &Cursor<'_>) -> Option<Seek> {
    match plan {
        Plan::Rows(_, _) | Plan::Union(_) => None,
        Plan::Keyed {
            root,
            key,
            collations,
            rowid_at,
            bounds,
            backwards,
        } => Some(Seek {
            root: *root,
            key: key.clone(),
            collations: collations.clone(),
            rowid_at: *rowid_at,
            bounds: bounds.clone(),
            backwards: *backwards,
        }),
        Plan::Joined {
            root,
            key,
            collation,
            affinity,
            rowid_at,
        } => {
            let reach = cursor.reach;
            let mut value = evaluate_row(reach.arena, *key, reach.sql, cursor).ok()?;
            // An index holds no entry a `=` against `NULL` reaches, and
            // it holds what the affinity of the column left of a value,
            // which is what the key is compared as.
            if value == Value::Null {
                return None;
            }
            crate::value::apply(&mut value, *affinity);
            Some(Seek {
                root: *root,
                key: alloc::vec![value],
                collations: alloc::vec![*collation],
                rowid_at: *rowid_at,
                bounds: Bounds::default(),
                backwards: false,
            })
        }
    }
}

/// The index of side `at` whose first column an `ON` compares against a
/// column of a side the walk reads before it, which is the key that
/// side answers per row.
///
/// The `ON` is read again for every row the descent answers, so a plan
/// that answers more rows than the key names is still the rows the
/// statement answers; a plan that answers fewer would not be.
///
/// Reading the terms costs O(t) in them and O(i) in the indexes.
fn joined(
    arena: &Arena,
    sql: &[u8],
    sides: &[Side<'_>],
    at: usize,
    stored: &Stored,
) -> Option<Plan> {
    let side = sides.get(at)?;
    // A `RIGHT` or a `FULL` join is walked twice, and the second walk
    // tells the rows it matched from the rows it did not by where they
    // stand in the walk, so both walks must answer the rows in one
    // order. A key read per row of another side answers another order,
    // so such a side is read out of its own tree.
    if matches!(side.kind, JoinKind::Right | JoinKind::Full) {
        return None;
    }
    let on = side.on?;
    for kept in &stored.indexes {
        // An index over an expression, and a partial index, answer
        // fewer entries than their places say.
        let found = kept.index.first_keyed().and_then(|(place, first)| {
            stored
                .table
                .columns
                .get(place)
                .map(|column| (place, first, column))
        });
        let Some((place, first, column)) = found else {
            continue;
        };
        // An index held in another order, or under another collation
        // than the column compares under, answers its entries in an
        // order the terms do not ask about.
        if first.order == crate::ast::Order::Descending || first.collation != column.collation {
            continue;
        }
        let Some(key) = keyed_by(arena, on, sql, sides, at, place) else {
            continue;
        };
        return Some(Plan::Joined {
            root: kept.root,
            key,
            collation: first.collation,
            affinity: column.affinity,
            rowid_at: kept.index.columns.len(),
        });
    }
    None
}

/// What the column `column` of side `at` is held equal to by the `AND`
/// spine of `on`, where a side the walk reads before `at` answers it.
fn keyed_by(
    arena: &Arena,
    on: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    at: usize,
    column: usize,
) -> Option<ExprId> {
    let mut spine = alloc::vec![on];
    while let Some(id) = spine.pop() {
        let Some(Node::Binary { op, left, right }) = arena.node(id) else {
            continue;
        };
        if op == BinaryOp::And {
            spine.push(left);
            spine.push(right);
            continue;
        }
        if op != BinaryOp::Eq {
            continue;
        }
        for (mine, theirs) in [(left, right), (right, left)] {
            if reached(arena, mine, sql, sides) != Some((at, Reached::Column(column))) {
                continue;
            }
            // Only a column of a side the walk has already read is a
            // key it can answer: a column of this side or of one after
            // it is not read yet.
            let Some((other, Reached::Column(_))) = reached(arena, theirs, sql, sides) else {
                continue;
            };
            if other < at {
                return Some(theirs);
            }
        }
    }
    None
}

/// The operator that says the same thing with its operands the other way
/// round.
const fn flipped(op: BinaryOp) -> BinaryOp {
    match op {
        BinaryOp::Lt => BinaryOp::Gt,
        BinaryOp::Le => BinaryOp::Ge,
        BinaryOp::Gt => BinaryOp::Lt,
        BinaryOp::Ge => BinaryOp::Le,
        other => other,
    }
}

/// Narrows a range by one term, which never widens it: two terms about
/// one rowid both hold.
fn narrow(range: &mut Option<i64>, past: &mut Option<i64>, op: BinaryOp, bound: i64) {
    let (first, last) = match op {
        BinaryOp::Eq => (Some(bound), Some(bound)),
        BinaryOp::Ge => (Some(bound), None),
        BinaryOp::Gt => (bound.checked_add(1), None),
        BinaryOp::Le => (None, Some(bound)),
        // Every operator but the five is left out of the terms.
        _ => (None, bound.checked_sub(1)),
    };
    if let Some(first) = first {
        *range = Some(range.map_or(first, |held| held.max(first)));
    }
    if let Some(last) = last {
        *past = Some(past.map_or(last, |held| held.min(last)));
    }
}

/// Whether a value is among the rows a statement answered.
///
/// The answer is three-valued: a `NULL` on either side is neither in
/// nor out, so an `IN` that matches nothing but read a `NULL` answers
/// `NULL` and a `NOT IN` over it answers `NULL` as well. This is
/// `sqlite3ExprCodeIN`, whose comparison converts under the affinity of
/// the two sides together and compares under the collation written on
/// the left, or the column's where the left writes none.
fn contained(
    arena: &Arena,
    value: ExprId,
    sql: &[u8],
    row: &dyn eval::Row,
    rows: &[Vec<Value>],
    columns: &[Column],
    negated: bool,
) -> Result<Value, Error> {
    let left = if eval::is_row_value(arena, value) {
        eval::evaluate_items(arena, value, sql, row)?
    } else {
        alloc::vec![evaluate_compared(arena, value, sql, row)?]
    };
    if left.len() != columns.len() {
        return Err(Error::Columns(columns.len(), left.len()));
    }
    let mut unknown = false;
    for held in rows {
        match same_values(&left, held, columns, row.collation()) {
            Some(true) => return Ok(Value::Int(i64::from(!negated))),
            Some(false) => {}
            None => unknown = true,
        }
    }
    if unknown {
        return Ok(Value::Null);
    }
    Ok(Value::Int(i64::from(negated)))
}

/// Whether one row a statement answered is the row looked for, which
/// is unknown where a null stands in a pair no other pair settles.
fn same_values(
    left: &[(Value, Affinity, Option<Collation>)],
    held: &[Value],
    columns: &[Column],
    default: Collation,
) -> Option<bool> {
    let mut unknown = false;
    for (at, ((mine, affinity, written), column)) in left.iter().zip(columns).enumerate() {
        let mut one = mine.clone();
        let mut other = held.get(at).cloned().unwrap_or(Value::Null);
        if one == Value::Null || other == Value::Null {
            unknown = true;
            continue;
        }
        apply_comparison(
            &mut one,
            &mut other,
            compare_affinity(*affinity, column.affinity),
        );
        let collation = written.or(column.collation).unwrap_or(default);
        if compare(&one, &other, collation) != core::cmp::Ordering::Equal {
            return Some(false);
        }
    }
    if unknown { None } else { Some(true) }
}

/// Whether the row the walk just read attaches to the ones above it:
/// the `ON` condition holds, and every column a `USING` or a `NATURAL`
/// names is equal on both sides.
fn attached(
    arena: &Arena,
    side: &Side<'_>,
    cursor: &Cursor<'_>,
    sql: &[u8],
) -> Result<bool, Error> {
    if let Some(on) = side.on
        && !evaluate_row(arena, on, sql, cursor)?.truth(false)
    {
        return Ok(false);
    }
    for name in &side.using {
        if !equal(cursor, name) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether every term of `terms` holds for the row the walk stands on.
///
/// Reading `n` terms costs what the `n` terms cost.
fn all_hold(
    arena: &Arena,
    terms: &[ExprId],
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<bool, Error> {
    for term in terms {
        if !evaluate_row(arena, *term, sql, cursor)?.truth(false) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the column `name` holds the same value on the side the walk
/// just read as on the side before it, which is the `a.x = b.x` a
/// `USING` stands for.
///
/// The side before it is the left of that comparison, so its collation
/// is the one the two are compared under.
fn equal(cursor: &Cursor<'_>, name: &[u8]) -> bool {
    // Both sides hold the column: a `USING` that names one they do not
    // both hold is refused before the walk begins.
    let mine = cursor
        .held
        .last()
        .and_then(|held| held.column(name, cursor.collation));
    // The leftmost side that holds the name is the one the column
    // belongs to, because a `USING` drops the copy the right side
    // carries and leaves the left side's: `t1 LEFT JOIN t2 USING(a)`
    // answers `t1.a` for `a`, whatever `t2` held. The value is the
    // first that is not nothing, which is the column a `RIGHT JOIN`
    // answers: the side on its left is empty for a row it kept, and
    // the name stands for the side that filled it.
    let before = cursor
        .held
        .get(..cursor.held.len().saturating_sub(1))
        .unwrap_or_default();
    let theirs = before
        .iter()
        .find_map(|held| held.column(name, cursor.collation))
        .map(|(value, affinity, collation)| {
            let filled = if value == Value::Null {
                before
                    .iter()
                    .filter_map(|held| held.column(name, cursor.collation))
                    .map(|(value, ..)| value)
                    .find(|value| *value != Value::Null)
                    .unwrap_or(Value::Null)
            } else {
                value
            };
            (filled, affinity, collation)
        });
    mine.zip(theirs).is_some_and(
        |((mut right, right_affinity, _), (mut left, left_affinity, collation))| {
            if left == Value::Null || right == Value::Null {
                // A `NULL` on either side matches nothing, which is
                // what `=` answers and what a join asks of it.
                return false;
            }
            let affinity = compare_affinity(left_affinity, right_affinity);
            apply_comparison(&mut left, &mut right, affinity);
            compare(&left, &right, collation) == core::cmp::Ordering::Equal
        },
    )
}

/// One row a feed answers: the rowid where the side keeps one, and the
/// values of the row.
type Read = Result<Fed, Error>;

/// The rowid a row keeps, where its side keeps one, and its values.
type Fed = (Option<i64>, Vec<Value>);

/// The rows of one side of a `FROM`.
enum Feed<'i, 'f> {
    /// A table of the schema, read out of its tree, which carries a
    /// frame per level and is boxed for it.
    Tree(Box<Tree<'i, 'f>>),
    /// The rows a statement answered, already read.
    Rows(core::slice::Iter<'f, Vec<Value>>),
    /// The rows an index names, read out of the index's tree and then
    /// out of the table's.
    Keyed(Box<Sought<'i, 'f>>),
    /// The rows of several walks, each row once, which is what an `OR`
    /// whose every branch names a key is read by.
    Union(Box<Union<'i, 'f>>),
}

/// Several walks of one table while they are read one after another.
struct Union<'i, 'f> {
    /// The walks, in the order the `OR` writes the branches.
    feeds: Vec<Feed<'i, 'f>>,
    /// Which of them the reader stands in.
    at: usize,
    /// The rowids already answered, which is what keeps a row two
    /// branches name from being answered twice. Reading one row costs
    /// O(n) in the rows already answered.
    seen: Vec<i64>,
}

impl Union<'_, '_> {
    /// The next row no earlier branch answered.
    fn read(&mut self) -> Option<Read> {
        loop {
            let feed = self.feeds.get_mut(self.at)?;
            let Some(read) = feed.next() else {
                self.at = self.at.saturating_add(1);
                continue;
            };
            let Ok((Some(rowid), values)) = read else {
                return Some(read);
            };
            if !self.seen.contains(&rowid) {
                self.seen.push(rowid);
                return Some(Ok((Some(rowid), values)));
            }
        }
    }
}

/// An index of a table while its tree is being walked.
struct Sought<'i, 'f> {
    /// The file the trees are in.
    image: Image<'i>,
    /// The table the index is over.
    stored: &'f Stored,
    /// Where the walk of the index stands.
    walk: crate::image::Entries<'i>,
    /// The values every entry the walk takes begins with, which a
    /// `ON` reads once per row of the sides before this one, so the
    /// walk holds them rather than borrowing them from the side.
    key: Vec<Value>,
    /// What each of them compares under, and after them what the column
    /// the bounds hold compares under.
    collations: Vec<Collation>,
    /// What the column after the key is held between, which ends the
    /// walk where an entry reaches past the end it runs towards.
    bounds: Bounds,
    /// Whether the index holds its entries backwards.
    backwards: bool,
    /// Where the rowid stands in an entry.
    rowid_at: usize,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
    /// The buffer one record is read into.
    payload: Vec<u8>,
}

impl<'i> Sought<'i, '_> {
    /// The next row the index names, or nothing once its entries no
    /// longer begin with the key.
    fn read(&mut self) -> Option<Read> {
        let entry = self.walk.next()?;
        match self.take(entry) {
            Ok(Some(row)) => Some(Ok(row)),
            // Every entry after this one sorts after it, so the entries
            // that begin with the key are read out.
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    /// Whether the entry's column after the key reaches past the high
    /// end of the bounds, which every entry after it reaches past as
    /// well, so the walk is read out there.
    fn is_past(&mut self, entry: &crate::page::Payload<'i>) -> Result<bool, Error> {
        let end = if self.backwards {
            &self.bounds.low
        } else {
            &self.bounds.high
        };
        let Some((wanted, inside)) = end.clone() else {
            return Ok(false);
        };
        let at = self.key.len();
        let held = value_of_entry(&self.image, entry, at, self.encoding, &mut self.payload)?;
        let collation = self
            .collations
            .get(at)
            .copied()
            .unwrap_or(Collation::Binary);
        let order = compare(&held, &wanted, collation);
        // The end the entries run towards stands where the end they
        // begin at stands for an index that holds them the other way.
        Ok(is_before(order, !self.backwards, inside))
    }

    /// One entry as the row it names, or nothing where the entry no
    /// longer begins with the key.
    fn take(
        &mut self,
        entry: Result<crate::page::Payload<'i>, crate::error::Error>,
    ) -> Result<Option<Fed>, Error> {
        let entry = entry?;
        let order = order_of_entry(
            &self.image,
            &entry,
            &self.key,
            &self.collations,
            self.encoding,
            &mut self.payload,
        )?;
        if order != core::cmp::Ordering::Equal {
            return Ok(None);
        }
        if self.is_past(&entry)? {
            return Ok(None);
        }
        let rowid = rowid_of(&self.image, &entry, self.rowid_at, &mut self.payload)?;
        // The row the entry names, which the table's tree is descended
        // to: O(log n) for each entry the index answers.
        let row = self
            .image
            .rows_between(self.stored.root, Some(rowid), Some(rowid))
            .next()
            // An entry naming a row the table does not hold is a file
            // whose index and table disagree.
            .ok_or(Error::Image(crate::error::Error::Page(self.stored.root)))??;
        read_payload(&self.image, &row.payload, &mut self.payload)?;
        let values = values_of(
            &self.payload,
            self.stored,
            Some(row.rowid),
            self.encoding,
            self.collation,
        )?;
        Ok(Some((Some(row.rowid), values)))
    }
}

/// A table of the schema while its tree is being walked.
struct Tree<'i, 'f> {
    /// The file the tree is in.
    image: Image<'i>,
    /// The table, for the places its columns stand at.
    stored: &'f Stored,
    /// Where the walk stands.
    walk: Walk<'i>,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
    /// The buffer one record is read into.
    payload: Vec<u8>,
}

impl<'i> Tree<'i, '_> {
    /// One step of the walk as the values of a row.
    fn read(
        &mut self,
        step: Result<(Option<i64>, crate::page::Payload<'i>), error::Error>,
    ) -> Result<(Option<i64>, Vec<Value>), Error> {
        let (rowid, holds) = step?;
        read_payload(&self.image, &holds, &mut self.payload)?;
        let values = values_of(
            &self.payload,
            self.stored,
            rowid,
            self.encoding,
            self.collation,
        )?;
        Ok((rowid, values))
    }
}

impl Iterator for Feed<'_, '_> {
    type Item = Read;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Feed::Tree(tree) => {
                let step = tree.walk.next()?;
                Some(tree.read(step))
            }
            Feed::Rows(rows) => rows.next().map(|values| Ok((None, values.clone()))),
            Feed::Keyed(sought) => sought.read(),
            Feed::Union(union) => union.read(),
        }
    }
}

/// What an index entry's first values compare as against a key.
///
/// An entry holds each value as the table's affinity left it, so the key
/// the planner builds has that affinity applied to it and nothing is
/// converted here.
fn order_of_entry(
    image: &Image<'_>,
    entry: &crate::page::Payload<'_>,
    key: &[Value],
    collations: &[Collation],
    encoding: Encoding,
    scratch: &mut Vec<u8>,
) -> Result<core::cmp::Ordering, Error> {
    // An entry that runs onto an overflow page is read whole before it
    // is compared, because the key may be the part that runs on.
    let held;
    let record = if entry.is_whole() {
        record::Record::parse(entry.local)?
    } else {
        read_payload(image, entry, scratch)?;
        held = scratch;
        record::Record::parse(held)?
    };
    for (at, wanted) in key.iter().enumerate() {
        let held = held_value(&record, at, encoding)?;
        let collation = collations.get(at).copied().unwrap_or(Collation::Binary);
        let order = compare(&held, wanted, collation);
        if order != core::cmp::Ordering::Equal {
            return Ok(order);
        }
    }
    Ok(core::cmp::Ordering::Equal)
}

/// Whether an entry standing at `order` against the end the entries of
/// the index begin at comes before that end.
///
/// `inside` says an entry holding the bound is one the walk takes, and
/// `backwards` says the entries run from the largest value down.
const fn is_before(order: core::cmp::Ordering, backwards: bool, inside: bool) -> bool {
    match (backwards, inside) {
        (false, true) => matches!(order, core::cmp::Ordering::Less),
        (false, false) => !matches!(order, core::cmp::Ordering::Greater),
        (true, true) => matches!(order, core::cmp::Ordering::Greater),
        (true, false) => !matches!(order, core::cmp::Ordering::Less),
    }
}

/// One value of an entry, as the engine holds values.
///
/// An entry that runs onto an overflow page is read whole before the
/// value is taken, because the value may be the part that runs on.
fn value_of_entry(
    image: &Image<'_>,
    entry: &crate::page::Payload<'_>,
    at: usize,
    encoding: Encoding,
    scratch: &mut Vec<u8>,
) -> Result<Value, Error> {
    let held;
    let record = if entry.is_whole() {
        record::Record::parse(entry.local)?
    } else {
        read_payload(image, entry, scratch)?;
        held = scratch;
        record::Record::parse(held)?
    };
    held_value(&record, at, encoding)
}

/// The rowid an entry ends with, which is the value after every column
/// the index holds.
fn rowid_of(
    image: &Image<'_>,
    entry: &crate::page::Payload<'_>,
    at: usize,
    scratch: &mut Vec<u8>,
) -> Result<i64, Error> {
    let held;
    let record = if entry.is_whole() {
        record::Record::parse(entry.local)?
    } else {
        read_payload(image, entry, scratch)?;
        held = scratch;
        record::Record::parse(held)?
    };
    match record.value(at)? {
        Some(record::Value::Int(rowid)) => Ok(rowid),
        // An index over a table that keeps its rows in the key's own
        // tree ends with that key and not with a rowid, and nothing
        // plans for one.
        _ => Err(Error::Image(crate::error::Error::Overrun)),
    }
}

/// One value of a record, as the engine holds values.
fn held_value(record: &record::Record<'_>, at: usize, encoding: Encoding) -> Result<Value, Error> {
    Ok(match record.value(at)? {
        None | Some(record::Value::Null) => Value::Null,
        Some(record::Value::Int(number)) => Value::Int(number),
        Some(record::Value::Real(number)) => Value::Real(number),
        Some(record::Value::Text(bytes)) => Value::Text(crate::value::decoded(bytes, encoding)),
        Some(record::Value::Blob(bytes)) => Value::Blob(bytes.to_vec()),
    })
}

/// A walk of a table's rows: down a table tree by rowid, or down the
/// key's own tree where the table keeps its rows there.
enum Walk<'a> {
    /// A table tree, which carries the rowid.
    Table(crate::image::Rows<'a>),
    /// An index tree, which carries none.
    Index(crate::image::Entries<'a>),
}

impl<'a> Iterator for Walk<'a> {
    type Item = Result<(Option<i64>, crate::page::Payload<'a>), error::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Walk::Table(rows) => rows
                .next()
                .map(|row| row.map(|row| (Some(row.rowid), row.payload))),
            Walk::Index(entries) => entries.next().map(|entry| entry.map(|held| (None, held))),
        }
    }
}

/// Where each column's value stands in the record.
///
/// A column computed and not stored takes no place. A table that keeps
/// its rows in the key's own tree puts the key's columns first, in the
/// order the key names them, and the rest after them in the order the
/// statement declares them, which is section 2.4 of the format.
pub(crate) fn places(table: &Table) -> Vec<usize> {
    let mut order: Vec<usize> = (0..table.columns.len())
        .filter(|at| {
            table
                .columns
                .get(*at)
                .is_some_and(|column| column.generated != Generated::Virtual)
        })
        .collect();
    if table.without_rowid {
        // A stable sort leaves the columns the key does not name in the
        // order they were declared in.
        order.sort_by_key(|at| {
            table
                .columns
                .get(*at)
                .map_or(u16::MAX, |column| match column.key {
                    0 => u16::MAX,
                    key => key,
                })
        });
    }
    let mut places = alloc::vec![usize::MAX; table.columns.len()];
    for (place, at) in order.iter().enumerate() {
        for slot in places.iter_mut().skip(*at).take(1) {
            *slot = place;
        }
    }
    places
}

/// Records that the row at `at_row` of the table at `at` was matched.
fn mark(kept: &mut [Vec<i64>], at: usize, at_row: i64) {
    for slot in kept.iter_mut().skip(at).take(1) {
        slot.push(at_row);
    }
}

/// One aggregate call of a statement.
struct Call {
    /// The node it was written as, which is what answers it: two
    /// `count(*)` in one statement are one column each.
    id: ExprId,
    /// Which aggregate.
    which: Aggregate,
    /// Whether `DISTINCT` precedes its arguments.
    distinct: bool,
    /// Its arguments.
    args: Range,
    /// The `FILTER`, which holds which rows it reads.
    filter: Option<ExprId>,
    /// The name it was written under, which names it in a refusal.
    name: Vec<u8>,
}

/// One group of rows while it is being accumulated.
struct Group<'a> {
    /// What the `GROUP BY` terms answered for it.
    key: Vec<(Value, Collation)>,
    /// The row its bare columns come from, where it has kept one.
    magnet: Option<Vec<Held<'a>>>,
    /// One accumulator per aggregate call.
    accumulators: Vec<Accumulator>,
}

impl<'a> Group<'a> {
    /// A group that has accumulated nothing.
    fn new(key: Vec<(Value, Collation)>, calls: &[Call]) -> Self {
        Group {
            key,
            magnet: None,
            accumulators: calls
                .iter()
                .map(|call| Accumulator::new(call.which, call.distinct))
                .collect(),
        }
    }

    /// Adds one row to every accumulator, and keeps the row where it is
    /// the one the group's bare columns come from.
    ///
    /// Which row that is: the first of the group, unless a `min` or a
    /// `max` is being accumulated, in which case it is the row the last
    /// of them took, so that `SELECT b, max(a)` answers the `b` beside
    /// the greatest `a`. This is `updateAccumulator` over the register
    /// `sqlite3SkipAccumulatorLoad` sets.
    fn step(
        &mut self,
        arena: &Arena,
        sql: &[u8],
        cursor: &Cursor<'a>,
        calls: &[Call],
    ) -> Result<(), Error> {
        let first = self.magnet.is_none();
        let mut magnet = None;
        for (call, accumulator) in calls.iter().zip(&mut self.accumulators) {
            // A `FILTER` decides which rows the aggregate is stepped
            // with, which is `sqlite3ExprIfFalse` jumping over the step.
            if !keep(arena, call.filter, sql, cursor)? {
                continue;
            }
            let mut values = Vec::new();
            let mut carried = Vec::new();
            let mut collation = cursor.collation;
            for (at, id) in arena.children(call.args).iter().enumerate() {
                // The collation of the first argument is the one the
                // aggregate compares under, which `min`, `max` and a
                // `DISTINCT` are what use.
                if at == 0 {
                    let (value, written) = evaluate_collated(arena, *id, sql, cursor)?;
                    collation = written;
                    values.push(value);
                    carried.push(crate::eval::evaluate_carried(arena, *id, sql, cursor)?.1);
                } else {
                    let (value, json) = crate::eval::evaluate_carried(arena, *id, sql, cursor)?;
                    values.push(value);
                    carried.push(json);
                }
            }
            accumulator.step(&values, &carried, collation)?;
            if accumulator.magnet() {
                magnet = Some(accumulator.kept());
            }
        }
        if magnet.unwrap_or(first) {
            self.magnet = Some(cursor.held.clone());
        }
        Ok(())
    }
}

/// What a `GROUP BY` groups by.
#[derive(Clone, Copy)]
enum Term {
    /// An expression over the row.
    Expr(ExprId),
    /// A column of one of the tables, which is what a place counting to
    /// a `*` lands on.
    Held(usize, usize),
}

/// What the statement answers at each place.
///
/// A `*` stands for the columns of its tables, so a place counts past
/// one `*` to reach what follows it.
fn answered(arena: &Arena, select: &Select, sql: &[u8], sides: &[Side<'_>]) -> Vec<Term> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            ResultColumn::Star => {
                for (at, side) in sides.iter().enumerate() {
                    for (place, column) in side.shape.columns.iter().enumerate() {
                        // A `GROUP BY` counts the columns a `*`
                        // answers.
                        if !left_out(column, &side.using) {
                            out.push(Term::Held(at, place));
                        }
                    }
                }
            }
            ResultColumn::TableStar(span) => {
                let called = dequote(span.text(sql));
                for (at, side) in sides
                    .iter()
                    .enumerate()
                    .filter(|(_, side)| side.named(&called))
                {
                    for place in 0..side.shape.columns.len() {
                        out.push(Term::Held(at, place));
                    }
                }
            }
            ResultColumn::Expr { expr, .. } => out.push(Term::Expr(expr)),
        }
    }
    out
}

/// The terms a `GROUP BY` groups by.
///
/// A whole number counts the answered columns from one, and a name the
/// table does not hold is looked for among the names the statement
/// answers under. This is `sqlite3ResolveOrderGroupBy` over
/// `resolveAlias`, which is why `GROUP BY 2-1` groups by the number one
/// and `GROUP BY 1` groups by the first column answered.
fn grouping(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
) -> Result<Vec<Term>, Error> {
    let results = arena.results(select.columns);
    let answers = answered(arena, select, sql, sides);
    let mut out = Vec::new();
    for (which, term) in arena.children(select.group).iter().enumerate() {
        if let Some(place) = whole_number(arena, *term, sql) {
            let refused =
                || Error::OrderRange(which.saturating_add(1), b"GROUP".to_vec(), answers.len());
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| refused())?;
            let found = answers.get(at).ok_or_else(refused)?;
            out.push(*found);
            continue;
        }
        // A name is a column of the table where the table has one, and
        // only where it has none is it the name something is answered
        // under.
        let aliased = column_named(arena, *term)
            .map(|name| dequote(name.text(sql)))
            .filter(|name| !holds(sides, name))
            .and_then(|name| aliased(results, sql, &name));
        out.push(Term::Expr(aliased.unwrap_or(*term)));
    }
    Ok(out)
}

/// What one `GROUP BY` term answers for a row, with the collation it
/// compares under.
fn grouped(
    term: Term,
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<(Value, Collation), Error> {
    match term {
        Term::Expr(expr) => Ok(evaluate_collated(arena, expr, sql, cursor)?),
        Term::Held(side, place) => {
            let held = cursor.held.get(side).ok_or(Error::NoTable(Vec::new()))?;
            let value = held.values.get(place).cloned().unwrap_or(Value::Null);
            let collation = held
                .shape
                .columns
                .get(place)
                .and_then(|column| column.collation)
                .unwrap_or(cursor.collation);
            Ok((value, collation))
        }
    }
}

/// Whether any of the sides answers a column of this name.
/// Refuses an `ON` of an outer join that names a table read after it.
///
/// `sqlite3ProcessJoin` marks the terms of an `ON` with the side the
/// join is on, and the walk of `select.c` refuses a column of a side
/// read later, because such a term would be read after the rows it
/// names were left behind. An inner join carries no such mark, its `ON`
/// being read as a `WHERE`. Costs O(n) over the nodes of each `ON`.
///
/// # Errors
///
/// [`Error::Rightward`] names what the `ON` belongs to.
fn rightward(arena: &Arena, sql: &[u8], sides: &[Side<'_>]) -> Result<(), Error> {
    for (at, side) in sides.iter().enumerate() {
        let Some(on) = side.on else { continue };
        if matches!(
            side.kind,
            JoinKind::Inner | JoinKind::Cross | JoinKind::None
        ) {
            continue;
        }
        leftward(arena, on, sql, (sides, at))?;
    }
    Ok(())
}

/// Every column of one `ON`, held to the sides read up to `at`.
///
/// # Errors
///
/// [`Error::Rightward`] where one of them names a side read later.
fn leftward(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    over: (&[Side<'_>], usize),
) -> Result<(), Error> {
    arena.node(id).map_or(Ok(()), |node| {
        named_leftward(node, sql, over)?;
        let mut deeper = Ok(());
        arena.under(node, |child| {
            if deeper.is_ok() {
                deeper = leftward(arena, child, sql, over);
            }
        });
        deeper
    })
}

/// The refusal where one node names a side read after `at`.
///
/// A name with a schema in front of it names no side a join reads, so
/// the walk passes over it.
///
/// # Errors
///
/// [`Error::Rightward`] where the node names such a side.
fn named_leftward(node: Node, sql: &[u8], over: (&[Side<'_>], usize)) -> Result<(), Error> {
    let (sides, at) = over;
    let Node::Column {
        schema: None,
        table,
        column,
    } = node
    else {
        return Ok(());
    };
    let named = dequote(column.text(sql));
    let place = match table {
        Some(span) => {
            let held = dequote(span.text(sql));
            sides
                .iter()
                .position(|one| one.name.eq_ignore_ascii_case(&held))
        }
        None => sides.iter().position(|one| one.shape.has(&named)),
    };
    if place.is_some_and(|held| held > at) {
        return Err(Error::Rightward);
    }
    Ok(())
}

fn holds(sides: &[Side<'_>], name: &[u8]) -> bool {
    sides.iter().any(|side| side.shape.has(name))
}

/// The expression a statement answers under `name`, where it answers
/// one under that name.
fn aliased(results: &[ResultColumn], sql: &[u8], name: &[u8]) -> Option<ExprId> {
    results.iter().find_map(|column| match *column {
        ResultColumn::Expr {
            expr,
            alias: Some(alias),
            ..
        } if dequote(alias.text(sql)).eq_ignore_ascii_case(name) => Some(expr),
        _ => None,
    })
}

/// How many columns a statement answers, worked out from its result
/// columns alone, and nothing where a `*` stands among them: that one
/// is as wide as the tables it is over and the walk here reads no
/// table.
///
/// A compound is as wide as its first core, which is the one
/// `sqlite3SubselectError` counts.
fn width_of(arena: &Arena, id: SelectId) -> Option<usize> {
    let select = arena.select(id)?;
    if !select.values.is_empty() {
        let first = arena.children(select.values).first().copied()?;
        // The parser builds each row of a `VALUES` as a row node, so
        // what the node names is what the row holds.
        let mut items: usize = 0;
        arena.node(first).into_iter().for_each(|node| {
            arena.under(node, |_| items = items.saturating_add(1));
        });
        return Some(items);
    }
    let mut width: usize = 0;
    for result in arena.results(select.columns) {
        match *result {
            ResultColumn::Expr { .. } => width = width.saturating_add(1),
            ResultColumn::Star | ResultColumn::TableStar(_) => return None,
        }
    }
    Some(width)
}

/// Every statement written inside an expression of this one, held to
/// the number of columns the place it stands in takes, which is
/// `sqlite3SubselectError` counting them where the statement is read
/// and not where a row is.
///
/// The walk reads every expression of the statement once, so it is
/// O(nodes) per statement.
fn subqueries(arena: &Arena, select: &Select) -> Result<(), Error> {
    let mut roots: Vec<ExprId> = Vec::new();
    for column in arena.results(select.columns) {
        if let ResultColumn::Expr { expr, .. } = *column {
            roots.push(expr);
        }
    }
    roots.extend(select.filter);
    roots.extend(select.having);
    roots.extend(arena.children(select.group).iter().copied());
    for term in arena.orders(select.order) {
        roots.push(term.expr);
    }
    for source in arena.sources(select.from) {
        roots.extend(source.on);
    }
    for root in roots {
        counted_columns(arena, root)?;
    }
    Ok(())
}

/// The same for one expression and everything under it.
fn counted_columns(arena: &Arena, id: ExprId) -> Result<(), Error> {
    arena
        .node(id)
        .map_or(Ok(()), |node| counted_node(arena, node))
}

/// The same for one node the arena holds.
fn counted_node(arena: &Arena, node: Node) -> Result<(), Error> {
    // Only an `IN` says on its own how many columns the statement it
    // looks in answers; a statement written as a value takes its width
    // from where it stands, which the walk over one node does not see.
    let wanted = match node {
        Node::InSelect { value, select, .. } => match arena.node(value) {
            Some(Node::Row(items)) => Some((select, items.len())),
            _ => Some((select, 1)),
        },
        _ => None,
    };
    if let Some((inner, wanted)) = wanted
        && let Some(answered) = width_of(arena, inner)
        && answered != wanted
    {
        return Err(Error::Columns(answered, wanted));
    }
    let mut deeper = Ok(());
    arena.under(node, |child| {
        if deeper.is_ok() {
            deeper = counted_columns(arena, child);
        }
    });
    deeper
}

/// Every aggregate call a statement answers with, and a refusal where
/// one stands somewhere no group has been made yet.
fn aggregates(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
    grouped: &'static [crate::func::Grouped],
) -> Result<Vec<Call>, Error> {
    let walk = Gathering {
        arena,
        sql,
        results: arena.results(select.columns),
        sides,
        grouped,
    };
    let mut calls = Vec::new();
    for column in walk.results {
        if let ResultColumn::Expr { expr, .. } = *column {
            walk.gather(expr, &mut calls, false)?;
        }
    }
    // What makes a statement an aggregate one is an aggregate among the
    // columns it answers, or a `GROUP BY`. Nothing else does, which is
    // why a `HAVING` without either is a refusal and not a filter over
    // one group.
    let grouped = !calls.is_empty() || !select.group.is_empty();
    if select.having.is_some() && !grouped {
        return Err(Error::Having);
    }
    if let Some(having) = select.having {
        walk.gather(having, &mut calls, false)?;
    }
    for term in arena.orders(select.order) {
        walk.gather(term.expr, &mut calls, false)?;
    }
    let mut misused = Vec::new();
    if let Some(id) = select.filter {
        walk.gather(id, &mut misused, false)?;
    }
    if let Some(call) = misused.first() {
        return Err(Error::MisusedAggregate(call.name.clone()));
    }
    for id in arena.children(select.group) {
        walk.gather(*id, &mut misused, false)?;
    }
    if !misused.is_empty() {
        return Err(Error::GroupedAggregate);
    }
    // An aggregate reached only by the `ORDER BY` of a statement that
    // groups nothing has no group to answer over, which
    // `sqlite3ExprCodeTarget` refuses when it finds no `AggInfo`.
    let loose = calls
        .first()
        .filter(|_| !grouped)
        .map(|call| call.name.clone());
    loose.map_or(Ok(calls), |name| Err(Error::LooseAggregate(name)))
}

/// What the walk for the aggregate calls of one statement reads.
struct Gathering<'a> {
    /// The tree the statement was parsed into.
    arena: &'a Arena,
    /// The text that tree points into.
    sql: &'a [u8],
    /// The columns the statement answers, whose aliases a name written
    /// inside an aggregate may stand for.
    results: &'a [ResultColumn],
    /// The sides of the `FROM`, which hold the names that are columns.
    sides: &'a [Side<'a>],
    /// The aggregates the application defined on the connection.
    grouped: &'static [crate::func::Grouped],
}

impl Gathering<'_> {
    /// Collects the aggregate calls of one expression.
    ///
    /// The walk is as deep as the tree is tall, which the parser has
    /// already bounded, so nothing here counts the steps. `inside` is
    /// set under an aggregate, where another one is a misuse rather
    /// than a call.
    fn gather(&self, id: ExprId, out: &mut Vec<Call>, inside: bool) -> Result<(), Error> {
        self.arena
            .node(id)
            .map_or(Ok(()), |node| self.collect(id, node, out, inside))
    }

    /// The same for one node the arena holds.
    fn collect(
        &self,
        id: ExprId,
        node: Node,
        out: &mut Vec<Call>,
        inside: bool,
    ) -> Result<(), Error> {
        let mut under = inside;
        if inside {
            self.aliasing(node)?;
        }
        if let Node::Call {
            name,
            args,
            distinct,
            star,
            filter,
        } = node
        {
            let count = if star {
                0
            } else {
                self.arena.children(args).len()
            };
            let called = dequote(name.text(self.sql));
            if let Some(which) = agg::lookup_in(self.grouped, &called, count) {
                if inside {
                    return Err(Error::MisusedAggregate(called));
                }
                // `DISTINCT` puts the rows through one column, so there
                // has to be exactly one for them to go through.
                if distinct && count != 1 {
                    return Err(Error::DistinctAggregate);
                }
                out.push(Call {
                    id,
                    which,
                    distinct,
                    args,
                    filter,
                    name: called,
                });
                under = true;
            }
        }
        let mut deeper = Ok(());
        self.arena.under(node, |child| {
            if deeper.is_ok() {
                deeper = self.gather(child, out, under);
            }
        });
        deeper
    }

    /// A refusal where the node is a bare name that no side holds and
    /// the statement answers under that name with an aggregate, which
    /// `lookupName` refuses once the name resolves to an alias marked
    /// `EP_Agg`.
    fn aliasing(&self, node: Node) -> Result<(), Error> {
        let Node::Column {
            schema: None,
            table: None,
            column,
        } = node
        else {
            return Ok(());
        };
        let name = dequote(column.text(self.sql));
        if holds(self.sides, &name) {
            return Ok(());
        }
        let aggregate = aliased(self.results, self.sql, &name)
            .is_some_and(|expr| aggregating(self.arena, expr, self.sql, self.grouped));
        if aggregate {
            return Err(Error::MisusedAlias(name));
        }
        Ok(())
    }
}

/// Whether an expression answers with an aggregate, which is `EP_Agg`.
fn aggregating(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    grouped: &'static [crate::func::Grouped],
) -> bool {
    arena.node(id).is_some_and(|node| {
        let mut found = match node {
            Node::Call { name, .. } => agg::named_in(grouped, &dequote(name.text(sql))),
            _ => false,
        };
        arena.under(node, |child| {
            found = found || aggregating(arena, child, sql, grouped);
        });
        found
    })
}

/// The columns a statement answers: their names, and what a
/// comparison against each of them does.
fn shape(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
    naming: Naming,
    collating: &[crate::value::Collating],
) -> Result<Shape, Error> {
    // `selectExpander` names a column a `*` stands for with the side it
    // came from in front of it where `full_column_names` is on and
    // `short_column_names` off, which is the one place the name of the
    // side and not of the table is read.
    let long = naming.full && !naming.short;
    let mut columns = Vec::new();
    for result in arena.results(select.columns) {
        match *result {
            ResultColumn::Star => {
                // `selectExpander` refuses a `*` written where the
                // statement reads no table.
                if sides.is_empty() {
                    return Err(Error::NoTables);
                }
                for (at, side) in sides.iter().enumerate() {
                    // A `*` stands for every column named by its side,
                    // so two sides of one name make every column of
                    // them a name two sides answer.
                    if sides.iter().take(at).any(|before| before.named(&side.name)) {
                        return Err(Error::Ambiguous);
                    }
                    // The columns a `USING` or a `NATURAL` matched are
                    // answered once and not twice, which is the side
                    // that hides them leaving them out.
                    for column in &side.shape.columns {
                        // A statement that stands for the tables
                        // inside brackets answers the column a `USING`
                        // matched as well, under the name of the table
                        // it came from, and says that a `*` leaves it
                        // out.
                        let hidden = left_out(column, &side.using);
                        if hidden && !select.nested {
                            continue;
                        }
                        let mut answered = side.answered_as(column);
                        answered.hidden = hidden;
                        answered.shown = side.shown_as(column, long);
                        columns.push(answered);
                    }
                }
            }
            ResultColumn::TableStar(span) => {
                let called = dequote(span.text(sql));
                let mut named = sides.iter().filter(|side| side.named(&called));
                let side = named.next().ok_or_else(|| Error::NoTable(called.clone()))?;
                if named.next().is_some() {
                    return Err(Error::Ambiguous);
                }
                columns.extend(side.shape.columns.iter().map(|column| {
                    let mut answered = side.answered_as(column);
                    answered.shown = side.shown_as(column, long);
                    answered
                }));
            }
            ResultColumn::Expr { expr, alias, text } => {
                let written = column_parts(arena, expr);
                let name = match alias {
                    Some(span) => dequote(span.text(sql)),
                    // With no name written, a column answers under its
                    // own name and everything else under the text it
                    // was written as.
                    None => match written {
                        Some((_, column)) => answered_name(&dequote(column.text(sql)), sides),
                        None => text.text(sql).to_vec(),
                    },
                };
                let shown = shown_name(
                    sql,
                    sides,
                    ShownAs {
                        naming,
                        alias: alias.is_some(),
                        name: &name,
                        text,
                        written,
                    },
                );
                let (affinity, collation) = compared(arena, expr, sql, sides, collating);
                columns.push(Column {
                    name,
                    shown,
                    from: Vec::new(),
                    hidden: false,
                    affinity,
                    collation,
                    datatype: data_type(arena, expr, sql, sides, collating),
                    declared: declared_of(arena, expr, sql, sides),
                });
            }
        }
    }
    Ok(Shape {
        columns,
        nested: select.nested,
        keyed: false,
        key: None,
    })
}

/// The type the schema declares for the column one result column names,
/// and nothing where the result column is not one column of a side.
///
/// A bare `rowid` of a side that keeps its rows under a key answers the
/// type of a whole number, which is what `columnTypeImpl` of
/// `research/sqlite/src/select.c` writes for it.
fn declared_of(arena: &Arena, expr: ExprId, sql: &[u8], sides: &[Side<'_>]) -> Vec<u8> {
    let Some(Node::Column { table, column, .. }) = arena.node(expr) else {
        return Vec::new();
    };
    let name = dequote(column.text(sql));
    let named = table.map(|table| dequote(table.text(sql)));
    for side in sides {
        if named.as_ref().is_some_and(|named| !side.named(named)) {
            continue;
        }
        if let Some(at) = side.shape.place(&name) {
            return side
                .shape
                .columns
                .get(at)
                .map_or_else(Vec::new, |column| column.declared.clone());
        }
        if side.shape.keyed && rowid_named(&name) {
            return b"INTEGER".to_vec();
        }
    }
    Vec::new()
}

/// The values one row answers.
fn project(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            ResultColumn::Star => {
                for (at, held) in cursor.held.iter().enumerate() {
                    held.each(|column, value| {
                        if left_out(column, held.using) && !select.nested {
                            return;
                        }
                        // A statement standing for the tables inside
                        // brackets answers each column as its own
                        // table holds it, and the name a `USING`
                        // matched is filled where a bare name reaches
                        // it.
                        if select.nested {
                            out.push(value.clone());
                            return;
                        }
                        // A bare name over the tables inside brackets
                        // reaches the column one of them filled, and
                        // then the sides after it fill what is left.
                        let mine = held
                            .filled(&column.name, cursor.collation)
                            .map_or_else(|| value.clone(), |(value, ..)| value);
                        out.push(cursor.coalesced(at, &column.name, &mine));
                    });
                }
            }
            ResultColumn::TableStar(span) => {
                let named = dequote(span.text(sql));
                for held in cursor.held.iter().filter(|held| held.named(&named)) {
                    held.each(|_, value| out.push(value.clone()));
                }
            }
            ResultColumn::Expr { expr, .. } => {
                out.push(evaluate_row(arena, expr, sql, cursor)?);
            }
        }
    }
    Ok(out)
}

/// One row: what it answers, and what it sorts by.
fn sorted(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cursor: &Cursor<'_>,
    keys: &[Key],
) -> Result<Sorted, Error> {
    let values = project(arena, select, sql, cursor)?;
    let mut sort = Vec::new();
    for key in keys {
        sort.push(match &key.of {
            // The collation of a column the answer holds is the one
            // the column was declared with, which is what
            // `sqlite3ExprCollSeq` reads off the expression the answer
            // came from.
            Keyed::Place(at, collation) => {
                (values.get(*at).cloned().unwrap_or(Value::Null), *collation)
            }
            Keyed::Expr(expr) => evaluate_collated(arena, *expr, sql, cursor)?,
        });
    }
    Ok(Sorted { values, keys: sort })
}

/// What each `ORDER BY` term sorts by: a place in the answer, or an
/// expression over the row.
fn keys(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    names: &[Vec<u8>],
    collations: &[Collation],
    collating: &[crate::value::Collating],
) -> Result<Vec<Key>, Error> {
    let mut keys = Vec::new();
    for (which, term) in arena.orders(select.order).iter().enumerate() {
        let descending = term.order == Order::Descending;
        let nulls = term.nulls;
        // A whole number counts the answered columns from one; a
        // name that is one of them names it; anything else is read
        // against the row.
        if let Some(place) = whole_number(arena, term.expr, sql) {
            let refused =
                || Error::OrderRange(which.saturating_add(1), b"ORDER".to_vec(), names.len());
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| refused())?;
            if at >= names.len() {
                return Err(refused());
            }
            // A `COLLATE` on the number is the collation the sort uses,
            // whatever the column it counts to was declared with.
            let written = term_collation(arena, term.expr, sql, collating)?;
            let collation = written.unwrap_or_else(|| collation_at(collations, at));
            keys.push(Key {
                of: Keyed::Place(at, collation),
                descending,
                nulls,
            });
            continue;
        }
        // `resolveOrderGroupBy` matches the term against the answered
        // names with any `COLLATE` on it taken off first, so
        // `ORDER BY m COLLATE binary` sorts by the column answered
        // under the name `m` and not by the column of that name.
        if let Some(name) = column_named(arena, uncollated(arena, term.expr)) {
            let name = &dequote(name.text(sql));
            if let Some(at) = names
                .iter()
                .position(|answered| answered.eq_ignore_ascii_case(name))
            {
                // A `COLLATE` on the term is the collation the sort
                // uses, whatever the column was declared with.
                let written = term_collation(arena, term.expr, sql, collating)?;
                let collation = written.unwrap_or_else(|| collation_at(collations, at));
                keys.push(Key {
                    of: Keyed::Place(at, collation),
                    descending,
                    nulls,
                });
                continue;
            }
        }
        keys.push(Key {
            of: Keyed::Expr(term.expr),
            descending,
            nulls,
        });
    }
    Ok(keys)
}

/// Which row of the queue of a recursive term is taken next, which is
/// the smallest of the rows waiting in it under `order`.
///
/// Reading the queue costs O(n) in the rows waiting in it.
fn waiting(rows: &[Vec<Value>], taken: &[bool], order: &[Ordered]) -> Option<usize> {
    let mut best: Option<usize> = None;
    for (at, row) in rows.iter().enumerate() {
        if taken.get(at).copied().unwrap_or(false) {
            continue;
        }
        let smaller = best
            .and_then(|held| rows.get(held))
            .is_none_or(|held| order_of_rows(row, held, order) == core::cmp::Ordering::Less);
        if smaller {
            best = Some(at);
        }
    }
    best
}

/// How many rows a `LIMIT` on a recursive term passes over and how many
/// it takes, which is nothing where it takes them all.
fn bounds(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    row: &dyn eval::Row,
) -> Result<(usize, Option<usize>), Error> {
    let Some(limit) = select.limit else {
        return Ok((0, None));
    };
    let count = evaluate_row(arena, limit.count, sql, row)?.to_integer();
    let skip = match limit.offset {
        None => 0,
        Some(offset) => evaluate_row(arena, offset, sql, row)?.to_integer(),
    };
    let most = (count >= 0).then(|| usize::try_from(count).unwrap_or(0));
    Ok((usize::try_from(skip).unwrap_or(0), most))
}

/// Drops the rows a `LIMIT` leaves out.
fn limit(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    rows: &mut Vec<Vec<Value>>,
    row: &dyn eval::Row,
) -> Result<(), Error> {
    let Some(limit) = select.limit else {
        return Ok(());
    };
    let count = evaluate_row(arena, limit.count, sql, row)?.to_integer();
    let skip = match limit.offset {
        None => 0,
        Some(offset) => evaluate_row(arena, offset, sql, row)?.to_integer(),
    };
    let skip = usize::try_from(skip).unwrap_or(0);
    rows.drain(..skip.min(rows.len()));
    if count < 0 {
        return Ok(());
    }
    rows.truncate(usize::try_from(count).unwrap_or(0));
    Ok(())
}

/// Reads a payload into `into`, refusing one longer than the whole file
/// before any room is made for it.
///
/// A payload's length is a number the file gives, and a file may give
/// one no machine holds.
fn read_payload(
    image: &Image<'_>,
    payload: &crate::page::Payload<'_>,
    into: &mut Vec<u8>,
) -> Result<(), Error> {
    if payload.total > image.size() {
        return Err(Error::Image(error::Error::Overrun));
    }
    into.clear();
    into.resize(payload.total, 0);
    image.read_payload(payload, into)?;
    Ok(())
}

/// Whether two rows hold the same values, which is what `DISTINCT` and
/// the set operators ask.
fn same(left: &[Value], right: &[Value], collations: &[Collation]) -> bool {
    // Two rows of one answer always hold the same number of values, and
    // there is one collation per column.
    left.iter()
        .zip(right)
        .zip(collations)
        .all(|((first, second), collation)| {
            compare(first, second, *collation) == core::cmp::Ordering::Equal
        })
}

/// Drops the rows another row already holds.
fn distinct(rows: &mut Vec<Vec<Value>>, collations: &[Collation]) {
    let mut seen: Vec<Vec<Value>> = Vec::new();
    rows.retain(|row| {
        let fresh = !seen.iter().any(|kept| same(kept, row, collations));
        if fresh {
            seen.push(row.clone());
        }
        fresh
    });
}

/// Puts two answers together.
///
/// `UNION ALL` is the rows of one after the rows of the other, in the
/// order they were answered. The other three go through the merge
/// `multiSelectOrderBy` compiles: each side sorted by every column and
/// holding each row once, then walked together. Which of two rows that
/// compare equal but are not the same bytes comes out is decided there
/// and not by chance — the first of them inside one side, and the right
/// side's where a `UNION` finds one on both.
fn combine(operator: Compound, left: &mut Answer, right: Answer, collations: &[Collation]) {
    if operator == Compound::UnionAll {
        left.rows.extend(right.rows);
        return;
    }
    ordered(&mut left.rows, collations);
    let mut theirs = right.rows;
    ordered(&mut theirs, collations);
    let wanted = matches!(operator, Compound::Intersect);
    left.rows.retain(|row| {
        let shared = theirs.iter().any(|other| same(other, row, collations));
        shared == wanted
    });
    if operator == Compound::Union {
        left.rows.extend(theirs);
        arrange(&mut left.rows, collations);
    }
}

/// One core of a chain, with the operator that stands between it and
/// the core after it.
#[derive(Clone, Copy)]
struct Link {
    /// Where the core is.
    id: SelectId,
    /// The core.
    core: Select,
    /// The operator after it, where a core follows.
    operator: Option<Compound>,
}

/// The cores of a statement, each with the operator after it.
fn chain(arena: &Arena, id: SelectId) -> Vec<Link> {
    let mut links = Vec::new();
    let mut at = Some(id);
    while let Some((id, core)) = at.and_then(|id| arena.select(id).map(|core| (id, core))) {
        links.push(Link {
            id,
            core,
            operator: core.compound.map(|(operator, _)| operator),
        });
        at = core.compound.map(|(_, next)| next);
    }
    links
}

/// Whether `wanted` names a `WITH` term of this statement.
///
/// A term already answered is one the statement finds, so a refusal
/// that names a term names one with no answer yet. Costs O(n) over the
/// terms.
fn holds_term(arena: &Arena, select: &Select, sql: &[u8], wanted: &[u8]) -> bool {
    arena
        .ctes(select.ctes)
        .iter()
        .any(|cte| dequote(cte.name.text(sql)).eq_ignore_ascii_case(wanted))
}

/// The term a circle is named after, which is the one the statement
/// reads, and nothing where the statement reads none of them.
///
/// `sqlite3WithPush` names the term the resolver reached the circle
/// through, and a term no statement reads is never resolved at all.
fn circling(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    later: &[(Vec<u8>, &crate::ast::Cte)],
) -> Option<Vec<u8>> {
    later
        .iter()
        .find(|(name, _)| reads(arena, select, sql, name))
        .map(|(name, _)| name.clone())
}

/// The operator that stands in front of the core at `at`.
fn before(links: &[Link], at: usize) -> Option<Compound> {
    at.checked_sub(1)
        .and_then(|at| links.get(at))
        .and_then(|link| link.operator)
}

/// Whether a core reads a table of this name in its `FROM`.
///
/// A name inside a statement of the `FROM` is not read, which is what
/// `sqlite3WithPush` refuses as a recursive reference in a subquery.
fn reads(arena: &Arena, core: &Select, sql: &[u8], name: &[u8]) -> bool {
    arena.sources(core.from).iter().any(|source| {
        matches!(
            source.kind,
            SourceKind::Table { schema: None, name: written, .. }
                if dequote(written.text(sql)).eq_ignore_ascii_case(name)
        )
    })
}

/// Puts the column names a `WITH` term writes over the ones its
/// statement answers.
fn renamed(
    arena: &Arena,
    columns: Range,
    (sql, name): (&[u8], &[u8]),
    answered: &mut Answered,
) -> Result<(), Error> {
    let written = arena.names(columns);
    if written.is_empty() {
        return Ok(());
    }
    if written.len() != answered.answer.names.len() {
        return Err(Error::Names(
            name.to_vec(),
            answered.answer.names.len(),
            written.len(),
        ));
    }
    for (column, span) in answered.shape.columns.iter_mut().zip(written) {
        column.name = dequote(span.text(sql));
    }
    for (name, span) in answered.answer.names.iter_mut().zip(written) {
        *name = dequote(span.text(sql));
    }
    Ok(())
}

/// Puts one row behind the rows a recursive term has answered, unless
/// `once` says a row those already hold stands for it.
fn add(rows: &mut Vec<Vec<Value>>, row: Vec<Value>, once: bool, collations: &[Collation]) {
    if once && rows.iter().any(|held| same(held, &row, collations)) {
        return;
    }
    rows.push(row);
}

/// Sorts rows by every column and drops the ones another row already
/// holds, which is what each side of a set operator goes through.
fn ordered(rows: &mut Vec<Vec<Value>>, collations: &[Collation]) {
    arrange(rows, collations);
    rows.dedup_by(|left, right| same(left, right, collations));
}

/// Sorts rows by every column, which is the order a set operator
/// answers in.
fn arrange(rows: &mut [Vec<Value>], collations: &[Collation]) {
    let every: Vec<Ordered> = collations
        .iter()
        .enumerate()
        .map(|(at, collation)| Ordered {
            at,
            descending: false,
            collation: *collation,
            nulls: Nulls::Unspecified,
        })
        .collect();
    sort_by_keys(rows, &every);
}

/// One term of the `ORDER BY` of a compound: which column of the
/// answer it counts to, which way it runs, and what it compares under.
#[derive(Clone, Copy)]
struct Ordered {
    /// Which column of the answer.
    at: usize,
    /// Whether it runs backwards.
    descending: bool,
    /// What the values compare under, which is what the term was
    /// written with where it was written with a `COLLATE`.
    collation: Collation,
    /// Where its nulls go.
    nulls: Nulls,
}

/// Sorts the rows of an answer under terms that each count to a column
/// of it, backwards where the term says so.
fn sort_by_keys(rows: &mut [Vec<Value>], terms: &[Ordered]) {
    rows.sort_by(|left, right| order_of_rows(left, right, terms));
}

/// Where one row stands against another under terms that each count to
/// a column of the answer.
fn order_of_rows(left: &[Value], right: &[Value], terms: &[Ordered]) -> core::cmp::Ordering {
    for term in terms {
        let first = left.get(term.at).unwrap_or(&Value::Null);
        let second = right.get(term.at).unwrap_or(&Value::Null);
        let order = order_under(first, second, term.collation, term.descending, term.nulls);
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    core::cmp::Ordering::Equal
}

/// What two cores of a compound that answer different numbers of
/// columns are refused with, which `sqlite3SelectWrongNumTermsError`
/// names by the word that joins them, or by the rows themselves where
/// the cores are `VALUES`.
fn unjoined(joined: Option<Compound>, values: bool) -> Error {
    if values {
        return Error::Values;
    }
    Error::Compound(compound_named(joined.unwrap_or(Compound::Union)))
}

/// The word one compound operator is written as, which is
/// `sqlite3SelectOpName`.
fn compound_named(operator: Compound) -> Vec<u8> {
    match operator {
        Compound::Union => b"UNION".to_vec(),
        Compound::UnionAll => b"UNION ALL".to_vec(),
        Compound::Except => b"EXCEPT".to_vec(),
        Compound::Intersect => b"INTERSECT".to_vec(),
    }
}

/// The name a statement above it reaches each column of a shape by.
fn named_of(shape: &Shape) -> Vec<Vec<u8>> {
    shape
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect()
}

/// The `ORDER BY` of a compound, whose every term has to count or name
/// a column of the answer: there is no row left to read an expression
/// against.
fn matched(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cores: &[Vec<Vec<u8>>],
    collations: &[Collation],
    collating: &[crate::value::Collating],
) -> Result<Vec<Ordered>, Error> {
    let first = cores.first().map_or(&[][..], Vec::as_slice);
    let mut out = Vec::new();
    for (which, key) in keys(arena, select, sql, first, collations, collating)?
        .into_iter()
        .enumerate()
    {
        let found = match key.of {
            // A `COLLATE` on the term is what the sort compares under,
            // which `multiSelectOrderBy` reads off the term and not off
            // the column it counts to.
            Keyed::Place(at, collation) => Some((at, collation)),
            Keyed::Expr(_) => elsewhere(arena, select, sql, cores, collations, which, collating)?,
        };
        let (at, collation) = found.ok_or(Error::OrderMatch(which.saturating_add(1)))?;
        out.push(Ordered {
            at,
            descending: key.descending,
            collation,
            nulls: key.nulls,
        });
    }
    Ok(out)
}

/// Where one `ORDER BY` term counts to among the cores after the first,
/// which `resolveCompoundOrderBy` reads from the left and stops at the
/// first core that answers a column of that name.
///
/// Reading one term against every core costs O(cores · terms).
fn elsewhere(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cores: &[Vec<Vec<u8>>],
    collations: &[Collation],
    which: usize,
    collating: &[crate::value::Collating],
) -> Result<Option<(usize, Collation)>, Error> {
    for names in cores.iter().skip(1) {
        let keyed = keys(arena, select, sql, names, collations, collating)?;
        if let Some(Key {
            of: Keyed::Place(at, collation),
            ..
        }) = keyed.into_iter().nth(which)
        {
            return Ok(Some((at, collation)));
        }
    }
    Ok(None)
}

/// One table of the schema, read out of the statement that made it,
/// and nothing where the statement makes something else.
///
/// Reading one statement costs O(n) in its bytes.
fn stored_of(
    sql: Vec<u8>,
    place: usize,
    root: u32,
    encoding: Encoding,
    collating: &[crate::value::Collating],
) -> Result<Option<Stored>, Error> {
    let (arena, definition) = parse::definition(&sql)?;
    let crate::ast::Definition::Table(written) = definition else {
        return Ok(None);
    };
    let mut table = schema::table(&arena, &written, &sql, collating)?;
    // `BINARY` is the same collation under the same name whatever the
    // encoding, and it answers by the bytes the file holds.
    let binary = binary_of(encoding);
    if binary != Collation::Binary {
        for column in &mut table.columns {
            if column.collation == Collation::Binary {
                column.collation = binary;
            }
        }
    }
    let places = places(&table);
    Ok(Some(Stored {
        table,
        place,
        root,
        sql,
        arena,
        places,
        indexes: Vec::new(),
    }))
}

/// The tables one file holds, each carrying the schema place of that
/// file.
///
/// A table whose statement this crate cannot read is passed over, so a
/// database that holds one is read for everything else it holds.
///
/// Reading the schema costs O(n) in its rows.
fn read_tables(
    image: &Image<'_>,
    place: usize,
    encoding: Encoding,
    collating: &'static [crate::value::Collating],
) -> Result<Vec<Stored>, Error> {
    let mut tables = Vec::new();
    let mut payload = Vec::new();
    for row in image.schema() {
        let row = row?;
        read_payload(image, &row.payload, &mut payload)?;
        let record = record::Record::parse(&payload)?;
        let text = |at: usize| -> Result<Vec<u8>, Error> {
            Ok(match record.value(at)? {
                Some(record::Value::Text(bytes)) => crate::value::decoded(bytes, encoding),
                _ => Vec::new(),
            })
        };
        if text(0)? != b"table" {
            continue;
        }
        let Some(record::Value::Int(root)) = record.value(3)? else {
            continue;
        };
        let sql = text(4)?;
        if sql.is_empty() {
            continue;
        }
        let root = u32::try_from(root).unwrap_or(0);
        let Some(stored) = stored_of(sql, place, root, encoding, collating)? else {
            continue;
        };
        tables.push(stored);
    }
    // `sqlite_schema` is a table of the schema like any other: it lies
    // on page one and holds the five columns every row of it is written
    // with, which `sqlite3InitOne` builds in memory rather than reading
    // out of a row.
    tables.extend(
        stored_of(
            SCHEMA_CREATE.to_vec(),
            place,
            crate::image::SCHEMA_ROOT,
            encoding,
            collating,
        )
        .ok()
        .flatten(),
    );
    Ok(tables)
}

/// The name the schema's own table is held under.
const SCHEMA_TABLE: &[u8] = b"sqlite_master";

/// The statement the schema's own table is read from, which is what
/// `sqlite3InitOne` builds it out of.
const SCHEMA_CREATE: &[u8] =
    b"CREATE TABLE sqlite_master(type text,name text,tbl_name text,rootpage int,sql text)";

/// What a `COMMIT` and a `ROLLBACK` outside a transaction are refused
/// with, which `sqlite3VdbeExec` of `research/sqlite/src/vdbe.c:4057`
/// writes one of for each.
fn rolled(rolling: bool) -> alloc::string::String {
    let word = if rolling { "rollback" } else { "commit" };
    alloc::format!("cannot {word} - no transaction is active")
}

/// Whether a name is one the temp schema's own table answers to, which
/// `PREFERRED_TEMP_SCHEMA_TABLE` and `LEGACY_TEMP_SCHEMA_TABLE` of
/// `research/sqlite/src/sqliteInt.h` are the two of.
const fn temp_named(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"sqlite_temp_master")
        || name.eq_ignore_ascii_case(b"sqlite_temp_schema")
}

/// Whether a name is the one the temp schema answers to.
const fn is_temp(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"temp")
}

/// Whether `name` is one of the three the schema's own table answers
/// to.
#[must_use]
pub const fn schema_named(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"sqlite_master")
        || name.eq_ignore_ascii_case(b"sqlite_schema")
        || name.eq_ignore_ascii_case(b"sqlite_temp_master")
        || name.eq_ignore_ascii_case(b"sqlite_temp_schema")
}

/// What a comparison against an expression does: the affinity and the
/// collation of the column it names, where it names one.
///
/// This is `sqlite3ExprAffinity` and `sqlite3ExprCollSeq` over the two
/// nodes that carry either.
fn compared(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    collating: &[crate::value::Collating],
) -> (Affinity, Option<Collation>) {
    match arena.node(id) {
        Some(Node::Collate { value, name }) => (
            compared(arena, value, sql, sides, collating).0,
            crate::value::collation_of(&dequote(name.text(sql)), collating),
        ),
        Some(Node::Column { column, .. }) => sides
            .iter()
            .flat_map(|side| &side.shape.columns)
            .find(|held| held.name.eq_ignore_ascii_case(&dequote(column.text(sql))))
            .map_or((Affinity::None, None), |held| {
                (held.affinity, held.collation)
            }),
        _ => (Affinity::None, None),
    }
}

/// Whether a `WHERE` or a `HAVING` keeps this row.
fn keep(
    arena: &Arena,
    clause: Option<ExprId>,
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<bool, Error> {
    let Some(clause) = clause else {
        return Ok(true);
    };
    Ok(evaluate_row(arena, clause, sql, cursor)?.truth(false))
}

/// The name a column reference is answered under: the column's own
/// name where the table has one, and `rowid` for the three names the
/// key answers to.
fn answered_name(name: &[u8], sides: &[Side<'_>]) -> Vec<u8> {
    if sides.is_empty() {
        return name.to_vec();
    }
    let held = sides
        .iter()
        .flat_map(|side| &side.shape.columns)
        .find(|column| column.name.eq_ignore_ascii_case(name));
    if let Some(column) = held {
        return column.name.clone();
    }
    // A key that is the rowid answers under its own name, whichever of
    // the rowid's three names was written; every other name no side
    // holds answers under the name that was written, which is what
    // `sqlite3ColumnsFromExprList` does for a `TK_ID`.
    if !rowid_named(name) {
        return name.to_vec();
    }
    sides
        .iter()
        .find_map(|side| side.shape.key.clone())
        .unwrap_or_else(|| b"rowid".to_vec())
}

/// Whether `name` is one of the three names the key of a table answers
/// to.
fn rowid_named(name: &[u8]) -> bool {
    [b"rowid".as_slice(), b"oid", b"_rowid_"]
        .iter()
        .any(|word| name.eq_ignore_ascii_case(word))
}

/// The whole number an expression is, where it is one.
///
/// This is `sqlite3ExprIsInteger`, which reads the sign as part of the
/// number so that `ORDER BY -1` counts to the column before the first.
fn whole_number(arena: &Arena, id: ExprId, sql: &[u8]) -> Option<i64> {
    match arena.node(uncollated(arena, id))? {
        Node::Literal(Literal::Integer(span)) => Some(number::integer(span.text(sql)).value),
        Node::Unary {
            op: UnaryOp::Negate,
            operand,
        } => whole_number(arena, operand, sql).map(i64::wrapping_neg),
        _ => None,
    }
}

/// The column an expression names, where it is nothing else.
///
/// A collation written around it counts as something else, which is why
/// `SELECT a COLLATE NOCASE` is answered under that whole text and not
/// under `a`.
/// The collation the outermost `COLLATE` on an `ORDER BY` term names,
/// or nothing where the term carries none.
///
/// `sqlite3ExprCollSeq` stops at the first `COLLATE` it reaches from
/// the top, so a term written with two names sorts under the outer one,
/// and a name no collation of this crate answers is refused there.
fn term_collation(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    collating: &[crate::value::Collating],
) -> Result<Option<Collation>, Error> {
    let Some(Node::Collate { name, .. }) = arena.node(id) else {
        return Ok(None);
    };
    let named = crate::schema::dequote(name.text(sql));
    // The refusal is built where the name is read, because
    // `crate::schema::collations` refuses the statement before it runs
    // and leaves this arm with no way to reach a closure.
    let collation = crate::value::collation_of(&named, collating)
        .ok_or(eval::Error::NoCollation(named.clone()))?;
    Ok(Some(collation))
}

/// The expression under every `COLLATE` written on it, which is
/// `sqlite3ExprSkipCollateAndLikely` for the half of it that stands
/// while names are being resolved.
fn uncollated(arena: &Arena, id: ExprId) -> ExprId {
    let mut id = id;
    while let Some(Node::Collate { value, .. }) = arena.node(id) {
        id = value;
    }
    id
}

fn column_named(arena: &Arena, id: ExprId) -> Option<Span> {
    column_parts(arena, id).map(|(_, column)| column)
}

/// The table a column reference names and the column itself, or nothing
/// where the expression is no column reference.
fn column_parts(arena: &Arena, id: ExprId) -> Option<(Option<Span>, Span)> {
    match arena.node(id)? {
        Node::Column { table, column, .. } => Some((table, column)),
        _ => None,
    }
}

/// One row with the values it sorts by beside it.
struct Sorted {
    /// What the statement answers for the row.
    values: Vec<Value>,
    /// What it sorts by, one per term.
    keys: Vec<(Value, Collation)>,
}

/// What an `ORDER BY` term sorts by.
struct Key {
    /// What it reads.
    of: Keyed,
    /// Whether it sorts backwards.
    descending: bool,
    /// Where its nulls go.
    nulls: Nulls,
}

/// What an `ORDER BY` term reads.
enum Keyed {
    /// The column of the answer at this place, compared under the
    /// collation that column was declared with.
    Place(usize, Collation),
    /// An expression over the row.
    Expr(ExprId),
}

/// Where two values stand under one term, a written `NULLS` deciding
/// where a null goes before the term's direction does.
fn order_under(
    left: &Value,
    right: &Value,
    collation: Collation,
    descending: bool,
    nulls: Nulls,
) -> core::cmp::Ordering {
    let order = compare(left, right, collation);
    if order == core::cmp::Ordering::Equal {
        return order;
    }
    let first = match nulls {
        Nulls::First => Some(true),
        Nulls::Last => Some(false),
        Nulls::Unspecified => None,
    };
    if let Some(first) = first
        && (*left == Value::Null || *right == Value::Null)
    {
        return if (*left == Value::Null) == first {
            core::cmp::Ordering::Less
        } else {
            core::cmp::Ordering::Greater
        };
    }
    if descending { order.reverse() } else { order }
}

/// The collation of the answered column at `at`, which is `BINARY`
/// where the answer holds no such column.
fn collation_at(collations: &[Collation], at: usize) -> Collation {
    collations.get(at).copied().unwrap_or(Collation::Binary)
}

/// Where two rows stand against each other under the terms.
fn order_of(
    left: &[(Value, Collation)],
    right: &[(Value, Collation)],
    keys: &[Key],
) -> core::cmp::Ordering {
    for ((first, second), key) in left.iter().zip(right).zip(keys) {
        let order = order_under(&first.0, &second.0, first.1, key.descending, key.nulls);
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    core::cmp::Ordering::Equal
}

/// Where two groups stand against each other under their keys, which
/// is the order a `GROUP BY` answers them in.
fn order_of_keys(left: &[(Value, Collation)], right: &[(Value, Collation)]) -> core::cmp::Ordering {
    for (first, second) in left.iter().zip(right) {
        let order = compare(&first.0, &second.0, first.1);
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    core::cmp::Ordering::Equal
}

/// Whether two rows belong to the same group.
fn alike(left: &[(Value, Collation)], right: &[(Value, Collation)]) -> bool {
    order_of_keys(left, right) == core::cmp::Ordering::Equal
}

/// The values of one row: what the record holds, the rowid where its
/// alias stands, and what a computed column computes.
///
/// A column that is computed and not stored takes no place in the
/// record, so the record is read by the storage place
/// `sqlite3TableColumnToStorage` gives each column and not by the
/// column's own place.
fn values_of(
    payload: &[u8],
    stored: &Stored,
    rowid: Option<i64>,
    encoding: Encoding,
    collation: Collation,
) -> Result<Vec<Value>, Error> {
    let record = record::Record::parse(payload)?;
    let table = &stored.table;
    let mut out: Vec<Option<Value>> = Vec::new();
    for (at, column) in table.columns.iter().enumerate() {
        let Some(place) = stored
            .places
            .get(at)
            .copied()
            .filter(|_| column.generated != Generated::Virtual)
        else {
            out.push(None);
            continue;
        };
        // A row written before the column was added holds no value for
        // it, which is what schema format 2 allows and format 3 gives a
        // fallback for: the column answers what it falls back to.
        if record.value(place)?.is_none()
            && let Some(expr) = column.falls_back
        {
            // `sqlite3AlterFinishAddColumn` refuses a column added with
            // a fallback that is not a constant, so no fallback read
            // here names the clock.
            let value = crate::eval::evaluate(&stored.arena, expr, &stored.sql, None)?;
            out.push(Some(value));
            continue;
        }
        let mut value = match record.value(place)? {
            None | Some(record::Value::Null) => Value::Null,
            Some(record::Value::Int(number)) => Value::Int(number),
            Some(record::Value::Real(number)) => Value::Real(number),
            Some(record::Value::Text(bytes)) => Value::Text(crate::value::decoded(bytes, encoding)),
            Some(record::Value::Blob(bytes)) => Value::Blob(bytes.to_vec()),
        };
        // A real that is a whole number is stored as an integer, and
        // `OP_RealAffinity` is what turns it back on the way out.
        if column.affinity == Affinity::Real
            && let Value::Int(number) = value
        {
            value = Value::Real(crate::value::integer_as_real(number));
        }
        // The column the rowid is another name for holds nothing of its
        // own: the key is what it answers. A table that keeps its rows
        // in the key's own tree has no such column.
        if let Some(key) = rowid.filter(|_| table.rowid_alias == Some(at)) {
            value = Value::Int(key);
        }
        out.push(Some(value));
    }
    compute(stored, &mut out, encoding, collation)?;
    Ok(out
        .into_iter()
        .map(|value| value.unwrap_or(Value::Null))
        .collect())
}

/// Fills in the columns that are computed and not stored.
///
/// One expression may name another such column, in either direction, so
/// the passes run until none settles anything: at most one pass per
/// computed column, which is O(k²) evaluations for `k` of them.
fn compute(
    stored: &Stored,
    values: &mut [Option<Value>],
    encoding: Encoding,
    collation: Collation,
) -> Result<(), Error> {
    let table = &stored.table;
    let waiting = values.iter().filter(|value| value.is_none()).count();
    for _ in 0..waiting {
        let mut settled: Vec<(usize, Value)> = Vec::new();
        for (at, column) in table.columns.iter().enumerate() {
            let unsettled = values.get(at).is_some_and(Option::is_none);
            let Some(expr) = column.computed.filter(|_| unsettled) else {
                continue;
            };
            let row = Computed {
                table,
                values,
                encoding,
                collation,
            };
            // A name this pass cannot answer yet refuses, and the next
            // pass asks again.
            if let Ok(mut value) = evaluate_row(&stored.arena, expr, &stored.sql, &row) {
                crate::value::apply(&mut value, column.affinity);
                settled.push((at, value));
            }
        }
        if settled.is_empty() {
            break;
        }
        for (at, value) in settled {
            for slot in values.iter_mut().skip(at).take(1) {
                *slot = Some(value.clone());
            }
        }
    }
    if values.iter().any(Option::is_none) {
        // A computed column that names itself, or names a column no
        // table has.
        return Err(Error::Computed);
    }
    Ok(())
}

/// A row while its computed columns are being filled in: it answers the
/// columns that are settled and refuses the ones that are not.
struct Computed<'a> {
    /// The table.
    table: &'a Table,
    /// What is settled so far, one per column.
    values: &'a [Option<Value>],
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
}

impl eval::Row for Computed<'_> {
    fn collation(&self) -> Collation {
        self.collation
    }

    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn column(
        &self,
        _schema: Option<&[u8]>,
        _table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        let at = self
            .table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(column))?;
        let held = self.table.columns.get(at)?;
        let value = self.values.get(at)?.clone()?;
        Some((value, held.affinity, held.collation))
    }
}

/// What `BINARY` is over text the file keeps in this encoding.
const fn binary_of(encoding: Encoding) -> Collation {
    match encoding {
        Encoding::Utf8 => Collation::Binary,
        Encoding::Utf16Le => Collation::Binary16Le,
        Encoding::Utf16Be => Collation::Binary16Be,
    }
}

/// One side of the `FROM`, as one row of the walk holds it.
#[derive(Clone)]
struct Held<'a> {
    /// The columns it answers.
    shape: &'a Shape,
    /// The name of the database the table it reads stands in, and
    /// nothing where no schema names it.
    schema: &'a [u8],
    /// What the statement calls it, which is empty for a statement
    /// written inside the `FROM` with no alias.
    name: &'a [u8],
    /// The columns it does not answer a bare name with, which are the
    /// ones a `USING` or a `NATURAL` matched it by.
    using: &'a [Vec<u8>],
    /// The rowid of the row, where the side stands for one.
    rowid: Option<i64>,
    /// Its values, one per column.
    values: Vec<Value>,
}

impl<'a> Held<'a> {
    /// The side with no row of it, which is what a `LEFT JOIN` holds
    /// where nothing matched.
    fn empty(shape: &'a Shape, schema: &'a [u8], name: &'a [u8], using: &'a [Vec<u8>]) -> Self {
        Held {
            shape,
            schema,
            name,
            using,
            rowid: None,
            values: alloc::vec![Value::Null; shape.columns.len()],
        }
    }

    /// Whether `named` is what the statement calls this side.
    const fn named(&self, named: &[u8]) -> bool {
        !self.name.is_empty() && self.name.eq_ignore_ascii_case(named)
    }

    /// Whether a bare name reaches this side's column of that name,
    /// which the side a `USING` matched does not answer.
    fn hides(&self, column: &[u8]) -> bool {
        self.using
            .iter()
            .any(|name| name.eq_ignore_ascii_case(column))
    }

    /// What a bare name reaches on a side standing for the tables
    /// inside brackets: the first of the columns of that name that is
    /// not nothing, which is the column a `RIGHT JOIN` inside the
    /// brackets filled, and the first of them where every one is
    /// nothing.
    fn filled(&self, column: &[u8], default: Collation) -> Option<(Value, Affinity, Collation)> {
        if !self.shape.nested {
            return None;
        }
        let mut first: Option<(Value, Affinity, Collation)> = None;
        for (at, held) in self.shape.columns.iter().enumerate() {
            if !held.name.eq_ignore_ascii_case(column) {
                continue;
            }
            let value = self.values.get(at).cloned().unwrap_or(Value::Null);
            let answered = (value, held.affinity, held.collation.unwrap_or(default));
            if answered.0 != Value::Null {
                return Some(answered);
            }
            first = first.or(Some(answered));
        }
        first
    }

    /// What this side answers for the column `column` of the side
    /// `from` it holds the columns of, or nothing where it holds none
    /// of that side. A side with a name of its own answers nothing
    /// here, because a name in front of a column names that side.
    fn column_of(
        &self,
        from: &[u8],
        column: &[u8],
        default: Collation,
    ) -> Option<(Value, Affinity, Collation)> {
        if !self.shape.nested {
            return None;
        }
        let at = self.shape.columns.iter().position(|held| {
            held.from.eq_ignore_ascii_case(from) && held.name.eq_ignore_ascii_case(column)
        })?;
        let held = self.shape.columns.get(at)?;
        let value = self.values.get(at).cloned().unwrap_or(Value::Null);
        Some((value, held.affinity, held.collation.unwrap_or(default)))
    }

    /// Calls `each` with every column and the value it holds.
    fn each(&self, mut each: impl FnMut(&Column, &Value)) {
        for (column, value) in self.shape.columns.iter().zip(&self.values) {
            each(column, value);
        }
    }

    /// What this side answers for `column`, or nothing where it has no
    /// such column. `default` is the collation of a column nothing was
    /// written about.
    fn column(&self, column: &[u8], default: Collation) -> Option<(Value, Affinity, Collation)> {
        if let Some(answered) = self.filled(column, default) {
            return Some(answered);
        }
        let Some(at) = self.shape.place(column) else {
            // The three names the rowid answers to, which a statement
            // inside the `FROM` and a table that keeps its rows in the
            // key's own tree both refuse.
            let rowid = self.shape.keyed
                && (column.eq_ignore_ascii_case(b"rowid")
                    || column.eq_ignore_ascii_case(b"oid")
                    || column.eq_ignore_ascii_case(b"_rowid_"));
            if rowid {
                let value = self.rowid.map_or(Value::Null, Value::Int);
                return Some((value, Affinity::Integer, Collation::Binary));
            }
            return None;
        };
        let held = self.shape.columns.get(at)?;
        let value = self.values.get(at).cloned().unwrap_or(Value::Null);
        Some((value, held.affinity, held.collation.unwrap_or(default)))
    }
}

/// Where a walk stands: one row of every table of the `FROM`.
struct Cursor<'a> {
    /// One per table, in the order they were written.
    held: Vec<Held<'a>>,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// What each aggregate call of the statement answered, where the
    /// row stands for a group.
    aggregates: Vec<(ExprId, Value)>,
    /// What a statement written inside an expression is answered by.
    reach: Reach<'a>,
}

impl<'a> Cursor<'a> {
    /// What the side at `at` answers for `column`, filled from the
    /// sides a `USING` or a `NATURAL` matched to it where its own value
    /// is `NULL`.
    ///
    /// This is the `coalesce` `sqlite3ProcessJoin` writes around such a
    /// column, and it shows in a `RIGHT JOIN`, where the side written
    /// first is the one that holds nothing.
    fn coalesced(&self, at: usize, column: &[u8], value: &Value) -> Value {
        if *value != Value::Null {
            return value.clone();
        }
        self.held
            .iter()
            .skip(at.saturating_add(1))
            .filter(|held| held.hides(column))
            .filter_map(|held| held.column(column, self.collation).map(|(value, ..)| value))
            .find(|filled| *filled != Value::Null)
            .unwrap_or(Value::Null)
    }

    /// What the sides answer for one name, which is `lookupName`
    /// counting the columns a name matches: it reads every side, so it
    /// is O(sides) per name.
    fn answering(&self, schema: Option<&[u8]>, table: Option<&[u8]>, column: &[u8]) -> Answering {
        let mut found = Answering::Nothing;
        for (at, held) in self.held.iter().enumerate() {
            // A schema in front of the column names the database the
            // side reads, which a side that reads no table has none of.
            if schema.is_some_and(|named| !held.schema.eq_ignore_ascii_case(named)) {
                continue;
            }
            // A name in front of a column names a side, or one of the
            // tables inside brackets a side holds the columns of.
            let answered = match table {
                Some(named) if held.named(named) => held.column(column, self.collation),
                Some(named) => held.column_of(named, column, self.collation),
                // A bare name does not reach the side a `USING` or a
                // `NATURAL` matched; the side before it answers.
                None if held.hides(column) => continue,
                None => held.column(column, self.collation),
            };
            let Some((value, affinity, collation)) = answered else {
                continue;
            };
            if matches!(found, Answering::One(..)) {
                return Answering::Many;
            }
            found = Answering::One(at, value, affinity, collation);
        }
        found
    }

    /// A cursor that holds no side, which is a statement with no
    /// `FROM` before it reads its one row of nothing.
    const fn new(collation: Collation, encoding: Encoding, reach: Reach<'a>) -> Self {
        Cursor {
            held: Vec::new(),
            collation,
            encoding,
            aggregates: Vec::new(),
            reach,
        }
    }
}

impl eval::Row for Cursor<'_> {
    fn collation(&self) -> Collation {
        self.collation
    }

    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn random(&self) -> Option<&crate::random::Source> {
        Some(&self.reach.database.random)
    }

    fn clock(&self) -> Option<i64> {
        self.reach.database.clock.map(crate::date::julian_of)
    }

    fn sensitive(&self) -> bool {
        self.reach.database.sensitive
    }

    fn grouped(&self) -> &'static [crate::func::Grouped] {
        self.reach.database.grouped
    }

    fn defined(&self, name: &[u8], count: usize) -> Option<crate::func::Defined> {
        crate::func::defined(self.reach.database.defined, name, count)
    }

    fn collating(&self) -> &'static [crate::value::Collating] {
        self.reach.database.collating
    }

    fn counted(&self) -> crate::func::Counted {
        self.reach.database.counted
    }

    fn aggregate(&self, id: ExprId) -> Option<Value> {
        self.aggregates
            .iter()
            .find(|(call, _)| *call == id)
            .map(|(_, value)| value.clone())
    }

    fn answered(&self, used: Used) -> Option<Value> {
        let reach = self.reach;
        reach
            .database
            .answer(reach.arena, reach.sql, used, reach.scope, self)
            .ok()
    }

    fn answered_items(
        &self,
        select: SelectId,
    ) -> Option<Vec<(Value, Affinity, Option<Collation>)>> {
        let reach = self.reach;
        reach
            .database
            .answer_items(reach.arena, reach.sql, select, reach.scope, self)
            .ok()
    }

    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        let Answering::One(at, value, affinity, collation) = self.answering(schema, table, column)
        else {
            // A name no side of this statement answers is the enclosing
            // statement's, which is what makes a statement correlated,
            // and a name more than one side answers is a refusal the
            // walk asks `ambiguous` for.
            return self
                .reach
                .scope
                .outer
                .and_then(|outer| outer.column(schema, table, column));
        };
        // A name written with its table is that table's value; only a
        // bare one is filled from the side a `USING` matched to it.
        let value = match table {
            Some(_) => value,
            None => self.coalesced(at, column, &value),
        };
        // A `rowid` the function the connection was told ignored a read
        // of answers a null: the key is no column of the side, so
        // `Database::nulled` does not reach it.
        let side = self.held.get(at).map_or(b"".as_slice(), |held| held.name);
        if self.reach.database.is_ignored(side, column) {
            return Some((Value::Null, affinity, collation));
        }
        Some((value, affinity, collation))
    }

    fn ambiguous(&self, schema: Option<&[u8]>, table: Option<&[u8]>, column: &[u8]) -> bool {
        matches!(self.answering(schema, table, column), Answering::Many)
    }
}

/// What the sides of one row answer for a name.
enum Answering {
    /// No side answers to the name.
    Nothing,
    /// One side answers, with its place, its value, and what a
    /// comparison against it does.
    One(usize, Value, Affinity, Collation),
    /// More than one side answers, which `lookupName` refuses rather
    /// than choosing between.
    Many,
}

/// One window function call of a statement.
struct Over {
    /// The node it was written as, which is what answers it: two
    /// `row_number()` in one statement are one column each.
    id: ExprId,
    /// Which window function.
    which: Which,
    /// Its arguments.
    args: Range,
    /// The `FILTER`, which only an aggregate may carry.
    filter: Option<ExprId>,
    /// The window it reads.
    window: WindowId,
}

/// Every window function call a statement answers with.
fn overs(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    grouped: &'static [crate::func::Grouped],
) -> Result<Vec<Over>, Error> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        if let ResultColumn::Expr { expr, .. } = *column {
            gather_overs(arena, expr, sql, &mut out, grouped)?;
        }
    }
    for term in arena.orders(select.order) {
        gather_overs(arena, term.expr, sql, &mut out, grouped)?;
    }
    Ok(out)
}

/// The same for one expression and everything under it.
fn gather_overs(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    out: &mut Vec<Over>,
    grouped: &'static [crate::func::Grouped],
) -> Result<(), Error> {
    arena.node(id).map_or(Ok(()), |node| {
        gather_one(arena, id, node, sql, out, grouped)
    })
}

/// The same for one node the arena holds.
fn gather_one(
    arena: &Arena,
    id: ExprId,
    node: Node,
    sql: &[u8],
    out: &mut Vec<Over>,
    grouped: &'static [crate::func::Grouped],
) -> Result<(), Error> {
    if let Node::Over {
        name,
        args,
        distinct,
        star,
        filter,
        window,
    } = node
    {
        let count = if star { 0 } else { arena.children(args).len() };
        let called = dequote(name.text(sql));
        let which = window::lookup_in(grouped, &called, count)
            .ok_or_else(|| refused_over(&called, count, grouped))?;
        // `sqlite3WindowRewrite` takes a `FILTER` for an aggregate and
        // refuses one for the eleven built-in window functions, and
        // refuses `DISTINCT` for every one of them.
        if distinct {
            return Err(Error::DistinctWindow);
        }
        if filter.is_some() && !which.filtered() {
            return Err(Error::Eval(eval::Error::Filtered(called)));
        }
        out.push(Over {
            id,
            which,
            args,
            filter,
            window,
        });
    }
    let mut deeper = Ok(());
    arena.under(node, |child| {
        if deeper.is_ok() {
            deeper = gather_overs(arena, child, sql, out, grouped);
        }
    });
    deeper
}

/// How many windows an `OVER` may name before the chain is read as one
/// that names itself.
const WINDOW_CHAIN: usize = 32;

/// The frame a window with no frame clause reads, which is
/// `RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW`.
const WHOLE: Frames = Frames {
    kind: Frame::Range,
    start: Edge::UnboundedPreceding,
    end: Edge::CurrentRow,
    exclude: Exclude::NoOthers,
};

/// A window with every window it names folded into it.
struct Framed {
    /// The `PARTITION BY` terms.
    partition: Range,
    /// The `ORDER BY` terms.
    order: Range,
    /// The frame.
    frame: Frames,
}

/// What a name an `OVER` follows that names no window function is
/// refused with: a name the table holds under another number of
/// arguments is that number, a name the scalars hold is one no window
/// reads, and any other name is no function at all.
fn refused_over(called: &[u8], count: usize, grouped: &'static [crate::func::Grouped]) -> Error {
    if window::named(called) || crate::agg::named_in(grouped, called) {
        return Error::Eval(eval::Error::WrongArguments(called.to_vec()));
    }
    if crate::func::lookup(called, count).is_ok() {
        return Error::NotAWindow(called.to_vec());
    }
    Error::Eval(eval::Error::NoFunction(called.to_vec()))
}

/// The window an `OVER` names, with the windows it builds on folded in.
///
/// `sqlite3WindowAssemble`: a window that names another takes that
/// window's partition, takes its order where it writes none of its own,
/// and may write a frame only where the named one writes none.
fn framed(arena: &Arena, select: &Select, sql: &[u8], id: WindowId) -> Result<Framed, Error> {
    let window = arena.window(id).ok_or(Error::Window)?;
    let mut partition = window.partition;
    let mut order = window.order;
    let mut frame = window.frame;
    let mut base = window.base;
    let mut whole = window.named;
    let mut chain = 0_usize;
    while let Some(name) = base {
        chain = chain.saturating_add(1);
        if chain > WINDOW_CHAIN {
            return Err(Error::Window);
        }
        let named = dequote(name.text(sql));
        let found = arena
            .named_windows(select.windows)
            .iter()
            .find(|held| dequote(held.name.text(sql)).eq_ignore_ascii_case(&named))
            .ok_or_else(|| Error::NoWindowNamed(named.clone()))?;
        let under = arena.window(found.window).ok_or(Error::Window)?;
        // `OVER name` is the named window itself, frame and all, which
        // is `sqlite3WindowUpdate`; `OVER (name ...)` builds on it, and
        // `sqlite3WindowChain` refuses a window that overrides the
        // named one's partition, order or frame.
        if whole {
            partition = under.partition;
            order = under.order;
            frame = under.frame;
        } else {
            // `sqlite3WindowChain` reads the three in this order, so a
            // window that overrides more than one is named by the
            // first of them.
            if !partition.is_empty() {
                return Err(Error::Override(b"PARTITION clause".to_vec(), named));
            }
            if !order.is_empty() && !under.order.is_empty() {
                return Err(Error::Override(b"ORDER BY clause".to_vec(), named));
            }
            if under.frame.is_some() {
                return Err(Error::Override(b"frame specification".to_vec(), named));
            }
            partition = under.partition;
            if order.is_empty() {
                order = under.order;
            }
        }
        whole = under.named;
        base = under.base;
    }
    Ok(Framed {
        partition,
        order,
        frame: frame.unwrap_or(WHOLE),
    })
}

/// Where two rows stand against each other under a window's order
/// terms.
fn order_of_window(
    left: &[(Value, Collation)],
    right: &[(Value, Collation)],
    terms: &[OrderTerm],
) -> core::cmp::Ordering {
    for ((first, second), term) in left.iter().zip(right).zip(terms) {
        let order = order_under(
            &first.0,
            &second.0,
            first.1,
            term.order == Order::Descending,
            term.nulls,
        );
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    core::cmp::Ordering::Equal
}

/// What a window's terms answer for one row.
fn keyed(
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
    terms: Range,
) -> Result<Vec<(Value, Collation)>, Error> {
    let mut key = Vec::new();
    for term in arena.children(terms) {
        key.push(evaluate_collated(arena, *term, sql, cursor)?);
    }
    Ok(key)
}

/// What one row of a partition sorted by, one value per term of the
/// window, each with what it compares under.
type Terms = Vec<(Value, Collation)>;

/// One partition of a window: which cursor each of its rows is, in the
/// order the window's terms sort them, and what those terms answered.
type Partition = (Vec<usize>, Vec<Terms>);

/// One partition while its rows are being gathered.
struct Gathered {
    /// What the `PARTITION BY` terms answered for it.
    key: Terms,
    /// Its rows, each with what the order terms answered.
    rows: Vec<(usize, Terms)>,
}

/// The rows of each partition of a window, each run in the order the
/// window's terms sort it, with the key each row sorted by.
///
/// Sorting one partition costs O(n log n) in its rows.
fn partitioned(
    arena: &Arena,
    sql: &[u8],
    cursors: &[Cursor<'_>],
    framed: &Framed,
) -> Result<Vec<Partition>, Error> {
    let terms = arena.orders(framed.order);
    let mut parts: Vec<Gathered> = Vec::new();
    for (at, cursor) in cursors.iter().enumerate() {
        let key = keyed(arena, sql, cursor, framed.partition)?;
        let mut sorts = Vec::new();
        for term in terms {
            sorts.push(evaluate_collated(arena, term.expr, sql, cursor)?);
        }
        let found = parts.iter().position(|held| alike(&held.key, &key));
        match found.and_then(|place| parts.get_mut(place)) {
            Some(held) => held.rows.push((at, sorts)),
            None => parts.push(Gathered {
                key,
                rows: alloc::vec![(at, sorts)],
            }),
        }
    }
    let mut out = Vec::new();
    for mut part in parts {
        part.rows
            .sort_by(|left, right| order_of_window(&left.1, &right.1, terms));
        let places = part.rows.iter().map(|(at, _)| *at).collect();
        let keys = part.rows.into_iter().map(|(_, key)| key).collect();
        out.push((places, keys));
    }
    Ok(out)
}

/// A frame offset counted in rows or in groups, which SQLite refuses
/// unless it is a whole number of no sign.
fn offset_of(
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
    expr: ExprId,
    starting: bool,
) -> Result<usize, Error> {
    let refused = || Error::FrameOffset(starting, true);
    let value = whole_of(&evaluate_row(arena, expr, sql, cursor)?).ok_or_else(refused)?;
    usize::try_from(value).map_err(|_| refused())
}

/// A value as the whole number `OP_MustBeInt` would make of it, or
/// nothing where it is not one.
///
/// `applyAffinity` under `SQLITE_AFF_NUMERIC` converts text that is a
/// number whole and leaves every other text alone, which `OP_MustBeInt`
/// then refuses along with a real that stands for no whole number.
fn whole_of(value: &Value) -> Option<i64> {
    let mut numeric = value.clone();
    apply_numeric(&mut numeric, true);
    match numeric {
        Value::Int(number) => Some(number),
        Value::Real(number) => {
            let whole = crate::value::real_as_integer(number);
            let same = Value::Real(crate::value::integer_as_real(whole)) == Value::Real(number);
            same.then_some(whole)
        }
        _ => None,
    }
}

/// One end of a frame counted in rows or in groups.
fn edge_of(
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
    bound: Edge,
    starting: bool,
) -> Result<window::Edge, Error> {
    Ok(match bound {
        Edge::UnboundedPreceding => window::Edge::Start,
        Edge::CurrentRow => window::Edge::Current,
        Edge::UnboundedFollowing => window::Edge::End,
        Edge::Preceding(expr) => {
            window::Edge::Preceding(offset_of(arena, sql, cursor, expr, starting)?)
        }
        Edge::Following(expr) => {
            window::Edge::Following(offset_of(arena, sql, cursor, expr, starting)?)
        }
    })
}

/// Where a `RANGE` frame that counts by an offset begins or ends.
///
/// `windowCodeRangeTest` counts the offset onto the term of one row and
/// compares it against the term of the other, and leaves a term that is
/// text or a blob alone, which makes such a term frame its peers and
/// nothing else. A term that sorts downwards counts the offset the
/// other way and compares the other way round. The first row the
/// comparison holds for is where the frame reaches, which is the row
/// the C library's cursor stops on.
///
/// Halving the rows costs O(log n), which a term whose nulls sort after
/// its values gives up: the comparison is not ordered over such a term,
/// so the rows are read one after another at O(n).
fn range_edge(
    terms: &[(Value, Collation)],
    at: usize,
    offset: &Value,
    part: &Part<'_>,
    back: bool,
    strict: bool,
) -> usize {
    let here = terms.get(at).cloned().unwrap_or(NO_TERM);
    let descending = part.descending;
    let op = if descending {
        BinaryOp::Subtract
    } else {
        BinaryOp::Add
    };
    let collation = here.1;
    // A frame that counts backwards counts onto each row it looks at; a
    // frame that counts forwards counts onto the row it frames.
    let moved = if back {
        None
    } else {
        Some(shifted(op, &here.0, offset))
    };
    // `KEYINFO_ORDER_BIGNULL`: a term whose nulls sort after its values
    // answers a comparison against a null by rule and not by order.
    let big = match part.nulls {
        Nulls::Last => !descending,
        Nulls::First => descending,
        Nulls::Unspecified => false,
    };
    let holds = |row: usize| -> bool {
        let value = terms.get(row).cloned().unwrap_or(NO_TERM);
        let (first, second) = if back {
            (&value.0, &here.0)
        } else {
            (&here.0, &value.0)
        };
        if big && let Some(answer) = by_rule(back != descending, strict, first, second) {
            return answer;
        }
        let (left, right) = match &moved {
            Some(target) => (value.0.clone(), target.clone()),
            None => (shifted(op, &value.0, offset), here.0.clone()),
        };
        let order = compare(&left, &right, collation);
        let order = if descending { order.reverse() } else { order };
        if strict {
            order == core::cmp::Ordering::Greater
        } else {
            order != core::cmp::Ordering::Less
        }
    };
    if big {
        return (0..terms.len())
            .find(|row| holds(*row))
            .unwrap_or(terms.len());
    }
    window::first_true(terms.len(), holds)
}

/// The term of a row that a window orders by nothing, which no `RANGE`
/// frame that counts by an offset reads: such a frame is refused
/// unless the window orders by exactly one term.
const NO_TERM: (Value, Collation) = (Value::Null, Collation::Binary);

/// What a comparison of a term whose nulls sort after its values
/// answers where either side is null, and nothing where neither is.
///
/// This is the `BIGNULL` block of `windowCodeRangeTest`, where
/// `greater` says the comparison reads `>=` or `>` and `strict` says it
/// reads `>` or `<`.
fn by_rule(greater: bool, strict: bool, first: &Value, second: &Value) -> Option<bool> {
    if *first == Value::Null {
        return Some(match (greater, strict) {
            (true, false) => true,
            (true, true) => *second != Value::Null,
            (false, false) => *second == Value::Null,
            (false, true) => false,
        });
    }
    if *second == Value::Null {
        return Some(!greater);
    }
    None
}

/// A term with a frame offset counted onto it, which is the term itself
/// where the term is text or a blob.
fn shifted(op: BinaryOp, value: &Value, offset: &Value) -> Value {
    match *value {
        Value::Text(_) | Value::Blob(_) => value.clone(),
        _ => arithmetic(op, value, offset),
    }
}

/// The frame of the row at `at`, measured in the one term the window
/// orders by.
///
/// The parser refuses a frame that begins unbounded forwards or ends
/// unbounded backwards, so neither bound is read here.
fn range_span(keys: &[Vec<(Value, Collation)>], part: &Part<'_>, at: usize) -> window::Span {
    let frame = &part.framed.frame;
    let peers = part.peers;
    let group = peers.of_row(at);
    let (low, high) = &part.offsets;
    let terms: Vec<(Value, Collation)> = keys
        .iter()
        .map(|key| key.first().cloned().unwrap_or(NO_TERM))
        .collect();
    let start = match (frame.start, low) {
        (Edge::UnboundedPreceding, _) => 0,
        (Edge::Preceding(_), Some(offset)) => range_edge(&terms, at, offset, part, true, false),
        (Edge::Following(_), Some(offset)) => range_edge(&terms, at, offset, part, false, false),
        _ => group.start,
    };
    let end = match (frame.end, high) {
        (Edge::UnboundedFollowing, _) => peers.rows(),
        (Edge::Preceding(_), Some(offset)) => range_edge(&terms, at, offset, part, true, true),
        (Edge::Following(_), Some(offset)) => range_edge(&terms, at, offset, part, false, true),
        _ => group.end,
    };
    window::Span::of(start, end)
}

/// What the offsets of a `RANGE` frame answer, which are read once for
/// the window and not once per row.
fn range_offsets(
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
    frame: &Frames,
) -> Result<(Option<Value>, Option<Value>), Error> {
    let mut out = (None, None);
    for (bound, slot) in [(frame.start, false), (frame.end, true)] {
        let value = match bound {
            Edge::Preceding(expr) | Edge::Following(expr) => {
                let mut numeric = evaluate_row(arena, expr, sql, cursor)?;
                // `RANGE` counts in the term's own values, so the offset
                // has to be a number and may not be negative.
                apply_numeric(&mut numeric, true);
                match numeric {
                    Value::Int(count) if count >= 0 => Some(Value::Int(count)),
                    Value::Real(count) if count >= 0.0 => Some(Value::Real(count)),
                    _ => return Err(Error::FrameOffset(!slot, false)),
                }
            }
            _ => None,
        };
        if slot {
            out.1 = value;
        } else {
            out.0 = value;
        }
    }
    Ok(out)
}

/// One partition of a window while its rows are answered.
struct Part<'a> {
    /// The window the rows are read under.
    framed: &'a Framed,
    /// Which cursor each row of the partition is, in the order the
    /// window's terms sort them.
    rows: &'a [usize],
    /// What those terms answered for each of those rows.
    keys: &'a [Vec<(Value, Collation)>],
    /// Where the rows that share their terms begin and end.
    peers: &'a window::Peers,
    /// Whether the window's first order term sorts downwards.
    descending: bool,
    /// Where that term's nulls go.
    nulls: Nulls,
    /// What a `RANGE` frame's offsets answered.
    offsets: (Option<Value>, Option<Value>),
}

/// The rows of the frame of the row at `at`, as places in the
/// partition, with the rows `EXCLUDE` leaves out taken away.
fn frame_rows(
    arena: &Arena,
    sql: &[u8],
    cursor: &Cursor<'_>,
    part: &Part<'_>,
    at: usize,
) -> Result<Vec<usize>, Error> {
    let frame = &part.framed.frame;
    let span = match frame.kind {
        Frame::Rows => window::rows_frame(
            edge_of(arena, sql, cursor, frame.start, true)?,
            edge_of(arena, sql, cursor, frame.end, false)?,
            at,
            part.peers.rows(),
        ),
        Frame::Groups => window::groups_frame(
            edge_of(arena, sql, cursor, frame.start, true)?,
            edge_of(arena, sql, cursor, frame.end, false)?,
            at,
            part.peers,
        ),
        Frame::Range => range_span(part.keys, part, at),
    };
    Ok(window::kept(span, frame.exclude, at, part.peers))
}

/// What `lag` or `lead` answers for the row at `at`.
fn offset_value(
    arena: &Arena,
    sql: &[u8],
    cursors: &[Cursor<'_>],
    over: &Over,
    part: &Part<'_>,
    at: usize,
    ahead: bool,
) -> Result<Value, Error> {
    let args = arena.children(over.args);
    let expr = *args.first().ok_or(Error::Frame)?;
    let place = part
        .rows
        .get(at)
        .copied()
        .ok_or(Error::NoTable(Vec::new()))?;
    let cursor = cursors.get(place).ok_or(Error::NoTable(Vec::new()))?;
    let step = match args.get(1) {
        Some(id) => whole_of(&evaluate_row(arena, *id, sql, cursor)?).ok_or(Error::Frame)?,
        None => 1,
    };
    let here = i64::try_from(at).unwrap_or(i64::MAX);
    let target = if ahead {
        here.checked_add(step)
    } else {
        here.checked_sub(step)
    };
    let found = target
        .and_then(|row| usize::try_from(row).ok())
        .and_then(|row| part.rows.get(row))
        .copied();
    if let Some(found) = found {
        let read = cursors.get(found).ok_or(Error::NoTable(Vec::new()))?;
        return Ok(evaluate_row(arena, expr, sql, read)?);
    }
    // A row that far out of the partition answers the third argument,
    // and `NULL` where none was written.
    match args.get(2) {
        Some(id) => Ok(evaluate_row(arena, *id, sql, cursor)?),
        None => Ok(Value::Null),
    }
}

/// What `first_value`, `last_value` or `nth_value` answers for the row
/// at `at`.
fn framed_value(
    arena: &Arena,
    sql: &[u8],
    cursors: &[Cursor<'_>],
    over: &Over,
    part: &Part<'_>,
    at: usize,
) -> Result<Value, Error> {
    let place = part
        .rows
        .get(at)
        .copied()
        .ok_or(Error::NoTable(Vec::new()))?;
    let cursor = cursors.get(place).ok_or(Error::NoTable(Vec::new()))?;
    let frame = frame_rows(arena, sql, cursor, part, at)?;
    let args = arena.children(over.args);
    // A frame that holds no row answers `NULL`, which is the value
    // function reading past its end.
    let read = |row: Option<&usize>| -> Result<Value, Error> {
        let held = row
            .and_then(|row| part.rows.get(*row))
            .and_then(|place| cursors.get(*place));
        let expr = *args.first().ok_or(Error::Frame)?;
        held.map_or(Ok(Value::Null), |held| {
            Ok(evaluate_row(arena, expr, sql, held)?)
        })
    };
    let row = match over.which {
        Which::FirstValue => frame.first(),
        Which::LastValue => frame.last(),
        // `nth_value` is the one of the three left, and it counts its
        // rows from one.
        _ => {
            let id = *args.get(1).ok_or(Error::Frame)?;
            let nth = whole_of(&evaluate_row(arena, id, sql, cursor)?).ok_or(Error::Nth)?;
            let place = usize::try_from(nth).map_err(|_| Error::Nth)?;
            if place < 1 {
                return Err(Error::Nth);
            }
            frame.get(place.saturating_sub(1))
        }
    };
    read(row)
}

/// What an aggregate over the rows of a frame answers.
///
/// Stepping it costs O(m) in the rows of the frame, so a moving frame
/// costs O(n·m) over a partition.
fn accumulated(
    arena: &Arena,
    sql: &[u8],
    cursors: &[Cursor<'_>],
    over: &Over,
    part: &Part<'_>,
    at: usize,
    which: Aggregate,
) -> Result<Value, Error> {
    let place = part
        .rows
        .get(at)
        .copied()
        .ok_or(Error::NoTable(Vec::new()))?;
    let cursor = cursors.get(place).ok_or(Error::NoTable(Vec::new()))?;
    let frame = frame_rows(arena, sql, cursor, part, at)?;
    let mut accumulator = Accumulator::new(which, false);
    for row in &frame {
        let place = part.rows.get(*row).copied().unwrap_or(0);
        let read = cursors.get(place).ok_or(Error::NoTable(Vec::new()))?;
        // A `FILTER` decides which rows of the frame are stepped, which
        // is `sqlite3WindowCodeStep` jumping over the step.
        if !keep(arena, over.filter, sql, read)? {
            continue;
        }
        let mut values = Vec::new();
        let mut carried = Vec::new();
        let mut collation = read.collation;
        for (at, id) in arena.children(over.args).iter().enumerate() {
            if at == 0 {
                let (value, written) = evaluate_collated(arena, *id, sql, read)?;
                collation = written;
                values.push(value);
                carried.push(crate::eval::evaluate_carried(arena, *id, sql, read)?.1);
            } else {
                let (value, json) = crate::eval::evaluate_carried(arena, *id, sql, read)?;
                values.push(value);
                carried.push(json);
            }
        }
        accumulator.step(&values, &carried, collation)?;
    }
    Ok(accumulator.finish()?)
}

/// What one window function call answers for the row at `at` of its
/// partition.
fn answered_over(
    arena: &Arena,
    sql: &[u8],
    cursors: &[Cursor<'_>],
    over: &Over,
    part: &Part<'_>,
    at: usize,
) -> Result<Value, Error> {
    let peers = part.peers;
    match over.which {
        Which::RowNumber => Ok(Value::Int(window::row_number(at))),
        Which::Rank => Ok(Value::Int(window::rank(at, peers))),
        Which::DenseRank => Ok(Value::Int(window::dense_rank(at, peers))),
        Which::PercentRank => Ok(Value::Real(window::percent_rank(at, peers))),
        Which::CumeDist => Ok(Value::Real(window::cume_dist(at, peers))),
        Which::Ntile => {
            let place = part
                .rows
                .get(at)
                .copied()
                .ok_or(Error::NoTable(Vec::new()))?;
            let cursor = cursors.get(place).ok_or(Error::NoTable(Vec::new()))?;
            let id = *arena.children(over.args).first().ok_or(Error::Frame)?;
            let tiles = whole_of(&evaluate_row(arena, id, sql, cursor)?).ok_or(Error::Tiles)?;
            if tiles < 1 {
                return Err(Error::Tiles);
            }
            let tiles = usize::try_from(tiles).map_err(|_| Error::Tiles)?;
            Ok(Value::Int(window::ntile(at, peers.rows(), tiles)))
        }
        Which::Lag => offset_value(arena, sql, cursors, over, part, at, false),
        Which::Lead => offset_value(arena, sql, cursors, over, part, at, true),
        Which::Aggregate(which) => accumulated(arena, sql, cursors, over, part, at, which),
        _ => framed_value(arena, sql, cursors, over, part, at),
    }
}

/// What every window function call of a statement answers for every
/// row, written into the rows so that the walk over an expression looks
/// each answer up, and the order the rows are answered in.
///
/// `sqlite3WindowRewrite` answers the rows out of a sorter over the
/// window's partition and order terms, so a statement that writes no
/// `ORDER BY` of its own answers its rows in that order; a statement
/// that writes more than one window nests one sorter inside the next,
/// which the terms of the windows read in order come to.
fn overed(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cursors: &mut [Cursor<'_>],
    overs: &[Over],
) -> Result<Vec<usize>, Error> {
    let mut sorting: Vec<OrderTerm> = Vec::new();
    for over in overs {
        let framed = framed(arena, select, sql, over.window)?;
        let mut values = alloc::vec![Value::Null; cursors.len()];
        let terms = arena.orders(framed.order);
        for term in arena.children(framed.partition) {
            sorting.push(OrderTerm {
                expr: *term,
                order: Order::Unspecified,
                nulls: Nulls::Unspecified,
            });
        }
        sorting.extend_from_slice(terms);
        let descending = terms
            .first()
            .is_some_and(|term| term.order == Order::Descending);
        let nulls = terms.first().map_or(Nulls::Unspecified, |term| term.nulls);
        // A `RANGE` frame counts in the one term the window orders by,
        // and `sqlite3WindowAlloc` refuses an offset where the window
        // orders by any other number of terms.
        let counted = matches!(framed.frame.start, Edge::Preceding(_) | Edge::Following(_))
            || matches!(framed.frame.end, Edge::Preceding(_) | Edge::Following(_));
        if framed.frame.kind == Frame::Range && counted && terms.len() != 1 {
            return Err(Error::Frame);
        }
        for (rows, keys) in partitioned(arena, sql, cursors, &framed)? {
            let peers = window::Peers::new(rows.len(), |at| {
                keys.get(at)
                    .zip(keys.get(at.saturating_add(1)))
                    .is_some_and(|(left, right)| alike(left, right))
            });
            // Only a `RANGE` frame counts in the term's own values, so
            // only one reads an offset.
            let first = rows.first().and_then(|at| cursors.get(*at));
            let offsets = match (framed.frame.kind, first) {
                (Frame::Range, Some(cursor)) => range_offsets(arena, sql, cursor, &framed.frame)?,
                _ => (None, None),
            };
            let part = Part {
                framed: &framed,
                rows: &rows,
                keys: &keys,
                peers: &peers,
                descending,
                nulls,
                offsets,
            };
            for (at, place) in rows.iter().enumerate() {
                let value = answered_over(arena, sql, cursors, over, &part, at)?;
                for slot in values.iter_mut().skip(*place).take(1) {
                    *slot = value.clone();
                }
            }
        }
        for (cursor, value) in cursors.iter_mut().zip(values) {
            cursor.aggregates.push((over.id, value));
        }
    }
    let mut order: Vec<usize> = (0..cursors.len()).collect();
    if sorting.is_empty() {
        return Ok(order);
    }
    let mut keys = Vec::new();
    for cursor in cursors.iter() {
        let mut key = Vec::new();
        for term in &sorting {
            key.push(evaluate_collated(arena, term.expr, sql, cursor)?);
        }
        keys.push(key);
    }
    let empty = Vec::new();
    order.sort_by(|left, right| {
        order_of_window(
            keys.get(*left).unwrap_or(&empty),
            keys.get(*right).unwrap_or(&empty),
            &sorting,
        )
    });
    Ok(order)
}

/// A class a column may answer, one bit each, which is what
/// `sqlite3ExprDataType` of `src/expr.c` answers.
const NUMBER: u8 = 0x01;
/// Text.
const TEXT: u8 = 0x02;
/// A blob.
const BLOB: u8 = 0x04;

/// The classes a column of an affinity may answer, which is what a
/// column of a table answers: a numeric affinity keeps a number or a
/// blob, `TEXT` keeps text or a blob, and no affinity keeps anything.
fn classes_of(affinity: Affinity) -> u8 {
    if affinity.numeric() {
        return NUMBER | BLOB;
    }
    if matches!(affinity, Affinity::Text) {
        return TEXT | BLOB;
    }
    NUMBER | TEXT | BLOB
}

/// Which classes the expression at `id` may answer.
///
/// This is `sqlite3ExprDataType`, which reads the shape of the
/// expression and not a row: a literal answers its own class, a call
/// answers any of them, and a column answers what its affinity keeps.
fn data_type(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    sides: &[Side<'_>],
    collating: &[crate::value::Collating],
) -> u8 {
    arena
        .node(id)
        .map_or(0, |node| class_of(arena, id, node, sql, sides, collating))
}

/// The same for one node the arena holds.
fn class_of(
    arena: &Arena,
    id: ExprId,
    node: Node,
    sql: &[u8],
    sides: &[Side<'_>],
    collating: &[crate::value::Collating],
) -> u8 {
    match node {
        Node::Collate { value, .. }
        | Node::Unary {
            op: UnaryOp::Identity,
            operand: value,
        } => data_type(arena, value, sql, sides, collating),
        Node::Literal(Literal::Null) => 0,
        Node::Literal(Literal::Text(_)) => TEXT,
        Node::Literal(Literal::Blob(_)) => BLOB,
        Node::Binary {
            op: BinaryOp::Concat,
            ..
        } => TEXT | BLOB,
        Node::Variable(_) | Node::Call { .. } | Node::Over { .. } => NUMBER | TEXT | BLOB,
        Node::Column { .. } | Node::Subquery(_) | Node::Cast { .. } | Node::Row(_) => {
            classes_of(compared(arena, id, sql, sides, collating).0)
        }
        Node::Case {
            branches,
            otherwise,
            ..
        } => {
            // A `CASE` answers whatever one of its answers answers,
            // which is every second child and the `ELSE`.
            let mut held = 0;
            for (at, child) in arena.children(branches).iter().enumerate() {
                if at % 2 == 1 {
                    held |= data_type(arena, *child, sql, sides, collating);
                }
            }
            match otherwise {
                Some(child) => held | data_type(arena, child, sql, sides, collating),
                None => held,
            }
        }
        _ => NUMBER,
    }
}

/// What a column of a compound converts before it is compared.
///
/// This is `sqlite3SubqueryColumnTypes`: the affinity is the first
/// core's, or the first core after it that has one, and a core beyond
/// that one which answers a class the affinity would convert takes the
/// affinity away, so a column whose cores are an integer one and a text
/// one converts nothing.
fn compounded(shape: &mut Shape, others: &[Answered]) {
    for (at, column) in shape.columns.iter_mut().enumerate() {
        let mut cores = alloc::vec![(column.affinity, column.datatype)];
        cores.extend(
            others
                .iter()
                .filter_map(|other| other.shape.columns.get(at))
                .map(|held| (held.affinity, held.datatype)),
        );
        let mut classes = 0_u8;
        let mut core = 0_usize;
        let mut held = column.affinity;
        while held == Affinity::None && core.saturating_add(1) < cores.len() {
            classes |= cores.get(core).map_or(0, |(_, held)| *held);
            core = core.saturating_add(1);
            held = cores.get(core).map_or(Affinity::None, |(held, _)| *held);
        }
        column.affinity = held;
        if held < Affinity::Text {
            continue;
        }
        for (_, datatype) in cores.iter().skip(core.saturating_add(1)) {
            classes |= *datatype;
        }
        let taken = if held == Affinity::Text {
            classes & NUMBER != 0
        } else {
            classes & TEXT != 0
        };
        if taken {
            column.affinity = Affinity::Blob;
        }
    }
}
