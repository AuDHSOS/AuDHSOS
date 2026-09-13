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
use crate::journal::Mode;
use crate::page::Kind;
use crate::schema::Table;
use crate::tree::{Pages, insert, largest};
use crate::value::{Affinity, Collation, Value};
use crate::wal::Log;

/// The sector a commit writes the journal header into, which is 512 for
/// every device that says a write of one sector cannot damage another,
/// and so for every build with `SQLITE_POWERSAFE_OVERWRITE`.
const SECTOR: u32 = 512;

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
    /// The mode a commit writes the rollback journal under.
    mode: Mode,
    /// The nonce every checksum of the journal begins at, which comes
    /// from SQLite's random source.
    nonce: u32,
    /// How many bytes the header of the journal takes, which is what
    /// the device says a write cannot damage beyond.
    sector: u32,
    /// What the commit of the last statement left beside the file.
    journal: Option<Vec<u8>>,
    /// The log the commits write frames into, where the file is in
    /// write-ahead logging mode.
    log: Option<Log>,
    /// The file as write-ahead logging began, which is what a database
    /// no checkpoint has run over holds.
    origin: Option<Vec<u8>>,
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
            mode: Mode::Delete,
            nonce: 0,
            sector: SECTOR,
            journal: None,
            log: None,
            origin: None,
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

    /// `PRAGMA journal_mode`, with the nonce every checksum of the
    /// journal begins at and the sector its header takes. The five
    /// modes here are the ones that write the database file itself;
    /// [`Writer::logging`] is the sixth.
    pub const fn journalling(&mut self, mode: Mode, nonce: u32, sector: u32) {
        self.mode = mode;
        self.nonce = nonce;
        self.sector = sector;
    }

    /// `PRAGMA auto_vacuum`, which gives the file pointer maps: page two
    /// becomes the first map page, so the first table takes page three.
    ///
    /// The pragma writes the file, so it stands before the first
    /// statement and before [`Writer::logging`].
    pub fn vacuuming(&mut self, incremental: bool) {
        self.pages.vacuums();
        // The pragma is itself a change, and the largest root a file
        // with no table holds is page one.
        self.header.change_counter = 1;
        self.header.version_valid_for = 1;
        self.header.largest_root = 1;
        self.header.incremental_vacuum = u32::from(incremental);
    }

    /// `PRAGMA journal_mode=wal`: the commits write frames into a log
    /// rather than pages into the file, and the file stays as the
    /// pragma left it until a checkpoint runs.
    ///
    /// The two salts come from SQLite's random source.
    pub fn logging(&mut self, salt: (u32, u32)) {
        // The pragma is itself a change, so the file it leaves counts
        // one more and names both versions two. A file that vacuums
        // itself counted the pragma that said so, so the two pragmas
        // count two between them.
        self.header.write_version = 2;
        self.header.read_version = 2;
        self.header.change_counter = self.header.change_counter.saturating_add(1);
        self.header.version_valid_for = self.header.change_counter;
        self.origin = Some(self.pages.written(&self.header));
        self.log = Some(Log::new(self.header.page_size, salt, 0, false));
    }

    /// The file the statements so far have made. In write-ahead logging
    /// mode that is the file the pragma left, because no checkpoint has
    /// written a frame back into it.
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        match &self.origin {
            Some(bytes) => bytes.clone(),
            None => self.pages.written(&self.header),
        }
    }

    /// The file the pages hold, which is what a statement reads its
    /// rows out of, whatever a log beside the file holds.
    fn image(&self) -> Vec<u8> {
        self.pages.written(&self.header)
    }

    /// What the commit of the last statement left beside the file: the
    /// rollback journal the mode keeps, and nothing where the mode
    /// keeps none.
    #[must_use]
    pub fn journal(&self) -> Option<&[u8]> {
        self.journal.as_deref()
    }

    /// The log the commits wrote their frames into, where the file is
    /// in write-ahead logging mode.
    #[must_use]
    pub fn log(&self) -> Option<&[u8]> {
        self.log.as_ref().map(Log::bytes)
    }

    /// Runs one statement, which is one transaction.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read, answer or write.
    pub fn run(&mut self, sql: &[u8]) -> Result<(), Error> {
        // The header as the transaction begins, which is the one the
        // record of page one in the journal holds.
        let was = self.header;
        self.pages.begin();
        if let Ok((arena, definition)) = crate::parse::definition(sql) {
            self.define(&arena, definition, sql)?;
        } else {
            let (arena, change) = crate::parse::change(sql)?;
            match change {
                Change::Insert(statement) => self.insert(&arena, &statement, sql)?,
                Change::Delete(statement) => self.delete(&arena, &statement, sql)?,
                Change::Update(statement) => self.update(&arena, &statement, sql)?,
            }
        }
        // A statement that wrote no page is one the commit has nothing
        // to write for, so the change counter stands where it stood.
        if self.pages.changed() {
            // `autoVacuumCommit`: a file that vacuums itself whole moves
            // the pages at its end into the free pages below them and is
            // cut back before the commit writes anything.
            if self.header.incremental_vacuum == 0 {
                self.pages.vacuum_commit()?;
            }
            self.header.pages = self.pages.count();
            (self.header.freelist, self.header.freelist_pages) = self.pages.freelist();
            // `pager_write_changecounter`: a frame of page one holds
            // the counter the file holds and one, and no checkpoint
            // writes the file, so every commit writes the same counter
            // there. A commit that writes the file itself carries the
            // counter on.
            if let Some(log) = &mut self.log {
                let mut now = self.header;
                now.change_counter = now.change_counter.saturating_add(1);
                now.version_valid_for = now.change_counter;
                log.commit(&self.pages.frames(&now), self.pages.count());
            } else {
                self.header.change_counter = self.header.change_counter.saturating_add(1);
                self.header.version_valid_for = self.header.change_counter;
                let written = self.pages.journal(&was, self.nonce, self.sector);
                self.journal = crate::journal::committed(&written, self.mode);
            }
        }
        Ok(())
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
        // `sqlite3BtreeCreateTable`: the root of a tree is named by no
        // page, and page one holds the largest root the file has.
        self.pages.point(root, crate::tree::Point::Root, 0)?;
        if self.header.largest_root != 0 {
            self.header.largest_root = root;
        }
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
            let bytes = self.image();
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

/// One row of a table, read against the `WHERE` of a statement that
/// changes rows.
struct Held<'a> {
    /// The table it belongs to, because a column may be written with
    /// the table's name before it.
    table: &'a Table,
    /// The value of each column, in the order the table was created
    /// with.
    values: &'a [Value],
    /// The key of the row, which the three names of the key answer.
    rowid: i64,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
}

