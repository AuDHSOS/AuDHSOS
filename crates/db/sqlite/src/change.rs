// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Statements that change what a database holds.
//!
//! [`Writer`] is a connection with a write transaction open: it holds
//! the pages of the database and the header beside them, runs one
//! statement per call, and answers the file those statements made.
//!
//! A statement is read by `crate::parse`, its rows are answered by
//! `crate::db` over the file as it stands, and the rows go in through
//! `crate::tree`. Each statement is its own transaction, which is what
//! a connection outside `BEGIN` does.

use alloc::vec::Vec;

use crate::ast::{Arena, Change, Definition, Span, TableBody};
use crate::db::{Database, Error};
use crate::header::{Encoding, Header, LIBRARY_VERSION};
use crate::page::Kind;
use crate::schema::Table;
use crate::tree::{Pages, insert, largest};
use crate::value::{Affinity, Value};

/// The columns of `sqlite_schema`, which every row of it is written
/// with: `type`, `name`, `tbl_name`, `rootpage` and `sql`.
const SCHEMA: [Affinity; 5] = [
    Affinity::Text,
    Affinity::Text,
    Affinity::Text,
    Affinity::Integer,
    Affinity::Text,
];

/// A database being written, statement by statement.
pub struct Writer {
    /// The pages of it.
    pages: Pages,
    /// The header beside them, which every commit writes again.
    header: Header,
}

impl Writer {
    /// A database of one page, which is what a connection that opened a
    /// file that was not there writes.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the page size or the reserved tail breaks.
    pub fn new(page_size: u32, reserved: u8, encoding: Encoding) -> Result<Self, Error> {
        let pages = Pages::new(page_size, reserved)?;
        Ok(Writer {
            pages,
            header: Header {
                page_size,
                write_version: 1,
                read_version: 1,
                reserved,
                change_counter: 0,
                pages: 1,
                freelist: 0,
                freelist_pages: 0,
                schema_cookie: 0,
                schema_format: 0,
                cache_size: 0,
                largest_root: 0,
                encoding,
                user_version: 0,
                incremental_vacuum: 0,
                application_id: 0,
                version_valid_for: 0,
                library_version: LIBRARY_VERSION,
            },
        })
    }

