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
    Arena, BinaryOp, Compound, Distinct, ExprId, JoinKind, Literal, Node, Order, Range,
    ResultColumn, Select, SelectId, SourceKind, Span, UnaryOp,
};
use crate::eval::{self, Used, evaluate_collated, evaluate_compared, evaluate_row};
use crate::header::Encoding;
use crate::image::Image;
use crate::parse;
use crate::record;
use crate::schema::{self, Generated, Table, dequote};
use crate::value::{Affinity, Collation, Value, apply_comparison, compare, compare_affinity};
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
    /// A shape of statement this engine does not answer yet: a `WITH`
    /// written `RECURSIVE`, a table-valued function.
    Unsupported,
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
}

/// The columns a side of a `FROM` answers.
///
/// A table of the schema, a statement written inside the `FROM` and a
/// `WITH` term answer the same question here, so nothing below this
/// asks which of the three it is reading.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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
}

impl Side<'_> {
    /// The rowids the walk of the side's own tree is held to.
    const fn range(&self) -> (Option<i64>, Option<i64>) {
        match self.plan {
            Plan::Rows(first, last) => (first, last),
            Plan::Keyed { .. } => (None, None),
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
    /// What encoding its text is in.
    encoding: Encoding,
}

/// What a statement answered.
#[derive(Clone, Debug, PartialEq)]
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
            let (arena, definition) = parse::definition(&sql)?;
            let crate::ast::Definition::Table(written) = definition else {
                continue;
            };
            let mut table = schema::table(&arena, &written, &sql)?;
            // `BINARY` is the same collation under the same name
            // whatever the encoding, and it answers by the bytes the
            // file holds.
            let binary = binary_of(encoding);
            if binary != Collation::Binary {
                for column in &mut table.columns {
                    if column.collation == Collation::Binary {
                        column.collation = binary;
                    }
                }
            }
            let places = places(&table);
            tables.push(Stored {
                table,
                root: u32::try_from(root).unwrap_or(0),
                sql,
                arena,
                places,
                indexes: Vec::new(),
            });
        }
        let mut database = Database {
            image,
            tables,
            encoding,
        };
        database.read_indexes()?;
        Ok(database)
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
                // statement, and its columns are the constraint's.
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
        self.tables.iter().map(|stored| &stored.table)
    }

    /// The table of `name` and the page its tree begins at, where the
    /// database holds one.
    #[must_use]
    pub fn table(&self, name: &[u8]) -> Option<(&Table, u32)> {
        self.tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
            .map(|stored| (&stored.table, stored.root))
    }

    /// The indexes over the table of `name` this crate holds, each with
    /// the page its tree begins at and the columns it is over.
    ///
    /// An index this crate cannot walk is not one it holds, so a table
    /// may have more indexes on disk than this answers; `has_others`
    /// says so.
    #[must_use]
    pub fn indexes(&self, name: &[u8]) -> Vec<(&schema::Index, u32)> {
        self.tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
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
        let stored = self
            .tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
            .ok_or(Error::NoTable)?;
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
    pub fn rows(&self, arena: &Arena, id: SelectId, sql: &[u8]) -> Result<Answer, Error> {
        let scope = Scope {
            terms: &[],
            outer: None,
        };
        Ok(self.statement(arena, id, sql, scope)?.answer)
    }

    /// What a comparison uses where nothing writes a collation, which
    /// is `BINARY` over the encoding the file keeps its text in.
    const fn collation(&self) -> Collation {
        binary_of(self.encoding)
    }

    /// The table `name` names.
    fn find(&self, name: &[u8]) -> Option<&Stored> {
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
        if let Ok(asked) = parse::pragma(sql) {
            return self.pragma(&asked, sql);
        }
        let (arena, root) = parse::statement(sql)?;
        let scope = Scope {
            terms: &[],
            outer: None,
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
        // A file being read is not being configured, and a pragma the
        // file does not hold has no answer to read out of it.
        if asked.value.is_some() {
            return Err(Error::Unsupported);
        }
        let value = setting
            .read(self.image.header())
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
        let collations = self.collations(&shape);
        for (operator, right) in operators.into_iter().zip(others) {
            combine(operator, &mut answer, right.answer, &collations);
        }
        let keys = matched(arena, &first, sql, &answer.names, &collations)?;
        if !keys.is_empty() {
            sort_by_keys(&mut answer.rows, &keys, &collations);
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
        let mut rows: Vec<Sorted> = Vec::new();
        if calls.is_empty() && select.group.is_empty() {
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
            let alias = source.alias.map(|span| span.text(sql).to_vec());
            let (shape, from, name) = match source.kind {
                SourceKind::Table { schema, name, .. } => {
                    if schema.is_some_and(|span| !is_main(span.text(sql))) {
                        // Only the one schema a file holds is readable,
                        // and a name in front of it that is not it names
                        // no table rather than another database.
                        return Err(Error::NoTable);
                    }
                    let written = name.text(sql);
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
                    } else {
                        let stored = self.find(written).ok_or(Error::NoTable)?;
                        (
                            shape_of(&stored.table),
                            Source::Table(stored),
                            stored.table.name.clone(),
                        )
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
                .map(|span| span.text(sql).to_vec())
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
        };
        match used {
            Used::Value(select) => {
                let answered = self.statement(arena, select, sql, inner)?;
                one(&answered.shape)?;
                // A statement that answers no row answers `NULL`, and
                // one that answers several answers its first.
                Ok(answered
                    .answer
                    .rows
                    .first()
                    .and_then(|first| first.first())
                    .cloned()
                    .unwrap_or(Value::Null))
            }
            Used::Exists(select) => {
                let answered = self.statement(arena, select, sql, inner)?;
                Ok(Value::Int(i64::from(!answered.answer.rows.is_empty())))
            }
            Used::In(value, select, negated) => {
                let answered = self.statement(arena, select, sql, inner)?;
                let column = one(&answered.shape)?;
                contained(
                    arena,
                    value,
                    sql,
                    row,
                    &answered.answer.rows,
                    column,
                    negated,
                )
            }
            Used::InTable(value, schema, table, negated) => {
                if schema.is_some_and(|span| !is_main(span.text(sql))) {
                    return Err(Error::NoTable);
                }
                let stored = self.find(table.text(sql)).ok_or(Error::NoTable)?;
                let side = Side {
                    shape: shape_of(&stored.table),
                    source: Source::Table(stored),
                    name: Vec::new(),
                    kind: JoinKind::Inner,
                    on: None,
                    using: Vec::new(),
                    plan: Plan::Rows(None, None),
                };
                let column = one(&side.shape)?;
                let mut rows = Vec::new();
                for step in self.feed(&side) {
                    rows.push(step?.1);
                }
                contained(arena, value, sql, row, &rows, column, negated)
            }
        }
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
            if select.recursive {
                // A term that reads itself is a later step.
                return Err(Error::Unsupported);
            }
            let mine = Scope {
                terms: &out,
                outer: scope.outer,
            };
            let mut answered = self.statement(arena, cte.select, sql, mine)?;
            let written = arena.names(cte.columns);
            if !written.is_empty() {
                if written.len() != answered.answer.names.len() {
                    return Err(Error::Names);
                }
                for (column, span) in answered.shape.columns.iter_mut().zip(written) {
                    column.name = span.text(sql).to_vec();
                }
                for (name, span) in answered.answer.names.iter_mut().zip(written) {
                    *name = span.text(sql).to_vec();
                }
            }
            out.push((cte.name.text(sql).to_vec(), answered));
        }
        Ok(out)
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
        let mut ignored: Vec<Vec<i64>> = alloc::vec![Vec::new(); sides.len()];
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
            let matched = kept.get(at).map_or(&[][..], Vec::as_slice);
            self.nest(
                sides,
                at,
                &mut cursor,
                arena,
                sql,
                each,
                &mut ignored,
                Some(matched),
            )?;
            cursor.held.clear();
        }
        Ok(())
    }

    /// The rows of one side of a `FROM`, each with the rowid where the
    /// side is a table that keeps one.
    fn feed<'f>(&self, side: &'f Side<'f>) -> Feed<'a, 'f> {
        let stored = match &side.source {
            Source::Rows(rows) => return Feed::Rows(rows.iter()),
            Source::Table(stored) => stored,
        };
        if let Plan::Keyed {
            root,
            key,
            collations,
            rowid_at,
        } = &side.plan
            // An entry this walk cannot read whole is one it cannot
            // compare, so the descent gives up and the table is scanned:
            // the same rows, and only the cost is not the same.
            && let Ok(walk) = {
                let mut scratch = Vec::new();
                self.image.entries_from(*root, &mut |entry| {
                    let order =
                        order_of_entry(&self.image, entry, key, collations, self.encoding, &mut scratch)
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
                rowid_at: *rowid_at,
                encoding: self.encoding,
                collation: self.collation(),
                payload: Vec::new(),
            }));
        }
        // A plan that names an index and could not be walked falls back
        // to the whole tree, which answers the same rows.
        let range = side.range();
        Feed::Tree(Box::new(Tree {
            image: self.image,
            stored,
            walk: self.walk(stored, range),
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
        for step in self.feed(side) {
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
                self.nest(sides, deeper, cursor, arena, sql, each, kept, None)?;
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
    let Some(filter) = filter else {
        return;
    };
    let terms = terms_of(arena, filter, sql, sides);
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
                plan_of(&terms, at, stored)
            }
            Source::Table(_) | Source::Rows(_) => None,
        };
        plans.push(keyed.unwrap_or(Plan::Rows(range.0, range.1)));
    }
    for (side, plan) in sides.iter_mut().zip(plans) {
        side.plan = plan;
    }
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
    if schema.is_some_and(|span| !is_main(span.text(sql))) {
        return None;
    }
    let named = table.map(|span| span.text(sql));
    let column = column.text(sql);
    let mut found = None;
    for (at, side) in sides.iter().enumerate() {
        if named.is_some_and(|named| !side.named(named)) {
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

/// The one column a statement used as a value answers, which is a
/// refusal where it answers any other number.
const fn one(shape: &Shape) -> Result<&Column, Error> {
    match shape.columns.as_slice() {
        [column] => Ok(column),
        _ => Err(Error::Columns),
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
    column: &Column,
    negated: bool,
) -> Result<Value, Error> {
    let (left, left_affinity, written) = evaluate_compared(arena, value, sql, row)?;
    let collation = written.or(column.collation).unwrap_or(row.collation());
    let affinity = compare_affinity(left_affinity, column.affinity);
    let mut unknown = false;
    for held in rows {
        let mut mine = left.clone();
        let mut theirs = held.first().cloned().unwrap_or(Value::Null);
        if mine == Value::Null || theirs == Value::Null {
            unknown = true;
            continue;
        }
        apply_comparison(&mut mine, &mut theirs, affinity);
        if compare(&mine, &theirs, collation) == core::cmp::Ordering::Equal {
            return Ok(Value::Int(i64::from(!negated)));
        }
    }
    if unknown {
        return Ok(Value::Null);
    }
    Ok(Value::Int(i64::from(negated)))
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
    /// The values every entry the walk takes begins with.
    key: &'f [Value],
    /// What each of them compares under.
    collations: &'f [Collation],
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
            self.key,
            self.collations,
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
                let called = span.text(sql);
                for (at, side) in sides
                    .iter()
                    .enumerate()
                    .filter(|(_, side)| side.named(called))
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
            .filter(|name| !holds(sides, name.text(sql)))
            .and_then(|name| aliased(results, sql, name.text(sql)));
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
        } if alias.text(sql).eq_ignore_ascii_case(name) => Some(expr),
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
    } = node
    {
        let count = if star { 0 } else { arena.children(args).len() };
        if let Some(which) = agg::lookup(name.text(sql), count) {
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
                let called = span.text(sql);
                let mut named = sides.iter().filter(|side| side.named(called));
                let side = named.next().ok_or(Error::NoTable)?;
                if named.next().is_some() {
                    return Err(Error::Ambiguous);
                }
                columns.extend(side.shape.columns.iter().cloned());
            }
            ResultColumn::Expr { expr, alias, text } => {
                let name = match alias {
                    Some(span) => span.text(sql).to_vec(),
                    // With no name written, a column answers under its
                    // own name and everything else under the text it
                    // was written as.
                    None => match column_named(arena, expr) {
                        Some(name) => answered_name(name.text(sql), sides),
                        None => text.text(sql).to_vec(),
                    },
                };
                let (affinity, collation) = compared(arena, expr, sql, sides);
                columns.push(Column {
                    name,
                    affinity,
                    collation,
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
                let named = span.text(sql);
                for held in cursor.held.iter().filter(|held| held.named(named)) {
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
        sort.push(match key {
            // The collation of a column the answer holds is the one
            // the column was declared with, which is what
            // `sqlite3ExprCollSeq` reads off the expression the answer
            // came from.
            Key::Place(at, _, collation) => {
                (values.get(*at).cloned().unwrap_or(Value::Null), *collation)
            }
            Key::Expr(expr, _) => evaluate_collated(arena, *expr, sql, cursor)?,
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
        // A whole number counts the answered columns from one; a
        // name that is one of them names it; anything else is read
        // against the row.
        if let Some(place) = whole_number(arena, term.expr, sql) {
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| Error::OrderRange)?;
            if at >= names.len() {
                return Err(Error::OrderRange);
            }
            keys.push(Key::Place(at, descending, collation_at(collations, at)));
            continue;
        }
        // `resolveOrderGroupBy` matches the term against the answered
        // names with any `COLLATE` on it taken off first, so
        // `ORDER BY m COLLATE binary` sorts by the column answered
        // under the name `m` and not by the column of that name.
        if let Some(name) = column_named(arena, uncollated(arena, term.expr)) {
            let name = name.text(sql);
            if let Some(at) = names
                .iter()
                .position(|answered| answered.eq_ignore_ascii_case(name))
            {
                // A `COLLATE` on the term is the collation the sort
                // uses, whatever the column was declared with.
                let written = term_collation(arena, term.expr, sql)?;
                let collation = written.unwrap_or_else(|| collation_at(collations, at));
                keys.push(Key::Place(at, descending, collation));
                continue;
            }
        }
        keys.push(Key::Expr(term.expr, descending));
    }
    Ok(keys)
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

/// Sorts rows by every column and drops the ones another row already
/// holds, which is what each side of a set operator goes through.
fn ordered(rows: &mut Vec<Vec<Value>>, collations: &[Collation]) {
    arrange(rows, collations);
    rows.dedup_by(|left, right| same(left, right, collations));
}

/// Sorts rows by every column, which is the order a set operator
/// answers in.
fn arrange(rows: &mut [Vec<Value>], collations: &[Collation]) {
    let every: Vec<(usize, bool)> = (0..collations.len()).map(|at| (at, false)).collect();
    sort_by_keys(rows, &every, collations);
}

/// Sorts the rows of an answer under terms that each count to a column
/// of it, backwards where the flag says so.
fn sort_by_keys(rows: &mut [Vec<Value>], terms: &[(usize, bool)], collations: &[Collation]) {
    rows.sort_by(|left, right| {
        for (at, descending) in terms.iter().copied() {
            let collation = collations.get(at).copied().unwrap_or(Collation::Binary);
            let first = left.get(at).unwrap_or(&Value::Null);
            let second = right.get(at).unwrap_or(&Value::Null);
            let order = compare(first, second, collation);
            if order != core::cmp::Ordering::Equal {
                return if descending { order.reverse() } else { order };
            }
        }
        core::cmp::Ordering::Equal
    });
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
) -> Result<Vec<(usize, bool)>, Error> {
    let mut out = Vec::new();
    for key in keys(arena, select, sql, names, collations)? {
        match key {
            Key::Place(at, descending, _) => out.push((at, descending)),
            Key::Expr(..) => return Err(Error::OrderMatch),
        }
    }
    Ok(out)
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
            Collation::of_name(name.text(sql)),
        ),
        Some(Node::Column { column, .. }) => sides
            .iter()
            .flat_map(|side| &side.shape.columns)
            .find(|held| held.name.eq_ignore_ascii_case(column.text(sql)))
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
    // the rowid's three names was written.
    sides
        .iter()
        .find_map(|side| side.shape.key.clone())
        .unwrap_or_else(|| b"rowid".to_vec())
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
enum Key {
    /// The column of the answer at this place, backwards where the flag
    /// says so, compared under the collation that column was declared
    /// with.
    Place(usize, bool, Collation),
    /// An expression over the row.
    Expr(ExprId, bool),
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
        let descending = match key {
            Key::Place(_, descending, _) | Key::Expr(_, descending) => *descending,
        };
        let order = compare(&first.0, &second.0, first.1);
        if order != core::cmp::Ordering::Equal {
            return if descending { order.reverse() } else { order };
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
