// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A database that answers a statement.
//!
//! The file is read where it lies, its schema is read out of the
//! `CREATE` text `sqlite_schema` holds, and a statement is answered by
//! walking a table's tree once: O(n) in its rows for a scan, and O(n log
//! n) where an `ORDER BY` has to sort them. Nothing is compiled and no
//! index is used yet, so the rows are right and the plan is not — which
//! is the order document 16 puts them in.
//!
//! What is answered is one `SELECT` over one table, or over none. Every
//! other shape refuses by name rather than answering something near it.

use alloc::vec::Vec;

use crate::ast::{
    Arena, Distinct, ExprId, Literal, Node, Order, ResultColumn, Select, SelectId, SourceKind,
    Span, UnaryOp,
};
use crate::eval::{self, evaluate_collated, evaluate_row};
use crate::header::Encoding;
use crate::image::Image;
use crate::parse;
use crate::record;
use crate::schema::{self, Generated, Table};
use crate::value::{Affinity, Collation, Value, compare};
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
    /// A table whose rows live in the key's own tree. Reading one is
    /// reading an index, which is a later step.
    WithoutRowid,
    /// A shape of statement this engine does not answer yet: a join, a
    /// grouping, a compound, a statement inside a statement.
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
        let image = Image::open(bytes)?;
        let encoding = image.header().encoding;
        let mut tables = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(&image, &row.payload, &mut payload)?;
            let record = record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(record::Value::Text(bytes)) => decode(bytes, encoding),
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
            tables.push(Stored {
                table,
                root: u32::try_from(root).unwrap_or(0),
            });
        }
        Ok(Database {
            image,
            tables,
            encoding,
        })
    }

    /// The tables of the schema, in the order the file holds them.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.tables.iter().map(|stored| &stored.table)
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
        let (arena, root) = parse::statement(sql)?;
        self.select(&arena, root, sql)
    }

    /// One statement of a parsed tree.
    fn select(&self, arena: &Arena, id: SelectId, sql: &[u8]) -> Result<Answer, Error> {
        let select = arena.select(id).ok_or(Error::Unsupported)?;
        refuse_what_is_not_written_yet(arena, &select)?;
        let sources = arena.sources(select.from);
        let stored = match sources.first() {
            None => None,
            Some(source) => {
                let SourceKind::Table {
                    schema: None, name, ..
                } = source.kind
                else {
                    return Err(Error::Unsupported);
                };
                Some(self.find(name.text(sql)).ok_or(Error::NoTable)?)
            }
        };
        let alias = sources
            .first()
            .and_then(|source| source.alias)
            .map(|span| span.text(sql).to_vec());
        let names = names(arena, &select, sql, stored)?;
        let keys = keys(arena, &select, sql, &names)?;
        let mut rows: Vec<Sorted> = Vec::new();
        match stored {
            None => {
                let cursor = Cursor::none(self.collation(), self.encoding);
                if keep(arena, &select, sql, &cursor)? {
                    rows.push(sorted(arena, &select, sql, &cursor, &keys)?);
                }
            }
            Some(stored) => {
                if stored.table.without_rowid {
                    return Err(Error::WithoutRowid);
                }
                if stored
                    .table
                    .columns
                    .iter()
                    .any(|column| column.generated == Generated::Virtual)
                {
                    // A virtual column is not in the row; computing one
                    // is a later step.
                    return Err(Error::Unsupported);
                }
                let mut payload = Vec::new();
                for row in self.image.rows(stored.root) {
                    let row = row?;
                    read_payload(&self.image, &row.payload, &mut payload)?;
                    let cursor = Cursor {
                        table: Some(&stored.table),
                        alias: alias.as_deref(),
                        collation: self.collation(),
                        encoding: self.encoding,
                        values: values_of(&payload, &stored.table, row.rowid, self.encoding)?,
                        rowid: row.rowid,
                    };
                    if keep(arena, &select, sql, &cursor)? {
                        rows.push(sorted(arena, &select, sql, &cursor, &keys)?);
                    }
                }
            }
        }
        if !keys.is_empty() {
            rows.sort_by(|left, right| order_of(&left.keys, &right.keys, &keys));
        }
        let mut rows: Vec<Vec<Value>> = rows.into_iter().map(|row| row.values).collect();
        if select.distinct == Distinct::Distinct {
            let mut seen: Vec<Vec<Value>> = Vec::new();
            let collation = self.collation();
            rows.retain(|row| {
                let fresh = !seen.iter().any(|kept| same(kept, row, collation));
                if fresh {
                    seen.push(row.clone());
                }
                fresh
            });
        }
        limit(arena, &select, sql, &mut rows)?;
        Ok(Answer { names, rows })
    }
}