    /// The file the statements so far have made.
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        self.pages.written(&self.header)
    }

    /// Runs one statement, which is one transaction.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read, answer or write.
    pub fn run(&mut self, sql: &[u8]) -> Result<(), Error> {
        self.pages.begin();
        self.header.change_counter = self.header.change_counter.saturating_add(1);
        self.header.version_valid_for = self.header.change_counter;
        if let Ok((arena, definition)) = crate::parse::definition(sql) {
            return self.define(&arena, definition, sql);
        }
        let (arena, change) = crate::parse::change(sql)?;
        let Change::Insert(statement) = change;
        self.insert(&arena, &statement, sql)
    }

    /// `CREATE TABLE`: a page for the tree of the table and a row of
    /// `sqlite_schema` that names it.
    fn define(&mut self, arena: &Arena, definition: Definition, sql: &[u8]) -> Result<(), Error> {
        let Definition::Table(table) = definition else {
            return Err(Error::Unsupported);
        };
        if !matches!(table.body, TableBody::Columns { .. }) {
            return Err(Error::Unsupported);
        }
        let root = self.pages.add(Kind::LeafTable, 0)?;
        let name = crate::schema::dequote(table.name.text(sql));
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                text(b"table"),
                text(&name),
                text(&name),
                Value::Int(i64::from(root)),
                text(statement_text(sql)),
            ],
            &SCHEMA,
            4,
        );
        let _ = arena;
        let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?.unwrap_or(0);
        insert(
            &mut self.pages,
            crate::image::SCHEMA_ROOT,
            rowid.saturating_add(1),
            &row,
        )?;
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        self.header.schema_format = 4;
        Ok(())
    }

    /// `INSERT`: the rows the statement answers, each put in the tree of
    /// the table it names.
    fn insert(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let named: Vec<Vec<u8>> = arena
            .names(statement.columns)
            .iter()
            .map(|span: &Span| crate::schema::dequote(span.text(sql)))
            .collect();
        let (root, alias, affinities, rows) = {
            let bytes = self.written();
            let database = Database::open(&bytes)?;
            let (table, root) = database.table(&name).ok_or(Error::Unsupported)?;
            if table.without_rowid {
                return Err(Error::Unsupported);
            }
            let places = places(table, &named)?;
            let answer = database.rows(arena, statement.select, sql)?;
            let affinities: Vec<Affinity> =
                table.columns.iter().map(|column| column.affinity).collect();
            let mut rows = Vec::new();
            for row in &answer.rows {
                if row.len() != places.len() {
                    return Err(Error::Unsupported);
                }
                let mut values = alloc::vec![Value::Null; table.columns.len()];
                let mut key = Value::Null;
                for (at, value) in places.iter().zip(row) {
                    match at {
                        Some(at) => {
                            for slot in values.iter_mut().skip(*at).take(1) {
                                *slot = stored(value, self.header.encoding);
                            }
                        }
                        None => key = value.clone(),
                    }
                }
                rows.push((key, values));
            }
            (root, table.rowid_alias, affinities, rows)
        };
        let mut next = largest(&self.pages, root)?.unwrap_or(0);
        for (key, mut values) in rows {
            let given = match key {
                Value::Int(given) => Some(given),
                Value::Null => alias.and_then(|at| match values.get(at) {
                    Some(Value::Int(given)) => Some(*given),
                    _ => None,
                }),
                _ => return Err(Error::Unsupported),
            };
            let rowid = given.unwrap_or_else(|| next.saturating_add(1));
            next = next.max(rowid);
            // The column the rowid is another name for is stored as
            // nothing, because the key carries it.
            for slot in values.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
                *slot = Value::Null;
            }
            let record = crate::record::write(&values, &affinities, 4);
            insert(&mut self.pages, root, rowid, &record)?;
        }
        Ok(())
    }
}

/// Where each value of a row belongs among the columns of the table, or
/// nothing where the value is the key of the row.
///
/// `rowid`, `oid` and `_rowid_` name the key of a table that has no
/// column of that name, which is what section 2.3 of the format calls
/// the rowid and what `sqlite3ColumnIndex` looks for last.
fn places(table: &Table, named: &[Vec<u8>]) -> Result<Vec<Option<usize>>, Error> {
    if named.is_empty() {
        return Ok((0..table.columns.len()).map(Some).collect());
    }
    named
        .iter()
        .map(|name| {
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(name));
            match at {
                Some(at) => Ok(Some(at)),
                None if is_rowid(name) => Ok(None),
                None => Err(Error::Unsupported),
            }
        })
        .collect()
}

/// Whether `name` is one of the three names the key of a table answers
/// to.
fn is_rowid(name: &[u8]) -> bool {
    [b"rowid".as_slice(), b"oid", b"_rowid_"]
        .iter()
        .any(|word| name.eq_ignore_ascii_case(word))
}

/// A value as the database stores it, which turns text into the
/// encoding the file names.
fn stored(value: &Value, encoding: Encoding) -> Value {
    match value {
        Value::Text(bytes) => Value::Text(crate::value::stored(bytes, encoding)),
        other => other.clone(),
    }
}

/// The statement without the semicolon that ends it and without the
/// space around it, which is the text `sqlite_schema` holds.
fn statement_text(sql: &[u8]) -> &[u8] {
    let mut text = sql;
    while text
        .last()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b';')
    {
        text = text.get(..text.len().saturating_sub(1)).unwrap_or_default();
    }
    let mut at = 0;
    while text.get(at).is_some_and(u8::is_ascii_whitespace) {
        at = at.saturating_add(1);
    }
    text.get(at..).unwrap_or_default()
}
