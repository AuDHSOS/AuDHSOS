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

use crate::ast::{Arena, Change, Conflict, Definition, Span, TableBody, TriggerEvent, TriggerTime};
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

/// The table `ANALYZE` writes its counts into.
const STAT: &[u8] = b"sqlite_stat1";

/// The table a key that counts up is counted in.
const SEQUENCE: &[u8] = b"sqlite_sequence";

/// Refuses a statement that writes the schema's own table, which is
/// `sqlite3SchemaMayNotBeModified`: the table is read and not written,
/// whatever the statement says.
const fn written_to(name: &[u8]) -> Result<(), Error> {
    if crate::db::schema_named(name) {
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// Whether a row of `sqlite_sequence` names the table `wanted`.
fn named_row(values: &[Value], wanted: &[u8]) -> bool {
    matches!(values.first(), Some(Value::Text(text)) if text.eq_ignore_ascii_case(wanted))
}

/// Whether the schema already names a table, an index or a view, which
/// are one namespace; a trigger is named in its own, so a trigger and
/// an index may share a name.
fn names(database: &Database<'_>, name: &[u8]) -> bool {
    database.table(name).is_some()
        || database.index(name).is_some()
        || database.view(name).is_some()
}

/// One index a `REINDEX` writes again.
struct Rebuilt {
    /// The index, as the schema describes it.
    index: crate::schema::Index,
    /// The page its tree begins at.
    root: u32,
    /// The table whose rows it holds the entries of.
    table: Vec<u8>,
}

/// What one `ANALYZE` counts.
struct Analyzed {
    /// The tables, in the order the rows are written for them.
    tables: Vec<Vec<u8>>,
    /// The one index the run is held to, where it names one.
    only: Option<Vec<u8>>,
}

/// The tables one `ANALYZE` counts, and the index it holds to where it
/// names one.
///
/// Reading the schema costs O(n) in its rows.
fn analyzed(database: &Database<'_>, named: Option<&[u8]>) -> Result<Analyzed, Error> {
    let Some(named) = named else {
        // `sqliteHashFirst` walks the tables of the schema with the one
        // made last first.
        let mut tables: Vec<Vec<u8>> = database.tables().map(|table| table.name.clone()).collect();
        tables.reverse();
        return Ok(Analyzed { tables, only: None });
    };
    // `sqlite_schema` is a table of the schema like any other, so a
    // name that begins with `sqlite_` names a table whatever the
    // database holds under it, and the counting passes it over.
    if database.table(named).is_some() || of_the_system(named) {
        return Ok(Analyzed {
            tables: alloc::vec![named.to_vec()],
            only: None,
        });
    }
    let over = database
        .index(named)
        .map(|(index, _)| index.table.clone())
        .ok_or_else(|| Error::NoTable(named.to_vec()))?;
    Ok(Analyzed {
        tables: alloc::vec![over],
        only: Some(named.to_vec()),
    })
}

/// Whether `name` names a table of the system, which is every name the
/// word `sqlite_` begins.
fn of_the_system(name: &[u8]) -> bool {
    name.get(..7)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"sqlite_"))
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
    /// The triggers running now, innermost last, which is what stops a
    /// trigger that reaches itself where `PRAGMA recursive_triggers`
    /// is off.
    running: Vec<Vec<u8>>,
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
    /// Whether the statement running now stopped where it stood, which
    /// is `OE_Fail`: the rows it wrote before that stand, and the
    /// refusal carries the message of the constraint all the same.
    stopped: bool,
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
            running: Vec::new(),
            sector: SECTOR,
            journal: None,
            log: None,
            origin: None,
            counting: false,
            began: None,
            stopped: false,
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
        // The row of the schema was found above, so the table and the
        // text that wrote it are both there.
        let root = database.table(&name).ok_or(Error::NoTable(Vec::new()))?.1;
        let (statement, add_at) = database
            .written_as(&name)
            .ok_or(Error::NoTable(Vec::new()))?;
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
        Err(Error::NoTable(name.to_vec()))
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
                Err(Error::NoTable(name.clone()))
            };
        }
        for rowid in rowids {
            crate::tree::remove(&mut self.pages, crate::image::SCHEMA_ROOT, rowid)?;
        }
        // `sqlite3CodeDropTable` takes the row of `sqlite_sequence`
        // that names the table away with the table.
        if asked.kind == crate::ast::Dropped::Table {
            self.uncount(&name)?;
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
        // `sqlite3SchemaMayNotBeModified`: the schema's own table is
        // read and not written, whatever the statement says.
        if crate::db::schema_named(name) {
            return Err(Error::Unsupported);
        }
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
            // A trigger names no tree either, so the row is what says
            // the database holds one.
            crate::ast::Dropped::Trigger => held = database.trigger(name).is_some(),
        }
        if !held {
            return Ok((Vec::new(), roots));
        }
        // The rows to take out are found by name and by kind: a table's
        // and a view's are every row whose `tbl_name` names it and
        // which is not a trigger, which is what `sqlite3CodeDropTable`
        // writes, and the triggers on it go first, because
        // `sqlite3DropTriggerPtr` runs before that. An index's and a
        // trigger's is the one row of its own name and its own kind.
        let image = crate::image::Image::open(&bytes)?;
        let over = matches!(kind, crate::ast::Dropped::Table | crate::ast::Dropped::View);
        let mut triggers = Vec::new();
        let mut rowids = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            let trigger = is_text(record.value(0)?, b"trigger");
            let named = if over {
                is_text(record.value(2)?, name)
            } else {
                is_text(record.value(1)?, name)
                    && trigger == matches!(kind, crate::ast::Dropped::Trigger)
            };
            if !named {
                continue;
            }
            if over && trigger {
                triggers.push(row.rowid);
            } else {
                rowids.push(row.rowid);
            }
        }
        triggers.extend(rowids);
        Ok((triggers, roots))
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
        // A text of comments alone holds no statement, so it writes no
        // byte and raises no counter of the header.
        if crate::parse::blank(sql) {
            return Ok(Vec::new());
        }
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
        if self.began.is_none() {
            self.pages.begin();
        }
        self.stopped = false;
        let ran = self.ran(sql);
        // A statement that refuses what it was given leaves the file
        // as it found it, which is what `OE_Abort` does: the pages go
        // back to where the transaction of the statement began. A
        // statement inside a transaction is left alone, because the
        // transaction is the unit of work there.
        let changed = match ran {
            Ok(rows) => rows,
            Err(error) => {
                // A statement that stopped where it stood keeps what it
                // wrote, which is `OE_Fail`. `OE_Rollback` undoes the
                // whole transaction, which this crate answers as
                // `OE_Abort`.
                if self.began.is_none() && !self.stopped {
                    self.pages.rollback();
                }
                return Err(error);
            }
        };
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

    /// `ANALYZE`: the rows of `sqlite_stat1`, written over the rows a
    /// run before this one left.
    ///
    /// `ANALYZE` alone counts every table of the schema, and a name
    /// counts the table it names or the table of the index it names.
    /// Counting costs what [`crate::analyze::stats_of`] costs per
    /// table.
    fn analyze(&mut self, asked: &crate::ast::Analyze, sql: &[u8]) -> Result<(), Error> {
        let named = asked
            .name
            .map(|span| crate::schema::dequote(span.text(sql)));
        let (Analyzed { tables, only }, held) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            (
                analyzed(&database, named.as_deref())?,
                database.table(STAT).is_some(),
            )
        };
        // `openStatTable` makes the table where the database holds
        // none, and takes out the rows a run before this one wrote:
        // every row for a whole database, the rows of the table for
        // one table.
        if held {
            let scope = named.as_ref().and_then(|_| tables.first().cloned());
            self.unstat(scope.as_deref())?;
        } else {
            self.ran(b"CREATE TABLE sqlite_stat1(tbl,idx,stat)")?;
        }
        let (root, stats) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (_, root) = database.table(STAT).ok_or(Error::NoTable(Vec::new()))?;
            let mut stats = Vec::new();
            for table in &tables {
                // `analyzeOneTable` counts no table of the system,
                // `sqlite_stat1` itself among them.
                if of_the_system(table) {
                    continue;
                }
                stats.extend(crate::analyze::stats_of(&database, table, only.as_deref())?);
            }
            (root, stats)
        };
        for stat in &stats {
            self.stat_row(root, stat)?;
        }
        Ok(())
    }

    /// The rows of `sqlite_stat1` a run before this one wrote, taken
    /// out: every row where `scope` names no table, and the rows of
    /// that table otherwise.
    ///
    /// Taking `n` rows out costs O(n log n).
    fn unstat(&mut self, scope: Option<&[u8]>) -> Result<(), Error> {
        let wanted = scope.map(|name| crate::value::stored(name, self.header.encoding));
        let (root, held) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (_, root) = database.table(STAT).ok_or(Error::NoTable(Vec::new()))?;
            let held: Vec<i64> = database
                .rows_of(STAT)?
                .iter()
                .filter(|(_, values)| {
                    wanted.as_ref().is_none_or(|name| {
                        matches!(values.first(), Some(Value::Text(text))
                            if text.eq_ignore_ascii_case(name))
                    })
                })
                .map(|(rowid, _)| *rowid)
                .collect();
            (root, held)
        };
        for rowid in held {
            crate::tree::remove(&mut self.pages, root, rowid)?;
        }
        Ok(())
    }

    /// One row of `sqlite_stat1`, written into the tree at `root`.
    ///
    /// Writing one row costs O(log n) in the rows of the table.
    fn stat_row(&mut self, root: u32, stat: &crate::analyze::Stat) -> Result<(), Error> {
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let values = [
            text(&stat.table),
            stat.index.as_deref().map_or(Value::Null, text),
            text(&stat.stat),
        ];
        let record = crate::record::write(&values, &[Affinity::None; 3], 4);
        let rowid = largest(&self.pages, root)?.unwrap_or(0).saturating_add(1);
        insert(&mut self.pages, root, rowid, &record)?;
        Ok(())
    }

    /// One statement that makes something or changes rows, and how
    /// many rows it changed.
    fn ran(&mut self, sql: &[u8]) -> Result<i64, Error> {
        if let Ok((arena, definition)) = crate::parse::definition(sql) {
            self.define(&arena, definition, sql)?;
            return Ok(0);
        }
        if let Ok(asked) = crate::parse::analyze(sql) {
            self.analyze(&asked, sql)?;
            return Ok(0);
        }
        if let Ok(asked) = crate::parse::reindex(sql) {
            self.reindex(&asked, sql)?;
            return Ok(0);
        }
        let (arena, change) = crate::parse::change(sql)?;
        crate::eval::rows_placed(&arena)?;
        match change {
            Change::Insert(statement) => self.insert(&arena, &statement, sql, None),
            Change::Delete(statement) => self.delete(&arena, &statement, sql, None),
            Change::Update(statement) => self.update(&arena, &statement, sql, None),
        }
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
        // The two pragmas that walk the file rather than read its
        // header answer the same rows on either connection.
        let quick = match setting {
            crate::pragma::Setting::Integrity => Some(false),
            crate::pragma::Setting::Quick => Some(true),
            _ => None,
        };
        if setting == crate::pragma::Setting::ForeignKeyList {
            let named = asked
                .value
                .map(|value| crate::schema::dequote(value.text(sql)))
                .unwrap_or_default();
            return self.listed_keys(&named);
        }
        if setting == crate::pragma::Setting::ForeignKeyCheck {
            let named = asked
                .value
                .map(|value| crate::schema::dequote(value.text(sql)));
            return self.checked_keys(named.as_deref());
        }
        if let Some(quick) = quick {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            return Ok(crate::check::integrity(&database, quick)?
                .into_iter()
                .map(|text| alloc::vec![Value::Text(text)])
                .collect());
        }
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

    /// `PRAGMA foreign_key_list(table)`: one row per foreign key of the
    /// table, newest first, which is the order `sqlite3Pragma` reads
    /// the list it built in.
    fn listed_keys(&self, name: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let Some((table, _)) = database.table(name) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (id, key) in table.foreign.iter().rev().enumerate() {
            for (seq, at) in key.columns.iter().enumerate() {
                let child = table
                    .columns
                    .get(*at)
                    .map_or_else(Vec::new, |column| column.name.clone());
                // A key that named no columns of the table it points at
                // answers nothing for them, which is what
                // `sqlite3Pragma` writes where `pFK->aCol[j].zCol` is
                // null.
                let pointed = key.parent.get(seq).cloned();
                out.push(alloc::vec![
                    Value::Int(i64::try_from(id).unwrap_or(0)),
                    Value::Int(i64::try_from(seq).unwrap_or(0)),
                    Value::Text(key.table.clone()),
                    Value::Text(child),
                    pointed.map_or(Value::Null, Value::Text),
                    Value::Text(action_text(key.on_update).to_vec()),
                    Value::Text(action_text(key.on_delete).to_vec()),
                    Value::Text(b"NONE".to_vec()),
                ]);
            }
        }
        Ok(out)
    }

    /// `PRAGMA foreign_key_check`: one row per row that points at no
    /// row, whatever `PRAGMA foreign_keys` says.
    ///
    /// Reading one table costs O(n·m) in its rows and the rows of the
    /// table each key points at.
    fn checked_keys(&self, only: Option<&[u8]>) -> Result<Vec<Vec<Value>>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let held: Vec<Table> = database
            .tables()
            .filter(|table| only.is_none_or(|only| table.name.eq_ignore_ascii_case(only)))
            .cloned()
            .collect();
        let mut out = Vec::new();
        for table in held {
            let name = table.name.clone();
            for (at, key) in table.foreign.iter().enumerate() {
                let Some((parent, _)) = database.table(&key.table) else {
                    continue;
                };
                let places = parent_places(key, parent)?;
                for (rowid, values) in database.rows_of(&name)? {
                    let wanted: Vec<Value> = key
                        .columns
                        .iter()
                        .map(|place| at_place(&table, &values, rowid, *place))
                        .collect();
                    if wanted.contains(&Value::Null) {
                        continue;
                    }
                    if found_parent(&database, &key.table, parent, &places, &wanted)? {
                        continue;
                    }
                    out.push(alloc::vec![
                        Value::Text(name.clone()),
                        Value::Int(rowid),
                        Value::Text(key.table.clone()),
                        Value::Int(i64::try_from(at).unwrap_or(0)),
                    ]);
                }
            }
        }
        Ok(out)
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

    /// `CREATE TABLE name AS <select>`: a table whose columns are the
    /// ones the statement answers, holding the rows it answered.
    ///
    /// `sqlite3ColumnsFromExprList` names the columns and
    /// `sqlite3SubqueryColumnTypes` gives each the type its affinity is
    /// written as, so the statement of the table is built rather than
    /// taken from the text, which is `createTableStmt`.
    ///
    /// Answering the statement costs what the statement costs and
    /// writing `n` rows costs O(n log n).
    ///
    /// # Errors
    ///
    /// [`Error`] names whatever answering the statement refuses.
    fn create_as(
        &mut self,
        arena: &Arena,
        table: &crate::ast::CreateTable,
        select: crate::ast::SelectId,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(table.name.text(sql));
        if self.already(&name, table.if_not_exists)? {
            return Ok(());
        }
        let (answer, affinities) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?.seeded(self.random.word());
            database.answered(arena, select, sql)?
        };
        let columns = crate::schema::columns_from(&answer.names);
        let written = crate::schema::created(&name, &columns, &affinities);
        let root = self.pages.add(Kind::LeafTable, 0)?;
        self.pages.point(root, crate::tree::Point::Root, 0)?;
        if self.header.largest_root != 0 {
            self.header.largest_root = root;
        }
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                text(b"table"),
                text(&name),
                text(&name),
                Value::Int(i64::from(root)),
                text(&written),
            ],
            &SCHEMA,
            4,
        );
        let at = self.schema_blank()?;
        self.schema_written(at, &row)?;
        let mut rowid = 0_i64;
        for row in &answer.rows {
            let values: Vec<Value> = row
                .iter()
                .map(|value| stored(value, self.header.encoding))
                .collect();
            let record = crate::record::write(&values, &affinities, 4);
            rowid = rowid.saturating_add(1);
            insert(&mut self.pages, root, rowid, &record)?;
        }
        Ok(())
    }

    /// `CREATE TRIGGER`: one row of `sqlite_schema` that names the
    /// table it is on and holds the statement that made it.
    ///
    /// `sqlite3FinishTrigger` writes the words `CREATE TRIGGER` and
    /// then the text from the name to the `END`, so the words
    /// `TEMPORARY` and `IF NOT EXISTS` are not in what the file holds.
    /// A trigger holds no row of its own, so its root page is nought.
    ///
    /// Writing one row costs O(log n) in the rows of the schema.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a trigger written `INSTEAD OF`,
    /// which only a view carries and this crate writes no row of, and
    /// for one on a table the database does not hold; [`Error::Nested`]
    /// for a name the database already holds.
    fn create_trigger(
        &mut self,
        trigger: &crate::ast::CreateTrigger,
        sql: &[u8],
    ) -> Result<(), Error> {
        if trigger.time == crate::ast::TriggerTime::InsteadOf {
            return Err(Error::Unsupported);
        }
        let name = crate::schema::dequote(trigger.name.text(sql));
        let over = crate::schema::dequote(trigger.table.text(sql));
        {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            if database.table(&over).is_none() {
                return Err(Error::NoTable(over));
            }
            if database.trigger(&name).is_some() {
                if trigger.if_not_exists {
                    return Ok(());
                }
                return Err(Error::Exists);
            }
        }
        let mut written = b"CREATE TRIGGER ".to_vec();
        written.extend_from_slice(trigger.written.text(sql));
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                text(b"trigger"),
                text(&name),
                text(&over),
                Value::Int(0),
                text(&written),
            ],
            &SCHEMA,
            4,
        );
        let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        self.header.schema_format = 4;
        Ok(())
    }

    /// The triggers of `over` that run on `event` at `time`, in the
    /// order they run.
    ///
    /// The statement that changes rows reads them once and runs them
    /// per row, so one read of the schema costs a statement and not a
    /// row.
    fn triggers_for(
        &self,
        over: &[u8],
        event: crate::ast::TriggerEvent,
        time: crate::ast::TriggerTime,
    ) -> Result<Vec<crate::db::Trigger>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        Ok(database
            .triggers_on(over, event, time)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Runs every trigger of `triggers` over one row, which is
    /// `sqlite3CodeRowTrigger`.
    ///
    /// Answers whether the statement keeps the row: `RAISE(IGNORE)`
    /// passes it over and every other `RAISE` refuses the statement.
    /// `written` is the columns an `UPDATE` writes, which an
    /// `UPDATE OF` is held to.
    ///
    /// # Errors
    ///
    /// [`Error::Eval`] for a `RAISE` other than `IGNORE` and for a
    /// `WHEN` that could not be answered, [`Error::Unsupported`] where
    /// the triggers reach deeper than `TRIGGER_DEPTH`, and whatever a
    /// statement of a body refuses.
    fn fire(
        &mut self,
        triggers: &[crate::db::Trigger],
        written: &[Vec<u8>],
        row: &Fired<'_>,
    ) -> Result<bool, Error> {
        for trigger in triggers {
            if !writes_one(&trigger.arena, &trigger.written, &trigger.sql, written) {
                continue;
            }
            if self.repeats(&trigger.name) {
                continue;
            }
            if self.running.len() >= TRIGGER_DEPTH {
                return Err(Error::Unsupported);
            }
            if let Some(condition) = trigger.written.condition
                && !crate::eval::evaluate_row(&trigger.arena, condition, &trigger.sql, row)?
                    .truth(false)
            {
                continue;
            }
            self.running.push(trigger.name.clone());
            let ran = self.body(trigger, row);
            self.running.pop();
            match ran {
                Err(Error::Eval(crate::eval::Error::Raised(crate::ast::Raise::Ignore))) => {
                    return Ok(false);
                }
                other => other?,
            }
        }
        Ok(true)
    }

    /// Whether the connection holds its rows to the foreign keys they
    /// carry, which `PRAGMA foreign_keys` turns on and which a
    /// connection told nothing leaves off.
    fn holding(&self) -> bool {
        self.told(b"foreign_keys") != 0
    }

    /// What the connection was told for the pragma `name`, or what the
    /// pragma falls back to where it was told nothing.
    fn told(&self, name: &[u8]) -> i64 {
        crate::pragma::HELD
            .iter()
            .position(|keeps| keeps.name == name)
            .and_then(|at| self.kept.get(at).copied().flatten())
            .unwrap_or(0)
    }

    /// Whether every foreign key of a row points at a row that is
    /// there, which is `I.1` of `src/fkey.c`.
    ///
    /// Looking one key up reads the rows of the table it points at, so
    /// a statement that writes n rows into a table with a foreign key
    /// over a table of m rows costs O(n·m).
    fn parented(&self, table: &Table, values: &[Value], rowid: i64) -> Result<(), Error> {
        if !self.holding() || table.foreign.is_empty() {
            return Ok(());
        }
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        for key in &table.foreign {
            let mut wanted = Vec::new();
            for at in &key.columns {
                wanted.push(at_place(table, values, rowid, *at));
            }
            if wanted.contains(&Value::Null) {
                continue;
            }
            let (parent, _) = database.table(&key.table).ok_or(Error::ForeignMismatch)?;
            let places = parent_places(key, parent)?;
            if !found_parent(&database, &key.table, parent, &places, &wanted)? {
                return Err(Error::Foreign);
            }
        }
        Ok(())
    }

    /// What happens to the rows that point at a row the statement takes
    /// away or changes, which is `D.2` and the `UPDATE` half of
    /// `src/fkey.c`.
    ///
    /// `NO ACTION` and `RESTRICT` refuse; `CASCADE` takes the rows away
    /// with it or writes the new key into them; `SET NULL` and
    /// `SET DEFAULT` write that into the columns that point. Reading
    /// the rows that point costs O(m) in the rows of each table that
    /// points at this one.
    fn orphaned(
        &mut self,
        name: &[u8],
        table: &Table,
        old: &[Value],
        rowid: i64,
        new: Option<(&[Value], i64)>,
    ) -> Result<(), Error> {
        if !self.holding() {
            return Ok(());
        }
        for points in self.pointing(name)? {
            let places = {
                let bytes = self.image();
                let database = Database::open(&bytes)?;
                let (parent, _) = database.table(name).ok_or(Error::ForeignMismatch)?;
                parent_places(&points.key, parent)?
            };
            let mut wanted = Vec::new();
            for at in &places {
                wanted.push(at_place(table, old, rowid, *at));
            }
            if wanted.contains(&Value::Null) {
                continue;
            }
            // An `UPDATE` that leaves the columns pointed at alone
            // leaves the rows that point alone as well.
            let after: Option<Vec<Value>> = new.map(|(values, key)| {
                places
                    .iter()
                    .map(|at| at_place(table, values, key, *at))
                    .collect()
            });
            if after.as_ref().is_some_and(|after| *after == wanted) {
                continue;
            }
            let action = if new.is_some() {
                points.key.on_update
            } else {
                points.key.on_delete
            };
            self.acted(&points, &wanted, after.as_deref(), action)?;
        }
        Ok(())
    }

    /// Every foreign key of every table that points at `name`.
    fn pointing(&self, name: &[u8]) -> Result<Vec<Points>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let mut out = Vec::new();
        for table in database.tables() {
            for key in &table.foreign {
                if key.table.eq_ignore_ascii_case(name) {
                    out.push(Points {
                        child: table.name.clone(),
                        key: key.clone(),
                    });
                }
            }
        }
        Ok(out)
    }

    /// What one foreign key says happens to the rows that point at a
    /// row that goes or changes.
    fn acted(
        &mut self,
        points: &Points,
        wanted: &[Value],
        after: Option<&[Value]>,
        action: crate::ast::Action,
    ) -> Result<(), Error> {
        let rows = self.pointing_rows(points, wanted)?;
        if rows.is_empty() {
            return Ok(());
        }
        match action {
            crate::ast::Action::Cascade => {
                for (rowid, values) in rows {
                    match after {
                        None => self.taken_away(&points.child, rowid)?,
                        Some(after) => self.written_over(points, rowid, &values, after)?,
                    }
                }
                Ok(())
            }
            crate::ast::Action::SetNull | crate::ast::Action::SetDefault => {
                let fallback = matches!(action, crate::ast::Action::SetDefault);
                for (rowid, values) in rows {
                    self.written_back(points, rowid, &values, fallback)?;
                }
                Ok(())
            }
            _ => Err(Error::Foreign),
        }
    }

    /// The rows of the table that points whose key is `wanted`.
    fn pointing_rows(
        &self,
        points: &Points,
        wanted: &[Value],
    ) -> Result<Vec<(i64, Vec<Value>)>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let (child, _) = database
            .table(&points.child)
            .ok_or(Error::NoTable(Vec::new()))?;
        let mut out = Vec::new();
        for (rowid, values) in database.rows_of(&points.child)? {
            let held: Vec<Value> = points
                .key
                .columns
                .iter()
                .map(|at| at_place(child, &values, rowid, *at))
                .collect();
            if held.contains(&Value::Null) {
                continue;
            }
            if alike_values(&held, wanted, child, &points.key.columns) {
                out.push((rowid, values));
            }
        }
        Ok(out)
    }

    /// One row of the table that points, taken away with the row it
    /// pointed at, which is `ON DELETE CASCADE`. The rows that point at
    /// that row go with it.
    fn taken_away(&mut self, name: &[u8], rowid: i64) -> Result<(), Error> {
        let (root, kept, table, values) = self.one_row(name, rowid)?;
        self.orphaned(name, &table, &values, rowid, None)?;
        self.unindex_row(&kept, &values, rowid)?;
        crate::tree::remove(&mut self.pages, root, rowid)?;
        Ok(())
    }

    /// One row of the table that points, with the key of the row it
    /// points at written into it, which is `ON UPDATE CASCADE`.
    fn written_over(
        &mut self,
        points: &Points,
        rowid: i64,
        held: &[Value],
        after: &[Value],
    ) -> Result<(), Error> {
        let mut values = held.to_vec();
        for (at, value) in points.key.columns.iter().zip(after) {
            for slot in values.iter_mut().skip(*at).take(1) {
                *slot = value.clone();
            }
        }
        self.rewrite(&points.child, rowid, &values)
    }

    /// One row of the table that points, with nothing or its fallback
    /// written into the columns that point, which is `ON DELETE SET
    /// NULL` and `ON DELETE SET DEFAULT`.
    fn written_back(
        &mut self,
        points: &Points,
        rowid: i64,
        held: &[Value],
        fallback: bool,
    ) -> Result<(), Error> {
        // The fallback of a column is the expression the statement that
        // made the table wrote, which the schema holds beside the arena
        // of that statement.
        let falls_back = if fallback {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            database.defaults(&points.child)?
        } else {
            Vec::new()
        };
        let mut values = held.to_vec();
        for at in &points.key.columns {
            let value = falls_back.get(*at).cloned().unwrap_or(Value::Null);
            for slot in values.iter_mut().skip(*at).take(1) {
                *slot = value.clone();
            }
        }
        self.rewrite(&points.child, rowid, &values)
    }

    /// One row of a table, written again with the values given.
    fn rewrite(&mut self, name: &[u8], rowid: i64, values: &[Value]) -> Result<(), Error> {
        let (root, kept, table, held) = self.one_row(name, rowid)?;
        let affinities: Vec<Affinity> =
            table.columns.iter().map(|column| column.affinity).collect();
        let mut stored = values.to_vec();
        // The column the key is another name for takes no place in the
        // record, which is what `sqlite3TableColumnToStorage` leaves.
        for slot in stored
            .iter_mut()
            .skip(table.rowid_alias.unwrap_or(usize::MAX))
            .take(1)
        {
            *slot = Value::Null;
        }
        self.unindex_row(&kept, &held, rowid)?;
        self.index_row(&kept, values, rowid)?;
        let record = crate::record::write(&stored, &affinities, 4);
        crate::tree::update(&mut self.pages, root, rowid, &record)?;
        Ok(())
    }

    /// The root, the indexes, the table and the values of one row.
    fn one_row(
        &self,
        name: &[u8],
        rowid: i64,
    ) -> Result<(u32, Vec<Kept>, Table, Vec<Value>), Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let (table, root) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
        let kept = kept_indexes(&database, name);
        let values = database
            .rows_of(name)?
            .into_iter()
            .find(|(held, _)| *held == rowid)
            .map(|(_, values)| values)
            .ok_or(Error::NoTable(Vec::new()))?;
        Ok((root, kept, table.clone(), values))
    }

    /// Whether a trigger of this name is running already and may not
    /// run again, which `PRAGMA recursive_triggers` turns off.
    fn repeats(&self, name: &[u8]) -> bool {
        let recursive = crate::pragma::HELD
            .iter()
            .position(|keeps| keeps.name == b"recursive_triggers")
            .and_then(|at| self.kept.get(at).copied().flatten())
            .unwrap_or(0);
        recursive == 0
            && self
                .running
                .iter()
                .any(|held| held.eq_ignore_ascii_case(name))
    }

    /// Runs the statements of one trigger's body.
    fn body(&mut self, trigger: &crate::db::Trigger, row: &Fired<'_>) -> Result<(), Error> {
        let arena = &trigger.arena;
        let sql = &trigger.sql;
        for step in arena.steps(trigger.written.body) {
            match *step {
                crate::ast::TriggerStep::Insert(statement) => {
                    self.insert(arena, &statement, sql, Some(row))?;
                }
                crate::ast::TriggerStep::Update(statement) => {
                    self.update(arena, &statement, sql, Some(row))?;
                }
                crate::ast::TriggerStep::Delete(statement) => {
                    self.delete(arena, &statement, sql, Some(row))?;
                }
                // A statement that answers rows runs for what it reads
                // and answers nothing, which is what a `SELECT` of a
                // body is for: it carries the `RAISE`.
                crate::ast::TriggerStep::Select(select) => {
                    let bytes = self.image();
                    let database = Database::open(&bytes)?.seeded(self.random.word());
                    database.rows_under(arena, select, sql, Some(row))?;
                }
            }
        }
        Ok(())
    }

    /// The record of five noughts `sqlite3StartTable` writes into
    /// `sqlite_schema` first, and the key it is written under.
    fn schema_blank(&mut self) -> Result<i64, Error> {
        let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
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
        Ok(rowid)
    }

    /// The row `sqlite3EndTable` writes over the blank record, which
    /// raises the schema cookie.
    fn schema_written(&mut self, rowid: i64, row: &[u8]) -> Result<(), Error> {
        crate::tree::update(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, row)?;
        self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
        self.header.schema_format = 4;
        Ok(())
    }

    /// `CREATE TABLE`: a page for the tree of the table and a row of
    /// `sqlite_schema` that names it.
    fn define(&mut self, arena: &Arena, definition: Definition, sql: &[u8]) -> Result<(), Error> {
        let (kind, name, over, already, written) = match definition {
            Definition::Drop(asked) => return self.drop_object(&asked, sql),
            Definition::AddColumn(asked) => return self.add_column(arena, &asked, sql),
            Definition::Trigger(trigger) => return self.create_trigger(&trigger, sql),
            Definition::Table(table) => {
                if let TableBody::Select(select) = table.body {
                    return self.create_as(arena, &table, select, sql);
                }
                let name = crate::schema::dequote(table.name.text(sql));
                (
                    Some(Kind::LeafTable),
                    name.clone(),
                    name,
                    table.if_not_exists,
                    written_statement(b"CREATE TABLE ", table.name, sql),
                )
            }
            Definition::Index(index) => (
                Some(Kind::LeafIndex),
                crate::schema::dequote(index.name.text(sql)),
                crate::schema::dequote(index.table.text(sql)),
                index.if_not_exists,
                written_statement(
                    if index.unique {
                        b"CREATE UNIQUE INDEX "
                    } else {
                        b"CREATE INDEX "
                    },
                    index.name,
                    sql,
                ),
            ),
            // A view holds no row of its own: it names a statement, and
            // the rows are the ones that statement answers.
            Definition::View(view) => {
                let name = crate::schema::dequote(view.name.text(sql));
                (
                    None,
                    name.clone(),
                    name,
                    view.if_not_exists,
                    written_statement(b"CREATE VIEW ", view.name, sql),
                )
            }
        };
        if self.already(&name, already)? {
            return Ok(());
        }
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
                text(&written),
            ],
            &SCHEMA,
            4,
        );
        // `sqlite3StartTable` writes a record of five noughts and
        // `sqlite3EndTable` writes over it, so the page keeps the bytes
        // of the blank record where the row no longer stands.
        // `sqlite3CreateIndex` writes its row once and has no blank.
        if let Definition::Index(index) = definition {
            let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?
                .unwrap_or(0)
                .saturating_add(1);
            insert(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
            self.header.schema_cookie = self.header.schema_cookie.saturating_add(1);
            self.header.schema_format = 4;
            return self.fill(arena, &index, sql, root, &over);
        }
        let rowid = self.schema_blank()?;
        // A `PRIMARY KEY` and a `UNIQUE` each carry an index of the
        // table's own, which the grammar makes as it reads the
        // constraint, so its row stands before the row of the table is
        // written over the blank one.
        let mut counts = false;
        if let Definition::Table(written) = definition {
            let table = crate::schema::table(arena, &written, sql)?;
            for at in 0..table.keys.len() {
                self.own_index(&table, at)?;
            }
            counts = table.autoincrement;
        }
        self.schema_written(rowid, &row)?;
        // `sqlite3StartTable` makes `sqlite_sequence` with the first
        // table that counts its keys up, and the row of that table is
        // written before it.
        if counts && !self.holds(SEQUENCE)? {
            self.ran(b"CREATE TABLE sqlite_sequence(name,seq)")?;
        }
        Ok(())
    }

    /// Whether the database holds the table `name`.
    fn holds(&self, name: &[u8]) -> Result<bool, Error> {
        let bytes = self.image();
        Ok(Database::open(&bytes)?.table(name).is_some())
    }

    /// Whether a `CREATE` of `name` writes nothing, because the schema
    /// already holds the name and the statement wrote `IF NOT EXISTS`.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn already(&self, name: &[u8], if_not_exists: bool) -> Result<bool, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        if !names(&database, name) {
            return Ok(false);
        }
        if if_not_exists {
            return Ok(true);
        }
        Err(Error::Exists)
    }

    /// One index of a table's own, written: a page for its tree and a
    /// row of `sqlite_schema` that names it and holds no statement.
    ///
    /// Writing one row costs O(log n) in the rows of the schema.
    fn own_index(&mut self, table: &crate::schema::Table, at: usize) -> Result<(), Error> {
        let index = crate::schema::own_index(table, at).ok_or(Error::Unsupported)?;
        let root = self.pages.add(Kind::LeafIndex, 0)?;
        self.pages.point(root, crate::tree::Point::Root, 0)?;
        if self.header.largest_root != 0 {
            self.header.largest_root = root;
        }
        let text = |bytes: &[u8]| Value::Text(crate::value::stored(bytes, self.header.encoding));
        let row = crate::record::write(
            &[
                text(b"index"),
                text(&index.name),
                text(&table.name),
                Value::Int(i64::from(root)),
                Value::Null,
            ],
            &SCHEMA,
            4,
        );
        let rowid = largest(&self.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
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
        self.write_entries(entries, &collations, root)
    }

    /// What an `INSERT` writes: the rows the statement answered, each
    /// with the key it was given where it was given one.
    ///
    /// Answering the statement costs what the statement costs.
    fn inserting(
        &self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
        name: &[u8],
    ) -> Result<Inserting, Error> {
        written_to(name)?;
        let named: Vec<Vec<u8>> = arena
            .names(statement.columns)
            .iter()
            .map(|span: &Span| crate::schema::dequote(span.text(sql)))
            .collect();
        let bytes = self.image();
        // Every statement draws from where the connection stands, so
        // two statements of one connection answer `randomblob`
        // differently.
        let database = Database::open(&bytes)?.seeded(self.random.word());
        let (table, root) = database
            .table(name)
            .ok_or_else(|| Error::NoTable(name.to_vec()))?;
        if table.without_rowid {
            return Err(Error::Unsupported);
        }
        let kept = kept_indexes(&database, name);
        let places = places(table, &named)?;
        // A column the statement names no value for holds what it falls
        // back to, which is nothing where it has no `DEFAULT`.
        let falls_back: Vec<Value> = database
            .defaults(name)?
            .iter()
            .map(|value| stored(value, self.header.encoding))
            .collect();
        let answer = database.rows_under(arena, statement.select, sql, outer)?;
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
        Ok(Inserting {
            root,
            alias: table.rowid_alias,
            affinities,
            rows,
            kept,
            table: table.clone(),
        })
    }

    /// The entries of one index written into the tree at `root`, which
    /// is what `sqlite3RefillIndex` writes.
    ///
    /// Sorting `n` entries costs O(n log n) and writing them O(n).
    fn write_entries(
        &mut self,
        entries: Vec<Vec<Value>>,
        collations: &[Collation],
        root: u32,
    ) -> Result<(), Error> {
        // `sqlite3VdbeSorterInit`: the entries are sorted before any is
        // written, so the pages fill in the order the entries run and
        // not in the order the rows do.
        let mut entries = entries;
        entries.sort_by(|one, other| order_of_keys(one, other, collations));
        let affinities = alloc::vec![Affinity::None; collations.len().saturating_add(1)];
        for key in entries {
            let record = crate::record::write(&key, &affinities, 4);
            crate::tree::insert_entry(&mut self.pages, root, &record, &key, collations, true)?;
        }
        Ok(())
    }

    /// `REINDEX`: the entries of every index it names written again out
    /// of the rows they belong to.
    ///
    /// `REINDEX` alone writes every index of the schema again. A name
    /// is the collation whose indexes are written again, or the table
    /// whose indexes are, or the one index, which is the order
    /// `sqlite3Reindex` reads the name in.
    ///
    /// Writing one index again costs what its rows cost to read and
    /// O(n log n) to order.
    fn reindex(&mut self, asked: &crate::ast::Reindex, sql: &[u8]) -> Result<(), Error> {
        let named = asked
            .name
            .map(|span| crate::schema::dequote(span.text(sql)));
        let held = self.reindexed(named.as_deref())?;
        for Rebuilt { index, root, table } in held {
            crate::tree::clear_tree(&mut self.pages, root, Kind::LeafIndex)?;
            let entries = {
                let bytes = self.image();
                let database = Database::open(&bytes)?;
                let mut entries = Vec::new();
                for (rowid, values) in database.rows_of(&table)? {
                    entries.push(entry_of(&index, &values, rowid));
                }
                entries
            };
            let collations = collations_of(&index);
            self.write_entries(entries, &collations, root)?;
        }
        Ok(())
    }

    /// The indexes one `REINDEX` writes again, with the one made last
    /// first, which is the order `reindexDatabases` reads them in.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn reindexed(&self, named: Option<&[u8]>) -> Result<Vec<Rebuilt>, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let collation = named.and_then(Collation::of_name);
        let over: Option<Vec<u8>> = match named {
            None => None,
            Some(_) if collation.is_some() => None,
            Some(named) if database.table(named).is_some() => Some(named.to_vec()),
            Some(named) => {
                let (index, root) = database
                    .index(named)
                    .ok_or_else(|| Error::NoTable(named.to_vec()))?;
                return Ok(alloc::vec![Rebuilt {
                    index: index.clone(),
                    root,
                    table: index.table.clone(),
                }]);
            }
        };
        let mut held = Vec::new();
        for table in database.tables() {
            if over.as_deref().is_some_and(|over| table.name != over) {
                continue;
            }
            for (index, root) in database.indexes(&table.name).iter().rev() {
                // `REINDEX <collation>` writes again every index one of
                // whose columns is held in that collation.
                if collation.is_some_and(|wanted| {
                    !index
                        .columns
                        .iter()
                        .any(|column| column.collation == wanted)
                }) {
                    continue;
                }
                held.push(Rebuilt {
                    index: (*index).clone(),
                    root: *root,
                    table: table.name.clone(),
                });
            }
        }
        held.reverse();
        Ok(held)
    }

    /// `INSERT`: the rows the statement answers, each put in the tree
    /// of the table it names and in every index over that table.
    fn insert(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let Inserting {
            root,
            alias,
            affinities,
            rows,
            kept,
            table,
        } = self.inserting(arena, statement, sql, outer, &name)?;
        let before = self.triggers_for(&name, TriggerEvent::Insert, TriggerTime::Before)?;
        let after = self.triggers_for(&name, TriggerEvent::Insert, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let mut next = largest(&self.pages, root)?.unwrap_or(0);
        // `autoIncBegin`: a key that counts up never gives a key back,
        // so the next one is past the largest the table ever held and
        // not past the largest it holds.
        let counted = table
            .autoincrement
            .then(|| self.counted(&name))
            .transpose()?;
        if let Some(held) = counted {
            next = next.max(held);
        }
        let mut written = 0_i64;
        for (key, mut values) in rows {
            // A trigger's body may write the table this statement
            // writes, so the largest key is read again per row where
            // one runs.
            if fires {
                next = next.max(largest(&self.pages, root)?.unwrap_or(0));
            }
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
            if fires {
                // `sqlite3Insert` writes the key as nought less one
                // where the statement named none, because the key is
                // given after the `BEFORE` triggers have run.
                let shown = given.unwrap_or(-1);
                let mut early = named.clone();
                for slot in early.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
                    *slot = Value::Int(shown);
                }
                let row = Fired {
                    table: &table,
                    old: None,
                    new: Some((&early, shown)),
                    encoding: self.header.encoding,
                };
                if !self.fire(&before, &[], &row)? {
                    continue;
                }
            }
            // `sqlite3GenerateConstraintChecks`: the columns that
            // refuse nothing and every `CHECK` of the table hold the
            // row before the keys do.
            if !self.constrained(&table, &mut named, rowid, statement.conflict)? {
                continue;
            }
            Self::refilled(&named, &mut values, alias);
            let row = Fired {
                table: &table,
                old: None,
                new: Some((&named, rowid)),
                encoding: self.header.encoding,
            };
            if !self.keyed(root, &kept, &table, alias, rowid, statement.conflict)? {
                continue;
            }
            // `sqlite3GenerateConstraintChecks`: a row that shares a
            // key with one the table holds is refused, passed over, or
            // written over the one that is there.
            if let Some((index, held)) = self.conflicting(&kept, &named, rowid, None)? {
                match resolved(statement.conflict, &kept, index) {
                    crate::ast::Conflict::Ignore => continue,
                    crate::ast::Conflict::Fail => {
                        self.stopped = true;
                        return Err(Error::Unique(Self::shown_key_of(&table, &kept, index)));
                    }
                    crate::ast::Conflict::Replace => {
                        self.replaced(root, &kept, &table, &held)?;
                    }
                    _ => return Err(Error::Unique(Self::shown_key_of(&table, &kept, index))),
                }
            }
            let record = crate::record::write(&values, &affinities, 4);
            // `I.1` of `src/fkey.c`: a row whose foreign key points at
            // no row is refused before it is written.
            self.parented(&table, &named, rowid)?;
            // `sqlite3CompleteInsertion` writes the entry of every
            // index before the row, so the pages an entry runs onto
            // are taken before the pages the row runs onto.
            self.index_row(&kept, &named, rowid)?;
            insert(&mut self.pages, root, rowid, &record)?;
            written = written.saturating_add(1);
            if fires {
                self.fire(&after, &[], &row)?;
            }
        }
        // `autoIncrementEnd` writes the largest key the table ever held
        // back into `sqlite_sequence`, where the statement wrote a row.
        if let Some(held) = counted
            && written > 0
            && next > held
        {
            self.count_up(&name, next)?;
        }
        Ok(written)
    }

    /// The row of `sqlite_sequence` that names `name`, taken away with
    /// the table it counts.
    ///
    /// Taking one row out costs O(log n).
    fn uncount(&mut self, name: &[u8]) -> Result<(), Error> {
        let wanted = crate::value::stored(name, self.header.encoding);
        let found = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let Some((_, root)) = database.table(SEQUENCE) else {
                // A database with no table that counts its keys up has
                // no table of counts to take a row out of.
                return Ok(());
            };
            database
                .rows_of(SEQUENCE)?
                .iter()
                .find(|(_, values)| named_row(values, &wanted))
                .map(|(rowid, _)| (root, *rowid))
        };
        if let Some((root, rowid)) = found {
            crate::tree::remove(&mut self.pages, root, rowid)?;
        }
        Ok(())
    }

    /// The largest key a table that counts its keys up ever held, which
    /// is the row of `sqlite_sequence` that names it.
    ///
    /// Reading it costs O(n) in the rows of that table.
    fn counted(&self, name: &[u8]) -> Result<i64, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let wanted = crate::value::stored(name, self.header.encoding);
        Ok(database
            .rows_of(SEQUENCE)?
            .iter()
            .find(|(_, values)| named_row(values, &wanted))
            .and_then(|(_, values)| values.get(1))
            .map_or(0, Value::to_integer))
    }

    /// The largest key a table ever held, written into the row of
    /// `sqlite_sequence` that names it, which is made where the table
    /// has none.
    ///
    /// Writing one row costs O(log n).
    fn count_up(&mut self, name: &[u8], held: i64) -> Result<(), Error> {
        let wanted = crate::value::stored(name, self.header.encoding);
        let (root, rowid) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (_, root) = database.table(SEQUENCE).ok_or(Error::NoTable(Vec::new()))?;
            let rowid = database
                .rows_of(SEQUENCE)?
                .iter()
                .find(|(_, values)| named_row(values, &wanted))
                .map(|(rowid, _)| *rowid);
            (root, rowid)
        };
        let record = crate::record::write(
            &[Value::Text(wanted), Value::Int(held)],
            &[Affinity::None; 2],
            4,
        );
        if let Some(rowid) = rowid {
            crate::tree::update(&mut self.pages, root, rowid, &record)?;
            return Ok(());
        }
        let rowid = largest(&self.pages, root)?.unwrap_or(0).saturating_add(1);
        insert(&mut self.pages, root, rowid, &record)?;
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
    /// Where `random` and `randomblob` take their bytes from.
    random: &'a crate::random::Source,
    /// The row a trigger's body reads as `new` and `old`, where this
    /// row is one of a statement a trigger runs.
    outer: Option<&'a dyn crate::eval::Row>,
}

