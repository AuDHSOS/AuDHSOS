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

/// What `sqlite3AlterFinishAddColumn` refuses to add: a column that is
/// `PRIMARY KEY` or `UNIQUE`, because the index over it would hold the
/// rows the table already has, and one that may not be nothing and
/// falls back to nothing, because the rows it already has hold nothing
/// for it.
fn refused_column(arena: &Arena, asked: &crate::ast::AddColumn) -> Result<(), Error> {
    let mut fallback = false;
    for constraint in arena.column_constraints(asked.column.constraints) {
        match constraint {
            crate::ast::ColumnConstraint::PrimaryKey { .. }
            | crate::ast::ColumnConstraint::Unique(_) => return Err(Error::Unsupported),
            crate::ast::ColumnConstraint::Default { .. } => fallback = true,
            crate::ast::ColumnConstraint::NotNull(_) if !fallback => {
                return Err(Error::Unsupported);
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether a column of a `sqlite_schema` row is the text `wanted`.
const fn is_text(value: Option<crate::record::Value<'_>>, wanted: &[u8]) -> bool {
    matches!(value, Some(crate::record::Value::Text(bytes)) if bytes.eq_ignore_ascii_case(wanted))
}

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
    /// Where `random` and `randomblob` take their bytes from, which
    /// comes from SQLite's random source as well.
    random: crate::random::Source,
    /// What the connection was told for each pragma of
    /// [`crate::pragma::HELD`], where it was told one.
    kept: Vec<Option<i64>>,
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
    /// Whether a statement that changes rows answers how many it
    /// changed, which `PRAGMA count_changes` sets.
    counting: bool,
    /// The header as the open transaction began, where a `BEGIN` opened
    /// one. A connection outside `BEGIN` holds none and commits every
    /// statement of its own.
    began: Option<Header>,
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
            random: crate::random::Source::default(),
            kept: alloc::vec![None; crate::pragma::HELD.len()],
            sector: SECTOR,
            journal: None,
            log: None,
            origin: None,
            counting: false,
            began: None,
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

    /// Where `random` and `randomblob` take their bytes from.
    ///
    /// SQLite seeds `sqlite3_randomness` from the operating system,
    /// which this crate has none of, so the caller says where the bytes
    /// come from and a connection that is not told answers the bytes
    /// nought draws.
    pub const fn randomness(&mut self, seed: u64) {
        self.random = crate::random::Source::new(seed);
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

    /// `ALTER TABLE ... ADD COLUMN`: the statement of the table gains
    /// the column, and the rows it already holds gain nothing.
    ///
    /// `sqlite3AlterFinishAddColumn` writes the column into the text
    /// between the last thing the columns hold and the bracket that
    /// closes them, so a row written before the column answers what the
    /// column falls back to and not a value of its own, which is the
    /// schema format of D-178.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the database holds no such table, and
    /// [`Error::Unsupported`] for a column SQLite refuses to add: one
    /// that is `PRIMARY KEY` or `UNIQUE`, and one that may not be
    /// nothing and falls back to nothing.
    fn add_column(
        &mut self,
        arena: &Arena,
        asked: &crate::ast::AddColumn,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(asked.table.text(sql));
        let written = asked.written.text(sql).to_vec();
        refused_column(arena, asked)?;
        let rowid = self.row_of(&name)?;
        let image = self.image();
        let database = Database::open(&image)?;
        let root = database.table(&name).ok_or(Error::NoTable)?.1;
        let (statement, add_at) = database.written_as(&name).ok_or(Error::NoTable)?;
        // The column goes where `addColOffset` names: in front of the
        // comma the constraints begin after, and in front of the
        // bracket that closes the columns where the table has none.
        let at = add_at.ok_or(Error::Unsupported)?;
        let mut text = statement.get(..at).unwrap_or_default().to_vec();
        text.extend_from_slice(b", ");
        text.extend_from_slice(&written);
        text.extend_from_slice(statement.get(at..).unwrap_or_default());
        // The row keeps its place, its type, its name and its root, and
        // gains the statement the column is written into.
        let value = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                value(b"table"),
                value(&name),
                value(&name),
                Value::Int(i64::from(root)),
                value(&text),
            ],
            &SCHEMA,
            4,
        );
        drop(database);
        crate::tree::update(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// Which row of `sqlite_schema` names the table, which is the row a
    /// statement that changes the table writes again.
    fn row_of(&self, name: &[u8]) -> Result<i64, Error> {
        let bytes = self.image();
        let image = crate::image::Image::open(&bytes)?;
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            if is_text(record.value(0)?, b"table") && is_text(record.value(1)?, name) {
                return Ok(row.rowid);
            }
        }
        Err(Error::NoTable)
    }

    /// `DROP TABLE` and `DROP INDEX`: the rows of `sqlite_schema` that
    /// name it go, and every page of every tree they named goes on the
    /// free list.
    ///
    /// `sqlite3CodeDropTable` writes the rows out first and destroys
    /// the trees after, largest root first, which is what `destroyTable`
    /// does so that a file that vacuums itself moves each root once.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the database holds no such table or
    /// index and the statement did not write `IF EXISTS`, and whatever
    /// reading or freeing a page refuses.
    fn drop_object(&mut self, asked: &crate::ast::Drop, sql: &[u8]) -> Result<(), Error> {
        let name = crate::schema::dequote(asked.name.text(sql));
        let (rowids, mut roots) = self.named(&name, asked.kind)?;
        if rowids.is_empty() {
            return if asked.if_exists {
                Ok(())
            } else {
                Err(Error::NoTable)
            };
        }
        for rowid in rowids {
            crate::tree::remove(&mut self.pages, crate::image::SCHEMA_ROOT, rowid)?;
        }
        roots.sort_unstable();
        for root in roots.into_iter().rev() {
            crate::tree::destroy(&mut self.pages, root)?;
        }
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// The rows of `sqlite_schema` a `DROP` takes out and the roots it
    /// destroys: a table takes its indexes with it, an index takes only
    /// itself.
    fn named(&self, name: &[u8], kind: crate::ast::Dropped) -> Result<(Vec<i64>, Vec<u32>), Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let mut roots: Vec<u32> = Vec::new();
        let mut held = false;
        match kind {
            crate::ast::Dropped::Table => {
                if let Some((_, root)) = database.table(name) {
                    roots.push(root);
                    held = true;
                }
                roots.extend(database.indexes(name).iter().map(|(_, root)| *root));
            }
            crate::ast::Dropped::Index => {
                if let Some((_, root)) = database.index(name) {
                    roots.push(root);
                    held = true;
                }
            }
            // A view names no tree, so what says the database holds one
            // is the row and not a root.
            crate::ast::Dropped::View => held = database.view(name).is_some(),
        }
        if !held {
            return Ok((Vec::new(), roots));
        }
        // The rows to take out are found by name: a table's are its own
        // and every index over it, which `sqlite_schema` names in the
        // column `tbl_name`; an index's is the one row of its own name.
        let image = crate::image::Image::open(&bytes)?;
        let at = if kind == crate::ast::Dropped::Table {
            2
        } else {
            1
        };
        let mut rowids = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            if is_text(record.value(at)?, name) {
                rowids.push(row.rowid);
            }
        }
        Ok((rowids, roots))
    }

    /// `PRAGMA journal_mode=wal` from a statement, which names no salt:
    /// the two the log carries come from SQLite's random source there
    /// and are nought here, because a salt tells one generation of a
    /// log from another and nothing else reads it.
    ///
    /// A file already in that mode stays in it, which is what the
    /// pragma answers for a connection that is already logging.
    fn log_mode(&mut self) {
        if self.log.is_none() {
            self.logging((0, 0));
        }
    }

    /// The file the pages hold, which is what a statement reads its
    /// rows out of, whatever a log beside the file holds.
    fn image(&self) -> Vec<u8> {
        self.pages.written(&self.header)
    }

    /// The header as the pages stand, which is what a pragma answers:
    /// the count of pages and the free list are the pages' own and not
    /// the header's until a commit writes them.
    fn now(&self) -> Header {
        let mut now = self.header;
        let (first, count) = self.pages.freelist();
        now.pages = self.pages.count();
        now.freelist = first;
        now.freelist_pages = count;
        now
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

    /// Runs one statement, which is one transaction, and answers the
    /// rows it makes.
    ///
    /// Only a `PRAGMA` that sets the journal mode answers a row here,
    /// which is the mode it left the connection in; every statement
    /// that writes rows answers none, and a statement that reads them
    /// is what [`crate::db::Database::query`] answers.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read, answer or write.
    pub fn run(&mut self, sql: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        // The header as the transaction begins, which is the one the
        // record of page one in the journal holds.
        if let Ok(asked) = crate::parse::transaction(sql) {
            return self.bound(asked);
        }
        if let Ok(asked) = crate::parse::pragma(sql) {
            return self.pragma(&asked, sql);
        }
        // A statement inside a transaction writes on what the
        // statements before it wrote, so the pages keep what they held
        // when the `BEGIN` ran and not when this statement began.
        let was = self.began.unwrap_or(self.header);
        let mut changed = 0_i64;
        if self.began.is_none() {
            self.pages.begin();
        }
        if let Ok((arena, definition)) = crate::parse::definition(sql) {
            self.define(&arena, definition, sql)?;
        } else {
            let (arena, change) = crate::parse::change(sql)?;
            changed = match change {
                Change::Insert(statement) => self.insert(&arena, &statement, sql)?,
                Change::Delete(statement) => self.delete(&arena, &statement, sql)?,
                Change::Update(statement) => self.update(&arena, &statement, sql)?,
            };
        }
        // A statement inside a transaction is written by the `COMMIT`
        // and not by itself, which is what makes the transaction one
        // unit of work.
        if self.began.is_none() {
            self.commit(&was)?;
        }
        // `PRAGMA count_changes`: a statement that changes rows answers
        // how many it changed, which is one row of one column.
        if self.counting {
            return Ok(alloc::vec![alloc::vec![Value::Int(changed)]]);
        }
        Ok(Vec::new())
    }

    /// `BEGIN`, `COMMIT` and `ROLLBACK`.
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] for a `BEGIN` inside a transaction, and
    /// [`Error::NoTransaction`] for a `COMMIT` or a `ROLLBACK` outside
    /// one.
    fn bound(&mut self, asked: crate::ast::Transaction) -> Result<Vec<Vec<Value>>, Error> {
        match asked {
            crate::ast::Transaction::Begin => {
                if self.began.is_some() {
                    return Err(Error::Nested);
                }
                self.pages.begin();
                self.began = Some(self.header);
            }
            crate::ast::Transaction::Commit => {
                let was = self.began.take().ok_or(Error::NoTransaction)?;
                self.commit(&was)?;
            }
            crate::ast::Transaction::Rollback => {
                let was = self.began.take().ok_or(Error::NoTransaction)?;
                // The pages go back to what they held and the header
                // with them, so the transaction leaves no trace.
                self.pages.rollback();
                self.header = was;
            }
        }
        Ok(Vec::new())
    }

    /// What the transaction wrote, written: the file is cut back where
    /// it vacuums itself, the header counts the pages and the free list
    /// it now has, and the commit leaves a journal or a frame.
    fn commit(&mut self, was: &Header) -> Result<(), Error> {
        // A transaction that wrote no page is one the commit has
        // nothing to write for, so the change counter stands where it
        // stood.
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
                let written = self.pages.journal(was, self.nonce, self.sector);
                self.journal = crate::journal::committed(&written, self.mode);
            }
        }
        Ok(())
    }

    /// `PRAGMA name = value`, which says how the file is written.
    ///
    /// The page size, the encoding and the auto-vacuum setting are what
    /// the first table is written under, so a statement that sets one
    /// after a table is there changes nothing, which is what
    /// `sqlite3Pragma` does for the first two and what leaves the third
    /// to `VACUUM`. A pragma the file does not hold is answered out of
    /// what the connection was told.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a pragma this crate does not write
    /// and for a value it does not name.
    fn pragma(&mut self, asked: &crate::ast::Pragma, sql: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        let name = crate::schema::dequote(asked.name.text(sql));
        let setting = crate::pragma::of_name(&name).ok_or(Error::Unsupported)?;
        let Some(value) = asked.value else {
            // A connection answers a pragma out of what it holds and
            // not out of the file, because a file with no table holds
            // no encoding: `sqlite3Pragma` reads the schema in memory.
            if setting == crate::pragma::Setting::CountChanges {
                return Ok(alloc::vec![alloc::vec![Value::Int(i64::from(
                    self.counting
                ))]]);
            }
            if let crate::pragma::Setting::Held(at) = setting {
                return Ok(alloc::vec![alloc::vec![self.held(at)]]);
            }
            let read = setting.read(&self.now()).ok_or(Error::Unsupported)?;
            return Ok(alloc::vec![alloc::vec![read]]);
        };
        let text = value.text(sql);
        if setting == crate::pragma::Setting::Ignored {
            return Ok(Vec::new());
        }
        if let crate::pragma::Setting::Held(at) = setting {
            return self.keep(at, text);
        }
        // The page size, the encoding and the vacuuming are what the
        // first table was written under, so a statement that sets one
        // after a table is there is refused. The journal mode belongs
        // to the connection and is set whenever `sqlite3PragmaJournalMode`
        // is asked.
        if setting == crate::pragma::Setting::CountChanges {
            self.counting = crate::pragma::truth(text).ok_or(Error::Unsupported)?;
            return Ok(Vec::new());
        }
        if self.header.schema_cookie != 0 && setting != crate::pragma::Setting::JournalMode {
            return Ok(Vec::new());
        }
        match setting {
            crate::pragma::Setting::PageSize => {
                let size = crate::pragma::whole_number(text).ok_or(Error::Unsupported)?;
                self.pages = Pages::new(size, self.header.reserved)?;
                self.header.page_size = size;
            }
            crate::pragma::Setting::Encoding => {
                self.header.encoding =
                    crate::pragma::encoding_of(text).ok_or(Error::Unsupported)?;
            }
            crate::pragma::Setting::AutoVacuum => {
                match crate::pragma::vacuum_of(text).ok_or(Error::Unsupported)? {
                    0 => {}
                    which => self.vacuuming(which == 2),
                }
            }
            crate::pragma::Setting::JournalMode => {
                if crate::pragma::is_log(text) {
                    self.log_mode();
                } else {
                    // A file in write-ahead logging leaves that mode
                    // through a checkpoint, which this crate does not
                    // write.
                    if self.log.is_some() {
                        return Err(Error::Unsupported);
                    }
                    self.mode = crate::pragma::mode_of(text).ok_or(Error::Unsupported)?;
                }
                // The mode the connection is left in is the one row
                // this pragma answers, which no other setting does.
                return Ok(alloc::vec![alloc::vec![Value::Text(
                    crate::schema::dequote(text).to_ascii_lowercase()
                )]]);
            }
            _ => return Err(Error::Unsupported),
        }
        Ok(Vec::new())
    }

    /// What the connection answers for the pragma at `at` of
    /// [`crate::pragma::HELD`], which is what it was told or what a
    /// connection told nothing answers.
    fn held(&self, at: usize) -> Value {
        let fallback = crate::pragma::HELD
            .get(at)
            .map_or(0, |keeps| keeps.fallback);
        let value = self.kept.get(at).copied().flatten().unwrap_or(fallback);
        crate::pragma::kept(at, value)
    }

    /// The pragma at `at` of [`crate::pragma::HELD`] set to what `text`
    /// names, which answers the value it was set to where that pragma
    /// answers one.
    fn keep(&mut self, at: usize, text: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        let value = crate::pragma::keeping(at, text).ok_or(Error::Unsupported)?;
        let keeps = crate::pragma::HELD.get(at);
        if !keeps.is_some_and(|keeps| keeps.fixed) {
            for slot in self.kept.iter_mut().skip(at).take(1) {
                *slot = Some(value);
            }
        }
        if !keeps.is_some_and(|keeps| keeps.answers) {
            return Ok(Vec::new());
        }
        Ok(alloc::vec![alloc::vec![self.held(at)]])
    }

    /// `CREATE TABLE`: a page for the tree of the table and a row of
    /// `sqlite_schema` that names it.
    fn define(&mut self, arena: &Arena, definition: Definition, sql: &[u8]) -> Result<(), Error> {
        let (kind, name, over) = match definition {
            Definition::Drop(asked) => return self.drop_object(&asked, sql),
            Definition::AddColumn(asked) => return self.add_column(arena, &asked, sql),
            Definition::Table(table) => {
                if !matches!(table.body, TableBody::Columns { .. }) {
                    return Err(Error::Unsupported);
                }
                let name = crate::schema::dequote(table.name.text(sql));
                (Some(Kind::LeafTable), name.clone(), name)
            }
            Definition::Index(index) => (
                Some(Kind::LeafIndex),
                crate::schema::dequote(index.name.text(sql)),
                crate::schema::dequote(index.table.text(sql)),
            ),
            // A view holds no row of its own: it names a statement, and
            // the rows are the ones that statement answers.
            Definition::View(view) => {
                let name = crate::schema::dequote(view.name.text(sql));
                (None, name.clone(), name)
            }
        };
        let root = match kind {
            Some(kind) => {
                let root = self.pages.add(kind, 0)?;
                // `sqlite3BtreeCreateTable`: the root of a tree is
                // named by no page, and page one holds the largest root
                // the file has.
                self.pages.point(root, crate::tree::Point::Root, 0)?;
                if self.header.largest_root != 0 {
                    self.header.largest_root = root;
                }
                root
            }
            // A view begins on no page, which `sqlite3EndTable` writes
            // as nought.
            None => 0,
        };
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                text(match kind {
                    Some(Kind::LeafIndex) => b"index".as_slice(),
                    Some(_) => b"table",
                    None => b"view",
                }),
                text(&name),
                text(&over),
                Value::Int(i64::from(root)),
                text(statement_text(sql)),
            ],
            &SCHEMA,
            4,
        );
        let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        // `sqlite3StartTable` writes a record of five noughts and
        // `sqlite3EndTable` writes over it, so the page keeps the bytes
        // of the blank record where the row no longer stands.
        // `sqlite3CreateIndex` writes its row once and has no blank.
        if matches!(definition, Definition::Index(_)) {
            insert(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        } else {
            let blank = crate::record::write(
                &[
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ],
                &SCHEMA,
                4,
            );
            insert(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &blank)?;
            crate::tree::update(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        }
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        self.header.schema_format = 4;
        if let Definition::Index(index) = definition {
            self.fill(arena, &index, sql, root, &over)?;
        }
        Ok(())
    }

    /// The entries a `CREATE INDEX` puts in the tree it just made: one
    /// per row of the table, holding the columns the index is over and
    /// the key of the row they belong to.
    fn fill(
        &mut self,
        arena: &Arena,
        index: &crate::ast::CreateIndex,
        sql: &[u8],
        root: u32,
        over: &[u8],
    ) -> Result<(), Error> {
        let (entries, collations) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, _) = database.table(over).ok_or(Error::Unsupported)?;
            let read = crate::schema::index(arena, index, sql, table)?;
            let collations = collations_of(&read);
            let mut entries = Vec::new();
            for (rowid, values) in database.rows_of(over)? {
                entries.push(entry_of(&read, &values, rowid));
            }
            (entries, collations)
        };
        // `sqlite3VdbeSorterInit`: the entries are sorted before any is
        // written, so the pages fill in the order the entries run and
        // not in the order the rows do.
        let mut entries = entries;
        entries.sort_by(|one, other| order_of_keys(one, other, &collations));
        let affinities = alloc::vec![Affinity::None; collations.len().saturating_add(1)];
        for key in entries {
            let record = crate::record::write(&key, &affinities, 4);
            crate::tree::insert_entry(&mut self.pages, root, &record, &key, &collations, true)?;
        }
        Ok(())
    }

    /// `INSERT`: the rows the statement answers, each put in the tree of
    /// the table it names.
    fn insert(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let named: Vec<Vec<u8>> = arena
            .names(statement.columns)
            .iter()
            .map(|span: &Span| crate::schema::dequote(span.text(sql)))
            .collect();
        let (root, alias, affinities, rows, kept) = {
            let bytes = self.image();
            // Every statement draws from where the connection stands,
            // so two statements of one connection answer `randomblob`
            // differently.
            let database = Database::open(&bytes)?.seeded(self.random.word());
            let (table, root) = database.table(&name).ok_or(Error::Unsupported)?;
            if table.without_rowid {
                return Err(Error::Unsupported);
            }
            let kept = kept_indexes(&database, &name);
            let places = places(table, &named)?;
            // A column the statement names no value for holds what it
            // falls back to, which is nothing where it has no
            // `DEFAULT`.
            let falls_back: Vec<Value> = database
                .defaults(&name)?
                .iter()
                .map(|value| stored(value, self.header.encoding))
                .collect();
            let answer = database.rows(arena, statement.select, sql)?;
            let affinities: Vec<Affinity> =
                table.columns.iter().map(|column| column.affinity).collect();
            let mut rows = Vec::new();
            for row in &answer.rows {
                if row.len() != places.len() {
                    return Err(Error::Unsupported);
                }
                let mut values = falls_back.clone();
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
            (root, table.rowid_alias, affinities, rows, kept)
        };
        let mut next = largest(&self.pages, root)?.unwrap_or(0);
        let mut written = 0_i64;
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
            // The column the rowid is another name for answers the
            // key, which is what an index over that column holds.
            let mut named = values.clone();
            for slot in named.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
                *slot = Value::Int(rowid);
            }
            // The column the rowid is another name for is stored as
            // nothing, because the key carries it.
            for slot in values.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
                *slot = Value::Null;
            }
            let record = crate::record::write(&values, &affinities, 4);
            insert(&mut self.pages, root, rowid, &record)?;
            self.index_row(&kept, &named, rowid)?;
            written = written.saturating_add(1);
        }
        Ok(written)
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
    /// Where `random` and `randomblob` take their bytes from.
    random: &'a crate::random::Source,
}