impl crate::eval::Row for Held<'_> {
    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        if schema.is_some_and(|name| !name.eq_ignore_ascii_case(b"main")) {
            return None;
        }
        if table.is_some_and(|name| !name.eq_ignore_ascii_case(&self.table.name)) {
            return None;
        }
        let at = self
            .table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(column));
        match at {
            Some(at) => {
                let held = self.table.columns.get(at)?;
                let value = self.values.get(at)?.clone();
                Some((value, held.affinity, held.collation))
            }
            None if is_rowid(column) => {
                Some((Value::Int(self.rowid), Affinity::Integer, Collation::Binary))
            }
            None => None,
        }
    }

    fn encoding(&self) -> Encoding {
        self.encoding
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

impl Writer {
    /// `DELETE`: the rows the `WHERE` keeps, taken out of the tree of
    /// the table it names.
    ///
    /// The keys are found first and the rows are taken out after, which
    /// is what `sqlite3DeleteFrom` does with its `RowSet`: a tree
    /// changes under a walk of it.
    fn delete(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Delete,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let (root, keys) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, root) = database.table(&name).ok_or(Error::NoTable)?;
            let rows = database.rows_of(&name)?;
            let mut keys = Vec::new();
            for (rowid, values) in &rows {
                let held = Held {
                    table,
                    values,
                    rowid: *rowid,
                    encoding: self.header.encoding,
                };
                let keep = match statement.filter {
                    None => true,
                    Some(filter) => {
                        crate::eval::evaluate_row(arena, filter, sql, &held)?.truth(false)
                    }
                };
                if keep {
                    keys.push(*rowid);
                }
            }
            (root, keys)
        };
        for key in keys {
            crate::tree::remove(&mut self.pages, root, key)?;
        }
        Ok(())
    }
}