impl crate::eval::Row for Held<'_> {
    fn random(&self) -> Option<&crate::random::Source> {
        Some(self.random)
    }

    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        self.mine(schema, table, column).or_else(|| {
            self.outer
                .and_then(|outer| outer.column(schema, table, column))
        })
    }
}

impl Held<'_> {
    /// The column of this row, which is the table the statement
    /// changes and not the row a trigger stands on.
    fn mine(
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
}

/// What a statement does where a row shares a key with one the table
/// holds: what the statement said where it said anything, what the
/// constraint said otherwise, and `ABORT` where neither said anything,
/// which is `sqlite3GenerateConstraintChecks` reading `OE_Default`.
fn resolved(written: Conflict, kept: &[Kept], at: usize) -> Conflict {
    if written != Conflict::Unspecified {
        return written;
    }
    let own = kept
        .get(at)
        .map_or(Conflict::Unspecified, |(index, _, _)| index.conflict);
    if own == Conflict::Unspecified {
        return Conflict::Abort;
    }
    own
}

/// Whether a trigger runs for a statement that writes these columns,
/// which is what `UPDATE OF` holds it to: a trigger that names columns
/// runs where the statement writes one of them.
fn writes_one(
    arena: &Arena,
    trigger: &crate::ast::CreateTrigger,
    sql: &[u8],
    written: &[Vec<u8>],
) -> bool {
    let named = arena.names(trigger.columns);
    named.is_empty()
        || named.iter().any(|span| {
            let name = crate::schema::dequote(span.text(sql));
            written.iter().any(|held| held.eq_ignore_ascii_case(&name))
        })
}

