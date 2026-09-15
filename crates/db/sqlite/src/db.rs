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

/// Why a database could not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file is not one this crate reads.
    Image(error::Error),
    /// The statement is not one this crate reads.
    Parse(parse::Error),
    /// A definition in the schema is not one this crate reads.
    Schema(schema::Error),
    /// An expression could not be answered.
    Eval(eval::Error),
    /// A table the statement names is not in the schema.
    NoTable,
    /// An `ORDER BY` that counts to a column the answer does not have.
    OrderRange,
    /// An aggregate where there is nothing to aggregate over: in a
    /// `WHERE`, in a `GROUP BY`, inside another aggregate, or in an
    /// `ORDER BY` of a statement that does not group.
    Aggregate,
    /// A `HAVING` on a statement that groups nothing.
    Having,
    /// Two sides of a compound that answer different numbers of columns.
    Compound,
    /// An `ORDER BY` of a compound that names none of its columns, which
    /// is the only thing one may name.
    OrderMatch,
    /// A `VALUES` whose rows are not all the same width.
    Values,
    /// A join that cannot be made: a `NATURAL` with a condition written
    /// on it as well, or a `USING` that names a column one of the two
    /// tables does not have.
    Join,
    /// A `*` over two tables of one name, every column of which two
    /// tables would answer.
    Ambiguous,
    /// A computed column that names itself, or names a column no table
    /// has.
    Computed,
    /// A `WITH` term that writes more or fewer column names than its
    /// statement answers columns.
    Names,
    /// A statement used as a value, or looked in by an `IN`, that
    /// answers more than the one column either reads.
    Columns,
    /// A `BEGIN` on a connection that already has a transaction open,
    /// which `sqlite3BeginTransaction` refuses.
    Nested,
    /// A `CREATE` of a name the schema already holds, which
    /// `sqlite3StartTable` and `sqlite3CreateIndex` refuse unless the
    /// statement writes `IF NOT EXISTS`.
    Exists,
    /// A `COMMIT` or a `ROLLBACK` on a connection with no transaction
    /// open.
    NoTransaction,
    /// A row that shares a key with one the table already holds, where
    /// the statement said to refuse it and undo what it wrote.
    Unique,
    /// The same, where the statement said to stop where it stands and
    /// keep what it wrote, which is `OE_Fail`.
    Stopped,
    /// A `WITH` term that reads itself and answered more rows than
    /// `RECURSION_ROWS` allows.
    Recursion,
    /// A shape of statement this engine does not answer yet: a `WITH`
    /// term that reads itself under an operator other than `UNION`, a
    /// table-valued function.
    Unsupported,
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
    ForeignMismatch,
}

impl Error {
    /// The text the C library writes for this refusal, which is what a
    /// `catchsql` of SQLite's own test files compares.
    ///
    /// A refusal the C library writes a name into is written here
    /// without it, because this crate's refusals carry no names yet.
    #[must_use]
    pub fn message(&self) -> alloc::string::String {
        use alloc::string::ToString as _;
        match self {
            Error::Foreign => "FOREIGN KEY constraint failed".to_string(),
            Error::ForeignMismatch => "foreign key mismatch".to_string(),
            Error::Unique => "UNIQUE constraint failed".to_string(),
            Error::Nested => "cannot start a transaction within a transaction".to_string(),
            Error::NoTransaction => "cannot commit - no transaction is active".to_string(),
            Error::Recursion => "recursive aggregate queries not supported".to_string(),
            Error::Eval(eval::Error::RowValue) => "row value misused".to_string(),
            other => alloc::format!("{other:?}"),
        }
    }
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

/// One table of the schema, and where its rows are.
#[derive(Clone, Debug)]
struct Stored {
    /// The table as its statement describes it.
    table: Table,
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

/// One index of a table, and where its tree is.
#[derive(Clone, Debug)]
struct Kept {
    /// The index as its statement describes it.
    index: schema::Index,
    /// The page its tree begins at.
    root: u32,
}

/// One column a statement answers, and what a comparison against it
/// does.
#[derive(Clone, Debug)]
struct Column {
    /// Its name.
    name: Vec<u8>,
    /// What is converted before it is compared.
    affinity: Affinity,
    /// How its text is compared, where anything was written about it.
    collation: Option<Collation>,
    /// Which classes the expression it came from may answer, which is
    /// what `sqlite3ExprDataType` reads off it: one bit for a number,
    /// one for text and one for a blob.
    datatype: u8,
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
    /// What the statement calls it: its alias, or the name of the table
    /// or the `WITH` term it reads, or nothing for a statement written
    /// inside the `FROM` with no alias.
    name: Vec<u8>,
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
        /// The values the key begins with.
        key: Vec<Value>,
        /// What each of them compares under.
        collations: Vec<Collation>,
        /// Where the rowid stands in an entry, which is after every
        /// column the index holds.
        rowid_at: usize,
    },
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
            Plan::Keyed { .. } | Plan::Joined { .. } => (None, None),
        }
    }

    /// Whether `named` is what the statement calls this side.
    fn named(&self, named: &[u8]) -> bool {
        !self.name.is_empty() && self.name.eq_ignore_ascii_case(named)
    }

    /// Whether a `*` leaves this side's column of that name out, which
    /// it does for the side a `USING` or a `NATURAL` matched.
    fn hides(&self, column: &[u8]) -> bool {
        self.using
            .iter()
            .any(|name| name.eq_ignore_ascii_case(column))
    }
}