impl crate::eval::Row for Held<'_> {
    fn random(&self) -> Option<&crate::random::Source> {
        Some(self.random)
    }

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
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let (root, keys, kept) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, root) = database.table(&name).ok_or(Error::NoTable)?;
            let kept = kept_indexes(&database, &name);
            let rows = database.rows_of(&name)?;
            let mut keys = Vec::new();
            for (rowid, values) in &rows {
                let held = Held {
                    table,
                    values,
                    rowid: *rowid,
                    encoding: self.header.encoding,
                    random: &self.random,
                };
                let keep = match statement.filter {
                    None => true,
                    Some(filter) => {
                        crate::eval::evaluate_row(arena, filter, sql, &held)?.truth(false)
                    }
                };
                if keep {
                    keys.push((*rowid, values.clone()));
                }
            }
            (root, keys, kept)
        };
        // `sqlite3GenerateRowDelete` writes the entries out before the
        // row, because the entries are found through the row.
        let taken = i64::try_from(keys.len()).unwrap_or(i64::MAX);
        for (key, values) in keys {
            self.unindex_row(&kept, &values, key)?;
            crate::tree::remove(&mut self.pages, root, key)?;
        }
        Ok(taken)
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
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let sets = arena.sets(statement.sets);
        let (root, alias, affinities, written, kept) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, root) = database.table(&name).ok_or(Error::NoTable)?;
            let kept = kept_indexes(&database, &name);
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
                    random: &self.random,
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
                written.push((rowid, key, values, next));
            }
            (root, table.rowid_alias, affinities, written, kept)
        };
        let changed = i64::try_from(written.len()).unwrap_or(i64::MAX);
        for (rowid, mut key, held, mut values) in written {
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
            // The column the key is another name for answers the key,
            // which is what an index over that column holds.
            let mut named = values.clone();
            for slot in named.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
                *slot = Value::Int(key);
            }
            let record = crate::record::write(&values, &affinities, 4);
            self.unindex_row(&kept, &held, rowid)?;
            if key == rowid {
                crate::tree::update(&mut self.pages, root, rowid, &record)?;
            } else {
                crate::tree::remove(&mut self.pages, root, rowid)?;
                insert(&mut self.pages, root, key, &record)?;
            }
            self.index_row(&kept, &named, key)?;
        }
        Ok(changed)
    }
}