/// How deep a trigger may reach.
///
/// SQLite runs a trigger's body as a program of its own machine, so
/// `SQLITE_MAX_TRIGGER_DEPTH` bounds it at a thousand. This crate runs
/// the body where the statement runs, so the count is the frames the
/// stack holds and is bounded the way a view that names itself is.
const TRIGGER_DEPTH: usize = 32;

/// What an `INSERT` writes.
struct Inserting {
    /// The tree of the table.
    root: u32,
    /// The column the key is another name for, where the table has one.
    alias: Option<usize>,
    /// What each column of the table converts a value under.
    affinities: Vec<Affinity>,
    /// One per row the statement answered: the key it was given, and
    /// the values of every column.
    rows: Vec<(Value, Vec<Value>)>,
    /// The indexes over the table.
    kept: Vec<Kept>,
    /// The table itself.
    table: Table,
}

/// What an `UPDATE` writes.
struct Updating {
    /// The tree of the table.
    root: u32,
    /// The column the key is another name for, where the table has one.
    alias: Option<usize>,
    /// What each column of the table converts a value under.
    affinities: Vec<Affinity>,
    /// One per row the `WHERE` keeps: its key, the key it is written
    /// under, the values it held and the values it is written with.
    written: Vec<(i64, i64, Vec<Value>, Vec<Value>)>,
    /// The indexes over the table.
    kept: Vec<Kept>,
    /// The table itself.
    table: Table,
}