impl Writer {
    /// `UPDATE`: the rows the `WHERE` keeps, written again with the
    /// columns the `SET` names.
    ///
    /// The rows are read first and written after, as a delete reads
    /// them first, because a tree changes under a walk of it. A row
    /// whose key the statement writes comes out and goes in again under
    /// the key it was given; every other row is written where it lies.
    fn update(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let sets = arena.sets(statement.sets);
        let (root, alias, affinities, written) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, root) = database.table(&name).ok_or(Error::NoTable)?;
            let places: Vec<Option<usize>> = sets
                .iter()
                .map(|set| {
                    let column = crate::schema::dequote(set.column.text(sql));
                    match table
                        .columns
                        .iter()
                        .position(|held| held.name.eq_ignore_ascii_case(&column))
                    {
                        Some(at) => Ok(Some(at)),
                        // `rowid` names the column the key is another name
                        // for, where the table has one.
                        None if is_rowid(&column) => Ok(table.rowid_alias),
                        None => Err(Error::Unsupported),
                    }
                })
                .collect::<Result<_, Error>>()?;
            let affinities: Vec<Affinity> =
                table.columns.iter().map(|column| column.affinity).collect();
            let mut written = Vec::new();
            for (rowid, values) in database.rows_of(&name)? {
                let held = Held {
                    table,
                    values: &values,
                    rowid,
                    encoding: self.header.encoding,
                };
                let keep = match statement.filter {
                    None => true,
                    Some(filter) => {
                        crate::eval::evaluate_row(arena, filter, sql, &held)?.truth(false)
                    }
                };
                if !keep {
                    continue;
                }
                let mut next = values.clone();
                let mut key = rowid;
                for (at, set) in places.iter().zip(sets) {
                    let value = crate::eval::evaluate_row(arena, set.value, sql, &held)?;
                    match at {
                        Some(at) => {
                            for slot in next.iter_mut().skip(*at).take(1) {
                                *slot = stored(&value, self.header.encoding);
                            }
                        }
                        None => match value {
                            Value::Int(given) => key = given,
                            _ => return Err(Error::Unsupported),
                        },
                    }
                }
                written.push((rowid, key, next));
            }
            (root, table.rowid_alias, affinities, written)
        };
        for (rowid, mut key, mut values) in written {
            // The column the key is another name for says the key, so a
            // statement that writes that column writes the key, and the
            // row holds no value for that column.
            if let Some(at) = alias {
                let mut given = values.get(at).cloned().unwrap_or(Value::Null);
                crate::value::apply(&mut given, Affinity::Integer);
                match given {
                    Value::Int(number) => key = number,
                    _ => return Err(Error::Unsupported),
                }
                for slot in values.iter_mut().skip(at).take(1) {
                    *slot = Value::Null;
                }
            }
            let record = crate::record::write(&values, &affinities, 4);
            if key == rowid {
                crate::tree::update(&mut self.pages, root, rowid, &record)?;
            } else {
                crate::tree::remove(&mut self.pages, root, rowid)?;
                insert(&mut self.pages, root, key, &record)?;
            }
        }
        Ok(())
    }
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