impl Writer {
    /// The entry every index over the table holds for one row, written,
    /// which is what `sqlite3GenerateConstraintChecks` writes beside
    /// the row.
    fn index_row(&mut self, kept: &[Kept], values: &[Value], rowid: i64) -> Result<(), Error> {
        for (index, collations, root) in kept {
            let key = entry_of(index, values, rowid);
            let plain = alloc::vec![Affinity::None; key.len()];
            let entry = crate::record::write(&key, &plain, 4);
            let pages = &mut self.pages;
            let root = *root;
            crate::tree::insert_entry(pages, root, &entry, &key, collations, false)?;
        }
        Ok(())
    }

    /// The entry every index over the table holds for one row, taken
    /// out, which is `sqlite3GenerateRowIndexDelete`.
    fn unindex_row(&mut self, kept: &[Kept], values: &[Value], rowid: i64) -> Result<(), Error> {
        for (index, collations, root) in kept {
            let key = entry_of(index, values, rowid);
            crate::tree::remove_entry(&mut self.pages, *root, &key, collations)?;
        }
        Ok(())
    }
}

/// One index over a table as a statement that writes rows reads it: the
/// index, the collation of each of its columns, and the page its tree
/// begins on.
type Kept = (crate::schema::Index, Vec<Collation>, u32);