/// The row a trigger's body reads as `new` and `old`.
///
/// `sqlite3CodeRowTrigger` puts the row the statement changes in two
/// cursors the body reaches by those names: a `DELETE` has `old` alone,
/// an `INSERT` has `new` alone, and an `UPDATE` has both.
struct Fired<'a> {
    /// The table the trigger is on.
    table: &'a Table,
    /// The row as it was, with its key.
    old: Option<(&'a [Value], i64)>,
    /// The row as it is, with its key.
    new: Option<(&'a [Value], i64)>,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
}

impl crate::eval::Row for Fired<'_> {
    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        // A name with a schema in front of it names no row a trigger
        // stands on, because `new` and `old` are cursors and not
        // tables.
        let table = table.filter(|_| schema.is_none())?;
        let (values, rowid) = if table.eq_ignore_ascii_case(b"new") {
            self.new
        } else if table.eq_ignore_ascii_case(b"old") {
            self.old
        } else {
            None
        }?;
        let at = self
            .table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(column));
        match at {
            Some(at) => {
                let held = self.table.columns.get(at)?;
                let value = values.get(at)?.clone();
                Some((value, held.affinity, held.collation))
            }
            None if is_rowid(column) => {
                Some((Value::Int(rowid), Affinity::Integer, Collation::Binary))
            }
            None => None,
        }
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
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        written_to(&name)?;
        let (root, keys, kept, table) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            let (table, root) = database
                .table(&name)
                .ok_or_else(|| Error::NoTable(name.clone()))?;
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
                    outer,
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
            (root, keys, kept, table.clone())
        };
        let before = self.triggers_for(&name, TriggerEvent::Delete, TriggerTime::Before)?;
        let after = self.triggers_for(&name, TriggerEvent::Delete, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        // `sqlite3GenerateRowDelete` writes the entries out before the
        // row, because the entries are found through the row.
        let mut taken = 0_i64;
        for (key, values) in keys {
            let row = Fired {
                table: &table,
                old: Some((&values, key)),
                new: None,
                encoding: self.header.encoding,
            };
            if fires && !self.fire(&before, &[], &row)? {
                continue;
            }
            // `D.2` of `src/fkey.c`: a row that rows of another table
            // point at is refused, or those rows are written, by what
            // the key says happens.
            self.orphaned(&name, &table, &values, key, None)?;
            self.unindex_row(&kept, &values, key)?;
            crate::tree::remove(&mut self.pages, root, key)?;
            taken = taken.saturating_add(1);
            if fires {
                self.fire(&after, &[], &row)?;
            }
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
    /// What an `UPDATE` writes: the rows its `WHERE` keeps, each with
    /// the key and the values it becomes.
    ///
    /// Reading `n` rows costs O(n) and answering the `SET` of one costs
    /// what its expressions cost.
    fn updating(
        &self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
        name: &[u8],
    ) -> Result<Updating, Error> {
        written_to(name)?;
        let sets = arena.sets(statement.sets);
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let (table, root) = database
            .table(name)
            .ok_or_else(|| Error::NoTable(name.to_vec()))?;
        let kept = kept_indexes(&database, name);
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
        for (rowid, values) in database.rows_of(name)? {
            let held = Held {
                table,
                values: &values,
                rowid,
                encoding: self.header.encoding,
                random: &self.random,
                outer,
            };
            let keep = match statement.filter {
                None => true,
                Some(filter) => crate::eval::evaluate_row(arena, filter, sql, &held)?.truth(false),
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
        Ok(Updating {
            root,
            alias: table.rowid_alias,
            affinities,
            written,
            kept,
            table: table.clone(),
        })
    }

    /// `UPDATE`: the rows the statement changes, each written again in
    /// the tree of the table and in every index over that table.
    fn update(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let name = crate::schema::dequote(statement.name.text(sql));
        let sets = arena.sets(statement.sets);
        let Updating {
            root,
            alias,
            affinities,
            written,
            kept,
            table,
        } = self.updating(arena, statement, sql, outer, &name)?;
        let columns: Vec<Vec<u8>> = sets
            .iter()
            .map(|set| crate::schema::dequote(set.column.text(sql)))
            .collect();
        let before = self.triggers_for(&name, TriggerEvent::Update, TriggerTime::Before)?;
        let after = self.triggers_for(&name, TriggerEvent::Update, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let mut changed = 0_i64;
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
            if fires {
                let row = Fired {
                    table: &table,
                    old: Some((&held, rowid)),
                    new: Some((&named, key)),
                    encoding: self.header.encoding,
                };
                if !self.fire(&before, &columns, &row)? {
                    continue;
                }
            }
            if !self.constrained(&table, &mut named, key, statement.conflict)? {
                continue;
            }
            Self::refilled(&named, &mut values, alias);
            let row = Fired {
                table: &table,
                old: Some((&held, rowid)),
                new: Some((&named, key)),
                encoding: self.header.encoding,
            };
            // A row that keeps the key it had shares it with nothing.
            if key != rowid && !self.keyed(root, &kept, &table, alias, key, statement.conflict)? {
                continue;
            }
            if let Some((index, other)) = self.conflicting(&kept, &named, key, Some(rowid))? {
                match resolved(statement.conflict, &kept, index) {
                    crate::ast::Conflict::Ignore => continue,
                    crate::ast::Conflict::Fail => {
                        self.stopped = true;
                        return Err(Error::Unique(Self::shown_key_of(&table, &kept, index)));
                    }
                    crate::ast::Conflict::Replace => {
                        self.replaced(root, &kept, &table, &other)?;
                    }
                    _ => return Err(Error::Unique(Self::shown_key_of(&table, &kept, index))),
                }
            }
            let record = crate::record::write(&values, &affinities, 4);
            // `I.1` of `src/fkey.c` over the row as it will stand, and
            // `D.2` over the row as it stands: a row that points at no
            // row is refused, and so is one that rows point at.
            self.parented(&table, &named, key)?;
            self.orphaned(&name, &table, &held, rowid, Some((&named, key)))?;
            // `sqlite3Update` removes the entries of the row, removes
            // the row itself where the key changes, and then writes
            // the new entries before the new row.
            self.unindex_row(&kept, &held, rowid)?;
            let moved = key != rowid;
            if moved {
                crate::tree::remove(&mut self.pages, root, rowid)?;
            }
            self.index_row(&kept, &named, key)?;
            if moved {
                insert(&mut self.pages, root, key, &record)?;
            } else {
                crate::tree::update(&mut self.pages, root, rowid, &record)?;
            }
            changed = changed.saturating_add(1);
            if fires {
                self.fire(&after, &columns, &row)?;
            }
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

    /// The row a `REPLACE` writes over, taken out with its entries and
    /// with the triggers a `DELETE` on it runs, which is
    /// `sqlite3GenerateRowDelete` under `OE_Replace`.
    fn replaced(
        &mut self,
        root: u32,
        kept: &[Kept],
        table: &Table,
        found: &Value,
    ) -> Result<(), Error> {
        // The entry names the key of the row it belongs to, so the row
        // the statement writes over is the one that key finds.
        let (rowid, values) = {
            let bytes = self.image();
            let database = Database::open(&bytes)?;
            database
                .rows_of(&table.name)?
                .into_iter()
                .find(|(key, _)| Value::Int(*key) == *found)
                .ok_or(Error::NoTable(Vec::new()))?
        };
        let before = self.triggers_for(&table.name, TriggerEvent::Delete, TriggerTime::Before)?;
        let after = self.triggers_for(&table.name, TriggerEvent::Delete, TriggerTime::After)?;
        let row = Fired {
            table,
            old: Some((&values, rowid)),
            new: None,
            encoding: self.header.encoding,
        };
        if !self.fire(&before, &[], &row)? {
            return Ok(());
        }
        self.unindex_row(kept, &values, rowid)?;
        crate::tree::remove(&mut self.pages, root, rowid)?;
        self.fire(&after, &[], &row)?;
        Ok(())
    }

    /// The unique index a row would share a key with, and the key of
    /// the row already there, which is
    /// `sqlite3GenerateConstraintChecks`.
    ///
    /// A key one of whose columns is nothing constrains no row, which
    /// is what makes a `UNIQUE` hold over the values and not over the
    /// rows. Looking one key up costs O(log n).
    fn conflicting(
        &self,
        kept: &[Kept],
        values: &[Value],
        rowid: i64,
        held: Option<i64>,
    ) -> Result<Option<(usize, Value)>, Error> {
        for (at, (index, collations, root)) in kept.iter().enumerate() {
            if !index.unique {
                continue;
            }
            let entry = entry_of(index, values, rowid);
            let key = entry.get(..index.columns.len()).unwrap_or_default();
            if key.contains(&Value::Null) {
                continue;
            }
            let Some(found) = crate::tree::entry_at(&self.pages, *root, key, collations)? else {
                continue;
            };
            if held.map(Value::Int).as_ref() == Some(&found) {
                continue;
            }
            return Ok(Some((at, found)));
        }
        Ok(None)
    }

    /// Whether a row is held to the columns that refuse nothing and to
    /// every `CHECK` of the table, which is
    /// `sqlite3GenerateConstraintChecks` before the keys are held.
    ///
    /// A column that refuses nothing and holds nothing writes what it
    /// falls back to where the resolution is `REPLACE`, and refuses
    /// where what it falls back to is nothing as well. The answer is
    /// `false` where the resolution is `IGNORE`, which is the row
    /// passed over.
    fn constrained(
        &mut self,
        table: &Table,
        values: &mut [Value],
        rowid: i64,
        written: Conflict,
    ) -> Result<bool, Error> {
        let bytes = self.image();
        let database = Database::open(&bytes)?;
        let falls_back = database.defaults(&table.name)?;
        for (at, column) in table.columns.iter().enumerate() {
            if !column.not_null || values.get(at) != Some(&Value::Null) {
                continue;
            }
            let mut answer = match written {
                Conflict::Unspecified => column.null_conflict,
                other => other,
            };
            if answer == Conflict::Replace {
                let held = falls_back.get(at).cloned().unwrap_or(Value::Null);
                for slot in values.iter_mut().skip(at).take(1) {
                    *slot = stored(&held, self.header.encoding);
                }
                if values.get(at) != Some(&Value::Null) {
                    continue;
                }
                // `sqlite3GenerateConstraintChecks`: a column that
                // falls back to nothing refuses the row instead.
                answer = Conflict::Abort;
            }
            let mut shown = table.name.clone();
            shown.push(b'.');
            shown.extend_from_slice(&column.name);
            match answer {
                Conflict::Ignore => return Ok(false),
                Conflict::Fail => {
                    self.stopped = true;
                    return Err(Error::NotNull(shown));
                }
                _ => return Err(Error::NotNull(shown)),
            }
        }
        let row = Held {
            table,
            values,
            rowid,
            encoding: self.header.encoding,
            random: &self.random,
            outer: None,
        };
        // `PRAGMA ignore_check_constraints` leaves every `CHECK` of
        // the table unread.
        if self.told(b"ignore_check_constraints") != 0 {
            return Ok(true);
        }
        let Some(shown) = database.refused_check(&table.name, &row)? else {
            return Ok(true);
        };
        match written {
            Conflict::Ignore => Ok(false),
            Conflict::Fail => {
                self.stopped = true;
                Err(Error::Check(shown))
            }
            _ => Err(Error::Check(shown)),
        }
    }

    /// What a row whose key the table already holds does, which is
    /// `sqlite3GenerateConstraintChecks` over the key of a table whose
    /// rowid a column is another name for: the row is refused, passed
    /// over, or written over the row that is there. The answer is
    /// `false` where the row is passed over.
    fn keyed(
        &mut self,
        root: u32,
        kept: &[Kept],
        table: &Table,
        alias: Option<usize>,
        key: i64,
        written: Conflict,
    ) -> Result<bool, Error> {
        if alias.is_none() || !crate::tree::holds(&self.pages, root, key)? {
            return Ok(true);
        }
        match written {
            Conflict::Ignore => return Ok(false),
            Conflict::Fail => self.stopped = true,
            Conflict::Replace => {
                self.replaced(root, kept, table, &Value::Int(key))?;
                return Ok(true);
            }
            _ => {}
        }
        Err(Error::Unique(Self::shown_column(table, alias)))
    }

    /// The values a row is written with, where a column that refuses
    /// nothing took what it falls back to. The column the key is another
    /// name for is stored as nothing, because the key carries it.
    fn refilled(named: &[Value], values: &mut [Value], alias: Option<usize>) {
        for (at, value) in named.iter().enumerate() {
            if Some(at) == alias {
                continue;
            }
            for slot in values.iter_mut().skip(at).take(1) {
                *slot = value.clone();
            }
        }
    }

    /// The column at `at` of the table, as a message names it:
    /// `table.column`.
    fn shown_column(table: &Table, at: Option<usize>) -> Vec<u8> {
        let mut out = table.name.clone();
        out.push(b'.');
        let named = at
            .and_then(|at| table.columns.get(at))
            .map_or(&[][..], |column| column.name.as_slice());
        out.extend_from_slice(named);
        out
    }

    /// The columns a key is over, as `sqlite3UniqueConstraint` writes
    /// them into a message: `table.column`, one after another with a
    /// comma between them.
    fn shown_key_of(table: &Table, kept: &[Kept], at: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let columns = kept
            .get(at)
            .map_or(&[][..], |(index, _, _)| index.columns.as_slice());
        for keyed in columns {
            if !out.is_empty() {
                out.extend_from_slice(b", ");
            }
            out.extend_from_slice(&table.name);
            out.push(b'.');
            let named = table
                .columns
                .get(keyed.column)
                .map_or(&[][..], |column| column.name.as_slice());
            out.extend_from_slice(named);
        }
        out
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
pub(crate) fn entry_of(index: &crate::schema::Index, values: &[Value], rowid: i64) -> Vec<Value> {
    let mut key: Vec<Value> = index
        .columns
        .iter()
        .map(|column| values.get(column.column).cloned().unwrap_or(Value::Null))
        .collect();
    key.push(Value::Int(rowid));
    key
}

/// The collation each column of an index is held in.
pub(crate) fn collations_of(index: &crate::schema::Index) -> Vec<Collation> {
    index
        .columns
        .iter()
        .map(|column| column.collation)
        .collect()
}

/// Where one index entry stands against another: column by column
/// under the collation each is held in, and the key of the row last,
/// which is what makes the order of an index total.
pub(crate) fn order_of_keys(
    one: &[Value],
    other: &[Value],
    collations: &[Collation],
) -> core::cmp::Ordering {
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

/// `sql` without the semicolon that ends it and without the space
/// after it.
fn trimmed(sql: &[u8]) -> &[u8] {
    let mut text = sql;
    while text
        .last()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b';')
    {
        text = text.get(..text.len().saturating_sub(1)).unwrap_or_default();
    }
    text
}

/// The statement a row of `sqlite_schema` holds: the words `CREATE` and
/// the kind, and then the text from the name to the end.
fn written_statement(prefix: &[u8], name: Span, sql: &[u8]) -> Vec<u8> {
    let mut out = prefix.to_vec();
    out.extend_from_slice(trimmed(sql.get(name.start..).unwrap_or_default()));
    out
}

/// A value as the database stores it, which turns text into the
/// encoding the file names.
fn stored(value: &Value, encoding: Encoding) -> Value {
    match value {
        Value::Text(bytes) => Value::Text(crate::value::stored(bytes, encoding)),
        other => other.clone(),
    }
}

/// One foreign key of a table that points at another, with the name of
/// the table that points.
struct Points {
    /// The table the key is written on.
    child: Vec<u8>,
    /// The key itself.
    key: crate::schema::Foreign,
}

/// The places the parent columns of a foreign key take in the table it
/// points at, which is that table's primary key where the key names no
/// columns.
///
/// `sqlite3FkLocateIndex` refuses a key whose columns are not the
/// primary key and carry no unique index of their own.
fn parent_places(key: &crate::schema::Foreign, parent: &Table) -> Result<Vec<usize>, Error> {
    if key.parent.is_empty() {
        let mut places: Vec<usize> = (0..parent.columns.len())
            .filter(|at| parent.columns.get(*at).is_some_and(|column| column.key > 0))
            .collect();
        places.sort_by_key(|at| parent.columns.get(*at).map_or(0, |column| column.key));
        if places.len() != key.columns.len() {
            return Err(Error::ForeignMismatch);
        }
        return Ok(places);
    }
    let mut places = Vec::new();
    for name in &key.parent {
        let at = parent
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .ok_or(Error::ForeignMismatch)?;
        places.push(at);
    }
    if places.len() != key.columns.len() {
        return Err(Error::ForeignMismatch);
    }
    // The columns pointed at must be unique, which is the primary key
    // or a `UNIQUE` over exactly those columns.
    let whole = |named: &[Vec<u8>]| {
        named.len() == places.len()
            && named
                .iter()
                .zip(&key.parent)
                .all(|(one, other)| one.eq_ignore_ascii_case(other))
    };
    let primary: Vec<Vec<u8>> = {
        let mut held: Vec<(u16, Vec<u8>)> = parent
            .columns
            .iter()
            .filter(|column| column.key > 0)
            .map(|column| (column.key, column.name.clone()))
            .collect();
        held.sort_by_key(|(key, _)| *key);
        held.into_iter().map(|(_, name)| name).collect()
    };
    if whole(&primary) {
        return Ok(places);
    }
    let unique = parent.keys.iter().any(|held| {
        let named: Vec<Vec<u8>> = held
            .columns
            .iter()
            .filter_map(|keyed| parent.columns.get(keyed.column))
            .map(|column| column.name.clone())
            .collect();
        whole(&named)
    });
    if unique {
        return Ok(places);
    }
    Err(Error::ForeignMismatch)
}

/// The value a row answers for a place, which is the rowid where the
/// place is the column the rowid is another name for.
fn at_place(table: &Table, values: &[Value], rowid: i64, at: usize) -> Value {
    if table.rowid_alias == Some(at) {
        return Value::Int(rowid);
    }
    values.get(at).cloned().unwrap_or(Value::Null)
}

/// Whether the table `name` holds a row whose columns at `places`
/// answer `wanted`, compared as the columns of that table compare.
fn found_parent(
    database: &Database<'_>,
    name: &[u8],
    parent: &Table,
    places: &[usize],
    wanted: &[Value],
) -> Result<bool, Error> {
    for (rowid, values) in database.rows_of(name)? {
        let same = places.iter().zip(wanted).all(|(at, value)| {
            let held = at_place(parent, &values, rowid, *at);
            let collation = parent
                .columns
                .get(*at)
                .map_or(Collation::Binary, |column| column.collation);
            let mut one = held;
            let mut other = value.clone();
            let affinity = parent
                .columns
                .get(*at)
                .map_or(Affinity::None, |column| column.affinity);
            crate::value::apply_comparison(&mut one, &mut other, affinity);
            crate::value::compare(&one, &other, collation) == core::cmp::Ordering::Equal
        });
        if same {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether two runs of values are the same under the affinities and
/// collations of the columns they came from.
fn alike_values(held: &[Value], wanted: &[Value], table: &Table, places: &[usize]) -> bool {
    held.iter()
        .zip(wanted)
        .zip(places)
        .all(|((one, other), at)| {
            let column = table.columns.get(*at);
            let affinity = column.map_or(Affinity::None, |column| column.affinity);
            let collation = column.map_or(Collation::Binary, |column| column.collation);
            let mut left = one.clone();
            let mut right = other.clone();
            crate::value::apply_comparison(&mut left, &mut right, affinity);
            crate::value::compare(&left, &right, collation) == core::cmp::Ordering::Equal
        })
}

/// What `PRAGMA foreign_key_list` writes for an action.
const fn action_text(action: crate::ast::Action) -> &'static [u8] {
    match action {
        crate::ast::Action::SetNull => b"SET NULL",
        crate::ast::Action::SetDefault => b"SET DEFAULT",
        crate::ast::Action::Cascade => b"CASCADE",
        crate::ast::Action::Restrict => b"RESTRICT",
        crate::ast::Action::NoAction | crate::ast::Action::Unspecified => b"NO ACTION",
    }
}