/// The columns a table of the schema answers.
fn shape_of(table: &Table) -> Shape {
    Shape {
        columns: table
            .columns
            .iter()
            .map(|column| Column {
                name: column.name.clone(),
                affinity: column.affinity,
                collation: Some(column.collation),
                datatype: classes_of(column.affinity),
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
}

/// One trigger of the schema: what it is on, and the statement that
/// made it.
#[derive(Clone, Debug)]
pub(crate) struct Trigger {
    /// The name, with its quotes taken off.
    pub(crate) name: Vec<u8>,
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

/// What a statement answered.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    /// The name of each column, as SQLite would name it.
    pub names: Vec<Vec<u8>>,
    /// The rows, each as many values as there are names.
    pub rows: Vec<Vec<Value>>,
}

impl<'a> Database<'a> {
    /// Opens `bytes` and reads its schema.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        Self::read(Image::open(bytes)?)
    }

    /// The same for a database whose newest pages are in its
    /// write-ahead log, which a reader must follow.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open_with_log(bytes: &'a [u8], log: &'a crate::wal::Wal<'a>) -> Result<Self, Error> {
        Self::read(Image::open_with_log(bytes, log)?)
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
        Self::read(Image::open_with_journal(bytes, journal)?)
    }

    /// Reads the schema of an open file.
    fn read(image: Image<'a>) -> Result<Self, Error> {
        let encoding = image.header().encoding;
        let mut tables = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(&image, &row.payload, &mut payload)?;
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
            let Some(stored) = stored_of(sql, u32::try_from(root).unwrap_or(0), encoding)? else {
                continue;
            };
            tables.push(stored);
        }
        // `sqlite_schema` is a table of the schema like any other: it
        // lies on page one and holds the five columns every row of it
        // is written with, which `sqlite3InitOne` builds in memory
        // rather than reading out of a row.
        tables.extend(
            stored_of(SCHEMA_CREATE.to_vec(), crate::image::SCHEMA_ROOT, encoding)
                .ok()
                .flatten(),
        );
        let mut database = Database {
            image,
            tables,
            views: Vec::new(),
            triggers: Vec::new(),
            encoding,
            random: crate::random::Source::default(),
        };
        database.read_indexes()?;
        database.read_views()?;
        database.read_triggers()?;
        Ok(database)
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

    /// Reads the views of the schema.
    ///
    /// A view names a statement rather than a tree, so it carries no
    /// root page; one whose statement this crate cannot read is passed
    /// over, and a statement that names it then answers no table.
    fn read_views(&mut self) -> Result<(), Error> {
        let mut views = Vec::new();
        let mut payload = Vec::new();
        for row in self.image.schema() {
            let row = row?;
            read_payload(&self.image, &row.payload, &mut payload)?;
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
                select: written.select,
                sql,
                arena,
                columns,
            });
        }
        self.views = views;
        Ok(())
    }

    /// Reads every trigger of the schema, keeping the statement that
    /// made each one.
    ///
    /// A trigger this crate cannot read is passed over, so a database
    /// that holds one is read for everything else it holds.
    fn read_triggers(&mut self) -> Result<(), Error> {
        let mut triggers = Vec::new();
        let mut payload = Vec::new();
        for row in self.image.schema() {
            let row = row?;
            read_payload(&self.image, &row.payload, &mut payload)?;
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
                table: schema::dequote(written.table.text(&sql)),
                sql,
                arena,
                written,
            });
        }
        self.triggers = triggers;
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

    /// The rows a view answers, with the shape they carry and the name
    /// the view is known by.
    ///
    /// The statement of a view is answered where the view is named, so
    /// a view over a table reads what the table holds now. A view that
    /// names itself, directly or through another, is stopped by the
    /// count of views the statement is already inside.
    fn viewed(&self, name: &[u8], scope: Scope<'_>) -> Result<Viewed, Error> {
        let view = self
            .views
            .iter()
            .find(|view| view.name.eq_ignore_ascii_case(name))
            .ok_or(Error::NoTable)?;
        if scope.views >= VIEW_DEPTH {
            return Err(Error::Unsupported);
        }
        let inner = Scope {
            terms: &[],
            outer: None,
            views: scope.views.saturating_add(1),
        };
        let mut answered = self.statement(&view.arena, view.select, &view.sql, inner)?;
        // `CREATE VIEW v(a,b) AS ...` answers its columns under the
        // names the definition wrote, and under the statement's own
        // where it wrote none.
        for (column, written) in answered.shape.columns.iter_mut().zip(&view.columns) {
            column.name.clone_from(written);
        }
        Ok(Viewed {
            shape: answered.shape,
            rows: answered.answer.rows,
            name: view.name.clone(),
        })
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
    fn read_indexes(&mut self) -> Result<(), Error> {
        let mut payload = Vec::new();
        for row in self.image.schema() {
            let row = row?;
            read_payload(&self.image, &row.payload, &mut payload)?;
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
                let Some(stored) = self
                    .tables
                    .iter_mut()
                    .find(|stored| stored.table.name.eq_ignore_ascii_case(&over))
                else {
                    continue;
                };
                let index = (0..stored.table.keys.len())
                    .filter_map(|at| schema::own_index(&stored.table, at))
                    .find(|index| index.name.eq_ignore_ascii_case(&name));
                if let Some(index) = index {
                    stored.indexes.push(Kept {
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
            let Some(stored) = self
                .tables
                .iter_mut()
                .find(|stored| stored.table.name.eq_ignore_ascii_case(&over))
            else {
                continue;
            };
            let Ok(index) = schema::index(&arena, &written, &sql, &stored.table) else {
                continue;
            };
            stored.indexes.push(Kept {
                index,
                root: u32::try_from(root).unwrap_or(0),
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

    /// The indexes over the table of `name` this crate holds, each with
    /// the page its tree begins at and the columns it is over.
    ///
    /// An index this crate cannot walk is not one it holds, so a table
    /// may have more indexes on disk than this answers; `has_others`
    /// says so.
    #[must_use]
    pub fn indexes(&self, name: &[u8]) -> Vec<(&schema::Index, u32)> {
        self.find(name)
            .map(|stored| {
                stored
                    .indexes
                    .iter()
                    .map(|kept| (&kept.index, kept.root))
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
        stored
            .table
            .columns
            .iter()
            .map(|column| match column.falls_back {
                Some(expr) => Ok(crate::eval::evaluate(&stored.arena, expr, &stored.sql)?),
                None => Ok(Value::Null),
            })
            .collect()
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
        let stored = self.find(name).ok_or(Error::NoTable)?;
        if stored.table.without_rowid {
            return Err(Error::Unsupported);
        }
        let mut out = Vec::new();
        let mut payload = Vec::new();
        for step in self.walk(stored, (None, None)) {
            let (rowid, held) = step?;
            let rowid = rowid.ok_or(Error::Unsupported)?;
            read_payload(&self.image, &held, &mut payload)?;
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
        let name = if schema_named(name) {
            SCHEMA_TABLE
        } else {
            name
        };
        self.tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
    }

    /// What `sql` answers.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    pub fn query(&self, sql: &[u8]) -> Result<Answer, Error> {
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
        let scope = Scope {
            terms: &[],
            outer: None,
            views: 0,
        };
        Ok(self.statement(&arena, root, sql, scope)?.answer)
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
        if let Some(quick) = quick {
            let rows = crate::check::integrity(self, quick)?
                .into_iter()
                .map(|text| alloc::vec![Value::Text(text)])
                .collect();
            return Ok(Answer {
                names: alloc::vec![name],
                rows,
            });
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
            other => other.read(self.image.header()),
        }
        .ok_or(Error::Unsupported)?;
        let rows = alloc::vec![alloc::vec![value]];
        Ok(Answer {
            names: alloc::vec![name],
            rows,
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
        let mut operators: Vec<Compound> = Vec::new();
        let mut at = Some(id);
        while let Some(id) = at {
            let core = arena.select(id).ok_or(Error::Unsupported)?;
            let mine = self.core(arena, id, sql, false, scope)?;
            if answers
                .first()
                .is_some_and(|first: &Answered| first.answer.names.len() != mine.answer.names.len())
            {
                return Err(Error::Compound);
            }
            answers.push(mine);
            at = core.compound.map(|(operator, next)| {
                operators.push(operator);
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
        let keys = matched(arena, &first, sql, &answer.names, &collations)?;
        if !keys.is_empty() {
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
        planned(arena, select.filter, sql, &mut sides);
        let shape = shape(arena, &select, sql, &sides)?;
        let collations = self.collations(&shape);
        let names: Vec<Vec<u8>> = shape
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        let keys = if whole {
            keys(arena, &select, sql, &names, &collations)?
        } else {
            Vec::new()
        };
        let calls = aggregates(arena, &select, sql)?;
        let overs = overs(arena, &select, sql)?;
        let mut rows: Vec<Sorted> = Vec::new();
        if !overs.is_empty() {
            // A window function reads the rows a statement has already
            // filtered and grouped, so the rows are kept and the window
            // answered over them before the statement answers.
            let mut kept = self.kept(arena, &select, sql, &sides, &calls, reach)?;
            for at in overed(arena, &select, sql, &mut kept, &overs)? {
                let cursor = kept.get(at).ok_or(Error::NoTable)?;
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
        if !keys.is_empty() {
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
            answer: Answer { names, rows },
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
            let (shape, from, name) = match source.kind {
                SourceKind::Table { schema, name, .. } => {
                    if schema.is_some_and(|span| !is_main(&dequote(span.text(sql)))) {
                        // Only the one schema a file holds is readable,
                        // and a name in front of it that is not it names
                        // no table rather than another database.
                        return Err(Error::NoTable);
                    }
                    let written = &dequote(name.text(sql));
                    // A `WITH` term is reached by its bare name; a name
                    // with a schema in front of it is a table.
                    let found = match schema {
                        Some(_) => None,
                        None => scope
                            .terms
                            .iter()
                            .find(|(term, _)| term.eq_ignore_ascii_case(written)),
                    };
                    if let Some((term, answered)) = found {
                        (
                            answered.shape.clone(),
                            Source::Rows(answered.answer.rows.clone()),
                            term.clone(),
                        )
                    } else if let Some(stored) = self.find(written) {
                        (
                            shape_of(&stored.table),
                            Source::Table(stored),
                            stored.table.name.clone(),
                        )
                    } else {
                        // A view names a statement, so the rows are the
                        // ones that statement answers, which is what
                        // `sqlite3SelectExpand` puts in its place.
                        let viewed = self.viewed(written, scope)?;
                        (viewed.shape, Source::Rows(viewed.rows), viewed.name)
                    }
                }
                SourceKind::Select(id) => {
                    let answered = self.statement(arena, id, sql, scope)?;
                    (
                        answered.shape,
                        Source::Rows(answered.answer.rows),
                        Vec::new(),
                    )
                }
                SourceKind::Function { .. } => return Err(Error::Unsupported),
            };
            let written: Vec<Vec<u8>> = arena
                .names(source.using)
                .iter()
                .map(|span| dequote(span.text(sql)))
                .collect();
            let using = if source.join.natural {
                if source.on.is_some() || !written.is_empty() {
                    return Err(Error::Join);
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
                        return Err(Error::Join);
                    }
                }
                written
            };
            out.push(Side {
                shape,
                source: from,
                name: alias.unwrap_or(name),
                kind: source.join.kind,
                on: source.on,
                using,
                plan: Plan::Rows(None, None),
                pushed: Vec::new(),
            });
        }
        Ok(out)
    }

    /// What one statement written inside an expression answers for a
    /// row: `(SELECT ...)` as a value, `EXISTS`, `IN (SELECT ...)` and
    /// `IN table`.
    ///
    /// The statement is answered here and not before, so a correlated
    /// one is answered once per row of the statement that encloses it:
    /// O(n·m) for `n` outer rows and `m` inner ones.
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
                if schema.is_some_and(|span| !is_main(&dequote(span.text(sql)))) {
                    return Err(Error::NoTable);
                }
                let stored = self.find(&dequote(table.text(sql))).ok_or(Error::NoTable)?;
                let side = Side {
                    shape: shape_of(&stored.table),
                    source: Source::Table(stored),
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
        for cte in arena.ctes(select.ctes) {
            let name = dequote(cte.name.text(sql));
            let mine = Scope {
                terms: &out,
                outer: scope.outer,
                views: scope.views,
            };
            // `RECURSIVE` says nothing: a term that reads its own name
            // reads itself whether the word was written or not, which
            // is what `sqlite3WithPush` decides by the name alone.
            let mut answered = match self.recursive(arena, cte, &name, sql, mine)? {
                Some(answered) => answered,
                None => self.statement(arena, cte.select, sql, mine)?,
            };
            renamed(arena, cte.columns, sql, &mut answered)?;
            out.push((name, answered));
        }
        Ok(out)
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
        renamed(arena, cte.columns, sql, &mut answered)?;
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
        let order = matched(arena, &head, sql, &answered.answer.names, &collations)?;
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
                        names: answered.answer.names.clone(),
                        rows: alloc::vec![row],
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
                    return Err(Error::Compound);
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
                return Err(Error::Compound);
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
                cursor
                    .held
                    .push(Held::empty(&before.shape, &before.name, &before.using));
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
        if let Some((root, key, collations, rowid_at)) = sought(&side.plan, cursor)
            // An entry this walk cannot read whole is one it cannot
            // compare, so the descent gives up and the table is scanned:
            // the same rows, and only the cost is not the same.
            && let Ok(walk) = {
                let mut scratch = Vec::new();
                self.image.entries_from(root, &mut |entry| {
                    let order =
                        order_of_entry(&self.image, entry, &key, &collations, self.encoding, &mut scratch)
                            .map_err(|_| crate::error::Error::Overrun)?;
                    Ok(order == core::cmp::Ordering::Less)
                })
            }
        {
            return Feed::Keyed(Box::new(Sought {
                image: self.image,
                stored,
                walk,
                key,
                collations,
                rowid_at,
                encoding: self.encoding,
                collation: self.collation(),
                payload: Vec::new(),
            }));
        }
        // A plan that names an index and could not be walked falls back
        // to the whole tree, which answers the same rows.
        self.scanned(side, stored)
    }

    /// The rows of one table read out of its own tree, which is what a
    /// side with no plan and a side whose plan could not be walked both
    /// answer.
    fn scanned<'f>(&self, side: &'f Side<'f>, stored: &'f Stored) -> Feed<'a, 'f> {
        Feed::Tree(Box::new(Tree {
            image: self.image,
            stored,
            walk: self.walk(stored, side.range()),
            encoding: self.encoding,
            collation: self.collation(),
            payload: Vec::new(),
        }))
    }

    /// The rows of a table, whichever kind of tree holds them, each with
    /// the rowid where the table has one.
    const fn walk(&self, stored: &Stored, range: (Option<i64>, Option<i64>)) -> Walk<'a> {
        if stored.table.without_rowid {
            Walk::Index(self.image.entries(stored.root))
        } else {
            Walk::Table(self.image.rows_between(stored.root, range.0, range.1))
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
            let (rowid, values) = step?;
            let at_row = ordinal;
            ordinal = ordinal.saturating_add(1);
            if spare.is_some_and(|skip| skip.contains(&at_row)) {
                continue;
            }
            cursor.held.push(Held {
                shape: &side.shape,
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
        if spare.is_none() && !any && matches!(side.kind, JoinKind::Left | JoinKind::Full) {
            // A `LEFT JOIN` answers the row on the left once with
            // nothing on the right where nothing on the right matched.
            cursor
                .held
                .push(Held::empty(&side.shape, &side.name, &side.using));
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
                    .map(|side| Held::empty(&side.shape, &side.name, &side.using))
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
            affinity: Affinity::None,
            collation: None,
            datatype: NUMBER,
        });
        names.push(name);
    }
    Ok(Answered {
        answer: Answer { names, rows },
        shape: Shape {
            columns,
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
fn planned(arena: &Arena, filter: Option<ExprId>, sql: &[u8], sides: &mut [Side<'_>]) {
    let terms = filter.map_or_else(Vec::new, |filter| terms_of(arena, filter, sql, sides));
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
                // side, so the first is taken where both are there.
                plan_of(&terms, at, stored).or_else(|| joined(arena, sql, sides, at, stored))
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
fn terms_of(arena: &Arena, filter: ExprId, sql: &[u8], sides: &[Side<'_>]) -> Vec<Bound> {
    let mut out = Vec::new();
    let mut spine = alloc::vec![filter];
    while let Some(id) = spine.pop() {
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
            // A term that names a column is a term about the row, and
            // the row is what is being planned for, so only a value the
            // walk needs no row to read is one it can be held to.
            let Ok(value) = evaluate_row(arena, value, sql, &eval::NoRow) else {
                continue;
            };
            out.push(Bound {
                at,
                reached,
                op,
                value,
            });
        }
    }
    out
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
    if schema.is_some_and(|span| !is_main(&dequote(span.text(sql)))) {
        return None;
    }
    let named = table.map(|span| dequote(span.text(sql)));
    let column = &dequote(column.text(sql));
    let mut found = None;
    for (at, side) in sides.iter().enumerate() {
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
fn plan_of(terms: &[Bound], at: usize, stored: &Stored) -> Option<Plan> {
    for kept in &stored.indexes {
        let first = kept.index.columns.first()?;
        let column = stored.table.columns.get(first.column)?;
        // An index held in another order, or under another collation
        // than the column compares under, answers its entries in an
        // order the terms do not ask about.
        if first.order == crate::ast::Order::Descending || first.collation != column.collation {
            continue;
        }
        let Some(term) = terms.iter().find(|term| {
            term.at == at
                && term.op == BinaryOp::Eq
                && term.reached == Reached::Column(first.column)
                // An index holds no entry a `=` against `NULL` reaches,
                // which is what `NULL = NULL` answering nothing means.
                && term.value != Value::Null
        }) else {
            continue;
        };
        // An entry holds what the table's affinity left of the value, so
        // the key is what that affinity leaves of the bound.
        let mut value = term.value.clone();
        crate::value::apply(&mut value, column.affinity);
        return Some(Plan::Keyed {
            root: kept.root,
            key: alloc::vec![value],
            collations: alloc::vec![first.collation],
            rowid_at: kept.index.columns.len(),
        });
    }
    None
}

/// What one plan holds a walk of an index to: the tree, the values
/// every entry begins with, what each compares under, and where the
/// rowid stands.
///
/// A plan that names no index answers nothing, and so does one whose
/// key a row of the sides before it does not answer, which is what
/// makes the walk fall back to the whole tree.
///
/// Reading one key costs what the expression it is read from costs.
type Sought_ = (u32, Vec<Value>, Vec<Collation>, usize);

fn sought(plan: &Plan, cursor: &Cursor<'_>) -> Option<Sought_> {
    match plan {
        Plan::Rows(_, _) => None,
        Plan::Keyed {
            root,
            key,
            collations,
            rowid_at,
        } => Some((*root, key.clone(), collations.clone(), *rowid_at)),
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
            Some((
                *root,
                alloc::vec![value],
                alloc::vec![*collation],
                *rowid_at,
            ))
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
        let first = kept.index.columns.first()?;
        let column = stored.table.columns.get(first.column)?;
        // An index held in another order, or under another collation
        // than the column compares under, answers its entries in an
        // order the terms do not ask about.
        if first.order == crate::ast::Order::Descending || first.collation != column.collation {
            continue;
        }
        let Some(key) = keyed_by(arena, on, sql, sides, at, first.column) else {
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
        return Err(Error::Columns);
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
    // answers `t1.a` for `a`, whatever `t2` held.
    let theirs = cursor
        .held
        .get(..cursor.held.len().saturating_sub(1))
        .unwrap_or_default()
        .iter()
        .find_map(|held| held.column(name, cursor.collation));
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
    /// What each of them compares under.
    collations: Vec<Collation>,
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
            let mut collation = cursor.collation;
            for (at, id) in arena.children(call.args).iter().enumerate() {
                // The collation of the first argument is the one the
                // aggregate compares under, which `min`, `max` and a
                // `DISTINCT` are what use.
                if at == 0 {
                    let (value, written) = evaluate_collated(arena, *id, sql, cursor)?;
                    collation = written;
                    values.push(value);
                } else {
                    values.push(evaluate_row(arena, *id, sql, cursor)?);
                }
            }
            accumulator.step(&values, collation);
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
                        if !side.hides(&column.name) {
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
    for term in arena.children(select.group) {
        if let Some(place) = whole_number(arena, *term, sql) {
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| Error::OrderRange)?;
            let found = answers.get(at).ok_or(Error::OrderRange)?;
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
            let held = cursor.held.get(side).ok_or(Error::NoTable)?;
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

/// Every aggregate call a statement answers with, and a refusal where
/// one stands somewhere no group has been made yet.
fn aggregates(arena: &Arena, select: &Select, sql: &[u8]) -> Result<Vec<Call>, Error> {
    let mut calls = Vec::new();
    for column in arena.results(select.columns) {
        if let ResultColumn::Expr { expr, .. } = *column {
            gather(arena, expr, sql, &mut calls, false)?;
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
        gather(arena, having, sql, &mut calls, false)?;
    }
    for term in arena.orders(select.order) {
        gather(arena, term.expr, sql, &mut calls, false)?;
    }
    let mut misused = Vec::new();
    for id in select.filter.iter().chain(arena.children(select.group)) {
        gather(arena, *id, sql, &mut misused, false)?;
    }
    if !misused.is_empty() || (!grouped && !calls.is_empty()) {
        return Err(Error::Aggregate);
    }
    Ok(calls)
}

/// Collects the aggregate calls of one expression.
///
/// The walk is as deep as the tree is tall, which the parser has already
/// bounded, so nothing here counts the steps. `inside` is set under an
/// aggregate, where another one is a misuse rather than a call.
fn gather(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    out: &mut Vec<Call>,
    inside: bool,
) -> Result<(), Error> {
    arena
        .node(id)
        .map_or(Ok(()), |node| collect(arena, id, node, sql, out, inside))
}

/// The same for one node the arena holds.
fn collect(
    arena: &Arena,
    id: ExprId,
    node: Node,
    sql: &[u8],
    out: &mut Vec<Call>,
    inside: bool,
) -> Result<(), Error> {
    let mut under = inside;
    if let Node::Call {
        name,
        args,
        distinct,
        star,
        filter,
    } = node
    {
        let count = if star { 0 } else { arena.children(args).len() };
        if let Some(which) = agg::lookup(&dequote(name.text(sql)), count) {
            // `DISTINCT` puts the rows through one column, so there has
            // to be exactly one for them to go through.
            if inside || (distinct && count != 1) {
                return Err(Error::Aggregate);
            }
            out.push(Call {
                id,
                which,
                distinct,
                args,
                filter,
            });
            under = true;
        }
    }
    let mut deeper = Ok(());
    arena.under(node, |child| {
        if deeper.is_ok() {
            deeper = gather(arena, child, sql, out, under);
        }
    });
    deeper
}

/// The columns a statement answers: their names, and what a
/// comparison against each of them does.
fn shape(arena: &Arena, select: &Select, sql: &[u8], sides: &[Side<'_>]) -> Result<Shape, Error> {
    let mut columns = Vec::new();
    for result in arena.results(select.columns) {
        match *result {
            ResultColumn::Star => {
                if sides.is_empty() {
                    return Err(Error::NoTable);
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
                        if !side.hides(&column.name) {
                            columns.push(column.clone());
                        }
                    }
                }
            }
            ResultColumn::TableStar(span) => {
                let called = dequote(span.text(sql));
                let mut named = sides.iter().filter(|side| side.named(&called));
                let side = named.next().ok_or(Error::NoTable)?;
                if named.next().is_some() {
                    return Err(Error::Ambiguous);
                }
                columns.extend(side.shape.columns.iter().cloned());
            }
            ResultColumn::Expr { expr, alias, text } => {
                let name = match alias {
                    Some(span) => dequote(span.text(sql)),
                    // With no name written, a column answers under its
                    // own name and everything else under the text it
                    // was written as.
                    None => match column_named(arena, expr) {
                        Some(name) => answered_name(&dequote(name.text(sql)), sides),
                        None => text.text(sql).to_vec(),
                    },
                };
                let (affinity, collation) = compared(arena, expr, sql, sides);
                columns.push(Column {
                    name,
                    affinity,
                    collation,
                    datatype: data_type(arena, expr, sql, sides),
                });
            }
        }
    }
    Ok(Shape {
        columns,
        keyed: false,
        key: None,
    })
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
                    held.each(|name, value| {
                        if !held.hides(name) {
                            out.push(cursor.coalesced(at, name, value));
                        }
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
) -> Result<Vec<Key>, Error> {
    let mut keys = Vec::new();
    for term in arena.orders(select.order) {
        let descending = term.order == Order::Descending;
        let nulls = term.nulls;
        // A whole number counts the answered columns from one; a
        // name that is one of them names it; anything else is read
        // against the row.
        if let Some(place) = whole_number(arena, term.expr, sql) {
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| Error::OrderRange)?;
            if at >= names.len() {
                return Err(Error::OrderRange);
            }
            keys.push(Key {
                of: Keyed::Place(at, collation_at(collations, at)),
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
                let written = term_collation(arena, term.expr, sql)?;
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
    sql: &[u8],
    answered: &mut Answered,
) -> Result<(), Error> {
    let written = arena.names(columns);
    if written.is_empty() {
        return Ok(());
    }
    if written.len() != answered.answer.names.len() {
        return Err(Error::Names);
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

/// The `ORDER BY` of a compound, whose every term has to count or name
/// a column of the answer: there is no row left to read an expression
/// against.
fn matched(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    names: &[Vec<u8>],
    collations: &[Collation],
) -> Result<Vec<Ordered>, Error> {
    let mut out = Vec::new();
    for key in keys(arena, select, sql, names, collations)? {
        match key.of {
            // A `COLLATE` on the term is what the sort compares under,
            // which `multiSelectOrderBy` reads off the term and not off
            // the column it counts to.
            Keyed::Place(at, collation) => out.push(Ordered {
                at,
                descending: key.descending,
                collation,
                nulls: key.nulls,
            }),
            Keyed::Expr(_) => return Err(Error::OrderMatch),
        }
    }
    Ok(out)
}

/// One table of the schema, read out of the statement that made it,
/// and nothing where the statement makes something else.
///
/// Reading one statement costs O(n) in its bytes.
fn stored_of(sql: Vec<u8>, root: u32, encoding: Encoding) -> Result<Option<Stored>, Error> {
    let (arena, definition) = parse::definition(&sql)?;
    let crate::ast::Definition::Table(written) = definition else {
        return Ok(None);
    };
    let mut table = schema::table(&arena, &written, &sql)?;
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
        root,
        sql,
        arena,
        places,
        indexes: Vec::new(),
    }))
}

/// The name the schema's own table is held under.
const SCHEMA_TABLE: &[u8] = b"sqlite_master";

/// The statement the schema's own table is read from, which is what
/// `sqlite3InitOne` builds it out of.
const SCHEMA_CREATE: &[u8] =
    b"CREATE TABLE sqlite_master(type text,name text,tbl_name text,rootpage int,sql text)";

/// Whether `name` is one of the three the schema's own table answers
/// to.
#[must_use]
pub const fn schema_named(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"sqlite_master")
        || name.eq_ignore_ascii_case(b"sqlite_schema")
        || name.eq_ignore_ascii_case(b"sqlite_temp_master")
        || name.eq_ignore_ascii_case(b"sqlite_temp_schema")
}

/// Whether a schema name is the one a file holds.
const fn is_main(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"main")
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
) -> (Affinity, Option<Collation>) {
    match arena.node(id) {
        Some(Node::Collate { value, name }) => (
            compared(arena, value, sql, sides).0,
            Collation::of_name(&dequote(name.text(sql))),
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
    match arena.node(id)? {
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
fn term_collation(arena: &Arena, id: ExprId, sql: &[u8]) -> Result<Option<Collation>, Error> {
    let Some(Node::Collate { name, .. }) = arena.node(id) else {
        return Ok(None);
    };
    let named = crate::schema::dequote(name.text(sql));
    let collation = Collation::of_name(&named).ok_or(eval::Error::NoCollation)?;
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
    match arena.node(id)? {
        Node::Column { column, .. } => Some(column),
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
            let value = crate::eval::evaluate(&stored.arena, expr, &stored.sql)?;
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
    fn empty(shape: &'a Shape, name: &'a [u8], using: &'a [Vec<u8>]) -> Self {
        Held {
            shape,
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

    /// Calls `each` with the name and the value of every column.
    fn each(&self, mut each: impl FnMut(&[u8], &Value)) {
        for (column, value) in self.shape.columns.iter().zip(&self.values) {
            each(&column.name, value);
        }
    }

    /// What this side answers for `column`, or nothing where it has no
    /// such column. `default` is the collation of a column nothing was
    /// written about.
    fn column(&self, column: &[u8], default: Collation) -> Option<(Value, Affinity, Collation)> {
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
        if schema.is_some_and(|named| !is_main(named)) {
            return None;
        }
        let mut found: Option<(usize, Value, Affinity, Collation)> = None;
        for (at, held) in self.held.iter().enumerate() {
            match table {
                Some(named) if !held.named(named) => continue,
                // A bare name does not reach the side a `USING` or a
                // `NATURAL` matched; the side before it answers.
                None if held.hides(column) => continue,
                _ => {}
            }
            let Some((value, affinity, collation)) = held.column(column, self.collation) else {
                continue;
            };
            if found.is_some() {
                // A name two tables answer is ambiguous, which is a
                // refusal and not a choice.
                return None;
            }
            found = Some((at, value, affinity, collation));
        }
        let Some((at, value, affinity, collation)) = found else {
            // A name no side of this statement answers is the enclosing
            // statement's, which is what makes a statement correlated.
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
        Some((value, affinity, collation))
    }
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
fn overs(arena: &Arena, select: &Select, sql: &[u8]) -> Result<Vec<Over>, Error> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        if let ResultColumn::Expr { expr, .. } = *column {
            gather_overs(arena, expr, sql, &mut out)?;
        }
    }
    for term in arena.orders(select.order) {
        gather_overs(arena, term.expr, sql, &mut out)?;
    }
    Ok(out)
}

/// The same for one expression and everything under it.
fn gather_overs(arena: &Arena, id: ExprId, sql: &[u8], out: &mut Vec<Over>) -> Result<(), Error> {
    arena
        .node(id)
        .map_or(Ok(()), |node| gather_one(arena, id, node, sql, out))
}

/// The same for one node the arena holds.
fn gather_one(
    arena: &Arena,
    id: ExprId,
    node: Node,
    sql: &[u8],
    out: &mut Vec<Over>,
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
        let which = window::lookup(&dequote(name.text(sql)), count)
            .ok_or(Error::Eval(eval::Error::NoFunction))?;
        // `sqlite3WindowRewrite` takes a `FILTER` for an aggregate and
        // refuses one for the eleven built-in window functions, and
        // refuses `DISTINCT` for every one of them.
        if distinct || (filter.is_some() && !which.filtered()) {
            return Err(Error::Aggregate);
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
            deeper = gather_overs(arena, child, sql, out);
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
            .ok_or(Error::Window)?;
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
            if !partition.is_empty() || under.frame.is_some() {
                return Err(Error::Window);
            }
            partition = under.partition;
            if order.is_empty() {
                order = under.order;
            } else if !under.order.is_empty() {
                return Err(Error::Window);
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
fn offset_of(arena: &Arena, sql: &[u8], cursor: &Cursor<'_>, expr: ExprId) -> Result<usize, Error> {
    let value = whole_of(&evaluate_row(arena, expr, sql, cursor)?).ok_or(Error::Frame)?;
    usize::try_from(value).map_err(|_| Error::Frame)
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
) -> Result<window::Edge, Error> {
    Ok(match bound {
        Edge::UnboundedPreceding => window::Edge::Start,
        Edge::CurrentRow => window::Edge::Current,
        Edge::UnboundedFollowing => window::Edge::End,
        Edge::Preceding(expr) => window::Edge::Preceding(offset_of(arena, sql, cursor, expr)?),
        Edge::Following(expr) => window::Edge::Following(offset_of(arena, sql, cursor, expr)?),
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
                    _ => return Err(Error::Frame),
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
            edge_of(arena, sql, cursor, frame.start)?,
            edge_of(arena, sql, cursor, frame.end)?,
            at,
            part.peers.rows(),
        ),
        Frame::Groups => window::groups_frame(
            edge_of(arena, sql, cursor, frame.start)?,
            edge_of(arena, sql, cursor, frame.end)?,
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
    let place = part.rows.get(at).copied().ok_or(Error::NoTable)?;
    let cursor = cursors.get(place).ok_or(Error::NoTable)?;
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
        let read = cursors.get(found).ok_or(Error::NoTable)?;
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
    let place = part.rows.get(at).copied().ok_or(Error::NoTable)?;
    let cursor = cursors.get(place).ok_or(Error::NoTable)?;
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
            let nth = whole_of(&evaluate_row(arena, id, sql, cursor)?).ok_or(Error::Frame)?;
            let place = usize::try_from(nth).map_err(|_| Error::Frame)?;
            if place < 1 {
                return Err(Error::Frame);
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
    let place = part.rows.get(at).copied().ok_or(Error::NoTable)?;
    let cursor = cursors.get(place).ok_or(Error::NoTable)?;
    let frame = frame_rows(arena, sql, cursor, part, at)?;
    let mut accumulator = Accumulator::new(which, false);
    for row in &frame {
        let place = part.rows.get(*row).copied().unwrap_or(0);
        let read = cursors.get(place).ok_or(Error::NoTable)?;
        // A `FILTER` decides which rows of the frame are stepped, which
        // is `sqlite3WindowCodeStep` jumping over the step.
        if !keep(arena, over.filter, sql, read)? {
            continue;
        }
        let mut values = Vec::new();
        let mut collation = read.collation;
        for (at, id) in arena.children(over.args).iter().enumerate() {
            if at == 0 {
                let (value, written) = evaluate_collated(arena, *id, sql, read)?;
                collation = written;
                values.push(value);
            } else {
                values.push(evaluate_row(arena, *id, sql, read)?);
            }
        }
        accumulator.step(&values, collation);
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
            let place = part.rows.get(at).copied().ok_or(Error::NoTable)?;
            let cursor = cursors.get(place).ok_or(Error::NoTable)?;
            let id = *arena.children(over.args).first().ok_or(Error::Frame)?;
            let tiles = whole_of(&evaluate_row(arena, id, sql, cursor)?).ok_or(Error::Frame)?;
            if tiles < 1 {
                return Err(Error::Frame);
            }
            let tiles = usize::try_from(tiles).map_err(|_| Error::Frame)?;
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
fn data_type(arena: &Arena, id: ExprId, sql: &[u8], sides: &[Side<'_>]) -> u8 {
    arena
        .node(id)
        .map_or(0, |node| class_of(arena, id, node, sql, sides))
}

/// The same for one node the arena holds.
fn class_of(arena: &Arena, id: ExprId, node: Node, sql: &[u8], sides: &[Side<'_>]) -> u8 {
    match node {
        Node::Collate { value, .. }
        | Node::Unary {
            op: UnaryOp::Identity,
            operand: value,
        } => data_type(arena, value, sql, sides),
        Node::Literal(Literal::Null) => 0,
        Node::Literal(Literal::Text(_)) => TEXT,
        Node::Literal(Literal::Blob(_)) => BLOB,
        Node::Binary {
            op: BinaryOp::Concat,
            ..
        } => TEXT | BLOB,
        Node::Variable(_) | Node::Call { .. } | Node::Over { .. } => NUMBER | TEXT | BLOB,
        Node::Column { .. } | Node::Subquery(_) | Node::Cast { .. } | Node::Row(_) => {
            classes_of(compared(arena, id, sql, sides).0)
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
                    held |= data_type(arena, *child, sql, sides);
                }
            }
            match otherwise {
                Some(child) => held | data_type(arena, child, sql, sides),
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