/// The name SQLite gives each answered column.
fn names(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    stored: Option<&Stored>,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut names = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            ResultColumn::Star | ResultColumn::TableStar(_) => {
                let table = stored.ok_or(Error::NoTable)?;
                for column in &table.table.columns {
                    names.push(column.name.clone());
                }
            }
            ResultColumn::Expr { expr, alias, text } => names.push(match alias {
                Some(span) => span.text(sql).to_vec(),
                // With no name written, a column answers under its
                // own name and everything else under the text it was
                // written as.
                None => match column_named(arena, expr) {
                    Some(name) => answered_name(name.text(sql), stored),
                    None => text.text(sql).to_vec(),
                },
            }),
        }
    }
    Ok(names)
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
            ResultColumn::Star | ResultColumn::TableStar(_) => {
                for value in &cursor.values {
                    out.push(value.clone());
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
            Key::Place(at, _) => (
                values.get(*at).cloned().unwrap_or(Value::Null),
                cursor.collation,
            ),
            Key::Expr(expr, _) => evaluate_collated(arena, *expr, sql, cursor)?,
        });
    }
    Ok(Sorted { values, keys: sort })
}

/// What each `ORDER BY` term sorts by: a place in the answer, or an
/// expression over the row.
fn keys(arena: &Arena, select: &Select, sql: &[u8], names: &[Vec<u8>]) -> Result<Vec<Key>, Error> {
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
            keys.push(Key::Place(at, descending));
            continue;
        }
        if let Some(name) = column_named(arena, term.expr) {
            let name = name.text(sql);
            if let Some(at) = names
                .iter()
                .position(|answered| answered.eq_ignore_ascii_case(name))
            {
                keys.push(Key::Place(at, descending));
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
) -> Result<(), Error> {
    let Some(limit) = select.limit else {
        return Ok(());
    };
    let count = evaluate_row(arena, limit.count, sql, &eval::NoRow)?.to_integer();
    let skip = match limit.offset {
        None => 0,
        Some(offset) => evaluate_row(arena, offset, sql, &eval::NoRow)?.to_integer(),
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

/// Whether two rows hold the same values, which is what `DISTINCT` asks.
fn same(left: &[Value], right: &[Value], collation: Collation) -> bool {
    // Two rows of one answer always hold the same number of values.
    left.iter()
        .zip(right)
        .all(|(first, second)| compare(first, second, collation) == core::cmp::Ordering::Equal)
}

/// Whether the `WHERE` clause keeps this row.
fn keep(arena: &Arena, select: &Select, sql: &[u8], cursor: &Cursor<'_>) -> Result<bool, Error> {
    let Some(filter) = select.filter else {
        return Ok(true);
    };
    Ok(evaluate_row(arena, filter, sql, cursor)?.truth(false))
}

/// The shapes this engine does not answer.
fn refuse_what_is_not_written_yet(arena: &Arena, select: &Select) -> Result<(), Error> {
    if arena.sources(select.from).len() > 1
        || select.compound.is_some()
        || select.having.is_some()
        || !select.group.is_empty()
        || !select.values.is_empty()
        || !select.ctes.is_empty()
    {
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// The name a column reference is answered under: the column's own
/// name where the table has one, and `rowid` for the three names the
/// key answers to.
fn answered_name(written: &[u8], stored: Option<&Stored>) -> Vec<u8> {
    let Some(stored) = stored else {
        return written.to_vec();
    };
    let table = &stored.table;
    if let Some(column) = table
        .columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(written))
    {
        return column.name.clone();
    }
    // A key that is the rowid answers under its own name, whichever of
    // the rowid's three names was written.
    table
        .rowid_alias
        .and_then(|at| table.columns.get(at))
        .map_or_else(|| b"rowid".to_vec(), |column| column.name.clone())
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
    /// says so.
    Place(usize, bool),
    /// An expression over the row.
    Expr(ExprId, bool),
}

/// Where two rows stand against each other under the terms.
fn order_of(
    left: &[(Value, Collation)],
    right: &[(Value, Collation)],
    keys: &[Key],
) -> core::cmp::Ordering {
    for ((first, second), key) in left.iter().zip(right).zip(keys) {
        let descending = match key {
            Key::Place(_, descending) | Key::Expr(_, descending) => *descending,
        };
        let order = compare(&first.0, &second.0, first.1);
        if order != core::cmp::Ordering::Equal {
            return if descending { order.reverse() } else { order };
        }
    }
    core::cmp::Ordering::Equal
}

/// The values of one row, with the rowid put where its alias stands.
fn values_of(
    payload: &[u8],
    table: &Table,
    rowid: i64,
    encoding: Encoding,
) -> Result<Vec<Value>, Error> {
    let record = record::Record::parse(payload)?;
    let mut out = Vec::new();
    for (at, column) in table.columns.iter().enumerate() {
        let mut value = match record.value(at)? {
            None | Some(record::Value::Null) => Value::Null,
            Some(record::Value::Int(number)) => Value::Int(number),
            Some(record::Value::Real(number)) => Value::Real(number),
            Some(record::Value::Text(bytes)) => Value::Text(decode(bytes, encoding)),
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
        // own: the key is what it answers.
        if table.rowid_alias == Some(at) {
            value = Value::Int(rowid);
        }
        out.push(value);
    }
    Ok(out)
}

/// What `BINARY` is over text the file keeps in this encoding.
const fn binary_of(encoding: Encoding) -> Collation {
    match encoding {
        Encoding::Utf8 => Collation::Binary,
        Encoding::Utf16Le => Collation::Binary16Le,
        Encoding::Utf16Be => Collation::Binary16Be,
    }
}

/// Text as the engine holds it, which is UTF-8 whatever the file keeps.
fn decode(bytes: &[u8], encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => bytes.to_vec(),
        Encoding::Utf16Le => crate::utf8::from_utf16(bytes, false),
        Encoding::Utf16Be => crate::utf8::from_utf16(bytes, true),
    }
}

/// Where a walk stands: one row of one table, with its values read.
struct Cursor<'a> {
    /// The table, where the statement names one.
    table: Option<&'a Table>,
    /// The name the statement gave it, where it gave one.
    alias: Option<&'a [u8]>,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// The rowid of the row.
    rowid: i64,
    /// Its values, one per column.
    values: Vec<Value>,
}

impl Cursor<'_> {
    /// A cursor over no table, which is a statement with no `FROM`.
    const fn none(collation: Collation, encoding: Encoding) -> Self {
        Cursor {
            table: None,
            alias: None,
            collation,
            encoding,
            rowid: 0,
            values: Vec::new(),
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

    fn column(&self, table: Option<&[u8]>, column: &[u8]) -> Option<(Value, Affinity, Collation)> {
        let mine = self.table?;
        if let Some(named) = table {
            let matches = self.alias.map_or_else(
                || mine.name.eq_ignore_ascii_case(named),
                |alias| alias.eq_ignore_ascii_case(named),
            );
            if !matches {
                return None;
            }
        }
        let at = mine
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(column));
        let Some(at) = at else {
            // The three names the rowid answers to. A table with no
            // rowid is refused before a row of it is read, so there is
            // nothing to rule out here.
            let rowid = column.eq_ignore_ascii_case(b"rowid")
                || column.eq_ignore_ascii_case(b"oid")
                || column.eq_ignore_ascii_case(b"_rowid_");
            if rowid {
                return Some((Value::Int(self.rowid), Affinity::Integer, Collation::Binary));
            }
            return None;
        };
        let held = mine.columns.get(at)?;
        let value = self.values.get(at).cloned().unwrap_or(Value::Null);
        Some((value, held.affinity, held.collation))
    }
}