/// Every index over the table `name`, read once so that the statement
/// keeps them while it writes the rows the database answered.
fn kept_indexes(database: &Database<'_>, name: &[u8]) -> Vec<Kept> {
    database
        .indexes(name)
        .iter()
        .map(|(index, root)| ((*index).clone(), collations_of(index), *root))
        .collect()
}

/// The entry an index holds for one row: the columns it is over, and
/// the key of the row last, which is what makes its order total.
fn entry_of(index: &crate::schema::Index, values: &[Value], rowid: i64) -> Vec<Value> {
    let mut key: Vec<Value> = index
        .columns
        .iter()
        .map(|column| values.get(column.column).cloned().unwrap_or(Value::Null))
        .collect();
    key.push(Value::Int(rowid));
    key
}

/// The collation each column of an index is held in.
fn collations_of(index: &crate::schema::Index) -> Vec<Collation> {
    index
        .columns
        .iter()
        .map(|column| column.collation)
        .collect()
}

/// Where one index entry stands against another: column by column
/// under the collation each is held in, and the key of the row last,
/// which is what makes the order of an index total.
fn order_of_keys(one: &[Value], other: &[Value], collations: &[Collation]) -> core::cmp::Ordering {
    one.iter()
        .zip(other)
        .enumerate()
        .map(|(at, (mine, theirs))| {
            // The key of the row stands after the columns and is
            // compared as bytes, which is what the default here is.
            let collation = collations.get(at).copied().unwrap_or(Collation::Binary);
            crate::value::compare(mine, theirs, collation)
        })
        .find(|order| *order != core::cmp::Ordering::Equal)
        .unwrap_or(core::cmp::Ordering::Equal)
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
