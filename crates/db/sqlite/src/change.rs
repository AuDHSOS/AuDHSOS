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
fn refused_column(
    arena: &Arena,
    asked: &crate::ast::AddColumn,
    sql: &[u8],
    (rows, keys): (bool, bool),
) -> Result<(), Error> {
    let mut points = false;
    let mut not_null = false;
    let mut generated = false;
    let mut falls_back_to = None;
    let mut falls_back = None;
    for constraint in arena.column_constraints(asked.column.constraints) {
        match constraint {
            crate::ast::ColumnConstraint::PrimaryKey { .. } => {
                return Err(Error::Added(crate::db::Added::PrimaryKey));
            }
            crate::ast::ColumnConstraint::Unique(_) => {
                return Err(Error::Added(crate::db::Added::Unique));
            }
            crate::ast::ColumnConstraint::Default { text, value } => {
                falls_back_to = Some(text);
                falls_back = Some(value);
            }
            crate::ast::ColumnConstraint::NotNull(_) => not_null = true,
            crate::ast::ColumnConstraint::References(_) => points = true,
            crate::ast::ColumnConstraint::Generated { .. } => generated = true,
            _ => {}
        }
    }
    // A default written as the literal `NULL` is no default at all,
    // which `sqlite3AlterFinishAddColumn` reads before the two checks
    // below, and a generated column reaches neither.
    let written_null = falls_back_to
        .is_some_and(|text| crate::schema::dequote(text.text(sql)).eq_ignore_ascii_case(b"null"));
    if generated || !rows {
        return Ok(());
    }
    // `sqlite3AlterFinishAddColumn` of
    // `research/sqlite/src/alter.c:373`: a column that points at a row of
    // another table falls back to nothing, the rows the table already
    // holds gaining no value of their own and pointing at no row. The
    // refusal stands where the connection holds the keys, and it comes
    // before the two below.
    if keys && points && falls_back_to.is_some() && !written_null {
        return Err(Error::PointingDefault);
    }
    if not_null && (falls_back_to.is_none() || written_null) {
        return Err(Error::Added(crate::db::Added::NotNull));
    }
    // `sqlite3ValueFromExpr` reads a literal and a sign before one, and
    // answers nothing for every other expression, `CURRENT_TIME` among
    // them.
    if let Some(value) = falls_back.filter(|_| !written_null)
        && !constant_value(arena, *value)
    {
        return Err(Error::Added(crate::db::Added::NonConstant));
    }
    Ok(())
}

/// Whether the connection reads the expression as a value of its own,
/// which `sqlite3ValueFromExpr` of `research/sqlite/src/vdbemem.c` does
/// for a literal and for a sign before one, which is why
/// `DEFAULT -'x'` stands and answers nought.
///
/// Reading costs O(1), because a sign reaches one operand.
fn constant_value(arena: &Arena, value: crate::ast::ExprId) -> bool {
    match arena.node(value) {
        Some(crate::ast::Node::Literal(literal)) => {
            !matches!(literal, crate::ast::Literal::CurrentTime(_))
        }
        Some(crate::ast::Node::Unary { op, operand }) => {
            matches!(
                op,
                crate::ast::UnaryOp::Negate | crate::ast::UnaryOp::Identity
            ) && matches!(arena.node(operand), Some(crate::ast::Node::Literal(_)))
        }
        _ => false,
    }
}

/// Whether a column of a `sqlite_schema` row is the text `wanted`.
fn is_text(
    value: Option<crate::record::Value<'_>>,
    wanted: &[u8],
    encoding: crate::header::Encoding,
) -> bool {
    // The schema holds its text in the encoding of the file, which the
    // name a statement carries is never in, so the stored bytes are
    // read into UTF-8 before the two are compared.
    matches!(value, Some(crate::record::Value::Text(bytes))
        if crate::value::decoded(bytes, encoding).eq_ignore_ascii_case(wanted))
}

/// The table `ANALYZE` writes its counts into.
const STAT: &[u8] = b"sqlite_stat1";

/// The table a key that counts up is counted in.
const SEQUENCE: &[u8] = b"sqlite_sequence";

/// The name of the schema's own table, which a rename reads its rows
/// out of.
const SCHEMA_TABLE: &[u8] = b"sqlite_schema";

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

/// One row of `sqlite_schema` written again under the new name of a
/// table, and whether the row changed at all.
///
/// The row's own name changes where the row is the table itself or an
/// index SQLite made for a key of it, its `tbl_name` changes where it
/// named the table, and its text is written again wherever the
/// statement names the table.
fn renamed(values: &mut [Value], from: &[u8], to: &[u8]) -> bool {
    let text = |value: Option<&Value>| match value {
        Some(Value::Text(bytes)) => bytes.clone(),
        _ => Vec::new(),
    };
    let kind = text(values.first());
    let name = text(values.get(1));
    let over = text(values.get(2));
    let sql = text(values.get(4));
    let places = crate::rename::places(&sql, from);
    let automatic = crate::rename::automatic(&name, from, to);
    let mut changed = false;
    let mut write = |at: usize, bytes: &[u8]| {
        for slot in values.iter_mut().skip(at).take(1) {
            *slot = Value::Text(bytes.to_vec());
        }
    };
    // The table is the row of its own name, and an index SQLite made
    // for a key of it carries the name of the table in its own.
    if kind.eq_ignore_ascii_case(b"table") && name.eq_ignore_ascii_case(from) {
        write(1, to);
        changed = true;
    } else if let Some(made) = automatic {
        write(1, &made);
        changed = true;
    }
    // An index and a trigger both carry the table they are over.
    if over.eq_ignore_ascii_case(from) {
        write(2, to);
        changed = true;
    }
    if !places.is_empty() {
        write(4, &crate::rename::written(&sql, &places, to));
        changed = true;
    }
    changed
}

/// Whether `ALTER TABLE` may alter the table `name`, which is
/// `sqlite3AlterRenameTable` locating it and `isAlterableTable` holding
/// it.
///
/// # Errors
///
/// [`Error::NoTable`] where the schema holds no such table or view,
/// [`Error::NotAlterable`] for a table SQLite keeps for itself, and
/// [`Error::NotATable`] for a view.
fn alterable(database: &Database<'_>, name: &[u8]) -> Result<(), Error> {
    let view = database.view(name).is_some();
    if database.table(name).is_none() && !view {
        return Err(Error::NoTable(name.to_vec()));
    }
    // A table SQLite keeps for itself is read and not altered, and the
    // schema's own table answers under the name it is written with.
    if name
        .get(..7)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"sqlite_"))
    {
        let held = if crate::db::schema_named(name) {
            b"sqlite_master".to_vec()
        } else {
            name.to_vec()
        };
        return Err(Error::NotAlterable(held));
    }
    // A view is not a table, so it is not what these statements alter.
    if view {
        return Err(Error::NotATable(name.to_vec()));
    }
    Ok(())
}

/// The statement of a table with the column `column` taken out of it,
/// which is `sqlite_drop_column`: the text of the column goes, and the
/// comma that stood beside it goes with it.
///
/// # Errors
///
/// [`Error::NoTable`] where the statement makes no table of columns
/// written out, [`Error::NoSuchColumn`] where it writes no such column,
/// and [`Error::KeyColumn`] where the column carries a key of its own.
pub(crate) fn without_column(statement: &[u8], column: &[u8]) -> Result<Vec<u8>, Error> {
    let (arena, definition) = crate::parse::definition(statement)?;
    let Definition::Table(made) = definition else {
        return Err(Error::NoTable(Vec::new()));
    };
    let crate::ast::TableBody::Columns { columns, .. } = made.body else {
        return Err(Error::NoTable(Vec::new()));
    };
    let held = arena.columns(columns);
    let at = held
        .iter()
        .position(|def| {
            crate::schema::dequote(def.name.text(statement)).eq_ignore_ascii_case(column)
        })
        .ok_or_else(|| Error::NoSuchColumn(column.to_vec()))?;
    // `sqlite3AlterDropColumn`: a column that carries a key of its own
    // is not one a statement drops.
    let def = held.get(at).ok_or(Error::NoSuchColumn(Vec::new()))?;
    for constraint in arena.column_constraints(def.constraints) {
        let key = match constraint {
            crate::ast::ColumnConstraint::PrimaryKey { .. } => b"PRIMARY KEY".to_vec(),
            crate::ast::ColumnConstraint::Unique(_) => b"UNIQUE".to_vec(),
            _ => continue,
        };
        return Err(Error::KeyColumn(key, column.to_vec()));
    }
    if held.len() <= 1 {
        return Err(Error::LastColumn(column.to_vec()));
    }
    // The text of the column goes with the comma in front of it, or
    // with the comma after it where it is the first column.
    let span = def.written;
    let (start, end) = match held.get(at.saturating_add(1)) {
        Some(next) => (span.start, next.written.start),
        None => (
            statement
                .get(..span.start)
                .and_then(|head| head.iter().rposition(|byte| *byte == b','))
                .unwrap_or(span.start),
            span.start.saturating_add(span.len),
        ),
    };
    let mut out = statement.get(..start).unwrap_or_default().to_vec();
    out.extend_from_slice(statement.get(end..).unwrap_or_default());
    Ok(out)
}

/// Where a statement names the column `column`, as the statement wrote
/// it, which is what says the statement no longer reads once the column
/// is gone. A name written with a table in front of it carries that
/// table, which is what `renameTestSchema` answers.
fn names_column(statement: &[u8], column: &[u8]) -> Option<Vec<u8>> {
    let (arena, definition) = crate::parse::definition(statement).ok()?;
    // `sqlite3StringToId` reads an index term written as text as the
    // name of a column, which the tree holds as a literal.
    if let Definition::Index(made) = definition {
        let named = arena.orders(made.columns).iter().find_map(|term| {
            let Some(crate::ast::Node::Literal(crate::ast::Literal::Text(text))) =
                arena.node(term.expr)
            else {
                return None;
            };
            let name = crate::schema::dequote(text.text(statement));
            name.eq_ignore_ascii_case(column).then_some(name)
        });
        if named.is_some() {
            return named;
        }
    }
    arena.all().find_map(|(_, node)| {
        let crate::ast::Node::Column {
            table,
            column: named,
            ..
        } = node
        else {
            return None;
        };
        if !crate::schema::dequote(named.text(statement)).eq_ignore_ascii_case(column) {
            return None;
        }
        let mut shown = Vec::new();
        if let Some(held) = table {
            shown.extend_from_slice(&crate::schema::dequote(held.text(statement)));
            shown.push(b'.');
        }
        shown.extend_from_slice(&crate::schema::dequote(named.text(statement)));
        Some(shown)
    })
}

/// What a `CREATE` makes, which says what it is refused with where the
/// schema already holds the name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Making {
    /// A table or a view, which are one kind to a `CREATE INDEX`.
    Table,
    /// An index.
    Index,
}

/// What reading a statement of the schema again refuses once a column
/// carries the name `to`, or nothing where it reads, which is
/// `renameTestSchema` under `after rename`.
///
/// Reading one statement costs O(n) in its nodes.
fn reads_after(
    text: &[u8],
    to: &[u8],
    collating: &[crate::value::Collating],
) -> Option<alloc::string::String> {
    let (arena, definition) = crate::parse::definition(text).ok()?;
    let Definition::Table(made) = definition else {
        return None;
    };
    crate::schema::table(&arena, &made, text, collating).err()?;
    // A table the rename leaves with two columns of one name is the one
    // way a statement stops reading, because a name is all the rename
    // writes.
    Some(alloc::format!(
        "duplicate column name: {}",
        alloc::string::String::from_utf8_lossy(to)
    ))
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
    /// The `CREATE INDEX` text, which an expression it holds points
    /// into.
    sql: Vec<u8>,
    /// The tree that text was parsed into.
    arena: Arena,
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

/// One file a connection holds: its pages, its header and the journal
/// beside them.
///
/// A connection holds one of these per database, which `main` is the
/// first of.
struct HeldFile {
    /// The pages of it.
    pages: Pages,
    /// The bytes of the file as it stood under the schema cookie beside
    /// them, which a constraint reads the schema out of.
    ///
    /// A statement that writes n rows would otherwise build the file
    /// once per row, which is O(n) in its pages each time.
    schema_bytes: Option<(u32, Vec<u8>)>,
    /// The header beside them, which every commit writes again.
    header: Header,
    /// The mode a commit writes the rollback journal under.
    mode: Mode,
    /// The nonce every checksum of the journal begins at, which comes
    /// from SQLite's random source.
    nonce: u32,
    /// The page size a `PRAGMA page_size` named after the first table
    /// was made, which the next `VACUUM` writes the file under.
    wanted_page: Option<u32>,
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
    /// Whether the commit after this one begins the log again, which a
    /// checkpoint that left the frames where they are asks for.
    restarting: bool,
    /// The header as the open transaction began, where a `BEGIN` opened
    /// one. A connection outside `BEGIN` holds none and commits every
    /// statement of its own.
    began: Option<Header>,
    /// What the commits since the caller last read them did to the
    /// files, in the order they did it.
    did: Vec<Does>,
}

/// Which file of a database one write of a commit reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Onto {
    /// The database file.
    Main,
    /// The rollback journal beside it.
    Journal,
    /// The write-ahead log beside it.
    Log,
}

/// One thing the commit of a transaction does to the files, in the order
/// `sqlite3PagerCommitPhaseOne` of `research/sqlite/src/pager.c` does it.
///
/// A caller that simulates a machine losing power applies a prefix of
/// these and garbles what the prefix leaves pending, which is what the
/// crash layer of `research/sqlite/src/test6.c` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Does {
    /// Writes `bytes` at `at` bytes into one file.
    Write {
        /// Which file.
        onto: Onto,
        /// How many bytes into it the write begins.
        at: u64,
        /// What it writes.
        bytes: Vec<u8>,
    },
    /// Holds one file on the disk, which is `xSync`.
    Sync(Onto),
    /// Cuts one file back to `at` bytes.
    Truncate {
        /// Which file.
        onto: Onto,
        /// How many bytes are left.
        at: u64,
    },
    /// Takes one file away, which is `xDelete`.
    Remove(Onto),
}

/// What one statement names: the schema it wrote, the name the database
/// that holds it is found by, whether the refusal for a schema the
/// connection does not hold names that name, and whether the statement
/// writes the temp schema.
struct Names {
    /// The schema the statement wrote, where it wrote one.
    schema: Option<Span>,
    /// The name of the thing the statement locates, which says which
    /// database holds it where the statement wrote no schema.
    located: Option<Span>,
    /// What the statement names under the schema.
    locates: Located,
    /// Whether the statement was written `TEMP`.
    temporary: bool,
    /// Whether a statement that names the temp schema opens it, which an
    /// `ATTACH` and a `DETACH` do not.
    opens: bool,
}

/// What one statement names under the schema it wrote.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Located {
    /// Nothing the schema must hold, which a statement that makes
    /// something names. A schema the connection holds no database under
    /// is refused `unknown database`.
    Nothing,
    /// A name any database the connection holds may hold, which a `DROP`
    /// of an index, a view or a trigger names. A schema the connection
    /// holds no database under is refused `no such table:`.
    Name,
    /// A table the schema it wrote is the only one that may hold.
    Table,
}

impl Names {
    /// One statement that makes something, which is refused `unknown
    /// database` for a schema the connection does not hold.
    const fn made(schema: Option<Span>, located: Option<Span>, temporary: bool) -> Self {
        Names {
            schema,
            located,
            locates: Located::Nothing,
            temporary,
            opens: true,
        }
    }

    /// One statement that locates a name, which is refused `no such
    /// table:` for a schema the connection does not hold.
    const fn located(schema: Option<Span>, name: Span, locates: Located) -> Self {
        Names {
            schema,
            located: Some(name),
            locates,
            temporary: false,
            opens: true,
        }
    }

    /// One statement that names a database rather than reading or writing
    /// one, which an `ATTACH` and a `DETACH` are the two of.
    const fn naming() -> Self {
        Names {
            schema: None,
            located: None,
            locates: Located::Nothing,
            temporary: false,
            opens: false,
        }
    }
}

/// The files a connection holds, as a statement of it reads them.
struct Images {
    /// The file the statement writes.
    held: Vec<u8>,
    /// The others, each with the name a statement names it by.
    beside: Vec<(Vec<u8>, Vec<u8>)>,
}

/// What one database of a connection is called: the name a statement
/// names it by, the file name `PRAGMA database_list` answers, and its
/// schema place.
#[derive(Clone, Default)]
struct Called {
    /// The name, with its quotes taken off.
    name: Vec<u8>,
    /// The file name the `ATTACH` named, which is empty for `main` and
    /// for a database of the connection's own.
    file: Vec<u8>,
    /// Where it stands in the list `PRAGMA database_list` answers: nought
    /// for `main`, one for the temp schema, and two and up for an
    /// attached database in the order they were attached.
    place: usize,
}

/// One database of a connection beside the one the statement running now
/// writes.
struct Attached {
    /// What it is called.
    called: Called,
    /// The file itself.
    held: HeldFile,
}

/// The function a connection answers the image of a file name with,
/// which `ATTACH` calls.
///
/// A file name the function answers nothing for is refused `unable to
/// open database: <file>`. A name of no bytes and the name `:memory:`
/// reach no function, because both stand for a database of one page.
pub type Opening = fn(&[u8]) -> Option<Vec<u8>>;

/// The function `sqlite3_commit_hook` told the connection, which the
/// connection asks where a transaction that wrote a page ends.
///
/// A true answer refuses the commit and sends the transaction back,
/// which `sqlite3VdbeHalt` of `research/sqlite/src/vdbeaux.c:2982`
/// answers `SQLITE_CONSTRAINT_COMMITHOOK` for.
pub type Committing = fn() -> bool;

/// The function `sqlite3_rollback_hook` told the connection, which
/// `sqlite3RollbackAll` of `research/sqlite/src/main.c:1535` tells where
/// a transaction that had written goes back.
pub type Rolling = fn();

/// What a statement did to one row of a table that keeps a key of its
/// own, which the connection tells the function
/// `sqlite3_update_hook` told it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Did {
    /// The statement wrote the row.
    Insert,
    /// The statement changed the row.
    Update,
    /// The statement took the row away.
    Delete,
}

impl Did {
    /// The word of the action, which is what the second argument of the
    /// function stands for.
    #[must_use]
    pub const fn word(self) -> &'static [u8] {
        match self {
            Did::Insert => b"INSERT",
            Did::Update => b"UPDATE",
            Did::Delete => b"DELETE",
        }
    }
}

/// One row a statement wrote, as the connection tells it.
pub struct Wrote<'a> {
    /// What the statement did to the row.
    pub did: Did,
    /// The name of the database that holds the table.
    pub schema: &'a [u8],
    /// The name of the table.
    pub table: &'a [u8],
    /// The key of the row.
    pub rowid: i64,
}

/// The function `sqlite3_update_hook` told the connection, which every
/// row a statement writes in a table that keeps a key of its own tells.
///
/// `sqlite3_update_hook` of `research/sqlite/src/main.c:2439` reaches no
/// row of a `WITHOUT ROWID` table and no row of a table the library keeps
/// for itself.
pub type Writing = fn(&Wrote<'_>);

/// One row a statement is about to write, as the connection tells it.
pub struct Peeked<'a> {
    /// What the statement will do to the row.
    pub did: Did,
    /// The name of the database that holds the table.
    pub schema: &'a [u8],
    /// The name of the table.
    pub table: &'a [u8],
    /// The key the row stands under now, which a row being written
    /// carries as the key it takes.
    pub was: i64,
    /// The key the row will stand under.
    pub key: i64,
    /// The values the row holds now, and nothing where the statement
    /// writes the row.
    pub old: Option<&'a [Value]>,
    /// The values the row will hold, and nothing where the statement
    /// takes the row away.
    pub new: Option<&'a [Value]>,
    /// How many triggers deep the statement stands, which
    /// `sqlite3_preupdate_depth` answers.
    pub depth: usize,
}

/// The function `sqlite3_preupdate_hook` told the connection, which every
/// row a statement is about to write in a table that keeps a key of its
/// own tells before the write.
///
/// `sqlite3_preupdate_hook` of `research/sqlite/src/main.c:2415` is told
/// the row a `REPLACE` writes over as well, which the function
/// `sqlite3_update_hook` told the connection is not.
pub type Peeking = fn(&Peeked<'_>);

/// A database being written, statement by statement.
pub struct Writer {
    /// The file the statement running now writes, which is `main` where
    /// no statement of the connection named another database.
    held: HeldFile,
    /// What that database is called.
    called: Called,
    /// The databases of the connection beside it, in no order of their
    /// own: the schema place of each says where it stands.
    attached: Vec<Attached>,
    /// The function `ATTACH` answers a file name with, and nothing where
    /// the caller told the connection none.
    opening: Option<Opening>,
    /// The files a `VACUUM ... INTO` wrote, each with the name the
    /// statement gave it, which the client writes beside the databases
    /// the connection attached.
    made: Vec<(Vec<u8>, Vec<u8>)>,
    /// Where `random` and `randomblob` take their bytes from, which
    /// comes from SQLite's random source as well.
    random: crate::random::Source,
    /// The moment `now` names, as the seconds since 1970, and nothing
    /// where the caller told the connection none.
    clock: Option<i64>,
    /// What the connection was told for each pragma of
    /// [`crate::pragma::HELD`], where it was told one.
    kept: Vec<Option<i64>>,
    /// The triggers running now, innermost last, which is what stops a
    /// trigger that reaches itself where `PRAGMA recursive_triggers`
    /// is off.
    running: Vec<Vec<u8>>,
    /// What the connection was told for the two pragmas that hold a
    /// truth value outside [`crate::pragma::HELD`].
    truth: Truths,
    /// The function `sqlite3_set_authorizer` told the connection, which
    /// every statement is read against before it runs.
    asking: Option<crate::auth::Asking>,
    /// The function `sqlite3_commit_hook` told the connection, which a
    /// transaction that wrote a page asks before it writes.
    committing: Option<Committing>,
    /// The function `sqlite3_rollback_hook` told the connection, which a
    /// transaction that goes back tells.
    rolling: Option<Rolling>,
    /// The function `sqlite3_update_hook` told the connection, which
    /// every row a statement writes in a table that keeps a key of its
    /// own tells.
    writes: Option<Writing>,
    /// The function `sqlite3_preupdate_hook` told the connection, which
    /// every row a statement is about to write tells before the write.
    peeking: Option<Peeking>,
    /// The columns of the `UPDATE` running now that the function
    /// ignored, which keep the value they had.
    unwritten: Vec<Vec<u8>>,
    /// What the connection has written, which `changes()`,
    /// `total_changes()` and `last_insert_rowid()` answer.
    counted: crate::func::Counted,
    /// How many rows the statement running now has written, which is
    /// what `changes()` answers where the statement stops where it
    /// stands and keeps them.
    writing: i64,
    /// How many foreign keys held at the end of the transaction point
    /// at no row, which is the counter `sqlite3VdbeCheckFk` reads at a
    /// `COMMIT`.
    deferred: i64,

    /// What the statement running now does beyond refusing the row
    /// where it breaks a constraint.
    refusing: Refusing,
    /// The functions the application defined on this connection.
    defined: &'static [crate::func::Defined],
    /// The aggregates the application defined on this connection.
    grouped: &'static [crate::func::Grouped],
    /// The collations the application defined on this connection.
    collating: &'static [crate::value::Collating],
    /// The savepoints open now, the outermost first, each holding the
    /// file as it stood when the `SAVEPOINT` ran.
    saved: Vec<Saved>,
    /// The rows the `RETURNING` of the statement running now answered.
    returned: Vec<Vec<Value>>,
    /// Whether the statement running now is one this crate writes for
    /// itself, which is what `sqlite3CheckObjectName` lets name a table
    /// SQLite keeps for itself.
    making_own: bool,
}

/// One open savepoint: its name, and the file as it stood when the
/// `SAVEPOINT` that opened it ran.
struct Saved {
    /// The name the statement wrote, with its quotes taken off.
    name: Vec<u8>,
    /// Every database the connection held, as it stood.
    files: Vec<SavedFile>,
    /// How many foreign keys held at the end of the transaction pointed
    /// at no row.
    deferred: i64,
    /// Whether this savepoint opened the transaction, which is what
    /// makes releasing it a commit.
    opener: bool,
}

/// One database of a connection as a savepoint found it.
#[derive(Clone)]
struct SavedFile {
    /// The name the connection knows the database by.
    name: Vec<u8>,
    /// The pages as they stood.
    pages: Pages,
    /// The header as it stood.
    header: Header,
}

/// What a connection was told for the two pragmas that hold a truth
/// value and stand outside [`crate::pragma::HELD`], because neither
/// answers the row that table answers.
#[derive(Clone, Copy, Debug, Default)]
struct Truths {
    /// Whether a statement that changes rows answers how many it
    /// changed, which `PRAGMA count_changes` sets.
    counting: bool,
    /// Whether `LIKE` tells the twenty-six letters apart, which
    /// `PRAGMA case_sensitive_like` sets.
    sensitive: bool,
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
            held: HeldFile {
                pages,
                schema_bytes: None,
                mode: Mode::Delete,
                nonce: 0,
                wanted_page: None,
                sector: SECTOR,
                journal: None,
                log: None,
                origin: None,
                restarting: false,
                began: None,
                did: Vec::new(),
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
            },
            called: Called {
                name: b"main".to_vec(),
                file: Vec::new(),
                place: 0,
            },
            attached: Vec::new(),
            opening: None,
            made: Vec::new(),
            random: crate::random::Source::default(),
            clock: None,
            kept: alloc::vec![None; crate::pragma::HELD.len()],
            running: Vec::new(),
            truth: Truths::default(),
            asking: None,
            committing: None,
            rolling: None,
            writes: None,
            peeking: None,
            unwritten: Vec::new(),
            counted: crate::func::Counted::default(),
            writing: 0,
            deferred: 0,
            refusing: Refusing::Abort,
            defined: &[],
            grouped: &[],
            collating: &[],
            saved: Vec::new(),
            returned: Vec::new(),
            making_own: false,
        })
    }

    /// A connection over the three files a database is kept in: the file
    /// itself, the rollback journal beside it, and the write-ahead log
    /// beside it.
    ///
    /// A journal that holds what recovering the database needs is played
    /// back first, which is `pager_playback` running out of
    /// `sqlite3PagerSharedLock`; a log is read after it, and the pages it
    /// holds are the ones the connection reads and writes further frames
    /// beside. A journal that is not hot and a log that holds no frame
    /// each leave the file as it stands.
    ///
    /// Recovering costs O(p) in the bytes of the pages of the file and
    /// the frames of the log.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the header of the file breaks.
    pub fn recovered(image: &[u8], journal: &[u8], log: &[u8]) -> Result<Self, Error> {
        let played = crate::journal::Journal::open(journal).rolled_back(image);
        let Some(held) = Log::opened(log) else {
            return Self::opened(&played);
        };
        let read = crate::wal::Wal::open(log).ok();
        let pages = read.as_ref().map_or(0, crate::wal::Wal::pages);
        let page_size = crate::bytes::size(u64::from(held.page_size()));
        let mut image = played.clone();
        image.resize(
            page_size.saturating_mul(crate::bytes::size(u64::from(pages))),
            0,
        );
        for number in 1..=pages {
            let Some(bytes) = read.as_ref().and_then(|read| read.page_bytes(number)) else {
                continue;
            };
            let at =
                page_size.saturating_mul(crate::bytes::size(u64::from(number.saturating_sub(1))));
            for (slot, byte) in image.iter_mut().skip(at).take(page_size).zip(bytes) {
                *slot = *byte;
            }
        }
        // A log of no committed frame leaves the file as it stands, which
        // is what `walIndexReadHdr` finding no frame answers.
        let mut writer = match Self::opened(if pages == 0 { &played } else { &image }) {
            Ok(writer) => writer,
            // A file of no bytes beside a log of no committed frame is a
            // database of no page, which is what a process that was
            // ended before its first commit leaves.
            Err(error) if played.is_empty() => {
                Self::new(held.page_size(), 0, crate::header::Encoding::Utf8).or(Err(error))?
            }
            Err(error) => return Err(error),
        };
        writer.held.origin = Some(played);
        writer.held.log = Some(held);
        writer.held.header.write_version = 2;
        writer.held.header.read_version = 2;
        Ok(writer)
    }

    /// A connection over a database that is already written, which is
    /// what opening a file that was there answers.
    ///
    /// The header of the image says what the pages hold; the settings a
    /// connection is told, from the journal mode to the seed, are the
    /// ones a new connection begins with and not the ones the
    /// connection that wrote the file was told.
    ///
    /// Splitting the image costs O(n) in its pages.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the header breaks, and what the image
    /// breaks where it carries fewer bytes than the header claims
    /// pages.
    pub fn opened(image: &[u8]) -> Result<Self, Error> {
        let header = Header::parse(image)?;
        let pages = Pages::opened(image, &header)?;
        Ok(Writer {
            held: HeldFile {
                pages,
                schema_bytes: None,
                header,
                mode: Mode::Delete,
                nonce: 0,
                wanted_page: None,
                sector: SECTOR,
                journal: None,
                log: None,
                origin: None,
                restarting: false,
                began: None,
                did: Vec::new(),
            },
            called: Called {
                name: b"main".to_vec(),
                file: Vec::new(),
                place: 0,
            },
            attached: Vec::new(),
            opening: None,
            made: Vec::new(),
            random: crate::random::Source::default(),
            clock: None,
            kept: alloc::vec![None; crate::pragma::HELD.len()],
            running: Vec::new(),
            truth: Truths::default(),
            asking: None,
            committing: None,
            rolling: None,
            writes: None,
            peeking: None,
            unwritten: Vec::new(),
            counted: crate::func::Counted::default(),
            writing: 0,
            deferred: 0,
            refusing: Refusing::Abort,
            defined: &[],
            grouped: &[],
            collating: &[],
            saved: Vec::new(),
            returned: Vec::new(),
            making_own: false,
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

    /// The moment `now` names, as the seconds since 1970.
    ///
    /// SQLite reads the clock of the operating system, which this crate
    /// has none of, so the caller says what the clock says and a
    /// connection that is not told refuses a statement naming `now`,
    /// `CURRENT_TIME`, `CURRENT_DATE` or `CURRENT_TIMESTAMP`.
    pub const fn clocking(&mut self, seconds: i64) {
        self.clock = Some(seconds);
    }

    /// The moment the caller told the connection, as the seconds since
    /// 1970, and nothing where the caller told it none.
    ///
    /// Reading it costs O(1).
    #[must_use]
    pub const fn clock(&self) -> Option<i64> {
        self.clock
    }

    /// Whether `LIKE` tells the twenty-six letters apart on this
    /// connection, which `PRAGMA case_sensitive_like` sets and a caller
    /// that opens a [`Database`] of its own passes to
    /// [`Database::sensitively`].
    #[must_use]
    pub const fn sensitive(&self) -> bool {
        self.truth.sensitive
    }

    /// What the next draw follows from, which `save_prng_state` holds
    /// and hands back to [`Writer::randomness`].
    ///
    /// Reading it costs O(1).
    #[must_use]
    pub const fn randomness_held(&self) -> u64 {
        self.random.held()
    }

    /// `PRAGMA journal_mode`, with the nonce every checksum of the
    /// journal begins at and the sector its header takes. The five
    /// modes here are the ones that write the database file itself;
    /// [`Writer::logging`] is the sixth.
    pub const fn journalling(&mut self, mode: Mode, nonce: u32, sector: u32) {
        self.held.mode = mode;
        self.held.nonce = nonce;
        self.held.sector = sector;
    }

    /// `PRAGMA auto_vacuum`, which gives the file pointer maps: page two
    /// becomes the first map page, so the first table takes page three.
    ///
    /// The pragma writes the file, so it stands before the first
    /// statement and before [`Writer::logging`].
    pub fn vacuuming(&mut self, incremental: bool) {
        self.held.pages.vacuums();
        // The pragma is itself a change, and the largest root a file
        // with no table holds is page one.
        self.held.header.change_counter = 1;
        self.held.header.version_valid_for = 1;
        self.held.header.largest_root = 1;
        self.held.header.incremental_vacuum = u32::from(incremental);
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
        self.held.header.write_version = 2;
        self.held.header.read_version = 2;
        self.held.header.change_counter = self.held.header.change_counter.saturating_add(1);
        self.held.header.version_valid_for = self.held.header.change_counter;
        self.held.origin = Some(self.held.pages.written(&self.held.header));
        let log = Log::new(self.held.header.page_size, salt, 0, false);
        // The log is on the disk once the pragma answers, which is
        // `sqlite3WalOpen` writing its header and holding it there.
        self.held.did.push(Does::Write {
            onto: Onto::Log,
            at: 0,
            bytes: log.bytes().to_vec(),
        });
        self.held.did.push(Does::Sync(Onto::Log));
        self.held.log = Some(log);
    }

    /// The temp schema the connection holds, and nothing where it holds
    /// none.
    ///
    /// A temp table belongs to the connection that made it, which
    /// `sqlite3TwoPartName` of `research/sqlite/src/build.c` writes into
    /// the schema at place one, so a caller that shares one writer
    /// between connections carries this itself. Building the image costs
    /// O(n) in the pages of the temp schema.
    #[must_use]
    pub fn temp(&self) -> Option<Vec<u8>> {
        self.attached
            .iter()
            .find(|held| named_as(held, b"temp"))
            .map(|held| written_image(&held.held))
    }

    /// The temp schema of the connection set to what `bytes` holds, and
    /// given up where the caller hands over nothing.
    ///
    /// Reading the image costs O(n) in its pages.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the header of the image breaks.
    pub fn temps(&mut self, bytes: Option<&[u8]>) -> Result<(), Error> {
        self.attached.retain(|held| !named_as(held, b"temp"));
        let Some(bytes) = bytes.filter(|bytes| !bytes.is_empty()) else {
            return Ok(());
        };
        let mut held = self.opened_file(b":memory:")?;
        held.header = Header::parse(bytes)?;
        held.pages = Pages::opened(bytes, &held.header)?;
        self.attached.push(Attached {
            called: Called {
                name: b"temp".to_vec(),
                file: Vec::new(),
                place: 1,
            },
            held,
        });
        Ok(())
    }

    /// The encoding the connection keeps its text in, which a reader
    /// built over a file of no page is told by [`Database::encoded`].
    #[must_use]
    pub const fn encoding(&self) -> Encoding {
        self.held.header.encoding
    }

    /// What the connection has written, which `changes()`,
    /// `total_changes()` and `last_insert_rowid()` answer: a reader
    /// built over this connection's file is told it by
    /// [`Database::counting`].
    #[must_use]
    pub const fn counts(&self) -> crate::func::Counted {
        self.counted
    }

    /// The counters this connection stands at, which a caller that
    /// holds one writer per file and several connections over it sets
    /// before each statement.
    ///
    /// The three counters belong to a connection and the pages belong
    /// to a file, so a caller that shares a writer between connections
    /// carries them itself.
    pub const fn counts_as(&mut self, counted: crate::func::Counted) {
        self.counted = counted;
    }

    /// The file the statements so far have made. In write-ahead logging
    /// mode that is the file the pragma left, because no checkpoint has
    /// written a frame back into it.
    #[must_use]
    pub fn written(&self) -> Vec<u8> {
        written_image(&self.held)
    }

    /// The file as the connection that writes it reads it: every page
    /// it has written, which a log has taken the committed ones of and
    /// which holds the ones of an open transaction besides.
    ///
    /// `sqlite3PagerGet` of `research/sqlite/src/pager.c` answers the
    /// page of the cache, so the connection that wrote a page reads
    /// what it wrote whether the log has taken it or not.
    ///
    /// Building the image costs O(n) in the pages.
    #[must_use]
    pub fn inside(&self) -> Vec<u8> {
        self.held.pages.written(&self.held.header)
    }

    /// The file as the transaction of this connection found it, which is
    /// what a connection that did not begin that transaction reads:
    /// `sqlite3PagerSharedLock` answers the pages of the file and a
    /// transaction another connection began has written none of them
    /// yet.
    ///
    /// A connection with no transaction open has written every page it
    /// holds, and [`Writer::written`] is the file it answers.
    ///
    /// Building the image costs O(n) in the pages.
    #[must_use]
    pub fn outside(&self) -> Vec<u8> {
        match &self.held.origin {
            Some(bytes) => bytes.clone(),
            None => self.held.pages.committed(&self.held.header),
        }
    }

    /// The image of the database the connection holds under `name`, and
    /// nothing where it holds none under that name.
    ///
    /// The client writes the bytes back to the file it answered the
    /// `ATTACH` of that name with.
    ///
    /// Building the image costs O(n) in the pages of that database.
    #[must_use]
    pub fn attached_written(&self, name: &[u8]) -> Option<Vec<u8>> {
        let held = self.attached.iter().find(|held| named_as(held, name))?;
        Some(written_image(&held.held))
    }

    /// The databases beside the one the connection writes, in the order
    /// of their schema places, which is the order `sqlite3FindTable`
    /// reads them in.
    ///
    /// Sorting them costs O(n log n) in their number.
    fn in_place(&self) -> Vec<&Attached> {
        let mut out: Vec<&Attached> = self.attached.iter().collect();
        out.sort_by_key(|held| held.called.place);
        out
    }

    /// The image of every database an `ATTACH` added that names a file,
    /// each with that file name.
    ///
    /// The client writes the bytes back to the file it answered the
    /// `ATTACH` with, so a second connection over that file reads what
    /// this one wrote. The file a `VACUUM ... INTO` wrote stands among
    /// them. Building them costs O(n) in the pages of those databases.
    #[must_use]
    pub fn attached_files(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut out: Vec<(Vec<u8>, Vec<u8>)> = self
            .attached
            .iter()
            .filter(|held| !held.called.file.is_empty())
            .map(|held| (held.called.file.clone(), written_image(&held.held)))
            .collect();
        out.extend(self.made.iter().cloned());
        out
    }

    /// The names of the databases an `ATTACH` added, in the order they
    /// were attached.
    ///
    /// Reading them costs O(n) in their number.
    #[must_use]
    pub fn attached_names(&self) -> Vec<Vec<u8>> {
        self.in_place()
            .into_iter()
            .map(|held| held.called.name.clone())
            .collect()
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
        let added = crate::schema::dequote(asked.column.name.text(sql));
        {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            // `sqlite3AlterBeginAddColumn` builds the table again with
            // the column written into it, so a view and a name the
            // table already holds are both refused before the column is
            // read.
            if database.view(&name).is_some() {
                return Err(Error::Added(crate::db::Added::View));
            }
            let (table, _) = database
                .table(&name)
                .ok_or_else(|| Error::NoTable(name.clone()))?;
            if table
                .columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(&added))
            {
                return Err(Error::Schema(crate::schema::Error::DuplicateColumn(added)));
            }
        }
        refused_column(arena, asked, sql, (self.holds_rows(&name)?, self.holding()))?;
        let rowid = self.row_of(&name)?.ok_or(Error::NoTable(Vec::new()))?;
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
        let value = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                value(b"table"),
                value(&name),
                value(&name),
                Value::Int(i64::from(root)),
                value(&text),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        drop(database);
        crate::tree::update(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        self.reread(&name, b"add column")
    }

    /// The schema read again after an `ALTER TABLE` wrote a statement
    /// into it, which is the `corruptSchema` of `src/prepare.c` naming
    /// the alter that left it unreadable.
    ///
    /// Reading the schema costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error::AfterAlter`] carries the table, the words for the kind
    /// of alter, and what reading the schema refused.
    fn reread(&self, name: &[u8], word: &[u8]) -> Result<(), Error> {
        let bytes = self.image();
        match Database::open(&bytes) {
            Ok(_) => Ok(()),
            Err(error) => Err(Error::AfterAlter(
                name.to_vec(),
                word.to_vec(),
                error.message().into_bytes(),
            )),
        }
    }

    /// Which row of `sqlite_schema` names the table, which is the row a
    /// statement that changes the table writes again, and nothing where
    /// no row of the schema names it.
    fn row_of(&self, name: &[u8]) -> Result<Option<i64>, Error> {
        let bytes = self.image();
        let image = crate::image::Image::open(&bytes)?;
        let encoding = image.header().encoding;
        let mut payload = Vec::new();
        let mut found = None;
        for row in image.schema() {
            let row = row?;
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            if is_text(record.value(0)?, b"table", encoding)
                && is_text(record.value(1)?, name, encoding)
            {
                found = Some(row.rowid);
                break;
            }
        }
        Ok(found)
    }

    /// `ALTER TABLE ... RENAME TO`: every row of `sqlite_schema` that
    /// names the table is written again under the new name, which is
    /// `sqlite3AlterRenameTable`.
    ///
    /// The tree of the table stays where it is, so the rename writes the
    /// schema and nothing else. Reading the schema costs O(n) in its
    /// rows, and each row is written again at O(m) in its text.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the schema holds no such table, and
    /// [`Error::Named`] where it already holds the new name.
    fn rename_table(&mut self, asked: &crate::ast::RenameTable, sql: &[u8]) -> Result<(), Error> {
        let from = crate::schema::dequote(asked.table.text(sql));
        let to = crate::schema::dequote(asked.name.text(sql));
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        alterable(&database, &from)?;
        if names(&database, &to) {
            return Err(Error::Named(to));
        }
        let mut written: Vec<(i64, Vec<Value>)> = Vec::new();
        for (rowid, values) in database.rows_of(SCHEMA_TABLE)? {
            let mut values = values;
            if renamed(&mut values, &from, &to) {
                written.push((rowid, values));
            }
        }
        drop(database);
        for (rowid, values) in written {
            self.schema_row(rowid, &values)?;
        }
        self.rename_sequence(&from, &to)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// `ATTACH [DATABASE] file AS name`: the image the opening function
    /// answers for the file name becomes the database the connection
    /// holds under that name, which is `attachFunc` of
    /// `research/sqlite/src/attach.c:81`.
    ///
    /// Reading the image costs O(n) in its pages.
    ///
    /// # Errors
    ///
    /// [`Error::TooManyAttached`] past the tenth database,
    /// [`Error::DatabaseInUse`] for a name the connection already holds
    /// a database under, [`Error::NoDatabaseFile`] for a file name the
    /// opening function answers nothing for, and
    /// [`Error::AttachEncoding`] for a file whose encoding is not the one
    /// of `main`.
    fn attach(
        &mut self,
        arena: &Arena,
        asked: crate::ast::Attach,
        sql: &[u8],
    ) -> Result<(), Error> {
        let file = self.text_of(arena, asked.file, sql)?;
        let name = self.text_of(arena, asked.name, sql)?;
        // The temp schema holds a place of its own and is none of the ten
        // an `ATTACH` may add, which `db->nDb>=db->aLimit+2` of
        // `attachFunc` counts by leaving `main` and `temp` out.
        let held = self
            .attached
            .iter()
            .filter(|held| held.called.place > 1)
            .count();
        if held >= crate::db::ATTACHED {
            return Err(Error::TooManyAttached);
        }
        if named_database(&name) || self.attached.iter().any(|held| named_as(held, &name)) {
            return Err(Error::DatabaseInUse(name));
        }
        let held = self.opened_file(&file)?;
        if held.header.encoding != self.held.header.encoding {
            return Err(Error::AttachEncoding);
        }
        // `sqlite3BtreeGetFilename` answers no name for a database of
        // this connection's own, which is what `PRAGMA database_list`
        // writes for one.
        let file = if fresh_file(&file) { Vec::new() } else { file };
        let held = joined(held, self.held.began.is_some());
        let place = self
            .attached
            .iter()
            .map(|held| held.called.place)
            .chain(core::iter::once(1))
            .max()
            .unwrap_or(1)
            .saturating_add(1);
        self.attached.push(Attached {
            called: Called { name, file, place },
            held,
        });
        Ok(())
    }

    /// The file an `ATTACH` names: a database of one page where the name
    /// is `:memory:` or no bytes at all, and the image the opening
    /// function answers otherwise.
    ///
    /// # Errors
    ///
    /// [`Error::NoDatabaseFile`] where the connection was told no
    /// opening function or the function answered nothing, and
    /// [`Error::Image`] for an image whose header the library refuses.
    fn opened_file(&self, file: &[u8]) -> Result<HeldFile, Error> {
        let fresh = || {
            let held = self.held.header;
            let made = Writer::new(held.page_size, held.reserved, held.encoding)?;
            Ok(made.held)
        };
        if fresh_file(file) {
            return fresh();
        }
        let missing = || Error::NoDatabaseFile(file.to_vec());
        let opening = self.opening.ok_or_else(missing)?;
        let image = opening(file).ok_or_else(missing)?;
        if image.is_empty() {
            return fresh();
        }
        let header = Header::parse(&image)?;
        let pages = Pages::opened(&image, &header)?;
        let mut made = self.opened_file(b":memory:")?;
        made.pages = pages;
        made.header = header;
        Ok(made)
    }

    /// `DETACH [DATABASE] name`: the database the connection holds under
    /// that name goes out of the list, which is `detachFunc` of
    /// `research/sqlite/src/attach.c:290`.
    ///
    /// # Errors
    ///
    /// [`Error::KeptDatabase`] for `main`, and [`Error::NoDatabase`] for
    /// a name the connection holds no database under, which `temp` is
    /// one of until the connection makes one.
    fn detach(
        &mut self,
        arena: &Arena,
        asked: crate::ast::Detach,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = self.text_of(arena, asked.name, sql)?;
        let found = self.attached.iter().position(|held| named_as(held, &name));
        let Some(at) = found else {
            if name.eq_ignore_ascii_case(b"main") {
                return Err(Error::KeptDatabase(name));
            }
            return Err(Error::NoDatabase(name));
        };
        if named_database(&name) {
            return Err(Error::KeptDatabase(name));
        }
        let held = self.attached.remove(at);
        // `sqlite3DetachDatabase` moves the databases after the one it
        // took away down, so the places count on with no gap.
        for beside in &mut self.attached {
            if beside.called.place > held.called.place {
                beside.called.place = beside.called.place.saturating_sub(1);
            }
        }
        Ok(())
    }

    /// The text one expression of a statement answers, which is what
    /// `sqlite3_value_text` of the argument of `attachFunc` reads.
    ///
    /// # Errors
    ///
    /// [`Error::Eval`] names what the expression refused.
    fn text_of(&self, arena: &Arena, id: crate::ast::ExprId, sql: &[u8]) -> Result<Vec<u8>, Error> {
        // `resolveAttachExpr` of `research/sqlite/src/attach.c:35` reads
        // a bare name as the text of that name and every other
        // expression as an expression.
        if let Some(name) = bare_text(arena, id, sql) {
            return Ok(name);
        }
        let value = crate::eval::evaluate(arena, id, sql, self.clock)?;
        Ok(value.text().unwrap_or_default())
    }

    /// `ALTER TABLE ... DROP CONSTRAINT`, and `ALTER TABLE ... ALTER
    /// COLUMN ... DROP NOT NULL`: the text of the constraint goes out
    /// of the statement that made the table, which is
    /// `alterDropConstraintFunc`.
    ///
    /// The rows of the table are not written again, because a
    /// constraint holds no value.
    ///
    /// Reading the tokens costs O(n) in the bytes of the statement.
    ///
    /// # Errors
    ///
    /// [`Error::NotAlterable`] for a table SQLite keeps for itself,
    /// [`Error::NoSuchColumn`] where the table holds no such column,
    /// [`Error::NoConstraint`] where it holds no constraint of that
    /// name, and [`Error::KeptConstraint`] where that constraint is
    /// neither a `CHECK` nor a `NOT NULL`.
    fn drop_constraint(
        &mut self,
        arena: &Arena,
        asked: &crate::ast::DropConstraint,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(asked.table.text(sql));
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        alterable(&database, &name)?;
        // The table was located above, so this refusal carries no name
        // of its own.
        let (table, root) = database.table(&name).ok_or(Error::NoTable(Vec::new()))?;
        let (statement, _) = database
            .written_as(&name)
            .ok_or(Error::NoTable(Vec::new()))?;
        let at = |named: Span| {
            let named = crate::schema::dequote(named.text(sql));
            table
                .columns
                .iter()
                .position(|held| held.name.eq_ignore_ascii_case(&named))
                .ok_or(Error::Eval(crate::eval::Error::NoColumn(named)))
        };
        let text = match asked.which {
            crate::ast::Constrained::Named(named) => {
                let named = crate::schema::dequote(named.text(sql));
                crate::constraint::without(statement, crate::constraint::Dropped::Named(&named))?
            }
            crate::ast::Constrained::NotNull(named) => {
                let held = crate::constraint::Dropped::NotNull(at(named)?);
                crate::constraint::without(statement, held)?
            }
            crate::ast::Constrained::SetNotNull(named, written) => {
                let place = at(named)?;
                self.holds_values(&name, place)?;
                crate::constraint::with(statement, Some(place), written.text(sql))
            }
            crate::ast::Constrained::Add(written, named, value) => {
                if let Some(named) = named {
                    let named = crate::schema::dequote(named.text(sql));
                    if crate::constraint::holds(statement, &named) {
                        return Err(Error::HeldConstraint(named));
                    }
                }
                self.holds_check((table, &name), (arena, value, sql))?;
                crate::constraint::with(statement, None, written.text(sql))
            }
        };
        let schema_rowid = self.row_of(&name)?.ok_or(Error::NoTable(Vec::new()))?;
        drop(database);
        let value = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                value(b"table"),
                value(&name),
                value(&name),
                Value::Int(i64::from(root)),
                value(&text),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        let schema = crate::image::SCHEMA_ROOT;
        crate::tree::update(&mut self.held.pages, schema, schema_rowid, &row)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// Whether every row of the table is held by a `CHECK` an `ALTER
    /// TABLE` writes, which `sqlite3AlterAddConstraint` reads before it
    /// writes the constraint.
    ///
    /// Reading the rows costs O(n) in them and what the clause costs
    /// per row.
    ///
    /// # Errors
    ///
    /// [`Error::Constraint`] where a row is not held, and whatever the
    /// clause could not answer.
    fn holds_check(
        &self,
        over: (&Table, &[u8]),
        held: (&Arena, crate::ast::ExprId, &[u8]),
    ) -> Result<(), Error> {
        let (table, name) = over;
        let (arena, value, sql) = held;
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        for (_, values) in database.held_rows_of(name)? {
            let row = Indexing {
                table,
                values: &values,
                encoding: self.held.header.encoding,
            };
            // `sqlite3ExprIfFalse`: a `CHECK` holds where it answers
            // anything but false, so a row that answers nothing holds.
            let answer = crate::eval::evaluate_row(arena, value, sql, &row)?;
            if !answer.truth(true) {
                return Err(Error::Constraint);
            }
        }
        Ok(())
    }

    /// Whether every row of the table holds a value in the column at
    /// `place`, which `sqlite3AlterSetNotNull` reads before it writes
    /// the constraint.
    ///
    /// Reading the rows costs O(n) in them.
    ///
    /// # Errors
    ///
    /// [`Error::Constraint`] where a row holds nothing there.
    fn holds_values(&self, name: &[u8], place: usize) -> Result<(), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        for (_, values) in database.held_rows_of(name)? {
            if values.get(place).is_none_or(|value| *value == Value::Null) {
                return Err(Error::Constraint);
            }
        }
        Ok(())
    }

    /// Whether the table of `name` holds a row, which
    /// `sqlite3ErrorIfNotEmpty` of `research/sqlite/src/alter.c:308`
    /// reads before it refuses an `ALTER TABLE`.
    ///
    /// Reading the table costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error`] names what reading the table refused.
    fn holds_rows(&self, name: &[u8]) -> Result<bool, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        Ok(!database.held_rows_of(name)?.is_empty())
    }

    /// `ALTER TABLE ... RENAME COLUMN`: every statement of the schema
    /// that names the column is written again under the new name, which
    /// is `sqlite3AlterRenameColumn`.
    ///
    /// The rows of the table are not written again, because a row holds
    /// no name.
    ///
    /// Reading the schema costs O(m) in its rows and O(n) per statement
    /// in the nodes of its tree.
    ///
    /// # Errors
    ///
    /// [`Error::NotAlterable`] for a table SQLite keeps for itself,
    /// [`Error::NoSuchColumn`] where the table holds no such column, and
    /// [`Error::AfterDrop`] where a statement of the schema no longer
    /// reads under the new name.
    fn rename_column(&mut self, asked: &crate::ast::RenameColumn, sql: &[u8]) -> Result<(), Error> {
        let name = crate::schema::dequote(asked.table.text(sql));
        let from = crate::schema::dequote(asked.column.text(sql));
        let to = crate::schema::dequote(asked.name.text(sql));
        // `sqlite3AlterRenameColumn` reads `bQuote` off the first byte
        // of the new name, so the name is written as the statement
        // wrote it.
        let as_written = asked.name.text(sql).to_vec();
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        alterable(&database, &name)?;
        // The table was located above, so this refusal carries no name
        // of its own.
        let (table, _) = database.table(&name).ok_or(Error::NoTable(Vec::new()))?;
        if !table
            .columns
            .iter()
            .any(|held| held.name.eq_ignore_ascii_case(&from))
        {
            return Err(Error::NoSuchColumn(from));
        }
        let mut written: Vec<(i64, Vec<Value>)> = Vec::new();
        for (rowid, values) in database.rows_of(SCHEMA_TABLE)? {
            // A row SQLite made for itself carries no statement, and an
            // index of a key is one of those.
            let held = |at: usize| match values.get(at) {
                Some(Value::Text(bytes)) => bytes.clone(),
                _ => Vec::new(),
            };
            let statement = held(4);
            let places = crate::rename::column_places(&statement, &name, &from);
            if places.is_empty() {
                continue;
            }
            let text = crate::rename::written_as(&statement, &places, &as_written);
            // `renameTestSchema` under `after rename`: a statement that
            // no longer reads refuses the whole rename.
            if let Some(refused) = reads_after(&text, &to, self.collating) {
                return Err(Error::AfterRename(held(0), held(1), refused));
            }
            let mut values = values.clone();
            for slot in values.iter_mut().skip(4).take(1) {
                *slot = Value::Text(text.clone());
            }
            written.push((rowid, values));
        }
        drop(database);
        for (rowid, values) in written {
            self.schema_row(rowid, &values)?;
        }
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// `ALTER TABLE ... DROP COLUMN`: the column goes out of the text
    /// that made the table and out of every row of it, which is
    /// `sqlite3AlterDropColumn`.
    ///
    /// Rewriting the rows costs O(n) in them and O(log n) per row, and
    /// reading the schema again costs O(m) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error::NoSuchColumn`] where the table holds no such column,
    /// [`Error::KeyColumn`] where the column carries a key of its own,
    /// [`Error::LastColumn`] where it is the one column the table has,
    /// and [`Error::AfterDrop`] where a statement of the schema no
    /// longer reads without it.
    fn drop_column(&mut self, asked: &crate::ast::DropColumn, sql: &[u8]) -> Result<(), Error> {
        let name = crate::schema::dequote(asked.table.text(sql));
        let column = crate::schema::dequote(asked.column.text(sql));
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        alterable(&database, &name)?;
        // The table was located above, so these refusals carry no name
        // of their own.
        let (table, root) = database.table(&name).ok_or(Error::NoTable(Vec::new()))?;
        let at = table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(&column))
            .ok_or_else(|| Error::NoSuchColumn(column.clone()))?;
        let (statement, _) = database
            .written_as(&name)
            .ok_or(Error::NoTable(Vec::new()))?;
        let text = without_column(statement, &column)?;
        let affinities: Vec<Affinity> = table
            .columns
            .iter()
            .enumerate()
            .filter(|(place, _)| *place != at)
            .map(|(_, held)| held.affinity)
            .collect();
        let rows = database.rows_of(&name)?;
        let schema_rowid = self.row_of(&name)?.ok_or(Error::NoTable(Vec::new()))?;
        drop(database);
        let value = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                value(b"table"),
                value(&name),
                value(&name),
                Value::Int(i64::from(root)),
                value(&text),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        let schema = crate::image::SCHEMA_ROOT;
        crate::tree::update(&mut self.held.pages, schema, schema_rowid, &row)?;
        // `sqlite3AlterDropColumn` writes every row again with the
        // value of that column left out.
        for (key, values) in rows {
            let held: Vec<Value> = values
                .into_iter()
                .enumerate()
                .filter(|(place, _)| *place != at)
                .map(|(_, value)| value)
                .collect();
            let record = crate::record::write_in(&held, &affinities, 4, self.held.header.encoding);
            crate::tree::update(&mut self.held.pages, root, key, &record)?;
        }
        self.reads_without(&name, &column)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// The row a `RETURNING` answers for one row the statement wrote,
    /// which is `sqlite3AddReturning` over the row as it stands.
    ///
    /// Reading one row costs O(c) in the columns the clause answers.
    ///
    /// # Errors
    ///
    /// [`Error::Eval`] names what the clause could not answer.
    fn returns(
        &mut self,
        arena: &Arena,
        returning: crate::ast::Range,
        sql: &[u8],
        row: (&Table, &[Value], Option<i64>),
    ) -> Result<(), Error> {
        if returning.is_empty() {
            return Ok(());
        }
        let (table, values, rowid) = row;
        let held = Held {
            table,
            values,
            rowid,
            encoding: self.held.header.encoding,
            random: &self.random,
            clock: self.clock.map(crate::date::julian_of),
            sensitive: self.truth.sensitive,
            counted: self.counted,
            defined: self.defined,
            grouped: self.grouped,
            collating: self.collating,
            outer: None,
            reading: None,
        };
        let mut answered = Vec::new();
        for column in arena.results(returning) {
            match column {
                crate::ast::ResultColumn::Star | crate::ast::ResultColumn::TableStar(_) => {
                    answered.extend(values.iter().cloned());
                }
                crate::ast::ResultColumn::Expr { expr, .. } => {
                    answered.push(crate::eval::evaluate_row(arena, *expr, sql, &held)?);
                }
            }
        }
        self.returned.push(answered);
        Ok(())
    }

    /// Whether every statement of the schema still reads now that the
    /// column is gone, which is `renameTestSchema` under the words
    /// `after drop column`.
    ///
    /// Reading the schema costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error::AfterDrop`] names the statement that no longer reads.
    fn reads_without(&self, name: &[u8], column: &[u8]) -> Result<(), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        for (_, values) in database.rows_of(SCHEMA_TABLE)? {
            let text = |at: usize| match values.get(at) {
                Some(Value::Text(bytes)) => bytes.clone(),
                _ => Vec::new(),
            };
            let (kind, held, over, statement) = (text(0), text(1), text(2), text(4));
            // A statement that names the table may name its columns:
            // the table's own, an index and a trigger over it, and a
            // view that reads it. One that was not written out names
            // nothing at all.
            if statement.is_empty()
                || !over.eq_ignore_ascii_case(name)
                    && crate::rename::places(&statement, name).is_empty()
            {
                continue;
            }
            if let Some(shown) = names_column(&statement, column) {
                let refused = alloc::format!(
                    "no such column: {}",
                    alloc::string::String::from_utf8_lossy(&shown)
                );
                return Err(Error::AfterDrop(kind, held, refused));
            }
        }
        Ok(())
    }

    /// The row of `sqlite_sequence` that counts for the table, written
    /// again under the new name, which is what `sqlite3AlterRenameTable`
    /// writes where the table counts up.
    ///
    /// Reading the rows costs O(n) in them.
    fn rename_sequence(&mut self, from: &[u8], to: &[u8]) -> Result<(), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        if database.table(SEQUENCE).is_none() {
            return Ok(());
        }
        let (_, root) = database.table(SEQUENCE).ok_or(Error::NoTable(Vec::new()))?;
        let held: Vec<(i64, Vec<Value>)> = database
            .rows_of(SEQUENCE)?
            .iter()
            .filter(|(_, values)| named_row(values, from))
            .cloned()
            .collect();
        drop(database);
        for (rowid, values) in held {
            let mut values = values;
            for slot in values.iter_mut().take(1) {
                *slot = Value::Text(to.to_vec());
            }
            let record = crate::record::write_in(
                &values,
                &[Affinity::None; 2],
                4,
                self.held.header.encoding,
            );
            crate::tree::update(&mut self.held.pages, root, rowid, &record)?;
        }
        Ok(())
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
        if matches!(
            asked.kind,
            crate::ast::Dropped::Table | crate::ast::Dropped::View
        ) && kept_name(&name)
        {
            return Err(Error::NotDroppable(name));
        }
        let (rowids, mut roots) = self.named(&name, asked.kind)?;
        if rowids.is_empty() {
            // `sqlite3DropTable` names the kind the schema holds where the
            // statement named the other, which is
            // `research/sqlite/src/build.c:3573`.
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            let other = match asked.kind {
                crate::ast::Dropped::Table => database.view(&name).is_some().then_some(true),
                crate::ast::Dropped::View => database.table(&name).is_some().then_some(false),
                _ => None,
            };
            if let Some(view) = other {
                return Err(Error::DropKind(view, name));
            }
            drop(database);
            return if asked.if_exists {
                Ok(())
            } else {
                // `sqlite3DropTable`, `sqlite3DropIndex` and
                // `sqlite3DropTrigger` each name what the statement
                // said it makes.
                Err(Error::NoObject(dropped_word(asked.kind), name.clone()))
            };
        }
        for rowid in rowids {
            crate::tree::remove(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid)?;
        }
        // `sqlite3CodeDropTable` takes the row of `sqlite_sequence`
        // that names the table away with the table.
        if asked.kind == crate::ast::Dropped::Table {
            self.uncount(&name)?;
        }
        roots.sort_unstable();
        for root in roots.into_iter().rev() {
            if !self.held.pages.vacuuming() {
                crate::tree::destroy(&mut self.held.pages, root)?;
                continue;
            }
            let largest = self.held.header.largest_root;
            let moved = crate::tree::destroy_moving(&mut self.held.pages, root, largest)?;
            if let Some(from) = moved {
                self.re_rooted(from, root)?;
            }
            self.held.header.largest_root = shrunk(&self.held.pages, largest);
        }
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        Ok(())
    }

    /// The row of `sqlite_schema` that names the root page `from` written
    /// again to name `into`, which is the `UPDATE` statement
    /// `destroyRootPage` of `research/sqlite/src/build.c` writes after the
    /// move.
    ///
    /// Finding the row costs O(n) in the rows of the schema.
    fn re_rooted(&mut self, from: u32, into: u32) -> Result<(), Error> {
        let bytes = self.image();
        let image = crate::image::Image::open(&bytes)?;
        let mut found = None;
        for row in image.schema() {
            let row = row?;
            let record = row.record()?;
            let mut values = Vec::with_capacity(SCHEMA.len());
            for at in 0..SCHEMA.len() {
                values.push(
                    record
                        .value(at)?
                        .map_or(Value::Null, crate::tree::held_value),
                );
            }
            if values.get(3) == Some(&Value::Int(i64::from(from))) {
                for slot in values.iter_mut().skip(3).take(1) {
                    *slot = Value::Int(i64::from(into));
                }
                found = Some((row.rowid, values));
                break;
            }
        }
        // A root the drop moved is one a row of the schema names, so the
        // walk finds that row.
        found.map_or(Ok(()), |(rowid, values)| self.schema_row(rowid, &values))
    }

    /// `PRAGMA integrity_check` over every database the connection holds,
    /// in the order `PRAGMA database_list` answers them.
    ///
    /// Each database is read on its own, because a tree of another file
    /// would be read as a second reference to the pages of this one, and
    /// the count of problems the pragma answers is spent across all of
    /// them. The walk costs what one walk costs per database.
    ///
    /// # Errors
    ///
    /// [`Error`] names what reading one of the files refuses.
    fn integrity(
        &self,
        quick: bool,
        asked: &crate::check::Checking,
    ) -> Result<Vec<Vec<Value>>, Error> {
        let mut held: Vec<(usize, Vec<u8>, Vec<u8>)> =
            alloc::vec![(self.called.place, self.called.name.clone(), self.image())];
        for beside in self.in_place() {
            held.push((
                beside.called.place,
                beside.called.name.clone(),
                written_image(&beside.held),
            ));
        }
        held.sort_by_key(|(place, _, _)| *place);
        let mut left = crate::check::allowed(asked);
        let mut found = Vec::new();
        let mut named = false;
        for (_, name, bytes) in &held {
            let database = self.reading(bytes)?;
            // `sqlite3LocateTable` of `research/sqlite/src/build.c` finds
            // the table a name carries in the first database that holds
            // it, and `tableSkipIntegrityCheck` then walks that table
            // alone, so no other database is read.
            if let crate::check::Checking::Table(wanted) = asked {
                if named || !crate::check::holds_object(&database, wanted) {
                    continue;
                }
                named = true;
            }
            let more = crate::check::integrity(&database, quick, (asked, name), &mut left)?;
            found.extend(more);
        }
        if let crate::check::Checking::Table(wanted) = asked
            && !named
        {
            return Err(Error::NoTable(wanted.clone()));
        }
        if found.is_empty() {
            found.push(b"ok".to_vec());
        }
        Ok(found
            .into_iter()
            .map(|text| alloc::vec![Value::Text(text)])
            .collect())
    }

    /// The rows of `sqlite_schema` a `DROP` takes out and the roots it
    /// destroys: a table takes its indexes with it, an index takes only
    /// itself.
    fn named(&self, name: &[u8], kind: crate::ast::Dropped) -> Result<(Vec<i64>, Vec<u32>), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let mut roots: Vec<u32> = Vec::new();
        let mut held = false;
        match kind {
            crate::ast::Dropped::Table => {
                if let Some((_, root)) = database.table(name) {
                    roots.push(root);
                    held = true;
                }
                roots.extend(database.indexes(name).iter().map(|kept| kept.root));
            }
            crate::ast::Dropped::Index => {
                if let Some((_, root)) = database.index(name) {
                    // `sqlite3DropIndex` refuses an index a `UNIQUE` or
                    // a `PRIMARY KEY` made, which stands and falls with
                    // the constraint that made it.
                    if database.constrained(name) {
                        return Err(Error::ConstraintIndex);
                    }
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
        let encoding = image.header().encoding;
        let over = matches!(kind, crate::ast::Dropped::Table | crate::ast::Dropped::View);
        let mut triggers = Vec::new();
        let mut rowids = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            let trigger = is_text(record.value(0)?, b"trigger", encoding);
            let named = if over {
                is_text(record.value(2)?, name, encoding)
            } else {
                is_text(record.value(1)?, name, encoding)
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
        if self.held.log.is_none() {
            self.logging((0, 0));
        }
    }

    /// `PRAGMA name`, which answers the one row the connection holds for
    /// that pragma.
    ///
    /// A connection answers a pragma out of what it holds and not out of
    /// the file, because a file with no table holds no encoding:
    /// `sqlite3Pragma` reads the schema in memory. Answering one costs
    /// O(1).
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a pragma no header of a file holds.
    fn pragma_read(&self, setting: crate::pragma::Setting) -> Result<Vec<Vec<Value>>, Error> {
        // `sqlite3Pragma` answers no row for a name it does not know, and
        // the names this crate accepts and holds nothing for are the ones
        // no version of the library holds either.
        if matches!(
            setting,
            crate::pragma::Setting::Ignored | crate::pragma::Setting::CaseSensitiveLike
        ) {
            return Ok(Vec::new());
        }
        if setting == crate::pragma::Setting::CountChanges {
            return Ok(alloc::vec![alloc::vec![Value::Int(i64::from(
                self.truth.counting
            ))]]);
        }
        if let crate::pragma::Setting::Held(at) = setting {
            return Ok(alloc::vec![alloc::vec![self.held(at)]]);
        }
        // The journal mode belongs to the connection, which holds it
        // whatever the header of the file says.
        if setting == crate::pragma::Setting::JournalMode {
            return Ok(alloc::vec![alloc::vec![Value::Text(
                self.journalled().to_vec()
            )]]);
        }
        let read = setting.read(&self.now()).ok_or(Error::Unsupported)?;
        Ok(alloc::vec![alloc::vec![read]])
    }

    /// The pages of the file written back and held on the disk, which
    /// is what `walCheckpoint` of `research/sqlite/src/wal.c` writes for
    /// every page a frame holds, and the image those writes leave.
    ///
    /// `pager_write_changecounter`: the file the checkpoint writes
    /// carries the counter the frames carried, which is one past the one
    /// the file holds. It costs O(n) in the pages of the file.
    fn writes_back(&mut self) -> Vec<u8> {
        let mut now = self.now();
        now.change_counter = now.change_counter.saturating_add(1);
        now.version_valid_for = now.change_counter;
        let image = self.held.pages.written(&now);
        let page_size = crate::bytes::size(u64::from(self.held.header.page_size));
        for (at, page) in image.chunks(page_size.max(1)).enumerate() {
            self.held.did.push(Does::Write {
                onto: Onto::Main,
                at: u64::try_from(at.saturating_mul(page_size)).unwrap_or(u64::MAX),
                bytes: page.to_vec(),
            });
        }
        self.held.did.push(Does::Sync(Onto::Main));
        self.held.header.change_counter = now.change_counter;
        self.held.header.version_valid_for = now.version_valid_for;
        image
    }

    /// The log written into the file and given up, which
    /// `sqlite3WalClose` of `research/sqlite/src/wal.c` does where the
    /// connection that closes is the last one over the file: the pages
    /// the frames hold reach the file, the file is held on the disk, and
    /// the log is removed. A connection that opens the file afterwards
    /// reads every page out of the file.
    ///
    /// It costs O(n) in the pages of the file.
    pub fn closing(&mut self) {
        if self.held.log.take().is_none() {
            return;
        }
        let _ = self.writes_back();
        self.held.did.push(Does::Remove(Onto::Log));
        self.held.origin = None;
        self.held.restarting = false;
    }

    /// `PRAGMA wal_checkpoint`: the pages the log holds are written into
    /// the database file, and the three columns
    /// `sqlite3_wal_checkpoint_v2` answers say so.
    ///
    /// The first column is nought, because one connection writes here
    /// and no other holds the log back. The second and the third are how
    /// many frames the log holds, and both are nought for `TRUNCATE`,
    /// which leaves a log of no frame. `RESTART` and `TRUNCATE` begin
    /// the log again at once; every other mode leaves the frames where
    /// they are and the commit after the checkpoint begins the log
    /// again, which is `walRestartLog`. Writing the file costs O(n) in
    /// its pages.
    fn checkpoint(&mut self, how: Option<&[u8]>) -> Vec<Vec<Value>> {
        let named = |word: &[u8]| how.is_some_and(|text| text.eq_ignore_ascii_case(word));
        let Some(mut log) = self.held.log.take() else {
            // A file that is not logging holds no frame, which the C
            // library says with minus one rather than nought.
            return alloc::vec![alloc::vec![Value::Int(0), Value::Int(-1), Value::Int(-1)]];
        };
        let frames = i64::try_from(log.frames()).unwrap_or(i64::MAX);
        let truncating = named(b"truncate");
        let restarting = truncating || named(b"restart");
        if restarting {
            log.restart();
        }
        self.held.log = Some(log);
        self.held.restarting = !restarting;
        let image = self.writes_back();
        if restarting {
            let bytes = self.held.log.as_ref().map(Log::bytes).unwrap_or_default();
            self.held.did.push(Does::Write {
                onto: Onto::Log,
                at: 0,
                bytes: bytes.to_vec(),
            });
            self.held.did.push(Does::Sync(Onto::Log));
        }
        self.held.origin = Some(image);
        let counted = if truncating { 0 } else { frames };
        alloc::vec![alloc::vec![
            Value::Int(0),
            Value::Int(counted),
            Value::Int(counted)
        ]]
    }

    /// `VACUUM`: the database is made again from nothing, with every
    /// table and index of the schema made in the order the schema holds
    /// them and every row written under the key it had, which is
    /// `sqlite3RunVacuum` running `INSERT INTO vacuum_db.x SELECT * FROM
    /// main.x` per table.
    ///
    /// The file the statement leaves holds the rows in the order of
    /// their keys and no free page, which is what a vacuum is for. It
    /// costs O(n) in the rows of the database and O(k log n) in the
    /// entries of its indexes.
    ///
    /// # Errors
    ///
    /// [`Error::VacuumInTransaction`] where the connection has a
    /// transaction open, [`Error::NoSchema`] for a schema this
    /// connection does not hold, and [`Error::Unsupported`] for
    /// `VACUUM INTO`, which writes a file this crate hands no caller.
    fn vacuum(&mut self, asked: &crate::ast::Vacuum, sql: &[u8]) -> Result<(), Error> {
        if let Some(schema) = asked.schema {
            let named = crate::schema::dequote(schema.text(sql));
            if !named.eq_ignore_ascii_case(b"main") && !named.eq_ignore_ascii_case(b"temp") {
                return Err(Error::NoSchema(named));
            }
        }
        if self.held.began.is_some() {
            return Err(Error::VacuumInTransaction);
        }
        if let Some(text) = asked.text {
            let named = self.named_file(text.text(sql))?;
            let fresh = self.vacuumed()?;
            self.made.push((named, fresh.written()));
            return Ok(());
        }
        let held = self.held.header;
        let fresh = self.vacuumed()?;
        self.held.pages = fresh.held.pages;
        self.held.header.page_size = fresh.held.header.page_size;
        self.held.header.pages = fresh.held.header.pages;
        self.held.header.freelist = fresh.held.header.freelist;
        self.held.header.freelist_pages = fresh.held.header.freelist_pages;
        self.held.header.largest_root = fresh.held.header.largest_root;
        self.held.header.schema_cookie = held.schema_cookie.saturating_add(1);
        self.held.header.change_counter = held.change_counter.saturating_add(1);
        self.held.header.version_valid_for = self.held.header.change_counter;
        Ok(())
    }

    /// The database written again into a file of its own, which is what
    /// a `VACUUM` keeps and what a `VACUUM ... INTO` writes out.
    ///
    /// Writing the rows again costs O(n) in the rows of the database and
    /// O(k log n) in the entries of its indexes.
    ///
    /// # Errors
    ///
    /// Whatever reading the schema or writing one of the rows refuses.
    fn vacuumed(&self) -> Result<Writer, Error> {
        let held = self.held.header;
        let size = self.held.wanted_page.unwrap_or(held.page_size);
        let mut fresh = Writer::new(size, held.reserved, held.encoding)?;
        fresh.defines(self.defined);
        fresh.groups(self.grouped);
        fresh.collates(self.collating);
        fresh.kept.clone_from(&self.kept);
        fresh.truth = self.truth;
        // `sqlite3RunVacuum` writes the new file as the old one was: the
        // pages it keeps for the free list it points at, and the numbers
        // the header carries for the application.
        if held.largest_root != 0 {
            fresh.vacuuming(held.incremental_vacuum != 0);
        }
        fresh.making_own = true;
        let bytes = self.image();
        let schema = self.schema_of(&bytes)?;
        let database = self.reading(&bytes)?;
        for made in &schema {
            // A table the engine writes for itself is there already
            // where a table of the schema made it, which
            // `sqlite_sequence` is for a key that counts up.
            let makes = made.kind == b"table" || made.kind == b"index";
            if makes && !fresh.holds(&made.name)? {
                fresh.run(&made.statement)?;
            }
        }
        for made in &schema {
            if made.kind != b"table" {
                continue;
            }
            let rows = database.held_rows_of(&made.name)?;
            fresh.put_rows(&made.name, &rows)?;
        }
        for made in &schema {
            if made.kind == b"view" || made.kind == b"trigger" {
                fresh.run(&made.statement)?;
            }
        }
        fresh.making_own = false;
        Ok(fresh)
    }

    /// The file a `VACUUM ... INTO` writes, which is what the expression
    /// after `INTO` answers as text.
    ///
    /// Answering the expression costs what one statement over the
    /// database costs.
    ///
    /// # Errors
    ///
    /// [`Error::NonTextFilename`] where the expression answers other
    /// than text, [`Error::OutputExists`] where the client already holds
    /// bytes under that name, and whatever answering the expression
    /// refuses.
    fn named_file(&self, text: &[u8]) -> Result<Vec<u8>, Error> {
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?;
        let mut sql = b"SELECT ".to_vec();
        sql.extend_from_slice(text);
        let answered = database.query(&sql)?;
        let value = answered
            .rows
            .first()
            .and_then(|row| row.first())
            .cloned()
            .unwrap_or(Value::Null);
        let Value::Text(named) = value else {
            return Err(Error::NonTextFilename);
        };
        // `sqlite3RunVacuum` opens the file and refuses one that holds a
        // page already, which the function the caller told the
        // connection answers the bytes of.
        let held = named.eq_ignore_ascii_case(b":memory:")
            || self
                .opening
                .and_then(|opening| opening(&named))
                .is_none_or(|bytes| bytes.is_empty());
        if held {
            return Ok(named);
        }
        Err(Error::OutputExists)
    }

    /// The rows of the schema, in the order the schema tree holds them:
    /// what each row makes, the name it makes it under, and the
    /// statement that made it.
    ///
    /// A row whose statement is empty is an index a `PRIMARY KEY` or a
    /// `UNIQUE` made, which the table's own statement makes again.
    /// Reading them costs O(n) in the rows of the schema.
    ///
    /// # Errors
    ///
    /// Whatever reading the schema tree refuses.
    fn schema_of(&self, bytes: &[u8]) -> Result<Vec<Made>, Error> {
        let image = crate::image::Image::open(bytes)?;
        let mut out = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            payload.clear();
            payload.resize(row.payload.total, 0);
            image.read_payload(&row.payload, &mut payload)?;
            let record = crate::record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(crate::record::Value::Text(bytes)) => {
                        crate::value::decoded(bytes, self.held.header.encoding)
                    }
                    _ => Vec::new(),
                })
            };
            let statement = text(4)?;
            if statement.is_empty() {
                continue;
            }
            out.push(Made {
                kind: text(0)?,
                name: text(1)?,
                statement,
            });
        }
        Ok(out)
    }

    /// The rows of a table written into the tree of the table and into
    /// every index over it, each under the key it carries, with no
    /// constraint read and no trigger run.
    ///
    /// `sqlite3RunVacuum` writes the rows of a table into the file it
    /// makes this way. Writing n rows costs O(n log n) in the rows and
    /// O(k n log n) in the entries of k indexes.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] where the file holds no such table, and
    /// whatever writing a row refuses.
    fn put_rows(&mut self, name: &[u8], rows: &[crate::db::Reading]) -> Result<(), Error> {
        let bytes = self.image();
        let (table, root, kept) = {
            let database = self.reading(&bytes)?;
            let (table, root) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
            (table.clone(), root, kept_indexes(&database, name))
        };
        let affinities = ordered_affinities(&table);
        for (key, values) in rows {
            self.index_row(&kept, &table, values, key)?;
            if table.without_rowid {
                let record = crate::record::write_in(
                    &ordered(&table, values),
                    &affinities,
                    4,
                    self.held.header.encoding,
                );
                let collations = crate::schema::key_collations(&table);
                let order = ordering(&collations, self.held.header.encoding);
                crate::tree::insert_entry(&mut self.held.pages, root, &record, key, order, false)?;
                continue;
            }
            let mut held = values.clone();
            for slot in held
                .iter_mut()
                .skip(table.rowid_alias.unwrap_or(usize::MAX))
                .take(1)
            {
                *slot = Value::Null;
            }
            let record = crate::record::write_in(
                &ordered(&table, &held),
                &affinities,
                4,
                self.held.header.encoding,
            );
            insert(&mut self.held.pages, root, keyed_rowid(key), &record)?;
        }
        Ok(())
    }

    /// The database `bytes` hold, told what this connection was told:
    /// the collations, the functions and the aggregates the application
    /// defined on it, and the moment its clock says.
    ///
    /// Opening one reads the schema, so it costs O(n) in the rows of
    /// `sqlite_schema`.
    ///
    /// # Errors
    ///
    /// Whatever reading the header or the schema refuses.
    /// The `SET` clauses of an `UPDATE` the statement writes, which are
    /// the ones the function the connection was told did not ignore.
    fn writing(
        &self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
    ) -> Vec<crate::ast::Set> {
        arena
            .sets(statement.sets)
            .iter()
            .filter(|set| {
                let column = crate::schema::dequote(set.column.text(sql));
                !self
                    .unwritten
                    .iter()
                    .any(|held| held.eq_ignore_ascii_case(&column))
            })
            .copied()
            .collect()
    }

    /// What the function the connection was told answers for one
    /// statement, read against the schema the statement runs over.
    ///
    /// A connection told no function opens nothing and answers `Ok`, so
    /// a statement costs what it did. A connection told one pays O(n) in
    /// the pages of the file for the schema the reading needs.
    ///
    /// # Errors
    ///
    /// [`Error::Auth`] names what the function refused.
    fn asked(
        &self,
        read: impl FnOnce(
            &mut crate::auth::Authorizer<'_>,
        ) -> Result<crate::auth::Answer, crate::auth::Error>,
    ) -> Result<crate::auth::Read, Error> {
        let Some(asking) = self.asking else {
            return Ok(crate::auth::Read {
                answer: crate::auth::Answer::Ok,
                unwritten: Vec::new(),
            });
        };
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?;
        let mut authorizer = crate::auth::Authorizer::new(asking, &database);
        let answer = read(&mut authorizer)?;
        Ok(authorizer.taken(answer))
    }

    fn reading_beside<'a>(&self, images: &'a Images) -> Result<Database<'a>, Error> {
        let mut database = self.reading(&images.held)?;
        for (name, bytes) in &images.beside {
            database = database.attaching(name, bytes)?;
        }
        Ok(database)
    }

    /// A reader over the one file the statement writes, which is what a
    /// constraint reads the schema out of.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the image breaks.
    fn reading<'a>(&self, bytes: &'a [u8]) -> Result<Database<'a>, Error> {
        let database = Database::open_collating(bytes, self.collating)?
            .writing(true)
            .defining(self.defined)
            .grouping(self.grouped)
            .sensitively(self.truth.sensitive)
            .named_main(&self.called.name);
        Ok(match self.clock {
            Some(seconds) => database.clocked(seconds),
            None => database,
        })
    }

    /// The files the connection holds: the one the statement writes, and
    /// each of the others with the name a statement names it by.
    ///
    /// Building them costs O(n) in the pages of every database.
    fn images(&self) -> Images {
        Images {
            held: self.image(),
            beside: self
                .in_place()
                .into_iter()
                .map(|held| (held.called.name.clone(), written_image(&held.held)))
                .collect(),
        }
    }

    /// The file the pages hold, which is what a statement reads its
    /// rows out of, whatever a log beside the file holds.
    fn image(&self) -> Vec<u8> {
        self.held.pages.written(&self.held.header)
    }

    /// The header as the pages stand, which is what a pragma answers:
    /// the count of pages and the free list are the pages' own and not
    /// the header's until a commit writes them.
    fn now(&self) -> Header {
        let mut now = self.held.header;
        let (first, count) = self.held.pages.freelist();
        now.pages = self.held.pages.count();
        now.freelist = first;
        now.freelist_pages = count;
        now
    }

    /// What the commits since this was last read did to the files, in
    /// the order they did it, which a caller that simulates a machine
    /// losing power applies a prefix of.
    ///
    /// Reading them clears the list, so each call answers what the
    /// commits since the last call did.
    pub fn did(&mut self) -> Vec<Does> {
        core::mem::take(&mut self.held.did)
    }

    /// What the commit of the last statement left beside the file: the
    /// rollback journal the mode keeps, and nothing where the mode
    /// keeps none.
    #[must_use]
    pub fn journal(&self) -> Option<&[u8]> {
        self.held.journal.as_deref()
    }

    /// The log the commits wrote their frames into, where the file is
    /// in write-ahead logging mode.
    #[must_use]
    pub fn log(&self) -> Option<&[u8]> {
        self.held.log.as_ref().map(Log::bytes)
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
        self.ran_statement(sql).map_err(|error| error.near(sql))
    }

    /// One statement run, with a parse answered as the parser wrote it.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the statement could not do.
    fn ran_statement(&mut self, sql: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        // A statement that names a database of its own writes that one,
        // so the connection takes it as the one it writes for the length
        // of the statement and puts the one it wrote back after it.
        let at = self.writing_at(sql)?;
        if let Some(held) = at {
            self.switch(held);
        }
        let answered = self.ran_held(sql);
        if let Some(held) = at {
            self.switch(held);
        }
        answered
    }

    /// Which database of the list the statement writes, and nothing where
    /// it writes the one the connection already writes.
    ///
    /// The statement is read for its schema alone, which costs O(n) in
    /// its bytes; a connection that holds one database reads nothing.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] for a statement that names a table of a
    /// database the connection does not hold, and [`Error::NoSchema`] for
    /// one that names the database alone, which is what
    /// `sqlite3TwoPartName` of `research/sqlite/src/build.c:596`
    /// answers.
    fn writing_at(&mut self, sql: &[u8]) -> Result<Option<usize>, Error> {
        let temping = holds_temp(sql);
        if self.attached.is_empty() && !temping {
            return Ok(None);
        }
        // A pragma answers out of the database its schema names, which
        // `sqlite3Pragma` of `research/sqlite/src/pragma.c:381` reads
        // with `sqlite3TwoPartName`.
        let names = match crate::parse::pragma(sql) {
            Ok(asked) => Names::made(asked.schema, None, false),
            Err(_) => match crate::parse::definition(sql) {
                Ok((_, definition)) => defined_under(definition),
                Err(_) => match crate::parse::change(sql) {
                    Ok((_, change)) => changed_under(change),
                    // A statement neither reading takes is one that
                    // writes no database of its own, which a `SELECT` is.
                    Err(_) => Names::made(None, None, false),
                },
            },
        };
        // A statement that names the temp schema opens it, which is what
        // `sqlite3OpenTempDatabase` of
        // `research/sqlite/src/build.c:2830` does where a statement reads
        // or writes one. An `ATTACH` and a `DETACH` name a database
        // rather than reading one, so neither opens it.
        if temping && names.opens {
            self.temping()?;
        }
        // A statement written `TEMP` writes the temp schema, which
        // `sqlite3TwoPartName` of `research/sqlite/src/build.c:596` reads
        // as one that names schema place one.
        if names.temporary {
            return self.temping().map(Some);
        }
        let held = self.switched(names.schema, sql).map_err(|held| {
            if names.locates == Located::Nothing {
                return Error::NoSchema(held);
            }
            let mut shown = held;
            shown.push(b'.');
            shown.extend_from_slice(&named_text(names.located, sql));
            Error::NoTable(shown)
        })?;
        if names.schema.is_some() {
            if names.locates == Located::Table {
                self.holds_named(held, &names, sql)?;
            }
            return Ok(held);
        }
        self.holding_at(names.located, sql)
    }

    /// Raises where the database a statement named holds no table of the
    /// name it wrote.
    ///
    /// `sqlite3TwoPartName` of `research/sqlite/src/build.c:596` reads a
    /// name written under a schema in that schema alone, so a table
    /// another database holds is no table of this statement.
    ///
    /// Reading the schema costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] names the schema and the table.
    fn holds_named(&self, at: Option<usize>, names: &Names, sql: &[u8]) -> Result<(), Error> {
        let name = named_text(names.located, sql);
        let bytes = match at {
            None => self.image(),
            Some(at) => {
                let mut out = Vec::new();
                for held in self.attached.iter().skip(at).take(1) {
                    out = written_image(&held.held);
                }
                out
            }
        };
        if Database::open(&bytes)?.holding(&name).is_some() {
            return Ok(());
        }
        let mut shown = named_text(names.schema, sql);
        shown.push(b'.');
        shown.extend_from_slice(&name);
        Err(Error::NoTable(shown))
    }

    /// The place in the list of the temp schema, which the connection
    /// makes where it holds none.
    ///
    /// The temp schema is a database of one page under the page size and
    /// the encoding of `main`, which no file of the client holds.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the page size of `main` breaks.
    fn temping(&mut self) -> Result<usize, Error> {
        let found = self
            .attached
            .iter()
            .position(|held| named_as(held, b"temp"));
        if let Some(at) = found {
            return Ok(at);
        }
        let held = joined(self.opened_file(b":memory:")?, self.held.began.is_some());
        self.attached.push(Attached {
            called: Called {
                name: b"temp".to_vec(),
                file: Vec::new(),
                place: 1,
            },
            held,
        });
        Ok(self.attached.len().saturating_sub(1))
    }

    /// Which database of the list holds the name a statement wrote with
    /// no schema in front of it, and nothing where the one the connection
    /// writes holds it or no database of it does.
    ///
    /// `sqlite3LocateTable` of `research/sqlite/src/build.c:408` reads
    /// the databases in turn, so a statement that names a table only an
    /// attached database holds writes that one. Reading the schemas costs
    /// O(n) in the pages of every database.
    ///
    /// # Errors
    ///
    /// [`Error`] names what one of the images breaks.
    fn holding_at(&self, named: Option<Span>, sql: &[u8]) -> Result<Option<usize>, Error> {
        let Some(name) = named else {
            return Ok(None);
        };
        let bytes = self.images();
        // `sqlite3FindTable` reads a name the statement wrote under no
        // schema in the temp schema first, so the database this answers
        // is the one that database holds the name in.
        let database = self.reading_beside(&bytes)?.writing(false);
        let held = crate::schema::dequote(name.text(sql));
        let Some(named) = database.holding(&held) else {
            return Ok(None);
        };
        Ok(self.attached.iter().position(|one| named_as(one, &named)))
    }

    /// One statement run against the database the connection writes.
    fn ran_held(&mut self, sql: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
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
        if let Ok(asked) = crate::parse::savepoint(sql) {
            return self.savepoint(asked, sql);
        }
        if let Ok(asked) = crate::parse::pragma(sql) {
            return self.pragma(&asked, sql);
        }
        // A statement inside a transaction writes on what the
        // statements before it wrote, so the pages keep what they held
        // when the `BEGIN` ran and not when this statement began.
        let was = self.held.began.unwrap_or(self.held.header);
        // A statement of a transaction is one unit of its own as well:
        // the pages it opens are kept as it found them, which is the
        // statement journal.
        if self.held.began.is_none() {
            self.held.pages.begin();
        } else {
            self.held.pages.mark();
        }
        let held = self.held.header;
        // The keys held at the end of the transaction are counted back
        // to what they were where the statement began, wherever the
        // statement leaves the file as it found it, which
        // `sqlite3VdbeCloseStatement` of
        // `research/sqlite/src/vdbeaux.c:3140` writes back from
        // `nStmtDefCons`.
        let counted = self.deferred;
        // `PRAGMA max_page_count` holds the file to a count of pages
        // from the statement that sets it onward, so the pages are told
        // it where each statement begins.
        self.held.pages.capped(capped(self.told(b"max_page_count")));
        self.refusing = Refusing::Abort;
        self.writing = 0;
        self.returned.clear();
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
                // A statement that leaves the file as it found it
                // wrote no row that stands, and one that stops where it
                // stands keeps what it wrote: `sqlite3_changes` counts
                // the rows that stand either way.
                let stands = if matches!(self.refusing, Refusing::Fail) {
                    self.writing
                } else {
                    0
                };
                self.counts_step(stands);
                // `OE_Rollback` undoes the whole transaction and ends
                // it, which is `sqlite3RollbackAll` where the statement
                // runs inside one and the same pages either way where
                // it runs on its own.
                if matches!(self.refusing, Refusing::Rollback) {
                    let was = self.held.began.take().unwrap_or(held);
                    self.saved.clear();
                    self.deferred = 0;
                    self.held.pages.rollback();
                    self.held.header = was;
                    self.tells_rollback();
                    return Err(error);
                }
                if matches!(self.refusing, Refusing::Abort) {
                    // A statement of its own that leaves the file as it
                    // found it sends its own transaction back, which
                    // `sqlite3RollbackAll` tells the rollback hook of
                    // where the statement had written a page.
                    let wrote = self.held.began.is_none()
                        && (self.held.pages.changed() || ran_refusal(&error));
                    if self.held.began.is_none() {
                        self.held.pages.rollback();
                    } else {
                        self.held.pages.undo();
                    }
                    self.held.header = held;
                    self.deferred = counted;
                    if wrote {
                        self.tells_rollback();
                    }
                }
                return Err(error);
            }
        };
        // A statement inside a transaction is written by the `COMMIT`
        // and not by itself, which is what makes the transaction one
        // unit of work.
        if self.held.began.is_none() {
            // A statement of its own commits at its end, so a foreign
            // key held at the end of the transaction is held there.
            if self.deferred != 0 {
                self.deferred = 0;
                self.held.pages.rollback();
                self.held.header = held;
                self.counts_step(0);
                return Err(Error::Foreign);
            }
            // `sqlite3VdbeHalt`: the commit hook is asked where the
            // statement wrote a page, and a true answer sends the
            // transaction back.
            if self.refuses_commit() {
                self.held.pages.rollback();
                self.held.header = held;
                self.counts_step(0);
                self.tells_rollback();
                return Err(Error::CommitHook);
            }
            self.commit(&was)?;
        }
        // A statement that writes a `RETURNING` answers one row per row
        // it wrote, which is `sqlite3AddReturning`.
        if !self.returned.is_empty() {
            return Ok(core::mem::take(&mut self.returned));
        }
        // `PRAGMA count_changes`: a statement that changes rows answers
        // how many it changed, which is one row of one column.
        if self.truth.counting {
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
        let asking = named.clone().unwrap_or_default();
        if self
            .asked(|authorizer| authorizer.analyze(&asking, b"main"))?
            .answer
            == crate::auth::Answer::Ignore
        {
            return Ok(());
        }
        let (Analyzed { tables, only }, held) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
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
            self.own_table(b"CREATE TABLE sqlite_stat1(tbl,idx,stat)")?;
        }
        let (root, stats) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
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
        let wanted = scope.map(|name| crate::value::stored(name, self.held.header.encoding));
        let (root, held) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
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
            crate::tree::remove(&mut self.held.pages, root, rowid)?;
        }
        Ok(())
    }

    /// One row of `sqlite_stat1`, written into the tree at `root`.
    ///
    /// Writing one row costs O(log n) in the rows of the table.
    fn stat_row(&mut self, root: u32, stat: &crate::analyze::Stat) -> Result<(), Error> {
        let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let values = [
            text(&stat.table),
            stat.index.as_deref().map_or(Value::Null, text),
            text(&stat.stat),
        ];
        let record =
            crate::record::write_in(&values, &[Affinity::None; 3], 4, self.held.header.encoding);
        let rowid = largest(&self.held.pages, root)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.held.pages, root, rowid, &record)?;
        Ok(())
    }

    /// One statement that makes something or changes rows, and how
    /// many rows it changed.
    fn ran(&mut self, sql: &[u8]) -> Result<i64, Error> {
        // The readings are tried in turn, and the one that took in most
        // of the statement says where the parse stopped.
        let mut held = None;
        match crate::parse::definition(sql) {
            Ok((arena, definition)) => {
                crate::schema::collations(&arena, sql, self.collating)?;
                crate::schema::likelihoods(&arena, sql)?;
                let read = self.asked(|authorizer| {
                    authorizer.functions(&arena, sql)?;
                    authorizer.definition(&arena, definition, sql)
                })?;
                if read.answer == crate::auth::Answer::Ignore {
                    return Ok(0);
                }
                self.define(&arena, definition, sql)?;
                return Ok(0);
            }
            Err(error) => held = Some(crate::parse::furthest(held, error)),
        }
        match crate::parse::analyze(sql) {
            Ok(asked) => {
                self.analyze(&asked, sql)?;
                return Ok(0);
            }
            Err(error) => held = Some(crate::parse::furthest(held, error)),
        }
        match crate::parse::reindex(sql) {
            Ok(asked) => {
                self.reindex(&asked, sql)?;
                return Ok(0);
            }
            Err(error) => held = Some(crate::parse::furthest(held, error)),
        }
        let (arena, change) = match crate::parse::change(sql) {
            Ok(read) => read,
            Err(error) => return Err(Error::Parse(crate::parse::furthest(held, error))),
        };
        crate::eval::rows_placed(&arena)?;
        crate::schema::collations(&arena, sql, self.collating)?;
        crate::schema::likelihoods(&arena, sql)?;
        let read = self.asked(|authorizer| {
            authorizer.functions(&arena, sql)?;
            authorizer.change(&arena, change, sql)
        })?;
        let changed = match change {
            Change::Insert(statement) => match read.answer {
                crate::auth::Answer::Ignore => Ok(0),
                _ => self.insert(&arena, &statement, sql, None),
            },
            Change::Delete(statement) => self.delete(&arena, &statement, sql, None),
            Change::Update(statement) => {
                self.unwritten.clone_from(&read.unwritten);
                let changed = self.update(&arena, &statement, sql, None);
                self.unwritten.clear();
                changed
            }
        }?;
        // `sqlite3_changes` counts the rows of the last statement that
        // changed rows, and `sqlite3_total_changes` the rows of every
        // statement of the connection.
        self.counts_step(changed);
        Ok(changed)
    }

    /// `BEGIN`, `COMMIT` and `ROLLBACK`.
    ///
    /// # Errors
    ///
    /// [`Error::Nested`] for a `BEGIN` inside a transaction, and
    /// [`Error::NoTransaction`] for a `COMMIT` or a `ROLLBACK` outside
    /// one.
    fn bound(&mut self, asked: crate::ast::Transaction) -> Result<Vec<Vec<Value>>, Error> {
        let word: &[u8] = match asked {
            crate::ast::Transaction::Begin => b"BEGIN",
            crate::ast::Transaction::Commit => b"COMMIT",
            crate::ast::Transaction::Rollback => b"ROLLBACK",
        };
        if self
            .asked(|authorizer| authorizer.transaction(word))?
            .answer
            == crate::auth::Answer::Ignore
        {
            return Ok(Vec::new());
        }
        match asked {
            crate::ast::Transaction::Begin => {
                if self.held.began.is_some() {
                    return Err(Error::Nested);
                }
                self.opening();
            }
            crate::ast::Transaction::Commit => {
                if self.held.began.is_none() {
                    return Err(Error::NoTransaction(false));
                }
                // `sqlite3VdbeCheckFk`: a transaction that leaves a
                // foreign key held at its end pointing at no row is not
                // written, and it stays open.
                if self.deferred != 0 {
                    self.refusing = Refusing::Fail;
                    return Err(Error::Foreign);
                }
                if self.refuses_commit() {
                    self.saved.clear();
                    self.deferred = 0;
                    self.rolled_back();
                    self.tells_rollback();
                    return Err(Error::CommitHook);
                }
                self.saved.clear();
                self.committed()?;
            }
            crate::ast::Transaction::Rollback => {
                if self.held.began.is_none() {
                    return Err(Error::NoTransaction(true));
                }
                self.saved.clear();
                self.deferred = 0;
                self.rolled_back();
                self.tells_rollback();
            }
        }
        Ok(Vec::new())
    }

    /// `SAVEPOINT`, `RELEASE` and `ROLLBACK TO`, which is
    /// `sqlite3Savepoint`.
    ///
    /// Opening one costs O(n) in the pages of the file, which is what
    /// the file it stands over is kept as.
    ///
    /// # Errors
    ///
    /// [`Error::NoSavepoint`] where the connection holds no savepoint of
    /// that name open.
    fn savepoint(
        &mut self,
        asked: crate::ast::Savepoint,
        sql: &[u8],
    ) -> Result<Vec<Vec<Value>>, Error> {
        let (crate::ast::Savepoint::Open(span)
        | crate::ast::Savepoint::Release(span)
        | crate::ast::Savepoint::Back(span)) = asked;
        let name = crate::schema::dequote(span.text(sql));
        let word: &[u8] = match asked {
            crate::ast::Savepoint::Open(_) => b"BEGIN",
            crate::ast::Savepoint::Release(_) => b"RELEASE",
            crate::ast::Savepoint::Back(_) => b"ROLLBACK",
        };
        if self
            .asked(|authorizer| authorizer.savepoint(word, &name))?
            .answer
            == crate::auth::Answer::Ignore
        {
            return Ok(Vec::new());
        }
        if let crate::ast::Savepoint::Open(_) = asked {
            // A `SAVEPOINT` outside a transaction opens one, which the
            // release of that savepoint commits.
            let opener = self.held.began.is_none();
            if opener {
                self.opening();
            }
            self.saved.push(Saved {
                name,
                files: self.saving(),
                deferred: self.deferred,
                opener,
            });
            return Ok(Vec::new());
        }
        // The innermost savepoint of that name is the one the statement
        // names, which is what `sqlite3Savepoint` walks the list for.
        let at = self
            .saved
            .iter()
            .rposition(|held| held.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| Error::NoSavepoint(name.clone()))?;
        if let crate::ast::Savepoint::Back(_) = asked {
            let held = self.saved.get(at).ok_or(Error::NoSavepoint(Vec::new()))?;
            let deferred = held.deferred;
            let files = held.files.clone();
            self.deferred = deferred;
            self.restoring(&files);
            // The savepoint the statement names stays open, and every
            // one inside it is gone.
            self.saved.truncate(at.saturating_add(1));
            return Ok(Vec::new());
        }
        let opener = self.saved.get(at).is_some_and(|held| held.opener);
        // Releasing the savepoint that opened the transaction writes
        // it, so a foreign key held at the end of the transaction is
        // held there and the savepoint stays open.
        if opener && self.deferred != 0 {
            self.refusing = Refusing::Fail;
            return Err(Error::Foreign);
        }
        self.saved.truncate(at);
        if opener {
            let was = self.held.began.take().ok_or(Error::NoTransaction(false))?;
            self.commit(&was)?;
        }
        Ok(Vec::new())
    }

    /// Every database the connection holds, as it stands, which is what a
    /// savepoint keeps.
    ///
    /// Keeping them costs O(n) in the pages of every database.
    fn saving(&self) -> Vec<SavedFile> {
        let mut out = alloc::vec![SavedFile {
            name: self.called.name.clone(),
            pages: self.held.pages.clone(),
            header: self.held.header,
        }];
        for held in &self.attached {
            out.push(SavedFile {
                name: held.called.name.clone(),
                pages: held.held.pages.clone(),
                header: held.held.header,
            });
        }
        out
    }

    /// Every database the connection holds put back as `files` found it,
    /// which is what a `ROLLBACK TO` writes.
    ///
    /// A database the savepoint did not hold is left as it stands, which
    /// an `ATTACH` inside the savepoint added.
    ///
    /// Writing them costs O(n) in the pages of every database.
    fn restoring(&mut self, files: &[SavedFile]) {
        for file in files {
            if file.name.eq_ignore_ascii_case(&self.called.name) {
                self.held.pages = file.pages.clone();
                self.held.header = file.header;
                continue;
            }
            for held in &mut self.attached {
                if held.called.name.eq_ignore_ascii_case(&file.name) {
                    held.held.pages = file.pages.clone();
                    held.held.header = file.header;
                }
            }
        }
    }

    /// A transaction opened on every database the connection holds, which
    /// is what a `BEGIN` and the `SAVEPOINT` that stands for one open.
    fn opening(&mut self) {
        for held in Self::files_mut(&mut self.held, &mut self.attached) {
            held.pages.begin();
            held.began = Some(held.header);
        }
    }

    /// What the transaction wrote on every database it wrote, written.
    ///
    /// Each database carries a journal of its own, so a run that stops
    /// between two of them leaves one written and one not.
    ///
    /// # Errors
    ///
    /// [`Error`] names what one of the commits refused.
    fn committed(&mut self) -> Result<(), Error> {
        for held in Self::files_mut(&mut self.held, &mut self.attached) {
            // Every database the connection holds joined the
            // transaction, so each carries the header it began under.
            let was = held.began.take().unwrap_or(held.header);
            commit_file(held, &was)?;
        }
        Ok(())
    }

    /// Whether the function `sqlite3_commit_hook` told the connection
    /// refuses the commit.
    ///
    /// `sqlite3VdbeHalt` of `research/sqlite/src/vdbeaux.c:2982` asks
    /// the function only where a database of the connection carries a
    /// transaction that wrote, which is `needXcommit`. Reading that
    /// costs O(1) per database the connection holds.
    fn refuses_commit(&mut self) -> bool {
        let Some(committing) = self.committing else {
            return false;
        };
        let wrote =
            Self::files_mut(&mut self.held, &mut self.attached).any(|held| held.pages.changed());
        wrote && committing()
    }

    /// Tells the function `sqlite3_rollback_hook` told the connection
    /// that a transaction went back, which `sqlite3RollbackAll` of
    /// `research/sqlite/src/main.c:1535` does.
    fn tells_rollback(&self) {
        if let Some(rolling) = self.rolling {
            rolling();
        }
    }

    /// Tells the function `sqlite3_preupdate_hook` told the connection of
    /// one row a statement is about to write.
    ///
    /// The row carries the key in the column the key is another name
    /// for, which `sqlite3_preupdate_old` answers, so copying it costs
    /// O(n) in the columns of the table.
    fn tells_peek(
        &self,
        did: Did,
        table: &Table,
        keys: (i64, i64),
        rows: (Option<&[Value]>, Option<&[Value]>),
    ) {
        let Some(peeking) = self.peeking else {
            return;
        };
        let (was, key) = keys;
        let old = rows.0.map(|values| shown_row(table, values, was));
        let new = rows.1.map(|values| shown_row(table, values, key));
        peeking(&Peeked {
            did,
            schema: &self.called.name,
            table: &table.name,
            was,
            key,
            old: old.as_deref(),
            new: new.as_deref(),
            depth: self.running.len(),
        });
    }

    /// Tells the function `sqlite3_update_hook` told the connection of one
    /// row a statement wrote, which costs O(1).
    ///
    /// The three callers are the paths that write a table keeping a key
    /// of its own, so a row of a `WITHOUT ROWID` table reaches no
    /// function; `sqlite_master` and `sqlite_sequence` are written by
    /// paths of their own and reach none either, which is
    /// `OP_Insert` of `research/sqlite/src/vdbe.c:5779` carrying the
    /// table in `p4` for the statements the hook is told of alone.
    fn tells_write(&self, did: Did, table: &Table, rowid: i64) {
        let Some(writing) = self.writes else {
            return;
        };
        writing(&Wrote {
            did,
            schema: &self.called.name,
            table: &table.name,
            rowid,
        });
    }

    /// Every database the connection holds back where its transaction
    /// began, which is what a `ROLLBACK` writes.
    fn rolled_back(&mut self) {
        for held in Self::files_mut(&mut self.held, &mut self.attached) {
            let was = held.began.take().unwrap_or(held.header);
            held.pages.rollback();
            held.header = was;
        }
    }

    /// Every database the connection holds, the one the statement writes
    /// first.
    fn files_mut<'a>(
        held: &'a mut HeldFile,
        attached: &'a mut [Attached],
    ) -> impl Iterator<Item = &'a mut HeldFile> {
        core::iter::once(held).chain(attached.iter_mut().map(|one| &mut one.held))
    }

    /// What the transaction wrote, written: the file is cut back where
    /// it vacuums itself, the header counts the pages and the free list
    /// it now has, and the commit leaves a journal or a frame.
    fn commit(&mut self, was: &Header) -> Result<(), Error> {
        commit_file(&mut self.held, was)
    }
}

/// Whether the name is one a `DROP TABLE` may not take away, which
/// `tableMayNotBeDropped` of `research/sqlite/src/build.c:3476` is: a name
/// that begins `sqlite_` is one SQLite keeps for itself, beside
/// `sqlite_stat` and `sqlite_parameters`, which `ANALYZE` and the shell
/// write and a statement may take away.
fn kept_name(name: &[u8]) -> bool {
    let Some(tail) = name.get(..7).and_then(|head| {
        head.eq_ignore_ascii_case(b"sqlite_")
            .then(|| name.get(7..).unwrap_or_default())
    }) else {
        return false;
    };
    let named = |word: &[u8]| {
        tail.get(..word.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(word))
    };
    !named(b"stat") && !named(b"parameters")
}

/// The row an `UPDATE` fires its triggers over: what it holds now under
/// the key it stands under, and what it will hold under the key it takes.
const fn changing<'a>(
    table: &'a Table,
    old: (&'a [Value], i64),
    new: (&'a [Value], i64),
    encoding: Encoding,
) -> Fired<'a> {
    Fired {
        table,
        old: Some(old),
        new: Some(new),
        encoding,
    }
}

/// The values of one row as the function `sqlite3_preupdate_hook` told the
/// connection sees them, which is the column the key is another name for
/// carrying the key.
///
/// Copying costs O(n) in the columns of the table.
fn shown_row(table: &Table, values: &[Value], rowid: i64) -> Vec<Value> {
    values
        .iter()
        .enumerate()
        .map(|(at, value)| {
            if Some(at) == table.rowid_alias {
                Value::Int(rowid)
            } else {
                value.clone()
            }
        })
        .collect()
}

/// Whether the refusal came while the statement ran rather than while the
/// connection read it, which says the statement had opened a transaction
/// that writes.
///
/// `sqlite3RollbackAll` of `research/sqlite/src/main.c:1495` tells the
/// rollback hook where `inTrans` stands, and the C library opens the
/// transaction that writes after it has read the statement and found the
/// objects it names, so a constraint the row breaks carries one and a
/// statement the connection could not read carries none.
const fn ran_refusal(error: &Error) -> bool {
    matches!(
        error,
        Error::Unique(_)
            | Error::NotNull(_)
            | Error::Check(_)
            | Error::Foreign
            | Error::ForeignMismatch(..)
            | Error::StoredType(..)
            | Error::Constraint
            | Error::HeldConstraint(_)
    )
}

/// What the transaction wrote on one database, written: the file is cut
/// back where it vacuums itself, the header counts the pages and the free
/// list it now has, and the commit leaves a journal or a frame.
fn commit_file(held: &mut HeldFile, was: &Header) -> Result<(), Error> {
    // A transaction that wrote no page is one the commit has nothing to
    // write for, so the change counter stands where it stood.
    if !held.pages.changed() {
        return Ok(());
    }
    // `autoVacuumCommit`: a file that vacuums itself whole moves the
    // pages at its end into the free pages below them and is cut back
    // before the commit writes anything.
    if held.header.incremental_vacuum == 0 {
        held.pages.vacuum_commit()?;
    }
    held.header.pages = held.pages.count();
    (held.header.freelist, held.header.freelist_pages) = held.pages.freelist();
    // `sqlite3PagerOpenWal`: a file whose header says version two is
    // read and written through a log, so a connection over one that
    // holds no log makes it, which a close that wrote the last log back
    // into the file leaves.
    if held.log.is_none() && held.header.write_version == 2 {
        let log = Log::new(held.header.page_size, (0, 0), 0, false);
        held.did.push(Does::Write {
            onto: Onto::Log,
            at: 0,
            bytes: log.bytes().to_vec(),
        });
        held.did.push(Does::Sync(Onto::Log));
        held.origin = Some(held.pages.committed(was));
        held.log = Some(log);
    }
    // `pager_write_changecounter`: a frame of page one holds the counter
    // the file holds and one, and no checkpoint writes the file, so every
    // commit writes the same counter there. A commit that writes the file
    // itself carries the counter on.
    if let Some(log) = &mut held.log {
        // `walRestartLog`: the commit after a checkpoint begins the log
        // again, because every frame of it is in the file already.
        if held.restarting {
            log.restart();
            held.restarting = false;
        }
        let mut now = held.header;
        now.change_counter = now.change_counter.saturating_add(1);
        now.version_valid_for = now.change_counter;
        let stood = log.bytes().len();
        log.commit(&held.pages.frames(&now), held.pages.count());
        let did = logged(log.bytes(), stood, held.header.page_size);
        held.did.extend(did);
        return Ok(());
    }
    held.header.change_counter = held.header.change_counter.saturating_add(1);
    held.header.version_valid_for = held.header.change_counter;
    let written = held.pages.journal(was, held.nonce, held.sector);
    let did = journalled(&written, held);
    held.journal = crate::journal::committed(&written, held.mode);
    held.did.extend(did);
    Ok(())
}

/// What a commit in write-ahead logging mode wrote: the header of a log
/// the commit began, one write per frame it appended, and the sync that
/// holds them, which is `walWriteToLog` followed by `sqlite3OsSync`.
///
/// `stood` is how many bytes the log held before the commit. Building the
/// list costs O(n) in the frames.
fn logged(bytes: &[u8], stood: usize, page_size: u32) -> Vec<Does> {
    let header = crate::wal::HEADER;
    let frame = crate::wal::FRAME.saturating_add(crate::bytes::size(u64::from(page_size)));
    let mut did = Vec::new();
    let mut at = stood.max(header);
    // A log the commit began, and one it began again, hold no frame the
    // commit before this one wrote, so the header is written again.
    if stood <= header {
        did.push(Does::Write {
            onto: Onto::Log,
            at: 0,
            bytes: bytes.get(..header).unwrap_or_default().to_vec(),
        });
    }
    while let Some(held) = bytes.get(at..at.saturating_add(frame)) {
        did.push(Does::Write {
            onto: Onto::Log,
            at: u64::try_from(at).unwrap_or(u64::MAX),
            bytes: held.to_vec(),
        });
        at = at.saturating_add(frame);
    }
    did.push(Does::Sync(Onto::Log));
    did
}

/// What a commit that keeps a rollback journal wrote: the header of the
/// journal, one write per record it holds, the sync that holds them, one
/// write per page of the file, the sync that holds those, and what the
/// journal mode leaves of the journal.
///
/// This is the order `sqlite3PagerCommitPhaseOne` writes in: the journal
/// is on the disk before a page of the file is written over, so a machine
/// that loses power between the two is recovered by playing the journal
/// back. Building the list costs O(n) in the pages.
fn journalled(written: &[u8], held: &HeldFile) -> Vec<Does> {
    let sector = crate::bytes::size(u64::from(held.sector));
    let page_size = crate::bytes::size(u64::from(held.header.page_size));
    let record = page_size.saturating_add(8);
    // `writeJournalHdr` writes the header with no count of the records
    // that follow it, and `syncJournal` writes the count after the first
    // sync and holds the journal a second time, so a machine that lost
    // power before that count reaches the disk plays nothing back.
    let mut header = written.get(..sector).unwrap_or_default().to_vec();
    let counted: Vec<u8> = header.get(8..12).unwrap_or_default().to_vec();
    for slot in header.iter_mut().skip(8).take(4) {
        *slot = 0;
    }
    let mut did = alloc::vec![Does::Write {
        onto: Onto::Journal,
        at: 0,
        bytes: header,
    }];
    let mut at = sector;
    while let Some(bytes) = written.get(at..at.saturating_add(record)) {
        did.push(Does::Write {
            onto: Onto::Journal,
            at: u64::try_from(at).unwrap_or(u64::MAX),
            bytes: bytes.to_vec(),
        });
        at = at.saturating_add(record);
    }
    did.push(Does::Sync(Onto::Journal));
    did.push(Does::Write {
        onto: Onto::Journal,
        at: 8,
        bytes: counted,
    });
    did.push(Does::Sync(Onto::Journal));
    for (number, page) in held.pages.frames(&held.header) {
        did.push(Does::Write {
            onto: Onto::Main,
            at: u64::from(number.saturating_sub(1))
                .saturating_mul(u64::try_from(page_size).unwrap_or(u64::MAX)),
            bytes: page,
        });
    }
    // `pager_truncate`: a file the commit shrank is cut back once every
    // page it keeps is written.
    did.push(Does::Truncate {
        onto: Onto::Main,
        at: u64::from(held.pages.count())
            .saturating_mul(u64::try_from(page_size).unwrap_or(u64::MAX)),
    });
    did.push(Does::Sync(Onto::Main));
    // `pager_end_transaction` takes the journal away under `delete`, cuts
    // it to nothing under `truncate`, and writes zeros over its header
    // under `persist`.
    did.push(match held.mode {
        Mode::Truncate => Does::Truncate {
            onto: Onto::Journal,
            at: 0,
        },
        Mode::Persist => Does::Write {
            onto: Onto::Journal,
            at: 0,
            bytes: alloc::vec![0u8; crate::journal::ZEROED],
        },
        Mode::Delete | Mode::Memory | Mode::Off => Does::Remove(Onto::Journal),
    });
    did
}

impl Writer {
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
        let written = asked.value.map(|value| value.text(sql)).unwrap_or_default();
        if self
            .asked(|authorizer| authorizer.pragma(&name, written, b""))?
            .answer
            == crate::auth::Answer::Ignore
        {
            return Ok(Vec::new());
        }
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
        if setting == crate::pragma::Setting::DatabaseList {
            return Ok(self.listed_databases());
        }
        if setting == crate::pragma::Setting::LockStatus {
            return Ok(self.locks_held());
        }
        if let Some(listed) = self.schema_rows(setting, asked, sql)? {
            return Ok(listed);
        }
        if setting == crate::pragma::Setting::WalCheckpoint {
            let how = asked.value.map(|value| value.text(sql));
            return Ok(self.checkpoint(how));
        }
        if setting == crate::pragma::Setting::IncrementalVacuum {
            let most = asked_steps(asked.value.map(|value| value.text(sql)));
            return self.vacuumed_steps(most);
        }
        if let Some(quick) = quick {
            let asked = crate::check::checking(asked.value.map(|value| value.text(sql)));
            return self.integrity(quick, &asked);
        }
        let Some(value) = asked.value else {
            return self.pragma_read(setting);
        };
        self.pragma_write(setting, value.text(sql), asked.schema.is_none())
    }

    /// The place of the value one blob handle names: the tree of its
    /// table, the key of its row, and where in the payload of that row
    /// the bytes of the value lie.
    ///
    /// `sqlite3_blob_open` of `research/sqlite/src/vdbeblob.c:74` reads
    /// the table, the column and the row this way, and refuses each of
    /// them in this order. Reading the row costs O(log n) in the rows.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] for a table no database holds,
    /// [`Error::BlobView`] for a view, [`Error::BlobKeyed`] for a table
    /// written `WITHOUT ROWID`, [`Error::NoSuchColumn`] for a column the
    /// table does not hold, [`Error::BlobColumn`] for a column an index,
    /// a primary key or a foreign key holds where the handle writes,
    /// [`Error::NoRowid`] for a key no row carries, and
    /// [`Error::BlobValue`] for a value that is neither text nor bytes.
    fn blob_place(&self, asked: &Blob<'_>) -> Result<Placed, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        if database.view(asked.table).is_some() {
            return Err(Error::BlobView(asked.table.to_vec()));
        }
        let (table, root) = database
            .table(asked.table)
            .ok_or_else(|| Error::NoTable(blob_named(asked, &self.called.name)))?;
        if table.without_rowid {
            return Err(Error::BlobKeyed(asked.table.to_vec()));
        }
        let column = table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(asked.column))
            .ok_or_else(|| Error::NoSuchColumn(asked.column.to_vec()))?;
        if asked.writing
            && let Some(held) = held_column(&database, table, column, self.holding())
        {
            return Err(Error::BlobColumn(held.to_vec()));
        }
        let row = database
            .image()
            .rows(root)
            .find_map(|row| match row {
                Ok(row) if row.rowid == asked.rowid => Some(Ok(row)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .transpose()?
            .ok_or(Error::NoRowid(asked.rowid))?;
        let mut payload = alloc::vec![0_u8; row.payload.total];
        database.image().read_payload(&row.payload, &mut payload)?;
        // A row written before the column was added holds no value for
        // it, which `sqlite3VdbeMemFromBtree` of
        // `research/sqlite/src/vdbeaux.c` reads as a null.
        let (at, len, serial) =
            crate::record::placed(&payload, column)?.unwrap_or((0, 0, crate::record::Serial::Null));
        match serial {
            crate::record::Serial::Text(_) | crate::record::Serial::Blob(_) => {}
            crate::record::Serial::Null => return Err(Error::BlobValue(b"null".to_vec())),
            crate::record::Serial::Real => return Err(Error::BlobValue(b"real".to_vec())),
            crate::record::Serial::Int(_)
            | crate::record::Serial::Zero
            | crate::record::Serial::One => {
                return Err(Error::BlobValue(b"integer".to_vec()));
            }
        }
        Ok(Placed {
            root,
            at,
            len,
            payload,
        })
    }

    /// How many bytes the value one blob handle names holds, which
    /// `sqlite3_blob_bytes` answers.
    ///
    /// # Errors
    ///
    /// What the place of the value is refused with.
    pub fn blob_bytes(&mut self, asked: &Blob<'_>) -> Result<usize, Error> {
        let (len, _) = self.blob_asked(asked, &Asking::Bytes)?;
        Ok(len)
    }

    /// The bytes of that value from `at`, as many as `len`, which
    /// `sqlite3_blob_read` answers.
    ///
    /// Reading them costs O(n) in the bytes of the value.
    ///
    /// # Errors
    ///
    /// What the place of the value is refused with, and
    /// [`Error::BlobRange`] where the value ends before the bytes asked
    /// for do.
    pub fn blob_read(&mut self, asked: &Blob<'_>, at: usize, len: usize) -> Result<Vec<u8>, Error> {
        let (_, read) = self.blob_asked(asked, &Asking::Read(at, len))?;
        Ok(read)
    }

    /// Writes `bytes` over that value from `at`, which
    /// `sqlite3_blob_write` does: the value keeps the length it had, so
    /// no page moves and no index is written again.
    ///
    /// # Errors
    ///
    /// What the place of the value is refused with, and
    /// [`Error::BlobRange`] where the value ends before the bytes do.
    pub fn blob_write(&mut self, asked: &Blob<'_>, at: usize, bytes: &[u8]) -> Result<(), Error> {
        self.blob_asked(asked, &Asking::Write(at, bytes))?;
        Ok(())
    }

    /// What one command of a blob handle answers: how many bytes the
    /// value holds and what the command read of it.
    ///
    /// # Errors
    ///
    /// What the place of the value is refused with, and
    /// [`Error::BlobRange`] where the value ends before the bytes the
    /// command names.
    fn blob_asked(
        &mut self,
        asked: &Blob<'_>,
        doing: &Asking<'_>,
    ) -> Result<(usize, Vec<u8>), Error> {
        let place = match asked.schema {
            None => None,
            Some(named) if self.called.name.eq_ignore_ascii_case(named) => None,
            Some(named) => Some(
                self.attached
                    .iter()
                    .position(|held| named_as(held, named))
                    // `sqlite3LocateTable` of
                    // `research/sqlite/src/build.c:408` names the
                    // database and the table together, whichever of the
                    // two the connection does not hold.
                    .ok_or_else(|| Error::NoTable(blob_named(asked, &self.called.name)))?,
            ),
        };
        if let Some(place) = place {
            self.switch(place);
        }
        let answered = self.blob_held(asked, doing);
        if let Some(place) = place {
            self.switch(place);
        }
        answered
    }

    /// The same over the database the connection writes, which the one
    /// the handle names has become.
    ///
    /// # Errors
    ///
    /// What the place of the value is refused with, and
    /// [`Error::BlobRange`] where the value ends before the bytes the
    /// command names.
    fn blob_held(
        &mut self,
        asked: &Blob<'_>,
        doing: &Asking<'_>,
    ) -> Result<(usize, Vec<u8>), Error> {
        let placed = self.blob_place(asked)?;
        let (at, wanted) = match doing {
            Asking::Bytes => (0, 0),
            Asking::Read(at, len) => (*at, *len),
            Asking::Write(at, bytes) => (*at, bytes.len()),
        };
        if at.saturating_add(wanted) > placed.len {
            return Err(Error::BlobRange);
        }
        let from = placed.at.saturating_add(at);
        match doing {
            Asking::Bytes => Ok((placed.len, Vec::new())),
            Asking::Read(..) => Ok((
                placed.len,
                placed
                    .payload
                    .get(from..from.saturating_add(wanted))
                    .unwrap_or_default()
                    .to_vec(),
            )),
            Asking::Write(_, bytes) => {
                crate::tree::put_payload(
                    &mut self.held.pages,
                    placed.root,
                    asked.rowid,
                    from,
                    bytes,
                )?;
                Ok((placed.len, Vec::new()))
            }
        }
    }

    /// `PRAGMA auto_vacuum` over a file that holds a table: which of the
    /// two ways the file vacuums itself.
    ///
    /// `sqlite3BtreeSetAutoVacuum` of `research/sqlite/src/btree.c`
    /// writes the incremental flag of a file that vacuums itself
    /// already, and leaves the vacuuming as it is once the page size is
    /// fixed, so the word names the second and not the first.
    fn vacuuming_as(&mut self, text: &[u8]) {
        let which = crate::pragma::vacuum_of(text);
        if which != 0 {
            self.held.header.incremental_vacuum = u32::from(which == 2);
        }
    }

    /// `PRAGMA reserved_bytes = N`: how many bytes of every page the
    /// b-tree layer may not use, which `SQLITE_FCNTL_RESERVE_BYTES` of
    /// `research/sqlite/src/btree.c` writes and the first table is
    /// written under.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a count no byte holds, and
    /// [`Error::Image`] for one that leaves a page too little room.
    fn reserving(&mut self, text: &[u8]) -> Result<(), Error> {
        let bytes = crate::pragma::whole_number(text).ok_or(Error::Unsupported)?;
        let bytes = u8::try_from(bytes).map_err(|_| Error::Unsupported)?;
        self.held.pages = Pages::new(self.held.header.page_size, bytes)?;
        self.held.header.reserved = bytes;
        Ok(())
    }

    /// `PRAGMA incremental_vacuum(N)`: the file gives up as many as `N`
    /// pages at its end and the pragma answers one row of no column per
    /// page it gave up.
    ///
    /// `PragTyp_INCREMENTAL_VACUUM` of
    /// `research/sqlite/src/pragma.c:854` writes a loop of
    /// `OP_IncrVacuum` and an `OP_ResultRow` of no column, which ends
    /// where the step answers `SQLITE_DONE`. Giving up one page costs
    /// O(n) in the pages of the free list.
    ///
    /// # Errors
    ///
    /// Whatever the step refuses.
    fn vacuumed_steps(&mut self, most: u32) -> Result<Vec<Vec<Value>>, Error> {
        let mut rows = Vec::new();
        for _ in 0..most {
            if !self.held.pages.vacuum_incremental()? {
                break;
            }
            rows.push(Vec::new());
        }
        Ok(rows)
    }

    /// `PRAGMA name = value`, with `text` for what is written.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] for a pragma this crate does not write
    /// and for a value it does not name.
    fn pragma_write(
        &mut self,
        setting: crate::pragma::Setting,
        text: &[u8],
        every: bool,
    ) -> Result<Vec<Vec<Value>>, Error> {
        if setting == crate::pragma::Setting::Ignored {
            return Ok(Vec::new());
        }
        // `PragTyp_ENCODING` reads the name against its own table before
        // it reads whether the schema is there, so a name no encoding
        // carries is refused whatever the file already holds.
        if setting == crate::pragma::Setting::Encoding && crate::pragma::encoding_of(text).is_none()
        {
            return Err(Error::NoEncoding(crate::schema::dequote(text)));
        }
        // `PRAGMA freelist_count` and `PRAGMA page_count` read the file
        // whatever stands after the equals sign: the first carries
        // `PragFlg_ReadOnly`, and `PragTyp_PAGE_COUNT` reads where the
        // name begins with `p`.
        if matches!(
            setting,
            crate::pragma::Setting::FreelistCount | crate::pragma::Setting::PageCount
        ) {
            return self.pragma_read(setting);
        }
        if setting == crate::pragma::Setting::CaseSensitiveLike {
            self.truth.sensitive = crate::pragma::truth(text).unwrap_or(false);
            return Ok(Vec::new());
        }
        if setting == crate::pragma::Setting::DefaultCacheSize {
            // `PragTyp_DEFAULT_CACHE_SIZE` writes the header word and
            // the connection's own cache size together, so the pragma
            // that reads either answers what this one wrote.
            let size = crate::pragma::cache_word(text);
            self.held.header.cache_size = size;
            for slot in self.kept.iter_mut().skip(crate::pragma::CACHED).take(1) {
                *slot = Some(i64::from(size));
            }
            return Ok(Vec::new());
        }
        // `PragTyp_HEADER_VALUE` writes one word of the header in a
        // transaction of its own, and the three words a pragma writes are
        // the schema cookie, the number of the application and the
        // number of the user. `data_version` and `freelist_count` carry
        // `PragFlg_ReadOnly` and are read alone.
        let word = match setting {
            crate::pragma::Setting::SchemaVersion => Some(&mut self.held.header.schema_cookie),
            crate::pragma::Setting::UserVersion => Some(&mut self.held.header.user_version),
            crate::pragma::Setting::ApplicationId => Some(&mut self.held.header.application_id),
            _ => None,
        };
        if let Some(word) = word {
            *word = crate::pragma::header_word(text);
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
            self.truth.counting = crate::pragma::truth(text).ok_or(Error::Unsupported)?;
            return Ok(Vec::new());
        }
        if setting == crate::pragma::Setting::AutoVacuum
            && self.held.header.schema_cookie != 0
            && self.held.header.largest_root != 0
        {
            self.vacuuming_as(text);
            return Ok(Vec::new());
        }
        if self.held.header.schema_cookie != 0 && setting != crate::pragma::Setting::JournalMode {
            // `sqlite3BtreeSetPageSize` keeps the size the pragma named
            // for the next `VACUUM`, which is the only thing that can
            // write the file again under another size.
            if setting == crate::pragma::Setting::PageSize {
                self.held.wanted_page = crate::pragma::whole_number(text);
            }
            return Ok(Vec::new());
        }
        match setting {
            crate::pragma::Setting::PageSize => {
                let size = crate::pragma::whole_number(text).ok_or(Error::Unsupported)?;
                self.held.pages = Pages::new(size, self.held.header.reserved)?;
                self.held.header.page_size = size;
            }
            crate::pragma::Setting::Reserved => self.reserving(text)?,
            crate::pragma::Setting::Encoding => {
                self.held.header.encoding =
                    crate::pragma::encoding_of(text).ok_or(Error::Unsupported)?;
            }
            crate::pragma::Setting::AutoVacuum => match crate::pragma::vacuum_of(text) {
                0 => {}
                which => self.vacuuming(which == 2),
            },
            crate::pragma::Setting::JournalMode => {
                // `sqlite3PragmaJournalMode` leaves the mode as it is
                // where the connection has a transaction open, so the
                // pragma answers the mode it did not change.
                let held = self.held.began.is_some();
                let wanted = crate::pragma::mode_of(text);
                if crate::pragma::is_log(text) {
                    if !held {
                        self.log_mode();
                    }
                } else {
                    // A file in write-ahead logging leaves that mode
                    // through a checkpoint, which this crate does not
                    // write.
                    if self.held.log.is_some() {
                        return Err(Error::Unsupported);
                    }
                    let mode = wanted.ok_or(Error::Unsupported)?;
                    if !held {
                        // A file whose log a close wrote back still says
                        // version two, which `sqlite3PagerSetJournalMode`
                        // writes back to one where the pragma names
                        // another mode.
                        self.held.header.write_version = 1;
                        self.held.header.read_version = 1;
                        self.held.mode = mode;
                        // `PRAGMA journal_mode = X` with no schema in
                        // front of it sets the mode of every database the
                        // connection holds, which
                        // `sqlite3PragmaJournalMode` of
                        // `research/sqlite/src/pragma.c:520` writes to
                        // each and keeps as the connection's own.
                        if every {
                            for beside in &mut self.attached {
                                beside.held.mode = mode;
                            }
                        }
                    }
                }
                // The mode the connection is left in is the one row
                // this pragma answers, which no other setting does.
                return Ok(alloc::vec![alloc::vec![Value::Text(
                    self.journalled().to_vec()
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
        let database = self.reading(&bytes)?;
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
        let database = self.reading(&bytes)?;
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
                let places = parent_places(&database, (&name, key), parent)?;
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
        // `sqlite3InitOne` reads the cache size a connection begins with
        // out of the header word, so the file and not the crate says
        // what a connection told nothing answers.
        let fallback = match at {
            crate::pragma::CACHED => crate::pragma::default_cache(self.held.header.cache_size),
            _ => crate::pragma::HELD
                .get(at)
                .map_or(0, |keeps| keeps.fallback),
        };
        let value = self.kept.get(at).copied().flatten().unwrap_or(fallback);
        crate::pragma::kept(at, value)
    }

    /// The pragma at `at` of [`crate::pragma::HELD`] set to what `text`
    /// names, which answers the value it was set to where that pragma
    /// answers one.
    fn keep(&mut self, at: usize, text: &[u8]) -> Result<Vec<Vec<Value>>, Error> {
        let value = crate::pragma::keeping(at, text).ok_or(Error::Unsupported)?;
        let keeps = crate::pragma::HELD.get(at);
        // `PragTyp_FLAG` takes `SQLITE_ForeignKeys` out of the mask
        // where the connection has a transaction open, so a statement
        // there changes nothing.
        // `PragTyp_SYNCHRONOUS` refuses inside a transaction, because the
        // level says what a commit writes and a commit is waiting.
        if self.held.began.is_some() && keeps.is_some_and(|keeps| keeps.name == b"synchronous") {
            return Err(Error::SafetyInTransaction);
        }
        let held =
            self.held.began.is_some() && keeps.is_some_and(|keeps| keeps.name == b"foreign_keys");
        if !held && !keeps.is_some_and(|keeps| keeps.fixed) {
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
        let written = table.name.text(sql);
        if self.already((&name, written), table.if_not_exists, Making::Table)? {
            return Ok(());
        }
        let (answer, affinities) = {
            let bytes = self.images();
            let database = self
                .reading_beside(&bytes)?
                .seeded(self.random.word())
                .counting(self.counted);
            database.answered(arena, select, sql)?
        };
        let columns = crate::schema::columns_from(&answer.names);
        let written = crate::schema::created(&name, &columns, &affinities);
        let root = self.held.pages.add(Kind::LeafTable, 0)?;
        self.held.pages.point(root, crate::tree::Point::Root, 0)?;
        if self.held.header.largest_root != 0 {
            self.held.header.largest_root = root;
        }
        let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                text(b"table"),
                text(&name),
                text(&name),
                Value::Int(i64::from(root)),
                text(&written),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        let at = self.schema_blank()?;
        self.schema_written(at, &row)?;
        let mut rowid = 0_i64;
        for row in &answer.rows {
            let record = crate::record::write_in(row, &affinities, 4, self.held.header.encoding);
            rowid = rowid.saturating_add(1);
            insert(&mut self.held.pages, root, rowid, &record)?;
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
    /// [`Error::Timed`] for a trigger whose time the table it is over
    /// does not take, [`Error::SystemTrigger`] for one over a table
    /// SQLite keeps for itself, [`Error::NoTable`] for one over a table
    /// the database does not hold, and [`Error::Exists`] for a name the
    /// database already holds.
    fn create_trigger(
        &mut self,
        arena: &Arena,
        trigger: &crate::ast::CreateTrigger,
        sql: &[u8],
    ) -> Result<(), Error> {
        let name = crate::schema::dequote(trigger.name.text(sql));
        let over = crate::schema::dequote(trigger.table.text(sql));
        // `sqlite3CreateTrigger`: a trigger runs with no statement of
        // its own to bind against, so a variable anywhere in it stands
        // for nothing.
        if crate::token::holds_variable(trigger.written.text(sql)) {
            return Err(Error::TriggerVariable);
        }
        // `sqlite3TriggerInsertStep` and the two beside it take the
        // name of a table alone, so a schema in front of one is
        // refused.
        for step in arena.steps(trigger.body) {
            let (schema, indexed) = match *step {
                crate::ast::TriggerStep::Insert(ref statement) => {
                    (statement.schema, crate::ast::Indexed::Unspecified)
                }
                crate::ast::TriggerStep::Update(ref statement) => {
                    (statement.schema, statement.indexed)
                }
                crate::ast::TriggerStep::Delete(ref statement) => {
                    (statement.schema, statement.indexed)
                }
                crate::ast::TriggerStep::Select(_) => (None, crate::ast::Indexed::Unspecified),
            };
            if schema.is_some() {
                return Err(Error::QualifiedInTrigger);
            }
            // `sqlite3TriggerUpdateStep` and `sqlite3TriggerDeleteStep`
            // of `research/sqlite/src/trigger.c:436` take no index, so a
            // body that names one is refused, under the clause it wrote.
            if indexed != crate::ast::Indexed::Unspecified {
                return Err(Error::IndexedInTrigger(indexed == crate::ast::Indexed::Not));
            }
        }
        // `sqlite3CreateTrigger`: a table SQLite keeps for itself
        // carries no trigger at all.
        if over
            .get(..7)
            .is_some_and(|head| head.eq_ignore_ascii_case(b"sqlite_"))
        {
            return Err(Error::SystemTrigger);
        }
        {
            let bytes = self.images();
            let database = self.reading_beside(&bytes)?;
            // `sqlite3CreateTrigger`: only a view carries an `INSTEAD
            // OF` trigger, and only a table carries the other two.
            let on_view = database.view(&over).is_some();
            let instead = trigger.time == crate::ast::TriggerTime::InsteadOf;
            if on_view != instead {
                let word = match trigger.time {
                    crate::ast::TriggerTime::Before => b"BEFORE".as_slice(),
                    crate::ast::TriggerTime::After => b"AFTER",
                    crate::ast::TriggerTime::InsteadOf => b"INSTEAD OF",
                };
                let held: &[u8] = if on_view { b"view" } else { b"table" };
                return Err(Error::Timed(word.to_vec(), held.to_vec(), over));
            }
            // `sqlite3TriggerBeginStep` names the schema the table
            // would stand in, which a trigger of the temporary schema
            // names no schema for.
            if !instead && database.table(&over).is_none() {
                let named = if trigger.temporary {
                    over
                } else {
                    schema_named_as(&over)
                };
                return Err(Error::NoTable(named));
            }
            self.reserved(&name)?;
            if database.held_trigger(&name).is_some() {
                if trigger.if_not_exists {
                    return Ok(());
                }
                let written = trigger.name.text(sql).to_vec();
                return Err(Error::Exists(b"trigger".to_vec(), written));
            }
        }
        let mut written = b"CREATE TRIGGER ".to_vec();
        written.extend_from_slice(trigger.written.text(sql));
        let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                text(b"trigger"),
                text(&name),
                text(&over),
                Value::Int(0),
                text(&written),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        let rowid = largest(&self.held.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        self.held.header.schema_format = 4;
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
        let database = self.reading(&bytes)?;
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
            // `OP_Program` keeps `sqlite3_last_insert_rowid` over the
            // body, so a row the body writes is one the statement that
            // fired the trigger does not answer.
            let held = self.counted.rowid;
            let ran = self.body(trigger, row);
            self.counted.rowid = held;
            self.running.pop();
            match ran {
                Err(Error::Eval(crate::eval::Error::Raised(crate::ast::Raise::Ignore, _))) => {
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
        let at = crate::pragma::HELD
            .iter()
            .position(|keeps| keeps.name == name);
        let fallback = at
            .and_then(|at| crate::pragma::HELD.get(at))
            .map_or(0, |keeps| keeps.fallback);
        at.and_then(|at| self.kept.get(at).copied().flatten())
            .unwrap_or(fallback)
    }

    /// The functions the application defined on this connection, which
    /// a reader built over its file is told by [`Database::defining`].
    pub const fn groups(&mut self, grouped: &'static [crate::func::Grouped]) {
        self.grouped = grouped;
    }

    /// The functions the application defined on the connection, which a
    /// statement of this connection may call.
    pub const fn defines(&mut self, defined: &'static [crate::func::Defined]) {
        self.defined = defined;
    }

    /// The function `sqlite3_set_authorizer` told this connection, which
    /// every statement is read against before it runs, and which a
    /// reader built over its file is told by [`Database::asked`].
    pub const fn asks(&mut self, asking: crate::auth::Asking) {
        self.asking = Some(asking);
    }

    /// The same, with the connection told no function, which is
    /// `sqlite3_set_authorizer` with a null pointer.
    pub const fn asks_nothing(&mut self) {
        self.asking = None;
    }

    /// The function `sqlite3_commit_hook` told this connection, which
    /// the connection asks where a transaction that wrote a page ends.
    pub const fn commits(&mut self, committing: Committing) {
        self.committing = Some(committing);
    }

    /// The same, with the connection told no function, which is
    /// `sqlite3_commit_hook` with a null pointer.
    pub const fn commits_nothing(&mut self) {
        self.committing = None;
    }

    /// The function `sqlite3_rollback_hook` told this connection, which
    /// the connection tells where a transaction goes back.
    pub const fn rolls_back(&mut self, rolling: Rolling) {
        self.rolling = Some(rolling);
    }

    /// The same, with the connection told no function, which is
    /// `sqlite3_rollback_hook` with a null pointer.
    pub const fn rolls_back_nothing(&mut self) {
        self.rolling = None;
    }

    /// The function `sqlite3_update_hook` told this connection, which the
    /// connection tells of every row a statement writes.
    pub const fn writes_rows(&mut self, writing: Writing) {
        self.writes = Some(writing);
    }

    /// The same, with the connection told no function, which is
    /// `sqlite3_update_hook` with a null pointer.
    pub const fn writes_nothing(&mut self) {
        self.writes = None;
    }

    /// The function `sqlite3_preupdate_hook` told this connection, which
    /// the connection tells before it writes a row.
    pub const fn peeks(&mut self, peeking: Peeking) {
        self.peeking = Some(peeking);
    }

    /// The same, with the connection told no function, which is
    /// `sqlite3_preupdate_hook` with a null pointer.
    pub const fn peeks_nothing(&mut self) {
        self.peeking = None;
    }

    /// The database at `at` of the list becomes the one the connection
    /// writes, and the one it wrote takes its place in the list.
    ///
    /// Calling this again with the same place puts both back, which is
    /// what every statement does once it has run.
    ///
    /// The swap moves two files and costs O(1).
    fn switch(&mut self, at: usize) {
        let mut attached = core::mem::take(&mut self.attached);
        for held in attached.iter_mut().skip(at).take(1) {
            core::mem::swap(&mut self.held, &mut held.held);
            core::mem::swap(&mut self.called, &mut held.called);
        }
        self.attached = attached;
    }

    /// Which database of the list a schema names, and nothing where it
    /// names the one the connection already writes.
    ///
    /// `main` names the one the connection writes. `temp` names the
    /// database of its own that A6 of document 18 gave the temp schema,
    /// which the caller made before this reads the list.
    ///
    /// Reading the names costs O(n) in their number.
    ///
    /// # Errors
    ///
    /// The name the connection holds no database under, which the caller
    /// writes its own refusal for.
    fn switched(&self, schema: Option<Span>, sql: &[u8]) -> Result<Option<usize>, Vec<u8>> {
        let Some(span) = schema else {
            return Ok(None);
        };
        let named = crate::schema::dequote(span.text(sql));
        if named.eq_ignore_ascii_case(b"main") {
            return Ok(None);
        }
        let found = self.attached.iter().position(|held| named_as(held, &named));
        found.map(Some).ok_or(named)
    }

    /// The function `ATTACH` answers a file name with.
    ///
    /// This crate reads no file system, so a connection that is told no
    /// function refuses every `ATTACH` of a file name `unable to open
    /// database: <file>`.
    pub const fn opens(&mut self, opening: Opening) {
        self.opening = Some(opening);
    }

    /// Raises where `INDEXED BY name` names an index the table does not
    /// hold, which `sqlite3IndexedByLookup` of
    /// `research/sqlite/src/build.c:4600` refuses.
    ///
    /// Reading the indexes of the table costs O(n) in their number.
    ///
    /// # Errors
    ///
    /// [`Error::NoObject`] names the index.
    fn holds_index(
        &self,
        table: &[u8],
        indexed: crate::ast::Indexed,
        sql: &[u8],
    ) -> Result<(), Error> {
        let crate::ast::Indexed::By(span) = indexed else {
            return Ok(());
        };
        let name = crate::schema::dequote(span.text(sql));
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?;
        if database
            .indexes(table)
            .iter()
            .any(|kept| kept.index.name.eq_ignore_ascii_case(&name))
        {
            return Ok(());
        }
        Err(Error::NoObject(b"index".to_vec(), name))
    }

    /// The rows one pragma of the schema answers: the columns of a table,
    /// the places of an index, the indexes of a table, or the collations
    /// the connection holds.
    ///
    /// Nothing where the pragma is another. Reading the schema costs O(n)
    /// in its rows.
    ///
    /// # Errors
    ///
    /// [`Error`] names what the image breaks.
    fn schema_rows(
        &self,
        setting: crate::pragma::Setting,
        asked: &crate::ast::Pragma,
        sql: &[u8],
    ) -> Result<Option<Vec<Vec<Value>>>, Error> {
        use crate::pragma::Setting;
        if !matches!(
            setting,
            Setting::TableInfo
                | Setting::TableXinfo
                | Setting::IndexInfo
                | Setting::IndexXinfo
                | Setting::IndexList
                | Setting::CollationList
        ) {
            return Ok(None);
        }
        if setting == Setting::CollationList {
            return Ok(Some(listed_collations(self.collating)));
        }
        let named = asked
            .value
            .map(|value| crate::schema::dequote(value.text(sql)))
            .unwrap_or_default();
        // A bare name is the table of any database the connection holds,
        // which `sqlite3FindTable` reads them in turn for.
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?;
        Ok(Some(match setting {
            Setting::TableInfo => columns_of(&database, &named, false),
            Setting::TableXinfo => columns_of(&database, &named, true),
            Setting::IndexInfo => places_of(&database, &named, false),
            Setting::IndexXinfo => places_of(&database, &named, true),
            _ => listed_indexes(&database, &named),
        }))
    }

    /// The databases the connection holds, each as a row of the schema
    /// place, the name and the file name, which `PRAGMA database_list`
    /// answers and `PragTyp_DATABASE_LIST` of
    /// `research/sqlite/src/pragma.c:1436` writes.
    ///
    /// Reading them costs O(n) in their number.
    fn listed_databases(&self) -> Vec<Vec<Value>> {
        let row = |place: i64, name: &[u8], file: &[u8]| {
            alloc::vec![
                Value::Int(place),
                Value::Text(name.to_vec()),
                Value::Text(file.to_vec()),
            ]
        };
        let mut out = alloc::vec![row(0, b"main", b"")];
        for held in self.in_place() {
            let place = i64::try_from(held.called.place).unwrap_or(0);
            out.push(row(place, &held.called.name, &held.called.file));
        }
        out
    }

    /// `PRAGMA lock_status`: the lock each database of the connection is
    /// held under, which `PragTyp_LOCK_STATUS` of
    /// `research/sqlite/src/pragma.c:1877` answers out of the pager of
    /// each.
    ///
    /// This engine holds no file and takes no lock, so every database it
    /// holds answers `unlocked`; the temp schema answers `closed` until
    /// a statement opens it. Reading them costs O(n) in their number.
    fn locks_held(&self) -> Vec<Vec<Value>> {
        let row = |name: &[u8], held: &[u8]| {
            alloc::vec![Value::Text(name.to_vec()), Value::Text(held.to_vec())]
        };
        let mut out = alloc::vec![row(b"main", b"unlocked")];
        let temp = self.attached.iter().any(|held| named_as(held, b"temp"));
        out.push(row(b"temp", if temp { b"unlocked" } else { b"closed" }));
        for held in self.in_place() {
            if named_as(held, b"temp") {
                continue;
            }
            out.push(row(&held.called.name, b"unlocked"));
        }
        out
    }

    /// Whether this connection has a transaction open, which
    /// `sqlite3_get_autocommit` answers nought for.
    #[must_use]
    pub const fn began(&self) -> bool {
        self.held.began.is_some()
    }

    /// The function this connection was told, which a caller that opens
    /// a [`Database`] of its own passes to [`Database::asked`].
    #[must_use]
    pub const fn asking(&self) -> Option<crate::auth::Asking> {
        self.asking
    }

    /// The collations the application defined on this connection, which
    /// a reader built over its file is told by
    /// [`Database::open_collating`].
    ///
    /// The schema is read against them, so a connection is told of them
    /// before it reads a table that names one.
    pub const fn collates(&mut self, collating: &'static [crate::value::Collating]) {
        self.collating = collating;
    }

    /// The word the journal mode of this connection is written as,
    /// which is `wal` for a file in write-ahead logging and one of the
    /// five modes that write the file itself otherwise.
    #[must_use]
    pub const fn journalled(&self) -> &'static [u8] {
        // A file whose header says version two is in write-ahead
        // logging whether a log lies beside it or not, which
        // `sqlite3PagerOpenWal` reads off the file a connection opens.
        if self.held.log.is_some() || self.held.header.write_version == 2 {
            return b"wal";
        }
        crate::pragma::mode_word(self.held.mode)
    }

    /// What this connection was told for the pragmas it keeps a value
    /// for, which a caller that holds one writer per file and several
    /// connections over it reads before it changes connection.
    #[must_use]
    pub fn kept(&self) -> crate::pragma::Kept {
        crate::pragma::Kept(self.kept.clone())
    }

    /// The pragmas this connection stands at, which such a caller sets
    /// before each statement. A connection that was told nothing is set
    /// to the default, which is what opening one again does.
    pub fn kept_as(&mut self, kept: crate::pragma::Kept) {
        self.kept = kept.0;
        self.kept.resize(crate::pragma::HELD.len(), None);
    }

    /// How a statement of this connection names the columns it
    /// answers, which a reader built over this connection's file is
    /// told by [`Database::naming`].
    #[must_use]
    pub fn naming(&self) -> crate::db::Naming {
        crate::db::Naming {
            short: self.told(b"short_column_names") != 0,
            full: self.told(b"full_column_names") != 0,
        }
    }

    /// Whether every foreign key of a row points at a row that is
    /// there, which is `I.1` of `src/fkey.c`.
    ///
    /// Looking one key up reads the rows of the table it points at, so
    /// a statement that writes n rows into a table with a foreign key
    /// over a table of m rows costs O(n·m).
    fn parented(&mut self, table: &Table, values: &[Value], rowid: i64) -> Result<(), Error> {
        if !self.holding() {
            return Ok(());
        }
        self.counted_child(table, values, rowid, 1)?;
        self.rescued(table, values, rowid)
    }

    /// Whether a foreign key is held where the transaction ends rather
    /// than where a row is written, which is `DEFERRABLE INITIALLY
    /// DEFERRED` and `PRAGMA defer_foreign_keys`.
    fn deferring(&self, key: &crate::schema::Foreign) -> bool {
        key.deferred || self.told(b"defer_foreign_keys") != 0
    }

    /// The rows that point at a row the statement wrote, counted one
    /// fewer each, which is `fkScanChildren` over the row that
    /// appeared: a row is the parent the rows waiting for it were
    /// counted against.
    ///
    /// Reading the rows that point costs O(m) in the rows of each table
    /// that points at this one.
    fn rescued(&mut self, table: &Table, values: &[Value], rowid: i64) -> Result<(), Error> {
        // Nothing waits for a parent where no key is counted, so no
        // table needs reading.
        if self.deferred == 0 {
            return Ok(());
        }
        for points in self.pointing(&table.name)? {
            if !self.deferring(&points.key) {
                continue;
            }
            let places = {
                let bytes = self.image();
                let database = self.reading(&bytes)?;
                // The table is the one the statement wrote, so it is
                // there wherever this is reached.
                let (parent, _) = database
                    .table(&table.name)
                    .ok_or(Error::NoTable(Vec::new()))?;
                parent_places(&database, (&points.child, &points.key), parent)?
            };
            let wanted: Vec<Value> = places
                .iter()
                .map(|at| at_place(table, values, rowid, *at))
                .collect();
            if wanted.contains(&Value::Null) {
                continue;
            }
            let rows = self.pointing_rows(&points, &wanted)?;
            self.deferred = self.deferred.saturating_sub(counted(rows.len()));
        }
        Ok(())
    }

    /// The foreign keys of a row that goes, counted one fewer each
    /// where the key is held at the end of the transaction and the row
    /// pointed at no row, which is `sqlite3FkCheck` over the old row.
    ///
    /// # Errors
    ///
    /// Whatever reading the tables the keys point at refuses.
    fn unparented(&mut self, table: &Table, values: &[Value], rowid: i64) -> Result<(), Error> {
        if !self.holding() {
            return Ok(());
        }
        self.counted_child(table, values, rowid, -1)
    }

    /// The foreign keys of one row counted `by` each where the key is
    /// held at the end of the transaction and the row points at no row,
    /// which is `sqlite3FkCheck` over the row's own keys.
    ///
    /// # Errors
    ///
    /// [`Error::Foreign`] where a key held at once points at no row.
    fn counted_child(
        &mut self,
        table: &Table,
        values: &[Value],
        rowid: i64,
        by: i64,
    ) -> Result<(), Error> {
        if table.foreign.is_empty() {
            return Ok(());
        }
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        for key in &table.foreign {
            // The statement was held to keys that point at a table
            // that is there before a row was read, so the name is one
            // the schema holds and the refusal is what this reads it
            // with. `sqlite3FkLocateIndex` names the schema the table
            // would stand in, which is `main` for every table this
            // crate holds.
            let (parent, _) = database
                .table(&key.table)
                .ok_or(Error::NoTable(schema_named_as(&key.table)))?;
            let places = parent_places(&database, (&table.name, key), parent)?;
            let mut wanted = Vec::new();
            for at in &key.columns {
                wanted.push(at_place(table, values, rowid, *at));
            }
            // A row that holds nothing in a column of the key points at
            // no row, which is `R-...`: such a row is held to nothing.
            if wanted.contains(&Value::Null) {
                continue;
            }
            if found_parent(&database, &key.table, parent, &places, &wanted)? {
                continue;
            }
            if !self.deferring(key) {
                if by > 0 {
                    return Err(Error::Foreign);
                }
                continue;
            }
            self.deferred = self.deferred.saturating_add(by);
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
                let database = self.reading(&bytes)?;
                // The table is the one the statement changes, so it is
                // there wherever this is reached.
                let (parent, _) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
                parent_places(&database, (&points.child, &points.key), parent)?
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

    /// Every foreign key the statement reads, located, which is
    /// `sqlite3FkLocateIndex` running where the statement is read: a
    /// key that points at a table that is not there, or at columns that
    /// are no key of it, is refused whatever rows the statement reaches
    /// and whether it reaches any.
    ///
    /// It reads the keys of the table and the keys that point at it,
    /// which is O(keys) per statement.
    ///
    /// # Errors
    ///
    /// [`Error::NoTable`] for a key that points at a table that is not
    /// there, and [`Error::ForeignMismatch`] for one that points at
    /// columns that are no key of it.
    fn located(&self, name: &[u8]) -> Result<(), Error> {
        if !self.holding() {
            return Ok(());
        }
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let Some((table, _)) = database.table(name) else {
            return Ok(());
        };
        for key in &table.foreign {
            let (parent, _) = database
                .table(&key.table)
                .ok_or_else(|| Error::NoTable(schema_named_as(&key.table)))?;
            parent_places(&database, (&table.name, key), parent)?;
        }
        self.pointing(name)?;
        Ok(())
    }

    /// Every foreign key of every table that points at `name`.
    fn pointing(&self, name: &[u8]) -> Result<Vec<Points>, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        // The table is the one the statement changes, so it is there
        // wherever this is reached.
        let (parent, _) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
        let mut out = Vec::new();
        for table in database.tables() {
            for key in &table.foreign {
                if !key.table.eq_ignore_ascii_case(name) {
                    continue;
                }
                // `R-04240-13860`: the affinity and the collation of the
                // parent's column decide, so a child column written
                // under another collation is compared under the
                // parent's.
                let under = parent_places(&database, (&table.name, key), parent)?
                    .iter()
                    .map(|at| {
                        parent
                            .columns
                            .get(*at)
                            .map_or((Affinity::None, Collation::Binary), |column| {
                                (column.affinity, column.collation)
                            })
                    })
                    .collect();
                out.push(Points {
                    child: table.name.clone(),
                    key: key.clone(),
                    under,
                });
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
                // `sqlite3FkActions` writes the rows that point through
                // a trigger of its own, so the depth a trigger's body
                // reaches bounds the actions a chain of keys reaches.
                let held = self.deepened()?;
                for (key, values) in rows {
                    match after {
                        None => self.taken_away(&points.child, &key)?,
                        Some(after) => self.written_over(points, &key, &values, after)?,
                    }
                }
                self.running.truncate(held);
                Ok(())
            }
            crate::ast::Action::SetNull | crate::ast::Action::SetDefault => {
                let fallback = matches!(action, crate::ast::Action::SetDefault);
                let held = self.deepened()?;
                for (key, values) in rows {
                    self.written_back(points, &key, &values, fallback)?;
                }
                self.running.truncate(held);
                Ok(())
            }
            // `RESTRICT` is held where the row is written whatever the
            // key says, which is what `sqlite3FkCheck` leaves it as.
            crate::ast::Action::Restrict => Err(Error::Foreign),
            _ => {
                if !self.deferring(&points.key) {
                    return Err(Error::Foreign);
                }
                self.deferred = self.deferred.saturating_add(counted(rows.len()));
                Ok(())
            }
        }
    }

    /// One step deeper, which is what a foreign key action takes before
    /// it writes the rows that point, and how deep the writer stood
    /// before that step.
    ///
    /// The name pushed carries a byte no identifier holds, so no
    /// trigger of the schema is taken for it. Costs O(1).
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] where the chain of keys reaches deeper
    /// than a trigger's body may.
    fn deepened(&mut self) -> Result<usize, Error> {
        if self.running.len() >= TRIGGER_DEPTH {
            return Err(Error::Unsupported);
        }
        let held = self.running.len();
        self.running.push(b"\0foreign key".to_vec());
        Ok(held)
    }

    /// The rows of the table that points whose key is `wanted`.
    fn pointing_rows(
        &self,
        points: &Points,
        wanted: &[Value],
    ) -> Result<Vec<crate::db::Reading>, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let (child, _) = database
            .table(&points.child)
            .ok_or(Error::NoTable(Vec::new()))?;
        let mut out = Vec::new();
        for (key, values) in database.held_rows_of(&points.child)? {
            let held: Vec<Value> = points
                .key
                .columns
                .iter()
                .map(|at| at_place(child, &values, keyed_rowid(&key), *at))
                .collect();
            if held.contains(&Value::Null) {
                continue;
            }
            if alike_values(&held, wanted, &points.under) {
                out.push((key, values));
            }
        }
        Ok(out)
    }

    /// One row of the table that points, taken away with the row it
    /// pointed at, which is `ON DELETE CASCADE`. The rows that point at
    /// that row go with it.
    fn taken_away(&mut self, name: &[u8], key: &[Value]) -> Result<(), Error> {
        let (root, kept, table, values) = self.one_row(name, key)?;
        self.unparented(&table, &values, keyed_rowid(key))?;
        self.orphaned(name, &table, &values, keyed_rowid(key), None)?;
        self.unindex_row(&kept, &table, &values, key)?;
        if table.without_rowid {
            let collations = crate::schema::key_collations(&table);
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::remove_entry(&mut self.held.pages, root, key, order)?;
            self.counted.total = self.counted.total.saturating_add(1);
            return Ok(());
        }
        crate::tree::remove(&mut self.held.pages, root, keyed_rowid(key))?;
        self.counted.total = self.counted.total.saturating_add(1);
        Ok(())
    }

    /// One row of the table that points, with the key of the row it
    /// points at written into it, which is `ON UPDATE CASCADE`.
    fn written_over(
        &mut self,
        points: &Points,
        key: &[Value],
        held: &[Value],
        after: &[Value],
    ) -> Result<(), Error> {
        let mut values = held.to_vec();
        for (at, value) in points.key.columns.iter().zip(after) {
            for slot in values.iter_mut().skip(*at).take(1) {
                *slot = value.clone();
            }
        }
        self.rewrite(&points.child, key, &values)
    }

    /// One row of the table that points, with nothing or its fallback
    /// written into the columns that point, which is `ON DELETE SET
    /// NULL` and `ON DELETE SET DEFAULT`.
    fn written_back(
        &mut self,
        points: &Points,
        key: &[Value],
        held: &[Value],
        fallback: bool,
    ) -> Result<(), Error> {
        // The fallback of a column is the expression the statement that
        // made the table wrote, which the schema holds beside the arena
        // of that statement.
        let falls_back = if fallback {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
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
        self.rewrite(&points.child, key, &values)
    }

    /// One row of a table, written again with the values given.
    ///
    /// `sqlite3FkActions` writes the row through a trigger, so the row
    /// is held to the constraints of its table and the rows that point
    /// at it are acted on in turn, which is what carries a chain of
    /// keys past the first of them.
    fn rewrite(&mut self, name: &[u8], key: &[Value], values: &[Value]) -> Result<(), Error> {
        let (_, _, table, old) = self.one_row(name, key)?;
        let rowid = keyed_rowid(key);
        let mut named = values.to_vec();
        // `Conflict::Abort` refuses a row rather than passing it over,
        // so the row is held or the action raises.
        self.constrained(&table, &mut named, rowid, Conflict::Abort)?;
        // The row the action writes points at the row that changed,
        // which is written after the action runs, so the key it points
        // through is held by construction and not read again here.
        self.orphaned(name, &table, &old, rowid, Some((&named, rowid)))?;
        // The chain the action carried may have written this row as
        // well, so the row it stands at now is read again.
        let (root, kept, table, held) = self.one_row(name, key)?;
        let values = named.as_slice();
        self.unindex_row(&kept, &table, &held, key)?;
        self.index_row(&kept, &table, values, key)?;
        if table.without_rowid {
            // A table that keeps its rows in the key's own tree holds
            // the columns of the key in the record, and no action
            // writes a column of that key, because such a column
            // refuses nothing.
            let affinities = ordered_affinities(&table);
            let stored = ordered(&table, values);
            let record =
                crate::record::write_in(&stored, &affinities, 4, self.held.header.encoding);
            let collations = crate::schema::key_collations(&table);
            // The entry the row stands under is written again in place,
            // which is one taken out and one put back.
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::remove_entry(&mut self.held.pages, root, key, order)?;
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::insert_entry(&mut self.held.pages, root, &record, key, order, false)?;
            self.counted.total = self.counted.total.saturating_add(1);
            return Ok(());
        }
        let affinities = ordered_affinities(&table);
        let mut held = values.to_vec();
        // The column the key is another name for takes no place in the
        // record, which is what `sqlite3TableColumnToStorage` leaves.
        for slot in held
            .iter_mut()
            .skip(table.rowid_alias.unwrap_or(usize::MAX))
            .take(1)
        {
            *slot = Value::Null;
        }
        let record = crate::record::write_in(
            &ordered(&table, &held),
            &affinities,
            4,
            self.held.header.encoding,
        );
        crate::tree::update(&mut self.held.pages, root, keyed_rowid(key), &record)?;
        // `sqlite3_total_changes` counts the rows a foreign key action
        // writes, which `sqlite3FkActions` writes through a trigger of
        // its own.
        self.counted.total = self.counted.total.saturating_add(1);
        Ok(())
    }

    /// The root, the indexes, the table and the values of one row.
    fn one_row(
        &self,
        name: &[u8],
        key: &[Value],
    ) -> Result<(u32, Vec<Kept>, Table, Vec<Value>), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let (table, root) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
        let kept = kept_indexes(&database, name);
        let values = database
            .held_rows_of(name)?
            .into_iter()
            .find(|(held, _)| held == key)
            .map(|(_, values)| values)
            .ok_or(Error::NoTable(Vec::new()))?;
        Ok((root, kept, table.clone(), values))
    }

    /// Whether a trigger of this name is running already and may not
    /// run again, which `PRAGMA recursive_triggers` turns off.
    fn repeats(&self, name: &[u8]) -> bool {
        !self.recursive()
            && self
                .running
                .iter()
                .any(|held| held.eq_ignore_ascii_case(name))
    }

    /// The triggers a row written over by a `REPLACE` runs, which are
    /// the triggers of a `DELETE` where `PRAGMA recursive_triggers` is
    /// on and none where it is off, because the row is written out from
    /// inside an `INSERT`.
    fn deleting(
        &self,
        name: &[u8],
    ) -> Result<(Vec<crate::db::Trigger>, Vec<crate::db::Trigger>), Error> {
        if !self.recursive() {
            return Ok((Vec::new(), Vec::new()));
        }
        Ok((
            self.triggers_for(name, TriggerEvent::Delete, TriggerTime::Before)?,
            self.triggers_for(name, TriggerEvent::Delete, TriggerTime::After)?,
        ))
    }

    /// Whether a trigger runs from inside another statement of a
    /// trigger, which `PRAGMA recursive_triggers` turns on and
    /// `SQLITE_RecTriggers` reads.
    fn recursive(&self) -> bool {
        crate::pragma::HELD
            .iter()
            .position(|keeps| keeps.name == b"recursive_triggers")
            .and_then(|at| self.kept.get(at).copied().flatten())
            .unwrap_or(0)
            != 0
    }

    /// Runs the statements of one trigger's body.
    ///
    /// A trigger of the temp schema is fixed to no database, which
    /// `sqlite3FixInit` of `research/sqlite/src/attach.c:547` leaves
    /// alone where `bTemp` is one, so a statement of its body names its
    /// table the way a statement outside a trigger does: the temp
    /// schema first and then the databases in turn. Every other trigger
    /// writes the database it stands in.
    fn body(&mut self, trigger: &crate::db::Trigger, row: &Fired<'_>) -> Result<(), Error> {
        let arena = &trigger.arena;
        let sql = &trigger.sql;
        let loose = self.called.name.eq_ignore_ascii_case(b"temp");
        for step in arena.steps(trigger.written.body) {
            let at = if loose {
                self.stepping_at(step, sql)?
            } else {
                None
            };
            if let Some(held) = at {
                self.switch(held);
            }
            let answered = self.stepped(step, arena, sql, row);
            if let Some(held) = at {
                self.switch(held);
            }
            answered?;
        }
        Ok(())
    }

    /// Which database of the list one statement of a trigger's body
    /// writes, and nothing where it writes the one the connection
    /// already writes.
    ///
    /// A statement of a body writes no schema in front of its table,
    /// which `sqlite3TriggerInsertStep` refuses, so the name is read
    /// the way a bare name is read anywhere.
    ///
    /// # Errors
    ///
    /// Whatever reading the schemas refuses.
    fn stepping_at(
        &self,
        step: &crate::ast::TriggerStep,
        sql: &[u8],
    ) -> Result<Option<usize>, Error> {
        let named = match *step {
            crate::ast::TriggerStep::Insert(ref statement) => Some(statement.name),
            crate::ast::TriggerStep::Update(ref statement) => Some(statement.name),
            crate::ast::TriggerStep::Delete(ref statement) => Some(statement.name),
            // A statement that answers rows names its tables in the
            // reader, which reads every database of the connection.
            crate::ast::TriggerStep::Select(_) => None,
        };
        self.holding_at(named, sql)
    }

    /// Runs one statement of a trigger's body against the database the
    /// connection writes.
    fn stepped(
        &mut self,
        step: &crate::ast::TriggerStep,
        arena: &Arena,
        sql: &[u8],
        row: &Fired<'_>,
    ) -> Result<(), Error> {
        match *step {
            crate::ast::TriggerStep::Insert(statement) => {
                let changed = self.insert(arena, &statement, sql, Some(row))?;
                self.counts_step(changed);
            }
            crate::ast::TriggerStep::Update(statement) => {
                let changed = self.update(arena, &statement, sql, Some(row))?;
                self.counts_step(changed);
            }
            crate::ast::TriggerStep::Delete(statement) => {
                let changed = self.delete(arena, &statement, sql, Some(row))?;
                self.counts_step(changed);
            }
            // A statement that answers rows runs for what it reads
            // and answers nothing, which is what a `SELECT` of a
            // body is for: it carries the `RAISE`.
            crate::ast::TriggerStep::Select(select) => {
                let bytes = self.images();
                let database = self
                    .reading_beside(&bytes)?
                    .seeded(self.random.word())
                    .defining(self.defined)
                    .grouping(self.grouped)
                    .counting(self.counted);
                database.rows_under(arena, select, sql, Some(row))?;
            }
        }
        Ok(())
    }

    /// What one statement of a trigger's body leaves the counters at:
    /// every statement that changes rows sets `changes()` to its own
    /// count, and the statement that fired the trigger sets it again
    /// when it ends.
    const fn counts_step(&mut self, changed: i64) {
        self.counted.changes = changed;
        self.counted.total = self.counted.total.saturating_add(changed);
    }

    /// The record of five noughts `sqlite3StartTable` writes into
    /// `sqlite_schema` first, and the key it is written under.
    fn schema_blank(&mut self) -> Result<i64, Error> {
        let rowid = largest(&self.held.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        let blanks = alloc::vec![Value::Null; SCHEMA.len()];
        let blank = crate::record::write_in(&blanks, &SCHEMA, 4, self.held.header.encoding);
        let pages = &mut self.held.pages;
        insert(pages, crate::image::SCHEMA_ROOT, rowid, &blank)?;
        Ok(rowid)
    }

    /// One row of `sqlite_schema` written again over the row that
    /// carries the same key.
    ///
    /// Writing one row costs O(log n) in the rows of the schema.
    fn schema_row(&mut self, rowid: i64, values: &[Value]) -> Result<(), Error> {
        let record = crate::record::write_in(values, &SCHEMA, 4, self.held.header.encoding);
        let pages = &mut self.held.pages;
        crate::tree::update(pages, crate::image::SCHEMA_ROOT, rowid, &record)?;
        Ok(())
    }

    /// The row `sqlite3EndTable` writes over the blank record, which
    /// raises the schema cookie.
    fn schema_written(&mut self, rowid: i64, row: &[u8]) -> Result<(), Error> {
        crate::tree::update(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid, row)?;
        self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
        self.held.header.schema_format = 4;
        Ok(())
    }

    /// The page a tree of `kind` begins on, and nought for a view,
    /// which `sqlite3EndTable` writes as the root of a thing that
    /// begins on no page.
    fn rooted(&mut self, kind: Option<Kind>) -> Result<u32, Error> {
        let Some(kind) = kind else {
            return Ok(0);
        };
        // `sqlite3BtreeCreateTable`: the root of a tree is named by no
        // page, and page one holds the largest root the file has. A file
        // that keeps pointer maps holds its roots from page three up with
        // no gap, so the root takes the page after the largest one.
        if self.held.pages.vacuuming() {
            let root = self
                .held
                .pages
                .root_after(self.held.header.largest_root, kind)?;
            self.held.header.largest_root = root;
            return Ok(root);
        }
        let root = self.held.pages.add(kind, 0)?;
        self.held.pages.point(root, crate::tree::Point::Root, 0)?;
        Ok(root)
    }

    /// `CREATE TABLE`: a page for the tree of the table and a row of
    /// `sqlite_schema` that names it.
    fn define(&mut self, arena: &Arena, definition: Definition, sql: &[u8]) -> Result<(), Error> {
        let (kind, name, over, already, written, written_name) = match definition {
            Definition::Drop(asked) => return self.drop_object(&asked, sql),
            Definition::AddColumn(asked) => return self.add_column(arena, &asked, sql),
            Definition::Rename(asked) => return self.rename_table(&asked, sql),
            Definition::DropColumn(asked) => return self.drop_column(&asked, sql),
            Definition::RenameColumn(asked) => return self.rename_column(&asked, sql),
            Definition::DropConstraint(asked) => {
                return self.drop_constraint(arena, &asked, sql);
            }
            Definition::Trigger(trigger) => return self.create_trigger(arena, &trigger, sql),
            Definition::Vacuum(asked) => return self.vacuum(&asked, sql),
            Definition::Attach(asked) => return self.attach(arena, asked, sql),
            Definition::Detach(asked) => return self.detach(arena, asked, sql),
            Definition::Table(table) => {
                if let TableBody::Select(select) = table.body {
                    return self.create_as(arena, &table, select, sql);
                }
                let name = crate::schema::dequote(table.name.text(sql));
                (
                    Some(table_kind(table.options.without_rowid)),
                    name.clone(),
                    name,
                    table.if_not_exists,
                    written_statement(b"CREATE TABLE ", table.name, sql),
                    table.name.text(sql),
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
                index.name.text(sql),
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
                    view.name.text(sql),
                )
            }
        };
        if let Definition::Index(index) = definition {
            self.indexable(&over)?;
            windowless(arena, &index, sql)?;
        }
        if self.may_name((&name, written_name), already, &definition)? {
            return Ok(());
        }
        let root = self.rooted(kind)?;
        let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                text(match definition {
                    Definition::Index(_) => b"index".as_slice(),
                    Definition::View(_) => b"view",
                    _ => b"table",
                }),
                text(&name),
                text(&over),
                Value::Int(i64::from(root)),
                text(&written),
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        // `sqlite3StartTable` writes a record of five noughts and
        // `sqlite3EndTable` writes over it, so the page keeps the bytes
        // of the blank record where the row no longer stands.
        // `sqlite3CreateIndex` writes its row once and has no blank.
        if let Definition::Index(index) = definition {
            let rowid = largest(&self.held.pages, crate::image::SCHEMA_ROOT)?
                .unwrap_or(0)
                .saturating_add(1);
            insert(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
            self.held.header.schema_cookie = self.held.header.schema_cookie.saturating_add(1);
            self.held.header.schema_format = 4;
            return self.fill(arena, &index, sql, root, &over);
        }
        let rowid = self.schema_blank()?;
        let counts = self.own_indexes(arena, definition, sql)?;
        self.schema_written(rowid, &row)?;
        // `sqlite3StartTable` makes `sqlite_sequence` with the first
        // table that counts its keys up, and the row of that table is
        // written before it.
        if counts && !self.holds(SEQUENCE)? {
            // The two rows are one change of the schema, so the cookie
            // the statement raised already counts for both.
            let cookie = self.held.header.schema_cookie;
            self.own_table(b"CREATE TABLE sqlite_sequence(name,seq)")?;
            self.held.header.schema_cookie = cookie;
        }
        Ok(())
    }

    /// One table this crate writes for itself, which is a name SQLite
    /// keeps and a statement no connection wrote.
    ///
    /// # Errors
    ///
    /// Whatever making the table refuses.
    fn own_table(&mut self, sql: &[u8]) -> Result<(), Error> {
        // A `VACUUM` writes the tables of the schema with this flag
        // already set, and one of them may make a table of its own, so
        // the flag is left as it was found.
        let held = self.making_own;
        self.making_own = true;
        let made = self.ran(sql);
        self.making_own = held;
        made.map(|_| ())
    }

    /// Whether the database holds the table `name`.
    fn holds(&self, name: &[u8]) -> Result<bool, Error> {
        let bytes = self.image();
        Ok(self.reading(&bytes)?.table(name).is_some())
    }

    /// Whether a `CREATE` of `name` writes nothing, because the schema
    /// already holds the name and the statement wrote `IF NOT EXISTS`.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn already(
        &self,
        held: (&[u8], &[u8]),
        if_not_exists: bool,
        making: Making,
    ) -> Result<bool, Error> {
        let (name, written) = held;
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let held: Option<&[u8]> = if database.table(name).is_some() {
            Some(b"table")
        } else if database.view(name).is_some() {
            Some(b"view")
        } else if database.index(name).is_some() {
            Some(b"index")
        } else {
            None
        };
        let Some(held) = held else {
            return Ok(false);
        };
        if if_not_exists {
            return Ok(true);
        }
        // `sqlite3StartTable` and `sqlite3CreateIndex`: a name the same
        // kind holds is one that already exists, and a name the other
        // kind holds is one that is already named.
        let same = match making {
            Making::Index => held == b"index",
            Making::Table => held != b"index",
        };
        if same {
            // `sqlite3StartTable` writes the name as the statement
            // wrote it, which `%T` of the token answers with its
            // quotes, and `sqlite3CreateIndex` writes the name with its
            // quotes taken off.
            let shown = match making {
                Making::Index => name,
                Making::Table => written,
            };
            return Err(Error::Exists(held.to_vec(), shown.to_vec()));
        }
        let kind = if making == Making::Index {
            b"table"
        } else {
            b"index"
        };
        Err(Error::AlreadyNamed(kind.to_vec(), name.to_vec()))
    }

    /// Whether the schema may hold the name a `CREATE` writes, and
    /// whether it holds it already under an `IF NOT EXISTS`, which is
    /// `sqlite3StartTable` and `sqlite3CreateIndex`.
    ///
    /// Reading the schema costs O(n) in its rows.
    ///
    /// # Errors
    ///
    /// [`Error::Reserved`] for a name SQLite keeps for itself, and
    /// whatever [`Writer::already`] refuses.
    fn may_name(
        &self,
        held: (&[u8], &[u8]),
        if_not_exists: bool,
        definition: &Definition,
    ) -> Result<bool, Error> {
        let (name, _) = held;
        let making = if matches!(definition, Definition::Index(_)) {
            Making::Index
        } else {
            Making::Table
        };
        self.reserved(name)?;
        self.already(held, if_not_exists, making)
    }

    /// Whether a `CREATE INDEX` may be over this table, which is
    /// `sqlite3CreateIndex`: the table has to be there, a name SQLite
    /// keeps for itself may not be indexed, and a view holds no row of
    /// its own to hold an entry for.
    ///
    /// # Errors
    ///
    /// [`Error::IndexedView`], [`Error::NoTable`] and
    /// [`Error::NotIndexable`] name which of the three it is.
    fn indexable(&self, over: &[u8]) -> Result<(), Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        if database.view(over).is_some() {
            return Err(Error::IndexedView);
        }
        if database.table(over).is_none() {
            return Err(Error::NoTable(schema_named_as(over)));
        }
        if over
            .get(..7)
            .is_some_and(|head| head.eq_ignore_ascii_case(b"sqlite_"))
        {
            return Err(Error::NotIndexable(over.to_vec()));
        }
        Ok(())
    }

    /// Whether the name is one this crate may write, which is
    /// `sqlite3CheckObjectName`: a name that begins `sqlite_` is one
    /// SQLite keeps for itself, unless the connection set `PRAGMA
    /// writable_schema` or this crate writes the table for itself.
    ///
    /// # Errors
    ///
    /// [`Error::Reserved`] names the name it kept.
    fn reserved(&self, name: &[u8]) -> Result<(), Error> {
        if self.making_own
            || self.told(b"writable_schema") != 0
            || !name
                .get(..7)
                .is_some_and(|head| head.eq_ignore_ascii_case(b"sqlite_"))
        {
            return Ok(());
        }
        Err(Error::Reserved(name.to_vec()))
    }

    /// The indexes a `CREATE TABLE` carries of its own, written, and
    /// whether the table counts its keys up.
    ///
    /// A `PRIMARY KEY` and a `UNIQUE` each carry an index of the
    /// table's own, which the grammar makes as it reads the constraint,
    /// so its row stands before the row of the table is written over
    /// the blank one.
    ///
    /// # Errors
    ///
    /// Whatever reading the statement or writing an index refuses.
    fn own_indexes(
        &mut self,
        arena: &Arena,
        definition: Definition,
        sql: &[u8],
    ) -> Result<bool, Error> {
        let Definition::Table(written) = definition else {
            return Ok(false);
        };
        let table = crate::schema::table(arena, &written, sql, self.collating)?;
        for at in 0..table.keys.len() {
            self.own_index(&table, at)?;
        }
        Ok(table.autoincrement)
    }

    /// One index of a table's own, written: a page for its tree and a
    /// row of `sqlite_schema` that names it and holds no statement.
    ///
    /// Writing one row costs O(log n) in the rows of the schema.
    fn own_index(&mut self, table: &crate::schema::Table, at: usize) -> Result<(), Error> {
        // The `PRIMARY KEY` of a table that keeps its rows in the key's
        // own tree is that tree, so it is counted and not written.
        let Some(index) = crate::schema::own_index(table, at) else {
            return Ok(());
        };
        let root = self.held.pages.add(Kind::LeafIndex, 0)?;
        self.held.pages.point(root, crate::tree::Point::Root, 0)?;
        if self.held.header.largest_root != 0 {
            self.held.header.largest_root = root;
        }
        let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
        let row = crate::record::write_in(
            &[
                text(b"index"),
                text(&index.name),
                text(&table.name),
                Value::Int(i64::from(root)),
                Value::Null,
            ],
            &SCHEMA,
            4,
            self.held.header.encoding,
        );
        let rowid = largest(&self.held.pages, crate::image::SCHEMA_ROOT)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.held.pages, crate::image::SCHEMA_ROOT, rowid, &row)?;
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
            let database = self.reading(&bytes)?;
            // The statement was held to a table that is there before
            // the row was written, so the name is one the schema holds
            // and the refusal is what this reads it with.
            let (table, _) = database
                .table(over)
                .ok_or(Error::NoTable(schema_named_as(over)))?;
            let read = crate::schema::index(arena, index, sql, table, self.collating)?;
            let collations = collations_of(&read, self.held.header.schema_format);
            let over = Over {
                arena,
                sql,
                table,
                encoding: self.held.header.encoding,
            };
            let mut entries = Vec::new();
            for (key, values) in database.held_rows_of(&read.table)? {
                // A row read out of the file carries its text in UTF-8,
                // and an entry holds what the file holds, so the text
                // goes back into the encoding the file names.
                if !indexes_row(&read, &over, &values)? {
                    continue;
                }
                entries.push(entry_of(&read, &over, &values, &key)?);
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
        let bytes = self.images();
        // Every statement draws from where the connection stands, so
        // two statements of one connection answer `randomblob`
        // differently.
        let database = self
            .reading_beside(&bytes)?
            .seeded(self.random.word())
            .counting(self.counted);
        let (table, root) = database
            .table(name)
            .ok_or_else(|| Error::NoTable(name.to_vec()))?;
        let kept = kept_indexes(&database, name);
        let places = places(table, &named)?;
        // A column the statement names no value for holds what it falls
        // back to, which is nothing where it has no `DEFAULT`.
        let falls_back: Vec<Value> = database.defaults(name)?;
        let affinities = ordered_affinities(table);
        // `INSERT INTO t DEFAULT VALUES` writes one row of what every
        // column falls back to and reads no statement of its own.
        if statement.defaults {
            return Ok(Inserting {
                root,
                alias: table.rowid_alias,
                affinities,
                rows: alloc::vec![(Value::Null, falls_back)],
                kept,
                table: table.clone(),
            });
        }
        let answer = database.rows_under(arena, statement.select, sql, outer)?;
        let mut rows = Vec::new();
        for row in &answer.rows {
            // `sqlite3Insert` counts the values against the columns the
            // statement named, or against the columns of the table where
            // it named none.
            if row.len() != places.len() {
                if named.is_empty() {
                    return Err(Error::ColumnCount(
                        table.name.clone(),
                        places.len(),
                        row.len(),
                    ));
                }
                return Err(Error::ValueCount(row.len(), places.len()));
            }
            let mut values = falls_back.clone();
            let mut key = Value::Null;
            for (at, value) in places.iter().zip(row) {
                match at {
                    Some(at) => {
                        for slot in values.iter_mut().skip(*at).take(1) {
                            slot.clone_from(value);
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

    /// The `BEFORE` triggers of an `INSERT` run over the row as it
    /// stands before the key is given, which `sqlite3Insert` writes as
    /// nought less one where the statement named none.
    fn fired_early(
        &mut self,
        before: &[crate::db::Trigger],
        table: &Table,
        named: &[Value],
        alias: Option<usize>,
        given: Option<i64>,
    ) -> Result<bool, Error> {
        let shown = given.unwrap_or(-1);
        let mut early = named.to_vec();
        for slot in early.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
            *slot = Value::Int(shown);
        }
        let row = Fired {
            table,
            old: None,
            new: Some((&early, shown)),
            encoding: self.held.header.encoding,
        };
        self.fire(before, &[], &row)
    }

    /// Whether the schema holds a view of that name.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn is_view(&self, name: &[u8]) -> Result<bool, Error> {
        let bytes = self.image();
        Ok(self.reading(&bytes)?.view(name).is_some())
    }

    /// The `INSTEAD OF` triggers of a view for one event, which is what
    /// `sqlite3ViewIsEditable` holds a statement that writes a view to.
    ///
    /// # Errors
    ///
    /// [`Error::ViewWrite`] where the view carries no `INSTEAD OF`
    /// trigger of that event.
    fn instead_of(
        &self,
        name: &[u8],
        event: TriggerEvent,
    ) -> Result<Vec<crate::db::Trigger>, Error> {
        let triggers = self.triggers_for(name, event, TriggerTime::InsteadOf)?;
        if triggers.is_empty() {
            return Err(Error::ViewWrite(name.to_vec()));
        }
        Ok(triggers)
    }

    /// `INSERT INTO v` over a view: the row the statement writes is put
    /// in `new` and the `INSTEAD OF` triggers of the view run over it,
    /// the view itself keeping no row.
    ///
    /// The statement of the view is answered for its columns, so one
    /// row costs what that statement costs and what the triggers cost.
    ///
    /// # Errors
    ///
    /// [`Error::ViewWrite`] where the view carries no `INSTEAD OF
    /// INSERT` trigger, and whatever a statement of a body refuses.
    fn insert_view(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
        name: &[u8],
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let triggers = self.instead_of(name, TriggerEvent::Insert)?;
        let named: Vec<Vec<u8>> = arena
            .names(statement.columns)
            .iter()
            .map(|span: &Span| crate::schema::dequote(span.text(sql)))
            .collect();
        let (table, rows) = {
            let bytes = self.images();
            let database = self
                .reading_beside(&bytes)?
                .seeded(self.random.word())
                .counting(self.counted);
            let (table, _) = database.viewing(name)?;
            let places = places(&table, &named)?;
            let blank = alloc::vec![Value::Null; table.columns.len()];
            // A view holds no `DEFAULT`, so `DEFAULT VALUES` writes one
            // row of nothing.
            let mut rows = alloc::vec![blank.clone()];
            if !statement.defaults {
                let answer = database.rows_under(arena, statement.select, sql, outer)?;
                rows.clear();
                for row in &answer.rows {
                    if row.len() != places.len() {
                        if named.is_empty() {
                            return Err(Error::ColumnCount(table.name, places.len(), row.len()));
                        }
                        return Err(Error::ValueCount(row.len(), places.len()));
                    }
                    let mut values = blank.clone();
                    // A value the statement wrote for the key of the
                    // view goes nowhere, because a view has no key.
                    for (at, value) in places.iter().zip(row) {
                        for slot in values.iter_mut().skip(at.unwrap_or(usize::MAX)).take(1) {
                            slot.clone_from(value);
                        }
                    }
                    rows.push(values);
                }
            }
            (table, rows)
        };
        for values in &rows {
            let row = Fired::inserted(&table, values, self.held.header.encoding);
            self.fire(&triggers, &[], &row)?;
        }
        // `sqlite3_changes` counts the rows a statement wrote, and a
        // statement over a view writes none.
        Ok(0)
    }

    /// `UPDATE v` over a view: each row the `WHERE` keeps is put in
    /// `old`, the row the `SET` clauses make of it in `new`, and the
    /// `INSTEAD OF` triggers of the view run over the pair.
    ///
    /// The statement of the view is answered once, so the cost is what
    /// that statement costs plus what the triggers cost per row.
    ///
    /// # Errors
    ///
    /// [`Error::ViewWrite`] where the view carries no `INSTEAD OF
    /// UPDATE` trigger, [`Error::Eval`] for a column the view does not
    /// answer, and whatever a statement of a body refuses.
    fn update_view(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
        name: &[u8],
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let triggers = self.instead_of(name, TriggerEvent::Update)?;
        let sets = self.writing(arena, statement, sql);
        let columns: Vec<Vec<u8>> = sets
            .iter()
            .map(|set| crate::schema::dequote(set.column.text(sql)))
            .collect();
        let (table, written) = {
            let bytes = self.images();
            let database = self
                .reading_beside(&bytes)?
                .seeded(self.random.word())
                .counting(self.counted);
            let (table, held) = database.viewing(name)?;
            let places = set_places(&table, &columns)?;
            let joined = match statement.from {
                None => None,
                Some(id) => Some(database.joined(arena, id, sql)?),
            };
            let (of, sides) = beside(joined.as_ref());
            let mut written = Vec::new();
            for values in &held {
                // A view runs its triggers once per row the clause
                // holds, which is what `sqlite3Update` leaves an
                // `INSTEAD OF` trigger reading: the rows of the join
                // rather than the rows of the view.
                for side in &sides {
                    let aside = side.map(|row| Aside {
                        columns: of,
                        values: row,
                        outer,
                    });
                    let row = Held {
                        table: &table,
                        values,
                        rowid: None,
                        encoding: self.held.header.encoding,
                        random: &self.random,
                        clock: self.clock.map(crate::date::julian_of),
                        sensitive: self.truth.sensitive,
                        counted: self.counted,
                        defined: self.defined,
                        grouped: self.grouped,
                        collating: self.collating,
                        outer: aside
                            .as_ref()
                            .map(|one| -> &dyn crate::eval::Row { one })
                            .or(outer),
                        reading: Some(Reading {
                            database: &database,
                            arena,
                            sql,
                        }),
                    };
                    let keep = match statement.filter {
                        None => true,
                        Some(filter) => {
                            crate::eval::evaluate_row(arena, filter, sql, &row)?.truth(false)
                        }
                    };
                    if !keep {
                        continue;
                    }
                    let mut next = values.clone();
                    for (at, set) in places.iter().zip(&sets) {
                        let value = crate::eval::evaluate_row(arena, set.value, sql, &row)?;
                        for slot in next.iter_mut().skip(at.unwrap_or(usize::MAX)).take(1) {
                            slot.clone_from(&value);
                        }
                    }
                    written.push((values.clone(), next));
                }
            }
            (table, written)
        };
        for (old, new) in &written {
            let row = Fired {
                table: &table,
                old: Some((old, 0)),
                new: Some((new, 0)),
                encoding: self.held.header.encoding,
            };
            self.fire(&triggers, &columns, &row)?;
        }
        Ok(0)
    }

    /// `DELETE FROM v` over a view: each row the `WHERE` keeps is put
    /// in `old` and the `INSTEAD OF` triggers of the view run over it.
    ///
    /// The statement of the view is answered once, so the cost is what
    /// that statement costs plus what the triggers cost per row.
    ///
    /// # Errors
    ///
    /// [`Error::ViewWrite`] where the view carries no `INSTEAD OF
    /// DELETE` trigger, and whatever a statement of a body refuses.
    fn delete_view(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Delete,
        sql: &[u8],
        name: &[u8],
        outer: Option<&dyn crate::eval::Row>,
    ) -> Result<i64, Error> {
        let triggers = self.instead_of(name, TriggerEvent::Delete)?;
        let (table, taken) = {
            let bytes = self.images();
            let database = self
                .reading_beside(&bytes)?
                .seeded(self.random.word())
                .counting(self.counted);
            let (table, held) = database.viewing(name)?;
            let mut taken = Vec::new();
            for values in &held {
                let row = Held {
                    table: &table,
                    values,
                    rowid: None,
                    encoding: self.held.header.encoding,
                    random: &self.random,
                    clock: self.clock.map(crate::date::julian_of),
                    sensitive: self.truth.sensitive,
                    counted: self.counted,
                    defined: self.defined,
                    grouped: self.grouped,
                    collating: self.collating,
                    outer,
                    reading: Some(Reading {
                        database: &database,
                        arena,
                        sql,
                    }),
                };
                let keep = match statement.filter {
                    None => true,
                    Some(filter) => {
                        crate::eval::evaluate_row(arena, filter, sql, &row)?.truth(false)
                    }
                };
                if keep {
                    taken.push(values.clone());
                }
            }
            (table, taken)
        };
        for values in &taken {
            let row = Fired {
                table: &table,
                old: Some((values, 0)),
                new: None,
                encoding: self.held.header.encoding,
            };
            self.fire(&triggers, &[], &row)?;
        }
        Ok(0)
    }

    /// Whether the table `name` keeps its rows in the key's own tree.
    ///
    /// Reading the schema costs O(n) in its rows.
    fn keeps_rows(&self, name: &[u8]) -> Result<bool, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        Ok(database
            .table(name)
            .is_some_and(|(table, _)| table.without_rowid))
    }

    /// Whether the `WHERE` of a statement keeps one row of a table that
    /// keeps its rows in the key's own tree.
    ///
    /// Answering it costs what the expression costs.
    fn keeps(
        &self,
        filter: Option<crate::ast::ExprId>,
        table: &Table,
        values: &[Value],
        outer: Option<&dyn crate::eval::Row>,
        reading: Reading<'_>,
    ) -> Result<bool, Error> {
        let Some(filter) = filter else {
            return Ok(true);
        };
        // A table with no rowid answers none of the three names of the
        // key, which `Held` says by holding none.
        let held = Held {
            table,
            values,
            rowid: None,
            encoding: self.held.header.encoding,
            random: &self.random,
            clock: self.clock.map(crate::date::julian_of),
            sensitive: self.truth.sensitive,
            counted: self.counted,
            defined: self.defined,
            grouped: self.grouped,
            collating: self.collating,
            outer,
            reading: Some(reading),
        };
        Ok(crate::eval::evaluate_row(reading.arena, filter, reading.sql, &held)?.truth(false))
    }

    /// `DELETE` from a table that keeps its rows in the key's own tree:
    /// the entry the key finds is taken out, with the entry of every
    /// index over the table.
    ///
    /// Taking one row out costs O(log n) in the rows of the table.
    fn delete_keyed(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Delete,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
        name: &[u8],
    ) -> Result<i64, Error> {
        let (root, rows, kept, table) = {
            let bytes = self.images();
            let database = self.reading_beside(&bytes)?.counting(self.counted);
            // The table was found before this ran, so the refusal
            // carries no name to write into a message.
            let (table, root) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
            let kept = kept_indexes(&database, name);
            let mut rows = Vec::new();
            for values in database.keyed_rows_of(name)? {
                let reading = Reading {
                    database: &database,
                    arena,
                    sql,
                };
                if self.keeps(statement.filter, table, &values, outer, reading)? {
                    rows.push(values);
                }
            }
            (root, rows, kept, table.clone())
        };
        let before = self.triggers_for(name, TriggerEvent::Delete, TriggerTime::Before)?;
        let after = self.triggers_for(name, TriggerEvent::Delete, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let collations = crate::schema::key_collations(&table);
        let mut taken = 0_i64;
        for values in rows {
            let row = Fired {
                table: &table,
                old: Some((&values, 0)),
                new: None,
                encoding: self.held.header.encoding,
            };
            if fires && !self.fire(&before, &[], &row)? {
                continue;
            }
            // `D.2` of `src/fkey.c`: a row that rows of another table
            // point at is refused, or those rows are written.
            self.tells_peek(Did::Delete, &table, (0, 0), (Some(&values), None));
            self.unparented(&table, &values, 0)?;
            self.orphaned(name, &table, &values, 0, None)?;
            let key = crate::schema::key_of(&table, &values);
            self.unindex_row(&kept, &table, &values, &key)?;
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::remove_entry(&mut self.held.pages, root, &key, order)?;
            taken = taken.saturating_add(1);
            self.writing = taken;
            self.returns(arena, statement.returning, sql, (&table, &values, None))?;
            if fires {
                self.fire(&after, &[], &row)?;
            }
        }
        Ok(taken)
    }

    /// `INSERT` into a table that keeps its rows in the key's own tree:
    /// the row itself is the entry, placed under the columns of its
    /// `PRIMARY KEY`, which is section 2.4 of the format.
    ///
    /// Writing one row costs O(log n) in the rows of the table.
    fn insert_keyed(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Insert,
        sql: &[u8],
        name: &[u8],
        inserting: Inserting,
    ) -> Result<i64, Error> {
        let written = statement.conflict;
        let upserts = arena.upserts(statement.upserts);
        let Inserting {
            root,
            rows,
            kept,
            table,
            ..
        } = inserting;
        let order =
            Self::checked_order(upserts, (arena, sql), (&table, None, &kept), self.collating)?;
        let affinities = ordered_affinities(&table);
        let into = Insertion {
            collating: self.collating,
            arena,
            sql,
            upserts,
            table: &table,
            kept: &kept,
            root,
            alias: None,
            affinities: &affinities,
            conflict: written,
            returning: statement.returning,
            order: &order,
        };
        let before = self.triggers_for(name, TriggerEvent::Insert, TriggerTime::Before)?;
        let after = self.triggers_for(name, TriggerEvent::Insert, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let collations = crate::schema::key_collations(&table);
        let mut count = 0_i64;
        for (_, values) in rows {
            let mut named = values;
            let row = Fired::inserted(&table, &named, self.held.header.encoding);
            if fires && !self.fire(&before, &[], &row)? {
                continue;
            }
            if !self.constrained(&table, &mut named, 0, written)? {
                continue;
            }
            let stored = ordered(&table, &named);
            let key = crate::schema::key_of(&table, &named);
            match self.conflicted_keyed(&into, &named, &key)? {
                Conflicted::Over => continue,
                Conflicted::Wrote(rows) => {
                    count = count.saturating_add(rows);
                    self.writing = count;
                    continue;
                }
                Conflicted::Write => {}
            }
            self.tells_peek(Did::Insert, &table, (0, 0), (None, Some(&named)));
            self.parented(&table, &named, 0)?;
            self.index_row(&kept, &table, &named, &key)?;
            let record =
                crate::record::write_in(&stored, &affinities, 4, self.held.header.encoding);
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::insert_entry(&mut self.held.pages, root, &record, &key, order, false)?;
            count = count.saturating_add(1);
            self.writing = count;
            self.returns(arena, statement.returning, sql, (&table, &named, None))?;
            if fires {
                let row = Fired::inserted(&table, &named, self.held.header.encoding);
                self.fire(&after, &[], &row)?;
            }
        }
        Ok(count)
    }

    /// What a row that shares a key with one a table that keeps its
    /// rows in the key's own tree holds does: the `ON CONFLICT` clause
    /// it reaches writes the row the conflict found, or the resolution
    /// of the statement decides.
    ///
    /// Reading one key costs O(log n) in the rows of the table.
    ///
    /// # Errors
    ///
    /// [`Error::Unique`] where the resolution refuses the row.
    fn conflicted_keyed(
        &mut self,
        into: &Insertion<'_>,
        named: &[Value],
        key: &[Value],
    ) -> Result<Conflicted, Error> {
        let collations = crate::schema::key_collations(into.table);
        for at in into.order.iter().copied() {
            let Some(index) = at else {
                if crate::tree::find_entry(&self.held.pages, into.root, key, &collations)?.is_none()
                {
                    continue;
                }
                let columns = Self::key_column(into.table, None);
                if let Some(clause) =
                    upsert_of(into.arena, into.sql, into.upserts, &columns, into.collating)
                {
                    return Ok(Conflicted::Wrote(
                        self.upsert_keyed(into, clause, named, key)?,
                    ));
                }
                if self.placed(
                    into.root,
                    into.table,
                    key,
                    &collations,
                    into.kept,
                    into.conflict,
                )? {
                    continue;
                }
                return Ok(Conflicted::Over);
            };
            let found = into
                .kept
                .get(index)
                .map(|one| self.conflicts_at(one, into.table, (named, key), None))
                .transpose()?
                .flatten();
            let Some(held) = found else {
                continue;
            };
            let columns = Self::key_columns(into.table, into.kept, index).unwrap_or_default();
            if let Some(clause) =
                upsert_of(into.arena, into.sql, into.upserts, &columns, into.collating)
            {
                // The entry the row shares a key with ends with the key
                // of the row it belongs to.
                return Ok(Conflicted::Wrote(
                    self.upsert_keyed(into, clause, named, &held)?,
                ));
            }
            let answer = resolved(into.conflict, into.kept, index);
            match answer {
                Conflict::Ignore => return Ok(Conflicted::Over),
                Conflict::Replace
                    if self.replaced_keyed(into.root, into.kept, into.table, &held)? => {}
                _ => {
                    self.refusing(answer);
                    return Err(Error::Unique(Self::shown_key_of(
                        into.table, into.kept, index,
                    )));
                }
            }
        }
        Ok(Conflicted::Write)
    }

    /// What one `ON CONFLICT` clause does where a row of a table that
    /// keeps its rows in the key's own tree reaches it. Answers how
    /// many rows it wrote.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the written row
    /// with.
    fn upsert_keyed(
        &mut self,
        into: &Insertion<'_>,
        clause: &crate::ast::Upsert,
        proposed: &[Value],
        found: &[Value],
    ) -> Result<i64, Error> {
        if !clause.writes {
            return Ok(0);
        }
        let wanted = Keying {
            arena: into.arena,
            sql: into.sql,
            clause,
            table: into.table,
            kept: into.kept,
            root: into.root,
            affinities: into.affinities,
            proposed,
            found,
            returning: into.returning,
        };
        Ok(i64::from(self.upserted_keyed(&wanted)?))
    }

    /// The row a conflict found in a table that keeps its rows in the
    /// key's own tree, written again as a `DO UPDATE` says, and whether
    /// it was written.
    ///
    /// Reading the row costs O(n) in the rows of the table and writing
    /// it costs O(log n) in them and in the entries of each index.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the row with.
    fn upserted_keyed(&mut self, wanted: &Keying<'_>) -> Result<bool, Error> {
        let table = wanted.table;
        let held = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            database
                .keyed_rows_of(&table.name)?
                .into_iter()
                .find(|values| crate::schema::key_of(table, values) == wanted.found)
                .ok_or(Error::NoTable(Vec::new()))?
        };
        let row = Excluded {
            table,
            held: (&held, 0),
            proposed: (wanted.proposed, 0),
            encoding: self.held.header.encoding,
        };
        // The `WHERE` of the clause is read against the row the conflict
        // found, and a row it does not hold for is passed over.
        if let Some(filter) = wanted.clause.filter
            && !crate::eval::evaluate_row(wanted.arena, filter, wanted.sql, &row)?.truth(false)
        {
            return Ok(false);
        }
        let mut values = held.clone();
        let mut columns = Vec::new();
        for set in wanted.arena.sets(wanted.clause.sets) {
            let name = crate::schema::dequote(set.column.text(wanted.sql));
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| Error::Eval(crate::eval::Error::NoColumn(name.clone())))?;
            let value = crate::eval::evaluate_row(wanted.arena, set.value, wanted.sql, &row)?;
            for slot in values.iter_mut().skip(at).take(1) {
                slot.clone_from(&value);
            }
            columns.push(name);
        }
        self.upsert_keyed_row(wanted, &held, values, &columns)
    }

    /// The row a `DO UPDATE` computed over a table that keeps its rows
    /// in the key's own tree, written where the constraints of the
    /// table hold it, and whether it was written.
    ///
    /// Writing one row costs O(log n) in the rows of the table and in
    /// the entries of each index over it.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the row with.
    fn upsert_keyed_row(
        &mut self,
        wanted: &Keying<'_>,
        held: &[Value],
        mut named: Vec<Value>,
        columns: &[Vec<u8>],
    ) -> Result<bool, Error> {
        let table = wanted.table;
        let before = self.triggers_for(&table.name, TriggerEvent::Update, TriggerTime::Before)?;
        let after = self.triggers_for(&table.name, TriggerEvent::Update, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        if fires {
            let row = Fired {
                table,
                old: Some((held, 0)),
                new: Some((&named, 0)),
                encoding: self.held.header.encoding,
            };
            if !self.fire(&before, columns, &row)? {
                return Ok(false);
            }
        }
        // A clause writes under no resolution of its own, which is
        // `OE_Abort`, so the constraints of the table refuse the row
        // rather than passing it over.
        self.constrained(table, &mut named, 0, Conflict::Unspecified)?;
        // `sqlite3UpsertDoUpdate` builds the `UPDATE` under `OE_Abort`,
        // so a key the row shares with another refuses the statement
        // whatever the key's own clause says.
        self.rekeyed(
            wanted.root,
            table,
            wanted.kept,
            (held, &named),
            Conflict::Abort,
        )?;
        let root = wanted.root;
        let collations = crate::schema::key_collations(table);
        let was = crate::schema::key_of(table, held);
        let key = crate::schema::key_of(table, &named);
        self.unparented(table, held, 0)?;
        self.parented(table, &named, 0)?;
        self.orphaned(&table.name, table, held, 0, Some((&named, 0)))?;
        self.unindex_row(wanted.kept, table, held, &was)?;
        let order = ordering(&collations, self.held.header.encoding);
        crate::tree::remove_entry(&mut self.held.pages, root, &was, order)?;
        self.index_row(wanted.kept, table, &named, &key)?;
        let stored = ordered(table, &named);
        let record =
            crate::record::write_in(&stored, wanted.affinities, 4, self.held.header.encoding);
        let order = ordering(&collations, self.held.header.encoding);
        crate::tree::insert_entry(&mut self.held.pages, root, &record, &key, order, false)?;
        let answered = (table, named.as_slice(), None);
        self.returns(wanted.arena, wanted.returning, wanted.sql, answered)?;
        if fires {
            let row = Fired {
                table,
                old: Some((held, 0)),
                new: Some((&named, 0)),
                encoding: self.held.header.encoding,
            };
            self.fire(&after, columns, &row)?;
        }
        Ok(true)
    }

    /// Whether a row may be placed under `key` in the tree of a table
    /// that keeps its rows in the key's own tree: the key is free, or
    /// the row that holds it was written over, which is
    /// `sqlite3GenerateConstraintChecks` over the key of the table.
    ///
    /// Answers `false` where the row is passed over. Looking one key
    /// up costs O(log n).
    fn placed(
        &mut self,
        root: u32,
        table: &Table,
        key: &[Value],
        collations: &[crate::value::Placing],
        kept: &[Kept],
        written: Conflict,
    ) -> Result<bool, Error> {
        if crate::tree::find_entry(&self.held.pages, root, key, collations)?.is_none() {
            return Ok(true);
        }
        let mut answer = written;
        if answer == Conflict::Unspecified {
            answer = key_conflict(table);
        }
        match answer {
            Conflict::Ignore => return Ok(false),
            Conflict::Replace if self.replaced_keyed(root, kept, table, key)? => {
                return Ok(true);
            }
            _ => {}
        }
        self.refusing(answer);
        Err(Error::Unique(Self::shown_keys_of(table)))
    }

    /// The row a `REPLACE` writes over in a table that keeps its rows
    /// in the key's own tree, taken out with its entries and with the
    /// triggers a `DELETE` on it runs.
    fn replaced_keyed(
        &mut self,
        root: u32,
        kept: &[Kept],
        table: &Table,
        key: &[Value],
    ) -> Result<bool, Error> {
        let collations = crate::schema::key_collations(table);
        let values = self.keyed_row(table, key, &collations)?;
        // `sqlite3GenerateConstraintChecks` writes the row out under
        // `OE_Replace` with the triggers of a `DELETE` only where
        // `SQLITE_RecTriggers` is on, so a `REPLACE` runs none of them
        // on a connection that was told nothing.
        let (before, after) = self.deleting(&table.name)?;
        let row = Fired {
            table,
            old: Some((&values, 0)),
            new: None,
            encoding: self.held.header.encoding,
        };
        // A trigger that says the row stays leaves the key where it
        // was, so what the statement writes shares a key with it.
        if !self.fire(&before, &[], &row)? {
            return Ok(false);
        }
        self.tells_peek(Did::Delete, table, (0, 0), (Some(&values), None));
        self.unparented(table, &values, 0)?;
        self.orphaned(&table.name, table, &values, 0, None)?;
        self.unindex_row(kept, table, &values, key)?;
        let order = ordering(&collations, self.held.header.encoding);
        crate::tree::remove_entry(&mut self.held.pages, root, key, order)?;
        self.fire(&after, &[], &row)?;
        Ok(true)
    }

    /// The row of a table that keeps its rows in the key's own tree
    /// that `key` finds, and nothing where the table holds none.
    ///
    /// The walk is O(n) in the rows of the table.
    fn keyed_row(
        &self,
        table: &Table,
        key: &[Value],
        collations: &[crate::value::Placing],
    ) -> Result<Vec<Value>, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        // The key is one the tree answered, so the walk finds the row it
        // belongs to and the refusal carries no name to write into a
        // message.
        database
            .keyed_rows_of(&table.name)?
            .into_iter()
            .find(|values| {
                let mine = crate::schema::key_of(table, values);
                order_of_keys(&mine, key, collations) == core::cmp::Ordering::Equal
            })
            .ok_or(Error::NoTable(Vec::new()))
    }

    /// The columns of the `PRIMARY KEY` of a table, as a message names
    /// them: `table.column`, one after another with a comma between
    /// them.
    fn shown_keys_of(table: &Table) -> Vec<u8> {
        let mut out = Vec::new();
        for at in crate::schema::key_places(table) {
            if !out.is_empty() {
                out.extend_from_slice(b", ");
            }
            out.extend_from_slice(&table.name);
            out.push(b'.');
            let named = table
                .columns
                .get(at)
                .map_or(&[][..], |column| column.name.as_slice());
            out.extend_from_slice(named);
        }
        out
    }

    /// The rows an `UPDATE` changes in a table that keeps its rows in
    /// the key's own tree: the row as it stands and the row as the
    /// `SET` makes it.
    ///
    /// Reading `n` rows costs O(n) and answering the `SET` of one costs
    /// what its expressions cost.
    fn updated_keyed(
        &self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
        name: &[u8],
    ) -> Result<Rewriting, Error> {
        written_to(name)?;
        let sets = self.writing(arena, statement, sql);
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?.counting(self.counted);
        // The table was found before this ran, so the refusal carries
        // no name to write into a message.
        let (table, root) = database.table(name).ok_or(Error::NoTable(Vec::new()))?;
        let kept = kept_indexes(&database, name);
        let places: Vec<usize> = sets
            .iter()
            .map(|set| {
                let column = crate::schema::dequote(set.column.text(sql));
                table
                    .columns
                    .iter()
                    .position(|held| held.name.eq_ignore_ascii_case(&column))
                    .ok_or(Error::Unsupported)
            })
            .collect::<Result<_, Error>>()?;
        let joined = match statement.from {
            None => None,
            Some(id) => Some(database.joined(arena, id, sql)?),
        };
        let (columns, sides) = beside(joined.as_ref());
        let mut out = Vec::new();
        for values in database.keyed_rows_of(name)? {
            let reading = Reading {
                database: &database,
                arena,
                sql,
            };
            // A row of the table that the clause holds more than one
            // row for is written with the last of them.
            let mut found: Option<Vec<Value>> = None;
            for side in &sides {
                let aside = side.map(|row| Aside {
                    columns,
                    values: row,
                    outer,
                });
                let held = Held {
                    table,
                    values: &values,
                    rowid: None,
                    encoding: self.held.header.encoding,
                    random: &self.random,
                    clock: self.clock.map(crate::date::julian_of),
                    sensitive: self.truth.sensitive,
                    counted: self.counted,
                    defined: self.defined,
                    grouped: self.grouped,
                    collating: self.collating,
                    outer: aside
                        .as_ref()
                        .map(|one| -> &dyn crate::eval::Row { one })
                        .or(outer),
                    reading: Some(reading),
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
                for (at, set) in places.iter().zip(&sets) {
                    let value = crate::eval::evaluate_row(arena, set.value, sql, &held)?;
                    for slot in next.iter_mut().skip(*at).take(1) {
                        slot.clone_from(&value);
                    }
                }
                found = Some(next);
            }
            if let Some(next) = found {
                out.push((values, next));
            }
        }
        Ok(Rewriting {
            root,
            written: out,
            kept,
            table: table.clone(),
        })
    }

    /// `UPDATE` of a table that keeps its rows in the key's own tree:
    /// the entry the key finds is taken out and the row is written
    /// again, because the `SET` may write the key itself.
    ///
    /// Writing one row costs O(log n) in the rows of the table.
    fn update_keyed(
        &mut self,
        arena: &Arena,
        statement: &crate::ast::Update,
        sql: &[u8],
        outer: Option<&dyn crate::eval::Row>,
        name: &[u8],
    ) -> Result<i64, Error> {
        let columns: Vec<Vec<u8>> = arena
            .sets(statement.sets)
            .iter()
            .map(|set| crate::schema::dequote(set.column.text(sql)))
            .collect();
        let Rewriting {
            root,
            written: rows,
            kept,
            table,
        } = self.updated_keyed(arena, statement, sql, outer, name)?;
        let before = self.triggers_for(name, TriggerEvent::Update, TriggerTime::Before)?;
        let after = self.triggers_for(name, TriggerEvent::Update, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let collations = crate::schema::key_collations(&table);
        let affinities = ordered_affinities(&table);
        let mut changed = 0_i64;
        for (held, values) in rows {
            // A row this statement wrote over under `OE_Replace` is
            // gone, and the key it stood under may hold another row by
            // now, which is ticket #2832: the rows were read before the
            // first of them was written, so an entry that is no longer
            // the row that was read is passed over.
            let stood = crate::schema::key_of(&table, &held);
            let was = ordered(&table, &held);
            let tail: Vec<Value> = was
                .get(stood.len()..)
                .unwrap_or_default()
                .iter()
                .map(|value| stored(value, self.held.header.encoding))
                .collect();
            let order = ordering(&collations, self.held.header.encoding);
            let found =
                crate::tree::entry_tail_at(&self.held.pages, root, &stood, order, tail.len())?;
            if found.as_deref() != Some(tail.as_slice()) {
                continue;
            }
            let mut named = values;
            if fires {
                let row = Fired {
                    table: &table,
                    old: Some((&held, 0)),
                    new: Some((&named, 0)),
                    encoding: self.held.header.encoding,
                };
                if !self.fire(&before, &columns, &row)? {
                    continue;
                }
            }
            if !self.constrained(&table, &mut named, 0, statement.conflict)? {
                continue;
            }
            if !self.rekeyed(root, &table, &kept, (&held, &named), statement.conflict)? {
                continue;
            }
            let key = crate::schema::key_of(&table, &named);
            self.tells_peek(Did::Update, &table, (0, 0), (Some(&held), Some(&named)));
            self.unparented(&table, &held, 0)?;
            self.parented(&table, &named, 0)?;
            self.orphaned(name, &table, &held, 0, Some((&named, 0)))?;
            let was = crate::schema::key_of(&table, &held);
            self.unindex_row(&kept, &table, &held, &was)?;
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::remove_entry(&mut self.held.pages, root, &was, order)?;
            self.index_row(&kept, &table, &named, &key)?;
            let stored = ordered(&table, &named);
            let record =
                crate::record::write_in(&stored, &affinities, 4, self.held.header.encoding);
            let order = ordering(&collations, self.held.header.encoding);
            crate::tree::insert_entry(&mut self.held.pages, root, &record, &key, order, false)?;
            changed = changed.saturating_add(1);
            self.writing = changed;
            self.returns(arena, statement.returning, sql, (&table, &named, None))?;
            if fires {
                let row = Fired {
                    table: &table,
                    old: Some((&held, 0)),
                    new: Some((&named, 0)),
                    encoding: self.held.header.encoding,
                };
                self.fire(&after, &columns, &row)?;
            }
        }
        Ok(changed)
    }

    /// Whether the row an `UPDATE` writes may be placed under the key
    /// it is written with: the key it had, a key no row holds, or a key
    /// whose row was written over.
    ///
    /// Answers `false` where the row is passed over. Looking the keys
    /// up costs O(log n).
    fn rekeyed(
        &mut self,
        root: u32,
        table: &Table,
        kept: &[Kept],
        row: (&[Value], &[Value]),
        written: Conflict,
    ) -> Result<bool, Error> {
        let (held, named) = row;
        let collations = crate::schema::key_collations(table);
        let was = crate::schema::key_of(table, held);
        let key = crate::schema::key_of(table, named);
        // A row that keeps the key it had shares it with nothing.
        if order_of_keys(&key, &was, &collations) != core::cmp::Ordering::Equal
            && !self.placed(root, table, &key, &collations, kept, written)?
        {
            return Ok(false);
        }
        let Some((index, other)) = self.conflicting(kept, table, named, &key, Some(&was))? else {
            return Ok(true);
        };
        let answer = resolved(written, kept, index);
        match answer {
            Conflict::Ignore => return Ok(false),
            Conflict::Replace if self.replaced_keyed(root, kept, table, &other)? => {
                return Ok(true);
            }
            _ => {}
        }
        self.refusing(answer);
        Err(Error::Unique(Self::shown_key_of(table, kept, index)))
    }

    /// The entries of one index written into the tree at `root`, which
    /// is what `sqlite3RefillIndex` writes.
    ///
    /// Sorting `n` entries costs O(n log n) and writing them O(n).
    fn write_entries(
        &mut self,
        entries: Vec<Vec<Value>>,
        collations: &[crate::value::Placing],
        root: u32,
    ) -> Result<(), Error> {
        // `sqlite3VdbeSorterInit`: the entries are sorted before any is
        // written, so the pages fill in the order the entries run and
        // not in the order the rows do.
        let mut entries = entries;
        entries.sort_by(|one, other| order_of_keys(one, other, collations));
        let affinities = alloc::vec![Affinity::None; collations.len().saturating_add(1)];
        for key in entries {
            let record = crate::record::write_in(&key, &affinities, 4, self.held.header.encoding);
            let order = ordering(collations, self.held.header.encoding);
            crate::tree::insert_entry(&mut self.held.pages, root, &record, &key, order, true)?;
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
        let asking = named.unwrap_or_default();
        if self
            .asked(|authorizer| authorizer.reindex(&asking, b"main"))?
            .answer
            == crate::auth::Answer::Ignore
        {
            return Ok(());
        }
        for Rebuilt {
            index,
            root,
            table,
            sql: made,
            arena: tree,
        } in held
        {
            crate::tree::clear_tree(&mut self.held.pages, root, Kind::LeafIndex)?;
            let entries = {
                let bytes = self.image();
                let database = self.reading(&bytes)?;
                let (over, _) = database.table(&table).ok_or(Error::Unsupported)?;
                let over = Over {
                    arena: &tree,
                    sql: &made,
                    table: over,
                    encoding: self.held.header.encoding,
                };
                let mut entries = Vec::new();
                for (key, values) in database.held_rows_of(&table)? {
                    if !indexes_row(&index, &over, &values)? {
                        continue;
                    }
                    entries.push(entry_of(&index, &over, &values, &key)?);
                }
                entries
            };
            let collations = collations_of(&index, self.held.header.schema_format);
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
        let database = self.reading(&bytes)?;
        let collation = named.and_then(|name| crate::value::collation_of(name, self.collating));
        let over: Option<Vec<u8>> = match named {
            None => None,
            Some(_) if collation.is_some() => None,
            Some(named) if database.table(named).is_some() => Some(named.to_vec()),
            Some(named) => {
                let kept = database
                    .indexed(named)
                    .ok_or_else(|| Error::NoTable(named.to_vec()))?;
                return Ok(alloc::vec![Rebuilt {
                    index: kept.index.clone(),
                    root: kept.root,
                    table: kept.index.table.clone(),
                    sql: kept.sql.to_vec(),
                    arena: kept.arena.clone(),
                }]);
            }
        };
        let mut held = Vec::new();
        for table in database.tables() {
            if over.as_deref().is_some_and(|over| table.name != over) {
                continue;
            }
            for kept in database.indexes(&table.name).iter().rev() {
                // `REINDEX <collation>` writes again every index one of
                // whose places is held in that collation.
                if collation.is_some_and(|wanted| {
                    !kept
                        .index
                        .columns
                        .iter()
                        .any(|column| column.collation == wanted)
                }) {
                    continue;
                }
                held.push(Rebuilt {
                    index: kept.index.clone(),
                    root: kept.root,
                    table: table.name.clone(),
                    sql: kept.sql.to_vec(),
                    arena: kept.arena.clone(),
                });
            }
        }
        held.reverse();
        Ok(held)
    }

    /// The values an index over the table reads, with `rowid` in place
    /// of the column the key is another name for.
    ///
    /// The row itself stores nothing for that column, because the key
    /// carries the value. Costs O(n) over the columns of the row.
    fn keying(values: &mut [Value], alias: Option<usize>, rowid: i64) -> Vec<Value> {
        let mut named = values.to_vec();
        for slot in named.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
            *slot = Value::Int(rowid);
        }
        for slot in values.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
            *slot = Value::Null;
        }
        named
    }

    /// The key a row of an `INSERT` carries, where the row carries one:
    /// the key the statement wrote, or the value of the column the key
    /// is another name for.
    ///
    /// Reading it costs O(1).
    ///
    /// # Errors
    ///
    /// [`Error::Mismatch`] where the key is neither an integer nor
    /// `NULL`.
    fn given_key(key: Value, values: &[Value], alias: Option<usize>) -> Result<Option<i64>, Error> {
        let held = alias.map(|at| values.get(at).cloned().unwrap_or(Value::Null));
        match (key, held) {
            (Value::Int(given), _) | (Value::Null, Some(Value::Int(given))) => Ok(Some(given)),
            (Value::Null, None | Some(Value::Null)) => Ok(None),
            _ => Err(Error::Mismatch),
        }
    }

    /// The key a row that carries none is written under, where `next`
    /// is the largest key the table ever held.
    ///
    /// `OP_NewRowid`: one past `next`, except where `next` is the
    /// largest key an integer holds, and then a table whose key counts
    /// up refuses and any other table takes a key drawn at random that
    /// no row holds. The draw is tried 100 times and each try reads the
    /// tree, so one try costs O(log n).
    ///
    /// # Errors
    ///
    /// [`crate::error::Error::Full`] where 100 draws all name a key a row
    /// holds.
    fn free_key(&self, root: u32, next: i64, counting: bool) -> Result<i64, Error> {
        if next != i64::MAX {
            return Ok(next.saturating_add(1));
        }
        if counting {
            return Err(Error::Image(crate::error::Error::Full));
        }
        for _ in 0..100 {
            let word = i64::from_le_bytes(self.random.word().to_le_bytes());
            let drawn = (word & (i64::MAX >> 1)).saturating_add(1);
            if !crate::tree::holds(&self.held.pages, root, drawn)? {
                return Ok(drawn);
            }
        }
        Err(Error::Image(crate::error::Error::Full))
    }

    /// The key one row of an `UPDATE` is written under and the values
    /// an index over the table reads.
    ///
    /// The column the key is another name for says the key, so a
    /// statement that writes that column writes the key; the row holds
    /// no value for that column, and an index over it holds the key.
    /// Costs O(n) over the columns of the row.
    fn rekeying(
        values: &mut [Value],
        alias: Option<usize>,
        held: i64,
    ) -> Result<(i64, Vec<Value>), Error> {
        let mut key = held;
        if let Some(at) = alias {
            let mut given = values.get(at).cloned().unwrap_or(Value::Null);
            crate::value::apply(&mut given, Affinity::Integer);
            match given {
                Value::Int(number) => key = number,
                _ => return Err(Error::Mismatch),
            }
            for slot in values.iter_mut().skip(at).take(1) {
                *slot = Value::Null;
            }
        }
        let mut named = values.to_vec();
        for slot in named.iter_mut().skip(alias.unwrap_or(usize::MAX)).take(1) {
            *slot = Value::Int(key);
        }
        Ok((key, named))
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
        self.located(&name)?;
        if self.is_view(&name)? {
            return self.insert_view(arena, statement, sql, &name, outer);
        }
        let inserting = self.inserting(arena, statement, sql, outer, &name)?;
        let upserts = arena.upserts(statement.upserts);
        if inserting.table.without_rowid {
            return self.insert_keyed(arena, statement, sql, &name, inserting);
        }
        let Inserting {
            root,
            alias,
            affinities,
            rows,
            kept,
            table,
        } = inserting;
        let order = Self::checked_order(
            upserts,
            (arena, sql),
            (&table, alias, &kept),
            self.collating,
        )?;
        let into = Insertion {
            collating: self.collating,
            arena,
            sql,
            upserts,
            table: &table,
            kept: &kept,
            root,
            alias,
            affinities: &affinities,
            conflict: statement.conflict,
            returning: statement.returning,
            order: &order,
        };
        let before = self.triggers_for(&name, TriggerEvent::Insert, TriggerTime::Before)?;
        let after = self.triggers_for(&name, TriggerEvent::Insert, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        let (mut next, counted) = self.counting_from(root, &table, &name)?;
        let mut written = 0_i64;
        for (key, mut values) in rows {
            // A trigger's body may write the table this statement
            // writes, so the largest key is read again per row where
            // one runs.
            if fires {
                next = next.max(largest(&self.held.pages, root)?.unwrap_or(0));
            }
            let given = Self::given_key(key, &values, alias)?;
            let mut rowid =
                given.map_or_else(|| self.free_key(root, next, counted.is_some()), Ok)?;
            let mut named = Self::keying(&mut values, alias, rowid);
            next = next.max(rowid);
            if fires && !self.fired_early(&before, &table, &named, alias, given)? {
                continue;
            }
            // `sqlite3Insert` writes `OP_NewRowid` after the triggers
            // before the row have run, so a row that carries no key of
            // its own takes another key where the body of such a trigger
            // wrote the one it stood to take.
            if given.is_none() && crate::tree::holds(&self.held.pages, root, rowid)? {
                next = next.max(largest(&self.held.pages, root)?.unwrap_or(0));
                rowid = self.free_key(root, next, counted.is_some())?;
                named = Self::keying(&mut values, alias, rowid);
                next = next.max(rowid);
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
                encoding: self.held.header.encoding,
            };
            match self.conflicted(&into, &named, rowid)? {
                Conflicted::Write => {}
                Conflicted::Over => continue,
                Conflicted::Wrote(rows) => {
                    written = written.saturating_add(rows);
                    continue;
                }
            }
            self.wrote_row(root, &kept, &table, (&named, &values, rowid), &affinities)?;
            written = written.saturating_add(1);
            self.writing = written;
            // `sqlite3_last_insert_rowid` is the key of the last row an
            // `INSERT` wrote into a table with a rowid.
            self.counted.rowid = rowid;
            let answered = (&table, named.as_slice(), Some(rowid));
            self.returns(arena, statement.returning, sql, answered)?;
            if fires {
                self.fire(&after, &[], &row)?;
            }
        }
        // `autoIncrementEnd` writes the largest key the table ever held
        // back into `sqlite_sequence` at the end of the statement, which
        // it does whether or not the statement wrote a row, so an
        // `INSERT` that wrote none still makes the row.
        if counted.is_some() {
            self.count_up(&name, next)?;
        }
        Ok(written)
    }

    /// One row written into the tree of the table, with the entry of
    /// every index beside it.
    ///
    /// `sqlite3CompleteInsertion` writes the entry of every index before
    /// the row, so the pages an entry runs onto are taken before the
    /// pages the row runs onto. Writing costs O(log n) per tree.
    ///
    /// # Errors
    ///
    /// [`Error::Foreign`] where a foreign key of the row points at no
    /// row, which `I.1` of `src/fkey.c` refuses before the row is
    /// written, and whatever writing a tree refuses.
    fn wrote_row(
        &mut self,
        root: u32,
        kept: &[Kept],
        table: &Table,
        row: (&[Value], &[Value], i64),
        affinities: &[Affinity],
    ) -> Result<(), Error> {
        let (named, values, rowid) = row;
        let record = crate::record::write_in(
            &ordered(table, values),
            affinities,
            4,
            self.held.header.encoding,
        );
        self.tells_peek(Did::Insert, table, (rowid, rowid), (None, Some(named)));
        self.parented(table, named, rowid)?;
        self.index_row(kept, table, named, &keyed_as(rowid))?;
        insert(&mut self.held.pages, root, rowid, &record)?;
        self.tells_write(Did::Insert, table, rowid);
        Ok(())
    }

    /// The largest key the table ever held and the one `sqlite_sequence`
    /// counts, where the table counts its keys up.
    ///
    /// `autoIncBegin`: a key that counts up never gives a key back, so
    /// the next one is past the largest the table ever held and not
    /// past the largest it holds. Reading it costs O(log n).
    ///
    /// # Errors
    ///
    /// Whatever reading the tree of the table or the row that counts
    /// refuses.
    fn counting_from(
        &self,
        root: u32,
        table: &Table,
        name: &[u8],
    ) -> Result<(i64, Option<i64>), Error> {
        let largest = largest(&self.held.pages, root)?.unwrap_or(0);
        let counted = table
            .autoincrement
            .then(|| self.counted(name))
            .transpose()?;
        Ok((largest.max(counted.unwrap_or(0)), counted))
    }

    /// What a row that shares a key with one the table holds does: the
    /// `ON CONFLICT` clause it reaches writes the row the conflict
    /// found, or the resolution of the statement decides, which is
    /// `sqlite3GenerateConstraintChecks`.
    ///
    /// # Errors
    ///
    /// [`Error::Unique`] where the resolution refuses the row.
    fn conflicted(
        &mut self,
        into: &Insertion<'_>,
        named: &[Value],
        rowid: i64,
    ) -> Result<Conflicted, Error> {
        for key in into.order.iter().copied() {
            let Some(index) = key else {
                match self.keyed_conflict(into, named, rowid)? {
                    Some(answer) => return Ok(answer),
                    None => continue,
                }
            };
            let tail = keyed_as(rowid);
            let found = into
                .kept
                .get(index)
                .map(|one| self.conflicts_at(one, into.table, (named, &tail), None))
                .transpose()?
                .flatten();
            let Some(held) = found else {
                continue;
            };
            let columns = Self::key_columns(into.table, into.kept, index).unwrap_or_default();
            if let Some(clause) =
                upsert_of(into.arena, into.sql, into.upserts, &columns, into.collating)
            {
                // The entry the row shares a key with ends with the key
                // of the row it belongs to.
                let other = held.first().map_or(rowid, Value::to_integer);
                return Ok(Conflicted::Wrote(self.upsert(
                    into,
                    clause,
                    named,
                    (rowid, other),
                )?));
            }
            let answer = resolved(into.conflict, into.kept, index);
            match answer {
                crate::ast::Conflict::Ignore => return Ok(Conflicted::Over),
                crate::ast::Conflict::Replace
                    if self.replaced(into.root, into.kept, into.table, &held)? => {}
                _ => {
                    self.refusing(answer);
                    return Err(Error::Unique(Self::shown_key_of(
                        into.table, into.kept, index,
                    )));
                }
            }
        }
        Ok(Conflicted::Write)
    }

    /// What a row that shares the key of the table with one already
    /// there does, or nothing where it shares no key, which is
    /// `sqlite3UpsertOfIndex` reading the clause that names the column
    /// the key is another name for.
    ///
    /// # Errors
    ///
    /// [`Error::Unique`] where the resolution refuses the row.
    fn keyed_conflict(
        &mut self,
        into: &Insertion<'_>,
        named: &[Value],
        rowid: i64,
    ) -> Result<Option<Conflicted>, Error> {
        // The statement wrote the key where the row carries one, which
        // is the column the key is another name for or the name `rowid`,
        // and a key no row holds shares with none.
        if !crate::tree::holds(&self.held.pages, into.root, rowid)? {
            return Ok(None);
        }
        let key = Self::key_column(into.table, into.alias);
        if let Some(clause) = upsert_of(into.arena, into.sql, into.upserts, &key, into.collating) {
            let wrote = self.upsert(into, clause, named, (rowid, rowid))?;
            return Ok(Some(Conflicted::Wrote(wrote)));
        }
        if self.keyed(
            into.root,
            into.kept,
            into.table,
            into.alias,
            rowid,
            into.conflict,
        )? {
            return Ok(None);
        }
        Ok(Some(Conflicted::Over))
    }

    /// The order the row is held to the keys of the table in, which is
    /// `sqlite3GenerateConstraintChecks` reading `IndexListTerm`: the
    /// keys the `ON CONFLICT` clauses name, in the order the clauses
    /// name them, with the key of the table in the place of the clause
    /// that names it and after the named keys where no clause names it,
    /// and the indexes no clause names after them in the order the
    /// schema holds them.
    ///
    /// Ordering `n` clauses over `k` indexes costs O(n k).
    fn checked_order(
        upserts: &[crate::ast::Upsert],
        held: (&Arena, &[u8]),
        over: (&Table, Option<usize>, &[Kept]),
        collating: &[crate::value::Collating],
    ) -> Result<Vec<Option<usize>>, Error> {
        let (arena, sql) = held;
        let (table, alias, kept) = over;
        let mut named: Vec<usize> = Vec::new();
        let mut at_key: Option<usize> = None;
        for clause in upserts {
            // A clause that names no column is the one every conflict
            // reaches, so it orders no key and ends the run.
            if clause.targets.is_empty() {
                break;
            }
            let targets = targets_of(arena, sql, clause, collating).ok_or(Error::NoUpsertKey)?;
            if names_key(&Self::key_column(table, alias), &targets) {
                at_key.get_or_insert(named.len());
                continue;
            }
            let at = (0..kept.len())
                .find(|at| {
                    Self::key_columns(table, kept, *at).is_some_and(|key| names_key(&key, &targets))
                })
                .ok_or(Error::NoUpsertKey)?;
            // A clause that names a key another clause named already is
            // one no conflict reaches, which `bUsed` marks.
            if !named.contains(&at) {
                named.push(at);
            }
        }
        let mut out: Vec<Option<usize>> = Vec::new();
        for (place, index) in named.iter().enumerate() {
            if at_key == Some(place) {
                out.push(None);
            }
            out.push(Some(*index));
        }
        if !out.contains(&None) {
            out.push(None);
        }
        out.extend((0..kept.len()).filter(|at| !named.contains(at)).map(Some));
        Ok(out)
    }

    /// What one `ON CONFLICT` clause does where a row reaches it: a
    /// `DO NOTHING` passes the row over, and a `DO UPDATE` writes the
    /// row the conflict found. Answers how many rows it wrote.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the written row
    /// with.
    fn upsert(
        &mut self,
        into: &Insertion<'_>,
        clause: &crate::ast::Upsert,
        proposed: &[Value],
        keys: (i64, i64),
    ) -> Result<i64, Error> {
        let (given, rowid) = keys;
        if !clause.writes {
            return Ok(0);
        }
        let wanted = Upserting {
            arena: into.arena,
            sql: into.sql,
            clause,
            table: into.table,
            kept: into.kept,
            root: into.root,
            alias: into.alias,
            proposed,
            given,
            rowid,
            returning: into.returning,
        };
        Ok(i64::from(self.upserted(&wanted)?))
    }

    /// The row a conflict found, written again as a `DO UPDATE` says,
    /// which is `sqlite3UpsertDoUpdate`, and whether it was written.
    ///
    /// Reading the row costs O(n) in the rows of the table and writing
    /// it costs O(log n) in them and in the entries of each index.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the row with.
    fn upserted(&mut self, wanted: &Upserting<'_>) -> Result<bool, Error> {
        let table = wanted.table;
        let (rowid, held) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            let (rowid, held) = database
                .rows_of(&table.name)?
                .into_iter()
                .find(|(key, _)| *key == wanted.rowid)
                .ok_or(Error::NoTable(Vec::new()))?;
            (rowid, held)
        };
        let row = Excluded {
            table,
            held: (&held, rowid),
            proposed: (wanted.proposed, wanted.given),
            encoding: self.held.header.encoding,
        };
        // The `WHERE` of the clause is read against the row the conflict
        // found, and a row it does not hold for is passed over.
        if let Some(filter) = wanted.clause.filter
            && !crate::eval::evaluate_row(wanted.arena, filter, wanted.sql, &row)?.truth(false)
        {
            return Ok(false);
        }
        let mut values = held.clone();
        let mut columns = Vec::new();
        for set in wanted.arena.sets(wanted.clause.sets) {
            let name = crate::schema::dequote(set.column.text(wanted.sql));
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| Error::Eval(crate::eval::Error::NoColumn(name.clone())))?;
            let value = crate::eval::evaluate_row(wanted.arena, set.value, wanted.sql, &row)?;
            for slot in values.iter_mut().skip(at).take(1) {
                slot.clone_from(&value);
            }
            columns.push(name);
        }
        self.upsert_row(wanted, &held, rowid, values, &columns)
    }

    /// The row a `DO UPDATE` computed, written where the constraints of
    /// the table hold it, and whether it was written.
    ///
    /// Writing one row costs O(log n) in the rows of the table and in
    /// the entries of each index over it.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the row with.
    fn upsert_row(
        &mut self,
        wanted: &Upserting<'_>,
        held: &[Value],
        rowid: i64,
        mut values: Vec<Value>,
        columns: &[Vec<u8>],
    ) -> Result<bool, Error> {
        let table = wanted.table;
        let mut key = rowid;
        // The column the key is another name for says the key, so a
        // clause that writes that column writes the key.
        if let Some(at) = wanted.alias {
            let mut given = values.get(at).cloned().unwrap_or(Value::Null);
            crate::value::apply(&mut given, Affinity::Integer);
            match given {
                Value::Int(number) => key = number,
                _ => return Err(Error::Mismatch),
            }
            for slot in values.iter_mut().skip(at).take(1) {
                *slot = Value::Null;
            }
        }
        let mut named = values.clone();
        for slot in named
            .iter_mut()
            .skip(wanted.alias.unwrap_or(usize::MAX))
            .take(1)
        {
            *slot = Value::Int(key);
        }
        let before = self.triggers_for(&table.name, TriggerEvent::Update, TriggerTime::Before)?;
        let after = self.triggers_for(&table.name, TriggerEvent::Update, TriggerTime::After)?;
        let fires = !before.is_empty() || !after.is_empty();
        if fires {
            let row = Fired {
                table,
                old: Some((held, rowid)),
                new: Some((&named, key)),
                encoding: self.held.header.encoding,
            };
            if !self.fire(&before, columns, &row)? {
                return Ok(false);
            }
        }
        // A clause writes under no resolution of its own, which is
        // `OE_Abort`, so the constraints of the table refuse the row
        // rather than passing it over.
        self.constrained(table, &mut named, key, Conflict::Unspecified)?;
        Self::refilled(&named, &mut values, wanted.alias);
        let row = Fired {
            table,
            old: Some((held, rowid)),
            new: Some((&named, key)),
            encoding: self.held.header.encoding,
        };
        if key != rowid {
            self.keyed(
                wanted.root,
                wanted.kept,
                table,
                wanted.alias,
                key,
                Conflict::Unspecified,
            )?;
        }
        let (new, old) = (keyed_as(key), keyed_as(rowid));
        let found = self.conflicting(wanted.kept, wanted.table, &named, &new, Some(&old))?;
        if let Some((index, _)) = found {
            return Err(Error::Unique(Self::shown_key_of(table, wanted.kept, index)));
        }
        let affinities = ordered_affinities(table);
        let record = crate::record::write_in(
            &ordered(table, &values),
            &affinities,
            4,
            self.held.header.encoding,
        );
        self.unparented(table, held, rowid)?;
        self.parented(table, &named, key)?;
        self.orphaned(&table.name, table, held, rowid, Some((&named, key)))?;
        self.unindex_row(wanted.kept, wanted.table, held, &keyed_as(rowid))?;
        let moved = key != rowid;
        if moved {
            crate::tree::remove(&mut self.held.pages, wanted.root, rowid)?;
        }
        self.index_row(wanted.kept, wanted.table, &named, &keyed_as(key))?;
        if moved {
            insert(&mut self.held.pages, wanted.root, key, &record)?;
        } else {
            crate::tree::update(&mut self.held.pages, wanted.root, rowid, &record)?;
        }
        let answered = (table, named.as_slice(), Some(key));
        self.returns(wanted.arena, wanted.returning, wanted.sql, answered)?;
        if fires {
            self.fire(&after, columns, &row)?;
        }
        Ok(true)
    }

    /// The row of `sqlite_sequence` that names `name`, taken away with
    /// the table it counts.
    ///
    /// Taking one row out costs O(log n).
    fn uncount(&mut self, name: &[u8]) -> Result<(), Error> {
        let wanted = crate::value::stored(name, self.held.header.encoding);
        let found = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
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
            crate::tree::remove(&mut self.held.pages, root, rowid)?;
        }
        Ok(())
    }

    /// The largest key a table that counts its keys up ever held, which
    /// is the row of `sqlite_sequence` that names it.
    ///
    /// Reading it costs O(n) in the rows of that table.
    fn counted(&self, name: &[u8]) -> Result<i64, Error> {
        let bytes = self.image();
        let database = self.reading(&bytes)?;
        let wanted = crate::value::stored(name, self.held.header.encoding);
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
        let wanted = crate::value::stored(name, self.held.header.encoding);
        let (root, rowid, stood) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            let (_, root) = database.table(SEQUENCE).ok_or(Error::NoTable(Vec::new()))?;
            let found = database
                .rows_of(SEQUENCE)?
                .iter()
                .find(|(_, values)| named_row(values, &wanted))
                .map(|(rowid, values)| (*rowid, values.get(1).map_or(0, Value::to_integer)));
            (
                root,
                found.map(|(rowid, _)| rowid),
                found.map_or(0, |(_, seq)| seq),
            )
        };
        // A trigger's body that writes the same table counts the keys up
        // further than the statement that fired it, and
        // `autoIncrementEnd` writes one register per table, so the row
        // never counts back down.
        let held = held.max(stood);
        let record = crate::record::write_in(
            &[Value::Text(wanted), Value::Int(held)],
            &[Affinity::None; 2],
            4,
            self.held.header.encoding,
        );
        if let Some(rowid) = rowid {
            crate::tree::update(&mut self.held.pages, root, rowid, &record)?;
            return Ok(());
        }
        let rowid = largest(&self.held.pages, root)?
            .unwrap_or(0)
            .saturating_add(1);
        insert(&mut self.held.pages, root, rowid, &record)?;
        Ok(())
    }
}

/// One row of a table, read against the `WHERE` of a statement that
/// changes rows.
/// What a row of a statement that writes reads a statement written
/// inside an expression with: the database as the statement found it,
/// and the tree and text that statement was read from.
#[derive(Clone, Copy)]
struct Reading<'a> {
    /// The database as the statement found it.
    database: &'a Database<'a>,
    /// The tree the statement was read from.
    arena: &'a Arena,
    /// The text that tree points into.
    sql: &'a [u8],
}

struct Held<'a> {
    /// The table it belongs to, because a column may be written with
    /// the table's name before it.
    table: &'a Table,
    /// The value of each column, in the order the table was created
    /// with.
    values: &'a [Value],
    /// The key of the row, which the three names of the key answer,
    /// and nothing where the table keeps its rows in the key's own
    /// tree, which has no rowid for a name to answer.
    rowid: Option<i64>,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// Where `random` and `randomblob` take their bytes from.
    random: &'a crate::random::Source,
    /// What the clock says, which `now` names.
    clock: Option<i64>,
    /// Whether `LIKE` tells the twenty-six letters apart, which
    /// `PRAGMA case_sensitive_like` sets.
    sensitive: bool,
    /// What the connection has written, which `changes()`,
    /// `total_changes()` and `last_insert_rowid()` answer.
    counted: crate::func::Counted,
    /// The functions the application defined on the connection.
    defined: &'static [crate::func::Defined],
    /// The aggregates the application defined on the connection.
    grouped: &'static [crate::func::Grouped],
    /// The collations the application defined on the connection.
    collating: &'static [crate::value::Collating],
    /// The row a trigger's body reads as `new` and `old`, where this
    /// row is one of a statement a trigger runs.
    outer: Option<&'a dyn crate::eval::Row>,
    /// What a statement written inside an expression is answered with,
    /// where the caller holds the database open.
    reading: Option<Reading<'a>>,
}

impl crate::eval::Row for Held<'_> {
    fn defined(&self, name: &[u8], count: usize) -> Option<crate::func::Defined> {
        crate::func::defined(self.defined, name, count)
    }

    fn grouped(&self) -> &'static [crate::func::Grouped] {
        self.grouped
    }

    fn collating(&self) -> &'static [crate::value::Collating] {
        self.collating
    }

    fn random(&self) -> Option<&crate::random::Source> {
        Some(self.random)
    }

    fn clock(&self) -> Option<i64> {
        self.clock
    }

    fn sensitive(&self) -> bool {
        self.sensitive
    }

    fn counted(&self) -> crate::func::Counted {
        self.counted
    }

    fn answered(&self, used: crate::eval::Used) -> Option<Value> {
        let reading = self.reading?;
        reading
            .database
            .subquery(reading.arena, reading.sql, used, self)
            .ok()
    }

    fn answered_items(
        &self,
        select: crate::ast::SelectId,
    ) -> Option<Vec<(Value, Affinity, Option<Collation>)>> {
        let reading = self.reading?;
        reading
            .database
            .subquery_items(reading.arena, reading.sql, select, self)
            .ok()
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
            None if is_rowid(column) => self
                .rowid
                .map(|key| (Value::Int(key), Affinity::Integer, Collation::Binary)),
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
        .map_or(Conflict::Unspecified, |held| held.index.conflict);
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

/// One row of the schema: what it makes, the name it makes it under, and
/// the statement that made it.
struct Made {
    /// `table`, `index`, `view` or `trigger`.
    kind: Vec<u8>,
    /// The name it makes.
    name: Vec<u8>,
    /// The statement that made it.
    statement: Vec<u8>,
}

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

/// What an `UPDATE` of a table that keeps its rows in the key's own
/// tree writes.
struct Rewriting {
    /// The tree of the table.
    root: u32,
    /// One per row the `WHERE` keeps: the values it held and the values
    /// it is written with.
    written: Vec<(Vec<Value>, Vec<Value>)>,
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

impl<'a> Fired<'a> {
    /// The row an `INSERT` writes, which no key of its own names where
    /// the table keeps its rows in the key's own tree.
    const fn inserted(table: &'a Table, values: &'a [Value], encoding: Encoding) -> Self {
        Fired {
            table,
            old: None,
            new: Some((values, 0)),
            encoding,
        }
    }
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

/// What the rows of one `INSERT` are written under, which every row of
/// it is held to.
struct Insertion<'a> {
    /// The tree the statement was read from.
    arena: &'a Arena,
    /// The text of the statement.
    sql: &'a [u8],
    /// The `ON CONFLICT` clauses of the statement.
    upserts: &'a [crate::ast::Upsert],
    /// The table the rows go in.
    table: &'a Table,
    /// The indexes over the table.
    kept: &'a [Kept],
    /// The tree of the table.
    root: u32,
    /// The column the key is another name for, where the table has one.
    alias: Option<usize>,
    /// What each column of the table converts a value under.
    affinities: &'a [Affinity],
    /// What the statement says to do where a row shares a key.
    conflict: Conflict,
    /// The columns the `RETURNING` of the statement answers.
    returning: crate::ast::Range,
    /// The order every row is held to the keys of the table in.
    order: &'a [Option<usize>],
    /// The collations the application defined on the connection.
    collating: &'a [crate::value::Collating],
}

/// What a row that shares a key with one the table holds does.
enum Conflicted {
    /// Nothing shares a key with the row, so the statement writes it.
    Write,
    /// The row is passed over.
    Over,
    /// A clause wrote the row the conflict found, and how many rows it
    /// wrote.
    Wrote(i64),
}

/// What one `DO UPDATE` writes: the clause, the table it is over and
/// the row the statement would have written.
struct Upserting<'a> {
    /// The tree the statement was read from.
    arena: &'a Arena,
    /// The text of the statement.
    sql: &'a [u8],
    /// The clause the conflict reached.
    clause: &'a crate::ast::Upsert,
    /// The table the row belongs to.
    table: &'a Table,
    /// The indexes over the table.
    kept: &'a [Kept],
    /// The tree of the table.
    root: u32,
    /// The column the key is another name for, where the table has one.
    alias: Option<usize>,
    /// The row the statement would have written.
    proposed: &'a [Value],
    /// The key the row the statement would have written was given.
    given: i64,
    /// The key of the row the conflict found.
    rowid: i64,
    /// The columns the `RETURNING` of the statement answers.
    returning: crate::ast::Range,
}

/// One `DO UPDATE` over a table that keeps its rows in the key's own
/// tree, where a key names a row and a rowid does not.
struct Keying<'a> {
    /// The tree the statement was read from.
    arena: &'a Arena,
    /// The text of the statement.
    sql: &'a [u8],
    /// The clause the conflict reached.
    clause: &'a crate::ast::Upsert,
    /// The table the row belongs to.
    table: &'a Table,
    /// The indexes over the table.
    kept: &'a [Kept],
    /// The tree of the table.
    root: u32,
    /// What each column converts a value under, in the order the record
    /// holds them.
    affinities: &'a [Affinity],
    /// The row the statement would have written.
    proposed: &'a [Value],
    /// The key of the row the conflict found.
    found: &'a [Value],
    /// The columns the `RETURNING` of the statement answers.
    returning: crate::ast::Range,
}

/// A count of rows as the whole number it is.
fn counted(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// The columns the `FROM` of an `UPDATE` answers and the rows of it to
/// write each row of the table against, which is one row of nothing
/// where the statement wrote no clause.
fn beside(
    joined: Option<&(Vec<crate::db::Beside>, Vec<Vec<Value>>)>,
) -> (&[crate::db::Beside], Vec<Option<&Vec<Value>>>) {
    match joined {
        None => (&[], alloc::vec![None]),
        Some((columns, rows)) => (columns.as_slice(), rows.iter().map(Some).collect()),
    }
}

/// One row of the `FROM` of an `UPDATE`, which the row of the table
/// reads the columns of the other tables through.
struct Aside<'a> {
    /// The columns the clause answers.
    columns: &'a [crate::db::Beside],
    /// The values of this row.
    values: &'a [Value],
    /// The row a trigger's body reads, where the statement is one of a
    /// body.
    outer: Option<&'a dyn crate::eval::Row>,
}

impl crate::eval::Row for Aside<'_> {
    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        if schema.is_some_and(|name| !name.eq_ignore_ascii_case(b"main")) {
            return None;
        }
        let found = self.columns.iter().enumerate().find(|(_, held)| {
            held.name.eq_ignore_ascii_case(column)
                && table.is_none_or(|name| name.eq_ignore_ascii_case(&held.from))
        });
        let Some((at, held)) = found else {
            return self
                .outer
                .and_then(|outer| outer.column(schema, table, column));
        };
        Some((
            self.values.get(at).cloned().unwrap_or(Value::Null),
            held.affinity,
            held.collation,
        ))
    }
}

/// The row a `DO UPDATE` reads: the row the conflict found under the
/// name of the table, and the row the statement would have written
/// under the name `excluded`, which is `sqlite3UpsertDoUpdate`.
struct Excluded<'a> {
    /// The table both rows belong to.
    table: &'a Table,
    /// The row the conflict found, with its key.
    held: (&'a [Value], i64),
    /// The row the statement would have written, with the key it was
    /// given.
    proposed: (&'a [Value], i64),
    /// What encoding the file keeps its text in.
    encoding: Encoding,
}

impl crate::eval::Row for Excluded<'_> {
    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        // `excluded` is a cursor and not a table, so a name with a
        // schema in front of it never reaches it.
        let excluded = table
            .filter(|_| schema.is_none())
            .is_some_and(|name| name.eq_ignore_ascii_case(b"excluded"));
        if !excluded
            && let Some(name) = table
            && !name.eq_ignore_ascii_case(&self.table.name)
        {
            return None;
        }
        let (values, rowid) = if excluded { self.proposed } else { self.held };
        let at = self
            .table
            .columns
            .iter()
            .position(|column_of| column_of.name.eq_ignore_ascii_case(column));
        match at {
            Some(at) => {
                let column_of = self.table.columns.get(at)?;
                Some((
                    values.get(at)?.clone(),
                    column_of.affinity,
                    column_of.collation,
                ))
            }
            None if is_rowid(column) => {
                Some((Value::Int(rowid), Affinity::Integer, Collation::Binary))
            }
            None => None,
        }
    }
}

/// The columns of one key, each with the collation its entries are held
/// in, which is what an `ON CONFLICT` clause is held to.
type Keys = Vec<(Vec<u8>, Collation)>;

/// One term of an `ON CONFLICT` clause: the column by name, and the
/// collation the clause writes on it where it writes one.
type Target = (Vec<u8>, Option<Collation>);

/// The terms an `ON CONFLICT` clause names, and nothing where a term is
/// not a column.
fn targets_of(
    arena: &Arena,
    sql: &[u8],
    clause: &crate::ast::Upsert,
    collating: &[crate::value::Collating],
) -> Option<Vec<Target>> {
    arena
        .orders(clause.targets)
        .iter()
        .map(|term| {
            let (named, written) = crate::schema::collated(arena, term.expr, sql, collating);
            Some((crate::schema::dequote(named?.text(sql)), written))
        })
        .collect()
}

/// Which `ON CONFLICT` clause a conflict reaches, which is
/// `sqlite3UpsertOfIndex`: the clause whose columns are the columns of
/// the index the row shares a key with, or the clause that names no
/// columns at all.
fn upsert_of<'a>(
    arena: &'a Arena,
    sql: &[u8],
    upserts: &'a [crate::ast::Upsert],
    wanted: &Keys,
    collating: &[crate::value::Collating],
) -> Option<&'a crate::ast::Upsert> {
    upserts.iter().find(|clause| {
        clause.targets.is_empty()
            || targets_of(arena, sql, clause, collating)
                .is_some_and(|targets| names_key(wanted, &targets))
    })
}

/// Whether an `ON CONFLICT` clause names a key, which is
/// `sqlite3UpsertAnalyzeTarget`: the clause names as many terms as the
/// key has columns, and every column of the key is one the clause names
/// under the collation the key holds it in or under none, because
/// `sqlite3ExprCompare` answers one rather than two for a `COLLATE` one
/// side writes and the other does not.
///
/// Comparing a key of `k` columns costs O(k^2).
fn names_key(key: &Keys, targets: &[Target]) -> bool {
    key.len() == targets.len()
        && key.iter().all(|(name, collation)| {
            targets.iter().any(|(wanted, written)| {
                wanted.eq_ignore_ascii_case(name)
                    && written.is_none_or(|written| written == *collation)
            })
        })
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
                // `sqlite3Insert` names the table and the column it does
                // not hold.
                None => Err(Error::NoNamedColumn(table.name.clone(), name.clone())),
            }
        })
        .collect()
}

/// Where each `SET` clause of an `UPDATE` over a view writes, or
/// nothing where the clause names the key of the view, which a view has
/// none of and `sqlite3Update` writes nowhere.
///
/// # Errors
///
/// [`Error::Eval`] for a column the view does not answer.
fn set_places(table: &Table, columns: &[Vec<u8>]) -> Result<Vec<Option<usize>>, Error> {
    columns
        .iter()
        .map(|column| {
            let at = table
                .columns
                .iter()
                .position(|held| held.name.eq_ignore_ascii_case(column));
            match at {
                Some(at) => Ok(Some(at)),
                None if is_rowid(column) => Ok(None),
                None => Err(Error::Eval(crate::eval::Error::NoColumn(column.clone()))),
            }
        })
        .collect()
}

/// Every value of a row held to the type of the column it goes in,
/// which is `OP_TypeCheck`: a column of a `STRICT` table holds a value
/// of its own type and nothing else, and every column holds nothing.
///
/// The affinity of the column is applied first, so a column of `INT`
/// holds the text `'3'` as the number 3 and one of `REAL` holds the
/// number 1 as 1.0.
///
/// Holding one row costs O(n) in its columns.
///
/// # Errors
///
/// [`Error::StoredType`] names the type of the value and the type of
/// the column.
fn stored_types(table: &Table, values: &mut [Value]) -> Result<(), Error> {
    if !table.strict {
        return Ok(());
    }
    for (value, column) in values.iter_mut().zip(&table.columns) {
        crate::value::apply(value, column.affinity);
        // A column of `REAL` holds a whole number as a real, which is
        // `MEM_IntReal`.
        if column.declared == b"REAL"
            && let Value::Int(number) = *value
        {
            *value = Value::Real(crate::value::integer_as_real(number));
        }
        let Some(held) = refused_as(&column.declared, value) else {
            continue;
        };
        return Err(Error::StoredType(
            held.to_vec(),
            column.declared.clone(),
            table.name.clone(),
            column.name.clone(),
        ));
    }
    Ok(())
}

/// The type of a value as `vdbeMemTypeName` writes it, where a column
/// of a `STRICT` table that declares `declared` may not hold it, and
/// nothing where it may.
///
/// A column holds nothing whatever its type says, and a column of `ANY`
/// holds whatever it is given.
const fn refused_as(declared: &[u8], value: &Value) -> Option<&'static [u8]> {
    match (declared, value) {
        (_, Value::Null)
        | (b"INT" | b"INTEGER", Value::Int(_))
        | (b"REAL", Value::Real(_))
        | (b"TEXT", Value::Text(_))
        | (b"BLOB", Value::Blob(_)) => None,
        (b"INT" | b"INTEGER" | b"REAL" | b"TEXT" | b"BLOB", value) => Some(match value {
            Value::Int(_) => b"INT",
            Value::Real(_) => b"REAL",
            Value::Text(_) => b"TEXT",
            _ => b"BLOB",
        }),
        _ => None,
    }
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
        self.located(&name)?;
        self.holds_index(&name, statement.indexed, sql)?;
        if self.is_view(&name)? {
            return self.delete_view(arena, statement, sql, &name, outer);
        }
        if self.keeps_rows(&name)? {
            return self.delete_keyed(arena, statement, sql, outer, &name);
        }
        let (root, keys, kept, table) = {
            let bytes = self.images();
            let database = self.reading_beside(&bytes)?.counting(self.counted);
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
                    rowid: Some(*rowid),
                    encoding: self.held.header.encoding,
                    random: &self.random,
                    clock: self.clock.map(crate::date::julian_of),
                    sensitive: self.truth.sensitive,
                    counted: self.counted,
                    defined: self.defined,
                    grouped: self.grouped,
                    collating: self.collating,
                    outer,
                    reading: Some(Reading {
                        database: &database,
                        arena,
                        sql,
                    }),
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
                encoding: self.held.header.encoding,
            };
            if fires && !self.fire(&before, &[], &row)? {
                continue;
            }
            // `sqlite3GenerateRowDelete` of
            // `research/sqlite/src/delete.c:625` reads the row again
            // after the triggers before it have run, so a trigger whose
            // body took the row away leaves this statement nothing to
            // take.
            if fires && !crate::tree::holds(&self.held.pages, root, key)? {
                continue;
            }
            // `D.2` of `src/fkey.c`: a row that rows of another table
            // point at is refused, or those rows are written, by what
            // the key says happens.
            self.unparented(&table, &values, key)?;
            self.orphaned(&name, &table, &values, key, None)?;
            self.tells_peek(Did::Delete, &table, (key, key), (Some(&values), None));
            self.unindex_row(&kept, &table, &values, &keyed_as(key))?;
            crate::tree::remove(&mut self.held.pages, root, key)?;
            self.tells_write(Did::Delete, &table, key);
            taken = taken.saturating_add(1);
            self.writing = taken;
            let answered = (&table, values.as_slice(), Some(key));
            self.returns(arena, statement.returning, sql, answered)?;
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
        let sets = self.writing(arena, statement, sql);
        let bytes = self.images();
        let database = self.reading_beside(&bytes)?.counting(self.counted);
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
                    None => Err(Error::Eval(crate::eval::Error::NoColumn(column.clone()))),
                }
            })
            .collect::<Result<_, Error>>()?;
        let affinities = ordered_affinities(table);
        // The tables of a `FROM` are answered once, and every row of
        // the table is written against the first row of theirs the
        // `WHERE` holds for, which is what `sqlite3Update` does with a
        // join that answers each row of the table once.
        let joined = match statement.from {
            None => None,
            Some(id) => Some(database.joined(arena, id, sql)?),
        };
        let (columns, sides) = beside(joined.as_ref());
        let mut written = Vec::new();
        for (rowid, values) in database.rows_of(name)? {
            // A row of the table that the clause holds more than one
            // row for is written with the last of them, which is the
            // ephemeral table `sqlite3Update` writes the rows into.
            let mut found: Option<(i64, Vec<Value>)> = None;
            for side in &sides {
                let aside = side.map(|row| Aside {
                    columns,
                    values: row,
                    outer,
                });
                let held = Held {
                    table,
                    values: &values,
                    rowid: Some(rowid),
                    encoding: self.held.header.encoding,
                    random: &self.random,
                    clock: self.clock.map(crate::date::julian_of),
                    sensitive: self.truth.sensitive,
                    counted: self.counted,
                    defined: self.defined,
                    grouped: self.grouped,
                    collating: self.collating,
                    outer: aside
                        .as_ref()
                        .map(|one| -> &dyn crate::eval::Row { one })
                        .or(outer),
                    reading: Some(Reading {
                        database: &database,
                        arena,
                        sql,
                    }),
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
                for (at, set) in places.iter().zip(&sets) {
                    let value = crate::eval::evaluate_row(arena, set.value, sql, &held)?;
                    match at {
                        Some(at) => {
                            for slot in next.iter_mut().skip(*at).take(1) {
                                slot.clone_from(&value);
                            }
                        }
                        None => match value {
                            Value::Int(given) => key = given,
                            _ => return Err(Error::Mismatch),
                        },
                    }
                }
                found = Some((key, next));
            }
            if let Some((key, next)) = found {
                written.push((rowid, key, values, next));
            }
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
        self.located(&name)?;
        self.holds_index(&name, statement.indexed, sql)?;
        if self.is_view(&name)? {
            return self.update_view(arena, statement, sql, &name, outer);
        }
        if self.keeps_rows(&name)? {
            return self.update_keyed(arena, statement, sql, outer, &name);
        }
        let sets = self.writing(arena, statement, sql);
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
            // A row this statement wrote over under `OE_Replace` is
            // gone, which is ticket #2832: the rows were read before
            // the first of them was written, so a row the statement
            // took out on the way is passed over rather than written
            // again.
            if !crate::tree::holds(&self.held.pages, root, rowid)? {
                continue;
            }
            // The column the key is another name for says the key, so a
            // statement that writes that column writes the key, and the
            // row holds no value for that column.
            let mut named;
            (key, named) = Self::rekeying(&mut values, alias, key)?;
            let encoding = self.held.header.encoding;
            if fires {
                let row = changing(&table, (&held, rowid), (&named, key), encoding);
                if !self.fire(&before, &columns, &row)?
                    // `sqlite3Update` reads the row again after the
                    // triggers before it have run, so a trigger whose
                    // body took the row away leaves this statement
                    // nothing to write.
                    || !crate::tree::holds(&self.held.pages, root, rowid)?
                {
                    continue;
                }
            }
            if !self.constrained(&table, &mut named, key, statement.conflict)? {
                continue;
            }
            Self::refilled(&named, &mut values, alias);
            let row = changing(&table, (&held, rowid), (&named, key), encoding);
            // A row that keeps the key it had shares it with nothing.
            if key != rowid && !self.keyed(root, &kept, &table, alias, key, statement.conflict)? {
                continue;
            }
            if !self.shared_key(
                root,
                &kept,
                &table,
                (&named, key, rowid),
                statement.conflict,
            )? {
                continue;
            }
            let record = crate::record::write_in(
                &ordered(&table, &values),
                &affinities,
                4,
                self.held.header.encoding,
            );
            // `I.1` of `src/fkey.c` over the row as it will stand, and
            // `D.2` over the row as it stands: a row that points at no
            // row is refused, and so is one that rows point at.
            self.unparented(&table, &held, rowid)?;
            self.parented(&table, &named, key)?;
            self.orphaned(&name, &table, &held, rowid, Some((&named, key)))?;
            // `sqlite3Update` removes the entries of the row, removes
            // the row itself where the key changes, and then writes
            // the new entries before the new row.
            self.tells_peek(
                Did::Update,
                &table,
                (rowid, key),
                (Some(&held), Some(&named)),
            );
            self.unindex_row(&kept, &table, &held, &keyed_as(rowid))?;
            let moved = key != rowid;
            if moved {
                crate::tree::remove(&mut self.held.pages, root, rowid)?;
            }
            self.index_row(&kept, &table, &named, &keyed_as(key))?;
            if moved {
                insert(&mut self.held.pages, root, key, &record)?;
            } else {
                crate::tree::update(&mut self.held.pages, root, rowid, &record)?;
            }
            self.tells_write(Did::Update, &table, key);
            changed = changed.saturating_add(1);
            self.writing = changed;
            self.returns(arena, statement.returning, sql, (&table, &named, Some(key)))?;
            if fires {
                self.fire(&after, &columns, &row)?;
            }
        }
        Ok(changed)
    }
}

impl Writer {
    /// What a row an `UPDATE` writes does where it shares the key of a
    /// unique index with a row the table holds, which is
    /// `sqlite3GenerateConstraintChecks` reading the clause: `false`
    /// where the row is passed over and `true` where it is written.
    ///
    /// The row is the values it will hold, the key it will stand
    /// under, and the key it stands under now.
    ///
    /// # Errors
    ///
    /// [`Error::Unique`] names the key two rows share.
    fn shared_key(
        &mut self,
        root: u32,
        kept: &[Kept],
        table: &Table,
        row: (&[Value], i64, i64),
        written: Conflict,
    ) -> Result<bool, Error> {
        let (named, key, rowid) = row;
        let found = self.conflicting(kept, table, named, &keyed_as(key), Some(&keyed_as(rowid)))?;
        let Some((index, other)) = found else {
            return Ok(true);
        };
        let answer = resolved(written, kept, index);
        match answer {
            Conflict::Ignore => return Ok(false),
            Conflict::Replace if self.replaced(root, kept, table, &other)? => return Ok(true),
            _ => {}
        }
        self.refusing(answer);
        Err(Error::Unique(Self::shown_key_of(table, kept, index)))
    }

    /// The entry every index over the table holds for one row, written,
    /// which is what `sqlite3GenerateConstraintChecks` writes beside
    /// the row.
    fn index_row(
        &mut self,
        kept: &[Kept],
        table: &Table,
        values: &[Value],
        tail: &[Value],
    ) -> Result<(), Error> {
        let encoding = self.held.header.encoding;
        for held in kept {
            let over = held.over(table, encoding);
            if !indexes_row(&held.index, &over, values)? {
                continue;
            }
            let key = entry_of(&held.index, &over, values, tail)?;
            let plain = alloc::vec![Affinity::None; key.len()];
            let entry = crate::record::write_in(&key, &plain, 4, self.held.header.encoding);
            let pages = &mut self.held.pages;
            let order = ordering(&held.collations, encoding);
            crate::tree::insert_entry(pages, held.root, &entry, &key, order, false)?;
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
        found: &[Value],
    ) -> Result<bool, Error> {
        // The entry names the key of the row it belongs to, so the row
        // the statement writes over is the one that key finds.
        let (rowid, values) = {
            let bytes = self.image();
            let database = self.reading(&bytes)?;
            database
                .rows_of(&table.name)?
                .into_iter()
                .find(|(key, _)| found == [Value::Int(*key)])
                .ok_or(Error::NoTable(Vec::new()))?
        };
        let (before, after) = self.deleting(&table.name)?;
        let row = Fired {
            table,
            old: Some((&values, rowid)),
            new: None,
            encoding: self.held.header.encoding,
        };
        // A trigger that says the row stays leaves the key where it
        // was, so what the statement writes shares a key with it.
        if !self.fire(&before, &[], &row)? {
            return Ok(false);
        }
        // The row goes, so its own keys point at no row and the rows
        // that point at it are acted on, which
        // `sqlite3GenerateRowDelete` of
        // `research/sqlite/src/delete.c:625` does under `OE_Replace` as
        // it does for a `DELETE`.
        self.tells_peek(Did::Delete, table, (rowid, rowid), (Some(&values), None));
        self.unparented(table, &values, rowid)?;
        self.orphaned(&table.name, table, &values, rowid, None)?;
        self.unindex_row(kept, table, &values, &keyed_as(rowid))?;
        crate::tree::remove(&mut self.held.pages, root, rowid)?;
        self.fire(&after, &[], &row)?;
        Ok(true)
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
        table: &Table,
        values: &[Value],
        tail: &[Value],
        held: Option<&[Value]>,
    ) -> Result<Option<(usize, Vec<Value>)>, Error> {
        for (at, one) in kept.iter().enumerate() {
            if let Some(found) = self.conflicts_at(one, table, (values, tail), held)? {
                return Ok(Some((at, found)));
            }
        }
        Ok(None)
    }

    /// The key of the row the index `at` already holds the entry of
    /// this row's key under, or nothing where it holds none, which is
    /// the lookup `sqlite3GenerateConstraintChecks` writes per index.
    ///
    /// The row is the values it holds and the key it stands under.
    ///
    /// # Errors
    ///
    /// [`Error`] names what an expression of the index could not answer.
    fn conflicts_at(
        &self,
        one: &Kept,
        table: &Table,
        row: (&[Value], &[Value]),
        held: Option<&[Value]>,
    ) -> Result<Option<Vec<Value>>, Error> {
        let (values, tail) = row;
        if !one.index.unique {
            return Ok(None);
        }
        let over = one.over(table, self.held.header.encoding);
        if !indexes_row(&one.index, &over, values)? {
            return Ok(None);
        }
        let entry = entry_of(&one.index, &over, values, tail)?;
        let key = entry.get(..one.index.columns.len()).unwrap_or_default();
        if key.contains(&Value::Null) {
            return Ok(None);
        }
        let width = tail.len();
        let order = ordering(&one.collations, self.held.header.encoding);
        let found = crate::tree::entry_tail_at(&self.held.pages, one.root, key, order, width)?;
        Ok(found.filter(|found| held != Some(found.as_slice())))
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
        // A constraint reads the schema and no row: a `CHECK` reaches
        // no table and a generated column reaches none either, so the
        // bytes taken under one schema cookie answer for every row a
        // statement writes under it. Building them per row costs O(n)
        // in the pages of the file, which is O(n²) over a statement that
        // writes n rows.
        let cookie = self.held.header.schema_cookie;
        let bytes = match self.held.schema_bytes.take() {
            Some((held, bytes)) if held == cookie => bytes,
            _ => self.image(),
        };
        let answered = self.checked(&bytes, table, values, rowid, written);
        self.held.schema_bytes = Some((cookie, bytes));
        answered
    }

    /// The same, against the bytes the schema was read out of.
    ///
    /// # Errors
    ///
    /// Whatever the constraints of the table refuse the row with.
    fn checked(
        &mut self,
        bytes: &[u8],
        table: &Table,
        values: &mut [Value],
        rowid: i64,
        written: Conflict,
    ) -> Result<bool, Error> {
        let database = self.reading(bytes)?;
        // `sqlite3ComputeGeneratedColumns` runs before the opcode that
        // holds the row to the types of the table and before the
        // constraints.
        database.compute_row(&table.name, values)?;
        stored_types(table, values)?;
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
                    slot.clone_from(&held);
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
            if answer == Conflict::Ignore {
                return Ok(false);
            }
            self.refusing(answer);
            return Err(Error::NotNull(shown));
        }
        let row = Held {
            table,
            values,
            rowid: Some(rowid),
            encoding: self.held.header.encoding,
            random: &self.random,
            clock: self.clock.map(crate::date::julian_of),
            sensitive: self.truth.sensitive,
            counted: self.counted,
            defined: self.defined,
            grouped: self.grouped,
            collating: self.collating,
            outer: None,
            reading: None,
        };
        // `PRAGMA ignore_check_constraints` leaves every `CHECK` of
        // the table unread.
        if self.told(b"ignore_check_constraints") != 0 {
            return Ok(true);
        }
        let Some(shown) = database.refused_check(&table.name, &row)? else {
            return Ok(true);
        };
        if written == Conflict::Ignore {
            return Ok(false);
        }
        self.refusing(written);
        Err(Error::Check(shown))
    }

    /// What a broken constraint does to the statement beyond refusing
    /// the row: `OE_Fail` stops the statement where it stands and
    /// `OE_Rollback` undoes the transaction it runs in.
    const fn refusing(&mut self, written: Conflict) {
        self.refusing = match written {
            Conflict::Fail => Refusing::Fail,
            Conflict::Rollback => Refusing::Rollback,
            _ => Refusing::Abort,
        };
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
        if !crate::tree::holds(&self.held.pages, root, key)? {
            return Ok(true);
        }
        match written {
            Conflict::Ignore => return Ok(false),
            Conflict::Replace if self.replaced(root, kept, table, &keyed_as(key))? => {
                return Ok(true);
            }
            _ => {}
        }
        self.refusing(written);
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
    /// `table.column`, and `table.rowid` where the table has no column
    /// the key is another name for, which `sqlite3RowidConstraint` of
    /// `research/sqlite/src/build.c:4254` writes.
    fn shown_column(table: &Table, at: Option<usize>) -> Vec<u8> {
        let mut out = table.name.clone();
        out.push(b'.');
        let named = at
            .and_then(|at| table.columns.get(at))
            .map_or(b"rowid".as_slice(), |column| column.name.as_slice());
        out.extend_from_slice(named);
        out
    }

    /// The column the key of the table is another name for, or an empty
    /// name where the table has none.
    fn key_column(table: &Table, alias: Option<usize>) -> Keys {
        // The key of a table that keeps its rows in the key's own tree
        // is the columns of its `PRIMARY KEY`, each in its own
        // collation.
        if table.without_rowid {
            return crate::schema::key_places(table)
                .iter()
                .filter_map(|at| table.columns.get(*at))
                .map(|column| (column.name.clone(), column.collation))
                .collect();
        }
        let name = alias
            .and_then(|at| table.columns.get(at))
            .map(|column| column.name.clone())
            .unwrap_or_default();
        // The key of a table is one whole number, which no collation
        // reads.
        alloc::vec![(name, Collation::Binary)]
    }

    /// The columns the index `at` is over, each with the collation its
    /// entries are held in.
    fn key_columns(table: &Table, kept: &[Kept], at: usize) -> Option<Keys> {
        let held = kept.get(at)?;
        // An `ON CONFLICT` clause names no index that holds a place
        // over an expression, and none that holds entries for fewer
        // rows than the table has.
        if held.index.filter.is_some() {
            return None;
        }
        held.index
            .columns
            .iter()
            .map(|keyed| {
                let column = keyed.place().and_then(|at| table.columns.get(at))?;
                Some((column.name.clone(), keyed.collation))
            })
            .collect()
    }

    /// The key a message names, as `sqlite3UniqueConstraint` writes it:
    /// `table.column` per column with a comma between them, or `index
    /// 'NAME'` for an index that holds a place over an expression.
    fn shown_key_of(table: &Table, kept: &[Kept], at: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let columns = kept
            .get(at)
            .map_or(&[][..], |held| held.index.columns.as_slice());
        if columns.iter().any(|keyed| keyed.place().is_none()) {
            let named = kept
                .get(at)
                .map_or(&[][..], |held| held.index.name.as_slice());
            out.extend_from_slice(b"index '");
            out.extend_from_slice(named);
            out.push(b'\'');
            return out;
        }
        for keyed in columns {
            if !out.is_empty() {
                out.extend_from_slice(b", ");
            }
            out.extend_from_slice(&table.name);
            out.push(b'.');
            let named = keyed
                .place()
                .and_then(|at| table.columns.get(at))
                .map_or(&[][..], |column| column.name.as_slice());
            out.extend_from_slice(named);
        }
        out
    }

    /// The entry every index over the table holds for one row, taken
    /// out, which is `sqlite3GenerateRowIndexDelete`.
    fn unindex_row(
        &mut self,
        kept: &[Kept],
        table: &Table,
        values: &[Value],
        tail: &[Value],
    ) -> Result<(), Error> {
        let encoding = self.held.header.encoding;
        for held in kept {
            let over = held.over(table, encoding);
            if !indexes_row(&held.index, &over, values)? {
                continue;
            }
            let key = entry_of(&held.index, &over, values, tail)?;
            let order = ordering(&held.collations, self.held.header.encoding);
            crate::tree::remove_entry(&mut self.held.pages, held.root, &key, order)?;
        }
        Ok(())
    }
}

/// One index over a table as a statement that writes rows reads it.
#[derive(Clone)]
struct Kept {
    /// The index as its statement describes it.
    index: crate::schema::Index,
    /// How each of its places compares.
    collations: Vec<crate::value::Placing>,
    /// The page its tree begins on.
    root: u32,
    /// The `CREATE INDEX` text, which a place over an expression and a
    /// partial index's `WHERE` point into, and empty for an index that
    /// holds neither.
    sql: Vec<u8>,
    /// The tree that text was parsed into, and empty for an index that
    /// holds neither.
    arena: Arena,
}

impl Kept {
    /// What the expressions of this index are read against, over
    /// `table`.
    fn over<'a>(&'a self, table: &'a Table, encoding: Encoding) -> Over<'a> {
        Over {
            arena: &self.arena,
            sql: &self.sql,
            table,
            encoding,
        }
    }
}

/// Every index over the table `name`, read once so that the statement
/// keeps them while it writes the rows the database answered.
fn kept_indexes(database: &Database<'_>, name: &[u8]) -> Vec<Kept> {
    let format = database.schema_format();
    database
        .indexes(name)
        .iter()
        .map(|kept| {
            // The statement that made the index is read for a place
            // over an expression and for a partial index's `WHERE`, and
            // for nothing else, so an index that holds neither is kept
            // without it.
            let reads = reads_expressions(kept.index);
            Kept {
                index: kept.index.clone(),
                collations: collations_of(kept.index, format),
                root: kept.root,
                sql: if reads { kept.sql.to_vec() } else { Vec::new() },
                arena: if reads {
                    kept.arena.clone()
                } else {
                    Arena::default()
                },
            }
        })
        .collect()
}

/// Whether an index holds a place over an expression or a `WHERE`,
/// which are what the statement that made it is read for.
fn reads_expressions(index: &crate::schema::Index) -> bool {
    index.filter.is_some() || index.columns.iter().any(|keyed| keyed.place().is_none())
}

/// What the expressions of an index are read against: the statement
/// that made the index, the table it is over, and the encoding the file
/// keeps its text in.
#[derive(Clone, Copy)]
pub(crate) struct Over<'a> {
    /// The tree the `CREATE INDEX` was parsed into.
    pub arena: &'a Arena,
    /// The `CREATE INDEX` text the tree points into.
    pub sql: &'a [u8],
    /// The table the index is over.
    pub table: &'a Table,
    /// What encoding the file keeps its text in.
    pub encoding: Encoding,
}

/// What the statement running now does beyond refusing the row where
/// it breaks a constraint, which is the `ON CONFLICT` clause the
/// constraint carries.
#[derive(Clone, Copy)]
enum Refusing {
    /// `OE_Abort`: the statement leaves the file as it found it, and a
    /// transaction it runs inside stays open.
    Abort,
    /// `OE_Fail`: the statement stops where it stands, so the rows it
    /// wrote before that stand.
    Fail,
    /// `OE_Rollback`: the transaction the statement runs inside is
    /// undone and ended.
    Rollback,
}

/// One row of a table as the expressions of an index read it.
struct Indexing<'a> {
    /// The table.
    table: &'a Table,
    /// The value of each column, in the order the table was created
    /// with.
    values: &'a [Value],
    /// What encoding the file keeps its text in.
    encoding: Encoding,
}

impl crate::eval::Row for Indexing<'_> {
    fn encoding(&self) -> Encoding {
        self.encoding
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
            .position(|held| held.name.eq_ignore_ascii_case(column))?;
        let held = self.table.columns.get(at)?;
        Some((self.values.get(at)?.clone(), held.affinity, held.collation))
    }
}

/// Whether a partial index holds an entry for one row, which is
/// `sqlite3ExprIfFalse` over `pPartIdxWhere`: a row the `WHERE` answers
/// anything but true for has no entry.
///
/// Reading one row costs what the `WHERE` costs.
///
/// # Errors
///
/// [`Error`] names what the `WHERE` could not answer.
pub(crate) fn indexes_row(
    index: &crate::schema::Index,
    over: &Over<'_>,
    values: &[Value],
) -> Result<bool, Error> {
    let Some(filter) = index.filter else {
        return Ok(true);
    };
    let row = Indexing {
        table: over.table,
        values,
        encoding: over.encoding,
    };
    let value = crate::eval::evaluate_row(over.arena, filter, over.sql, &row)?;
    Ok(value.truth(false))
}

/// The entry an index holds for one row: what it holds at each of its
/// places, and the key of the row after them, which is what makes its
/// order total.
///
/// A place over an expression holds what that expression answers for
/// the row, which is `sqlite3GenerateIndexKey` reading `aColExpr`. The
/// key of a row is its rowid, one value, or the columns of the `PRIMARY
/// KEY` where the table keeps its rows in the key's own tree.
///
/// Reading one row costs what the expressions of the index cost.
///
/// # Errors
///
/// [`Error`] names what an expression could not answer.
pub(crate) fn entry_of(
    index: &crate::schema::Index,
    over: &Over<'_>,
    values: &[Value],
    tail: &[Value],
) -> Result<Vec<Value>, Error> {
    let row = Indexing {
        table: over.table,
        values,
        encoding: over.encoding,
    };
    let mut key = Vec::new();
    for keyed in &index.columns {
        key.push(match keyed.of {
            crate::schema::Of::Place(at) => {
                // `sqlite3TableAffinity` converts the row before both
                // the row and its entries are written, so an entry
                // holds the value the row holds and not the one the
                // statement wrote.
                let mut value = values.get(at).cloned().unwrap_or(Value::Null);
                let affinity = over
                    .table
                    .columns
                    .get(at)
                    .map_or(Affinity::None, |column| column.affinity);
                crate::value::apply(&mut value, affinity);
                value
            }
            crate::schema::Of::Term(term) => {
                crate::eval::evaluate_row(over.arena, term, over.sql, &row)?
            }
        });
    }
    key.extend_from_slice(tail);
    Ok(key)
}

/// The values of one row in the order the table stores them, which is
/// the columns of the key first where the table keeps its rows in the
/// key's own tree, and the columns a generated column takes no place
/// for left out.
fn ordered(table: &Table, values: &[Value]) -> Vec<Value> {
    let places = crate::db::places(table);
    let width = places.iter().filter(|place| **place != usize::MAX).count();
    let mut out = alloc::vec![Value::Null; width];
    for (at, place) in places.iter().enumerate() {
        let value = values.get(at).unwrap_or(&Value::Null);
        for slot in out.iter_mut().skip(*place).take(1) {
            slot.clone_from(value);
        }
    }
    out
}

/// What each place of a stored row converts a value under, in the order
/// the table stores them.
fn ordered_affinities(table: &Table) -> Vec<Affinity> {
    let places = crate::db::places(table);
    let width = places.iter().filter(|place| **place != usize::MAX).count();
    let mut out = alloc::vec![Affinity::None; width];
    for (at, place) in places.iter().enumerate() {
        let affinity = table
            .columns
            .get(at)
            .map_or(Affinity::None, |column| column.affinity);
        for slot in out.iter_mut().skip(*place).take(1) {
            *slot = affinity;
        }
    }
    out
}

/// What the `PRIMARY KEY` of a table says to do where two rows share a
/// key, which is the clause the constraint carries.
fn key_conflict(table: &Table) -> Conflict {
    table
        .keys
        .iter()
        .find(|keys| keys.primary)
        .map_or(Conflict::Unspecified, |keys| keys.conflict)
}

/// The key of a row of a table with a rowid, as the entries of the
/// indexes over that table carry it.
const fn keyed_as(rowid: i64) -> [Value; 1] {
    [Value::Int(rowid)]
}

/// How each place of an index compares: the collation it is held in,
/// and whether it runs backwards, which `sqlite3CreateIndex` honors on a
/// file whose schema format is 4 and ignores on one below it.
pub(crate) fn collations_of(
    index: &crate::schema::Index,
    format: u32,
) -> Vec<crate::value::Placing> {
    index
        .columns
        .iter()
        .map(|column| crate::value::Placing {
            collation: column.collation,
            backwards: format >= 4 && column.order == crate::ast::Order::Descending,
        })
        .collect()
}

/// Where one index entry stands against another: column by column
/// under the collation each is held in, and the key of the row last,
/// which is what makes the order of an index total.
pub(crate) fn order_of_keys(
    one: &[Value],
    other: &[Value],
    collations: &[crate::value::Placing],
) -> core::cmp::Ordering {
    one.iter()
        .zip(other)
        .enumerate()
        .map(|(at, (mine, theirs))| {
            // The key of the row stands after the columns and is
            // compared as bytes, which is what the default here is.
            let placing = collations.get(at).copied().unwrap_or_default();
            crate::value::compare_placed(mine, theirs, placing)
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

/// How an index tree orders its entries, from the collations of the
/// index and the encoding of the file.
const fn ordering(
    collations: &[crate::value::Placing],
    encoding: Encoding,
) -> crate::tree::Ordering<'_> {
    crate::tree::Ordering {
        collations,
        encoding,
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

/// One foreign key of a table that points at another, with the name of
/// the table that points.
struct Points {
    /// The table the key is written on.
    child: Vec<u8>,
    /// The key itself.
    key: crate::schema::Foreign,
    /// What each column of the key is compared under, which is the
    /// affinity and the collation of the column of the parent, not of
    /// the child.
    under: Vec<(Affinity, Collation)>,
}

/// The key of a row as a rowid, which is the one value a table that
/// keeps no rows in the key's own tree holds there; a table that does
/// has no rowid for a name to answer.
fn keyed_rowid(key: &[Value]) -> i64 {
    key.first().map_or(0, Value::to_integer)
}

/// The largest root page a file holds once the root at `largest` is given
/// up, which is the page before it and the page before that where the
/// file keeps a pointer map there.
///
/// This is what `btreeDropTable` of `research/sqlite/src/btree.c` writes
/// into the header as `BTREE_LARGEST_ROOT_PAGE`. The page that carries
/// the byte a lock stands on lies past any file this crate writes, so no
/// count steps over it.
fn shrunk(pages: &crate::tree::Pages, largest: u32) -> u32 {
    let mut held = largest.saturating_sub(1);
    while held > 1 && pages.is_map(held) {
        held = held.saturating_sub(1);
    }
    held
}

/// The word a `DROP` names what it takes away by, which is what the
/// refusal writes.
fn dropped_word(kind: crate::ast::Dropped) -> Vec<u8> {
    match kind {
        crate::ast::Dropped::Table => b"table".to_vec(),
        crate::ast::Dropped::Index => b"index".to_vec(),
        crate::ast::Dropped::View => b"view".to_vec(),
        crate::ast::Dropped::Trigger => b"trigger".to_vec(),
    }
}

/// How many pages a count the connection was told stands for, which is
/// every page a number counts to where the count is larger than that.
fn capped(most: i64) -> u32 {
    u32::try_from(most).unwrap_or(u32::MAX)
}

/// The name of a table under the schema it stands in, which is `main`
/// for every table this crate holds.
/// Raises where a term of a `CREATE INDEX` or the `WHERE` of a partial
/// index calls a window function.
///
/// `sqlite3ResolveExprNames` of `research/sqlite/src/resolve.c` reads
/// the expressions of an index with `NC_AllowWin` clear, so a window
/// there is a misuse: an index holds one value per row and a window
/// function reads the rows of a frame.
///
/// Reading the tree costs O(n) in its nodes.
///
/// # Errors
///
/// [`crate::eval::Error::NoWindow`] names the function.
fn windowless(arena: &Arena, index: &crate::ast::CreateIndex, sql: &[u8]) -> Result<(), Error> {
    let mut held = alloc::vec::Vec::new();
    for term in arena.orders(index.columns) {
        held.extend(arena.node(term.expr));
    }
    held.extend(index.filter.and_then(|id| arena.node(id)));
    while let Some(node) = held.pop() {
        if let crate::ast::Node::Over { name, .. } = node {
            return Err(
                crate::eval::Error::NoWindow(crate::schema::dequote(name.text(sql))).into(),
            );
        }
        arena.under(node, |child| held.extend(arena.node(child)));
    }
    Ok(())
}

fn schema_named_as(name: &[u8]) -> Vec<u8> {
    let mut out = b"main.".to_vec();
    out.extend_from_slice(name);
    out
}

/// The places the parent columns of a foreign key take in the table it
/// points at, which is that table's primary key where the key names no
/// columns.
///
/// `sqlite3FkLocateIndex` refuses a key whose columns are not the
/// primary key and carry no unique index of their own.
fn parent_places(
    database: &Database<'_>,
    over: (&[u8], &crate::schema::Foreign),
    parent: &Table,
) -> Result<Vec<usize>, Error> {
    let (child, key) = over;
    let mismatch = || Error::ForeignMismatch(child.to_vec(), parent.name.clone());
    if key.parent.is_empty() {
        let mut places: Vec<usize> = (0..parent.columns.len())
            .filter(|at| parent.columns.get(*at).is_some_and(|column| column.key > 0))
            .collect();
        places.sort_by_key(|at| parent.columns.get(*at).map_or(0, |column| column.key));
        if places.len() != key.columns.len() {
            return Err(mismatch());
        }
        return Ok(places);
    }
    let mut places = Vec::new();
    for name in &key.parent {
        let at = parent
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(name))
            .ok_or_else(mismatch)?;
        places.push(at);
    }
    // The columns pointed at must be unique, which is the primary key
    // or a `UNIQUE` over exactly those columns. They are as many as the
    // columns that point, which `sqlite3CreateForeignKey` held the
    // `CREATE TABLE` to.
    // `sqlite3FkLocateIndex` reads each column of the index against
    // every column the key names, so the two hold the same names in
    // whatever order: `UNIQUE(y, x)` is the key `REFERENCES p(x, y)`
    // points at.
    let whole = |named: &[Vec<u8>]| {
        named.len() == places.len()
            && key
                .parent
                .iter()
                .all(|one| named.iter().any(|other| one.eq_ignore_ascii_case(other)))
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
            .filter_map(|keyed| keyed.place().and_then(|at| parent.columns.get(at)))
            .map(|column| column.name.clone())
            .collect();
        whole(&named)
    });
    if unique {
        return Ok(places);
    }
    // `sqlite3FkLocateIndex`: an index of the parent's own counts where
    // it is unique, holds those columns and holds them in the collation
    // each column compares under.
    let indexed = database.indexes(&parent.name).iter().any(|kept| {
        if !kept.index.unique {
            return false;
        }
        let named: Vec<Vec<u8>> = kept
            .index
            .columns
            .iter()
            .filter(|keyed| {
                keyed
                    .place()
                    .and_then(|at| parent.columns.get(at))
                    .is_some_and(|column| column.collation == keyed.collation)
            })
            .filter_map(|keyed| keyed.place().and_then(|at| parent.columns.get(at)))
            .map(|column| column.name.clone())
            .collect();
        named.len() == kept.index.columns.len() && whole(&named)
    });
    if indexed {
        return Ok(places);
    }
    Err(mismatch())
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
    for (key, values) in database.held_rows_of(name)? {
        let rowid = keyed_rowid(&key);
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
fn alike_values(held: &[Value], wanted: &[Value], under: &[(Affinity, Collation)]) -> bool {
    held.iter()
        .zip(wanted)
        .zip(under)
        .all(|((one, other), (affinity, collation))| {
            let mut left = one.clone();
            let mut right = other.clone();
            crate::value::apply_comparison(&mut left, &mut right, *affinity);
            crate::value::compare(&left, &right, *collation) == core::cmp::Ordering::Equal
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

/// Whether a name is one of the two the connection holds a schema place
/// for whatever it attached, which are `main` and `temp`.
const fn named_database(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"main") || name.eq_ignore_ascii_case(b"temp")
}

/// Whether an attached database answers to `name`.
fn named_as(held: &Attached, name: &[u8]) -> bool {
    held.called.name.eq_ignore_ascii_case(name)
}

/// The image of one file a connection holds, which is the file as
/// write-ahead logging began where no checkpoint has written a frame
/// back into it.
fn written_image(held: &HeldFile) -> Vec<u8> {
    match &held.origin {
        Some(bytes) => bytes.clone(),
        None => held.pages.written(&held.header),
    }
}

/// How many pages a `PRAGMA incremental_vacuum` gives up, which is what
/// `sqlite3GetInt32` of `research/sqlite/src/util.c` reads of the text
/// after the name.
///
/// `PragTyp_INCREMENTAL_VACUUM` gives up every page of the free list
/// where the text names no number above nought, which a number past
/// what a signed word holds is one of.
fn asked_steps(text: Option<&[u8]>) -> u32 {
    const EVERY: u32 = 0x7fff_ffff;
    let held = text
        .map(crate::schema::dequote)
        .and_then(|text| {
            let digits = text.strip_prefix(b"+").unwrap_or(&text).to_vec();
            crate::pragma::signed_number(&digits)
        })
        .and_then(|number| i32::try_from(number).ok());
    match held {
        Some(most) if most > 0 => most.cast_unsigned(),
        _ => EVERY,
    }
}

/// What a blob handle names: the database, the table, the column and the
/// key of the row, with whether the handle writes.
///
/// `sqlite3_blob_open` of `research/sqlite/src/vdbeblob.c:74` takes the
/// same five.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blob<'a> {
    /// The database the table stands in, and nothing for the one the
    /// connection writes.
    pub schema: Option<&'a [u8]>,
    /// The name of the table.
    pub table: &'a [u8],
    /// The name of the column.
    pub column: &'a [u8],
    /// The key of the row.
    pub rowid: i64,
    /// Whether the handle writes the value rather than reading it.
    pub writing: bool,
}

/// The table a blob handle names, written under the database it names,
/// which `sqlite3_blob_open` refuses `no such table: <database>.<table>`
/// for.
fn blob_named(asked: &Blob<'_>, held: &[u8]) -> Vec<u8> {
    let mut out = asked.schema.unwrap_or(held).to_vec();
    out.push(b'.');
    out.extend_from_slice(asked.table);
    out
}

/// Where the bytes of the value a blob handle names lie: the tree of
/// its table, where in the payload of its row the value begins, how many
/// bytes it holds, and the payload itself.
struct Placed {
    /// The page the tree of the table begins on.
    root: u32,
    /// Where in the payload the value begins.
    at: usize,
    /// How many bytes the value holds.
    len: usize,
    /// The payload of the row, whole.
    payload: Vec<u8>,
}

/// What one command of a blob handle does: answer how many bytes the
/// value holds, read bytes of it from an offset, or write bytes over it
/// from an offset.
enum Asking<'a> {
    /// `sqlite3_blob_bytes`.
    Bytes,
    /// `sqlite3_blob_read`, with where to read from and how many bytes.
    Read(usize, usize),
    /// `sqlite3_blob_write`, with where to write from and the bytes.
    Write(usize, &'a [u8]),
}

/// What holds the column at `at` of `table` where a handle may not write
/// it: a foreign key of the table where `keys` says the rows are held to
/// them, or an index over the column or over an expression.
///
/// `sqlite3_blob_open` of `research/sqlite/src/vdbeblob.c:211` reads the
/// foreign keys and then the indexes, and names the index where both
/// hold the column.
fn held_column(
    database: &Database<'_>,
    table: &crate::schema::Table,
    at: usize,
    keys: bool,
) -> Option<&'static [u8]> {
    let mut held = None;
    if keys && table.foreign.iter().any(|key| key.columns.contains(&at)) {
        held = Some(b"foreign key".as_slice());
    }
    if database.indexes(&table.name).iter().any(|kept| {
        kept.index
            .columns
            .iter()
            .any(|keyed| keyed.place().is_none_or(|place| place == at))
    }) {
        held = Some(b"indexed".as_slice());
    }
    held
}

/// The kind of page the tree of a table begins with: an index page for a
/// table written `WITHOUT ROWID`, which keeps its rows in the tree of its
/// key, and a table page for every other table.
const fn table_kind(without_rowid: bool) -> Kind {
    if without_rowid {
        Kind::LeafIndex
    } else {
        Kind::LeafTable
    }
}

/// One database of a connection with the transaction of the connection
/// open on it where `began` says the connection has one, which is what a
/// database an `ATTACH` added inside a transaction joins.
fn joined(held: HeldFile, began: bool) -> HeldFile {
    let mut held = held;
    if began {
        held.pages.begin();
        held.began = Some(held.header);
    }
    held
}

/// Whether a file name stands for a database of the connection's own
/// rather than a file the client holds, which `:memory:` and a name of
/// no bytes both do.
const fn fresh_file(file: &[u8]) -> bool {
    file.is_empty() || file.eq_ignore_ascii_case(b":memory:")
}

/// The text of a bare name, which `resolveAttachExpr` of
/// `research/sqlite/src/attach.c:35` reads as a string and not as a
/// column, and nothing for every other expression.
fn bare_text(arena: &Arena, id: crate::ast::ExprId, sql: &[u8]) -> Option<Vec<u8>> {
    match arena.node(id) {
        Some(crate::ast::Node::Column {
            schema: None,
            table: None,
            column,
        }) => Some(crate::schema::dequote(column.text(sql))),
        _ => None,
    }
}

/// The schema one statement of the schema names, and the table it names
/// under it where it names one.
///
/// A statement that names a table is refused `no such table:` and one
/// that names the database alone `unknown database`, which is what
/// `sqlite3TwoPartName` of `research/sqlite/src/build.c:596` answers.
const fn defined_under(definition: Definition) -> Names {
    match definition {
        Definition::Table(made) => Names::made(made.schema, None, made.temporary),
        // An index and a trigger both stand in the database of the table
        // they are over, which `sqlite3CreateIndex` and
        // `sqlite3FinishTrigger` read out of that table.
        Definition::Index(made) => Names::made(made.schema, Some(made.table), false),
        Definition::View(made) => Names::made(made.schema, None, made.temporary),
        Definition::Trigger(made) => Names::made(made.schema, Some(made.table), made.temporary),
        // A `DROP` of an index, a view or a trigger names none of them a
        // table, so only a `DROP TABLE` holds its name to the schema.
        Definition::Drop(asked) => Names::located(
            asked.schema,
            asked.name,
            if matches!(asked.kind, crate::ast::Dropped::Table) {
                Located::Table
            } else {
                Located::Name
            },
        ),
        Definition::Rename(asked) => Names::located(asked.schema, asked.table, Located::Table),
        Definition::AddColumn(asked) => Names::located(asked.schema, asked.table, Located::Table),
        Definition::DropColumn(asked) => Names::located(asked.schema, asked.table, Located::Table),
        Definition::RenameColumn(asked) => {
            Names::located(asked.schema, asked.table, Located::Table)
        }
        Definition::DropConstraint(asked) => {
            Names::located(asked.schema, asked.table, Located::Table)
        }
        Definition::Attach(_) | Definition::Detach(_) => Names::naming(),
        Definition::Vacuum(_) => Names::made(None, None, false),
    }
}

/// The schema one statement that changes rows names, and the table it
/// names under it.
const fn changed_under(change: Change) -> Names {
    match change {
        Change::Insert(statement) => {
            Names::located(statement.schema, statement.name, Located::Table)
        }
        Change::Update(statement) => {
            Names::located(statement.schema, statement.name, Located::Table)
        }
        Change::Delete(statement) => {
            Names::located(statement.schema, statement.name, Located::Table)
        }
    }
}

/// Whether a text holds one of the words a statement that writes the temp
/// schema carries, which the statement is read for before it is parsed.
///
/// Reading the text costs O(n) in its bytes.
fn holds_temp(sql: &[u8]) -> bool {
    for name in [
        b"temp".as_slice(),
        b"temporary",
        b"sqlite_temp_master",
        b"sqlite_temp_schema",
    ] {
        if worded(sql, name) {
            return true;
        }
    }
    false
}

/// Whether a text holds `name` as a word of its own, which is one no byte
/// a bare name is written with stands beside.
///
/// Reading the text costs O(n) in its bytes.
fn worded(sql: &[u8], name: &[u8]) -> bool {
    let named = |byte: Option<&u8>| byte.is_some_and(|byte| is_name_byte(*byte));
    sql.windows(name.len()).enumerate().any(|(at, window)| {
        window.eq_ignore_ascii_case(name)
            && !named(at.checked_sub(1).and_then(|before| sql.get(before)))
            && !named(sql.get(at.saturating_add(name.len())))
    })
}

/// Whether a byte is one a bare name is written with.
const fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// The text of a name a statement wrote, and nothing where it wrote none.
fn named_text(named: Option<Span>, sql: &[u8]) -> Vec<u8> {
    named.map_or_else(Vec::new, |span| crate::schema::dequote(span.text(sql)))
}

/// The rows `PRAGMA collation_list` answers: one per collation the
/// connection holds, the three of the library first and the ones the
/// application defined after them.
pub(crate) fn listed_collations(collating: &[crate::value::Collating]) -> Vec<Vec<Value>> {
    let held = [
        crate::value::Collation::Binary,
        crate::value::Collation::NoCase,
        crate::value::Collation::Rtrim,
    ];
    let names = held
        .iter()
        .map(|collation| collation.word().to_vec())
        .chain(collating.iter().map(|one| one.name.to_vec()));
    names
        .enumerate()
        .map(|(seq, name)| {
            alloc::vec![
                Value::Int(i64::try_from(seq).unwrap_or(0)),
                Value::Text(name),
            ]
        })
        .collect()
}

/// The rows `PRAGMA table_info` and `PRAGMA table_xinfo` answer: one per
/// column of the table, with the place it takes, its name, the type it was
/// declared with, whether it refuses nothing, what it falls back to, and
/// where it stands in the primary key.
///
/// `PragTyp_TABLE_INFO` of `research/sqlite/src/pragma.c:1211` leaves a
/// computed column out of the first, and writes for the second which of
/// the two kinds of computed column each is.
///
/// Reading the table costs O(n) in its columns.
pub(crate) fn columns_of(database: &Database<'_>, name: &[u8], every: bool) -> Vec<Vec<Value>> {
    let Some((table, _)) = database.table(name) else {
        return view_columns(database, name, every);
    };
    let mut out = Vec::new();
    for (at, column) in table.columns.iter().enumerate() {
        let hidden = match column.generated {
            crate::schema::Generated::Never => 0,
            crate::schema::Generated::Virtual => 2,
            crate::schema::Generated::Stored => 3,
        };
        if !every && hidden != 0 {
            continue;
        }
        let mut row = alloc::vec![
            Value::Int(i64::try_from(at).unwrap_or(0)),
            Value::Text(column.name.clone()),
            Value::Text(column.declared.clone()),
            Value::Int(i64::from(column.not_null)),
            column.default.clone().map_or(Value::Null, Value::Text),
            Value::Int(i64::from(column.key)),
        ];
        if every {
            row.push(Value::Int(hidden));
        }
        out.push(row);
    }
    out
}

/// The columns a view answers, as `PRAGMA table_info` writes them, and no
/// row where the schema holds no view of that name or the statement of
/// the view does not run.
///
/// `sqlite3ViewGetColumnNames` reads the statement of the view for its
/// columns, so the names and the declared types are the ones a statement
/// over the view answers. Reading them costs what that statement costs
/// over no row.
fn view_columns(database: &Database<'_>, name: &[u8], every: bool) -> Vec<Vec<Value>> {
    if database.view(name).is_none() {
        return Vec::new();
    }
    let mut sql = b"SELECT * FROM \"".to_vec();
    for byte in name {
        if *byte == b'"' {
            sql.push(b'"');
        }
        sql.push(*byte);
    }
    sql.extend_from_slice(b"\" LIMIT 0");
    let Ok(answered) = database.query(&sql) else {
        return Vec::new();
    };
    answered
        .names
        .iter()
        .zip(answered.declared.iter())
        .enumerate()
        .map(|(at, (column, declared))| {
            let mut row = alloc::vec![
                Value::Int(i64::try_from(at).unwrap_or(0)),
                Value::Text(column.clone()),
                Value::Text(declared.clone()),
                Value::Int(0),
                Value::Null,
                Value::Int(0),
            ];
            if every {
                row.push(Value::Int(0));
            }
            row
        })
        .collect()
}

/// One place of an index as `PRAGMA index_xinfo` writes it.
struct Place {
    /// The place in the table of the column it holds, which is -1 for the
    /// rowid and -2 for an expression.
    cid: i64,
    /// The name of the column, and nothing for the rowid and for an
    /// expression.
    name: Option<Vec<u8>>,
    /// Whether the entries run backwards in it.
    back: bool,
    /// The collation its text is compared under.
    collation: crate::value::Collation,
    /// Whether the entries are held in the order of it.
    key: bool,
}

/// The rows `PRAGMA index_info` and `PRAGMA index_xinfo` answer: one per
/// place of the index, with the place in the table of the column it holds.
///
/// `PragTyp_INDEX_INFO` writes for the second the order the entries run
/// in, the collation they are compared under, and whether the place is one
/// the entries are held in the order of; the places after those are the
/// ones an entry carries to name the row.
///
/// Reading the index costs O(n) in its places.
pub(crate) fn places_of(database: &Database<'_>, name: &[u8], every: bool) -> Vec<Vec<Value>> {
    let Some(indexed) = database.indexed(name) else {
        return Vec::new();
    };
    let mut held: Vec<Place> = indexed
        .index
        .columns
        .iter()
        .map(|keyed| Place {
            cid: keyed
                .place()
                .and_then(|at| i64::try_from(at).ok())
                .unwrap_or(-2),
            name: keyed
                .place()
                .and_then(|at| indexed.table.columns.get(at))
                .map(|column| column.name.clone()),
            back: keyed.order == crate::ast::Order::Descending,
            collation: keyed.collation,
            key: true,
        })
        .collect();
    if every {
        held.extend(naming_places(&indexed));
    }
    held.iter()
        .enumerate()
        .map(|(seq, place)| {
            let mut row = alloc::vec![
                Value::Int(i64::try_from(seq).unwrap_or(0)),
                Value::Int(place.cid),
                place.name.clone().map_or(Value::Null, Value::Text),
            ];
            if every {
                row.push(Value::Int(i64::from(place.back)));
                row.push(Value::Text(place.collation.word().to_vec()));
                row.push(Value::Int(i64::from(place.key)));
            }
            row
        })
        .collect()
}

/// The places an entry of the index carries to name the row: the rowid of
/// a table that holds one, and the columns of the primary key of a table
/// that keeps its rows in the key's own tree.
///
/// The tree of the primary key of such a table is the table itself and is
/// no index this crate holds, so no index answers every other column here.
fn naming_places(indexed: &crate::db::Indexed<'_>) -> Vec<Place> {
    let place = |at: usize| Place {
        cid: i64::try_from(at).unwrap_or(-2),
        name: indexed
            .table
            .columns
            .get(at)
            .map(|column| column.name.clone()),
        back: false,
        collation: indexed
            .table
            .columns
            .get(at)
            .map_or(crate::value::Collation::Binary, |column| column.collation),
        key: false,
    };
    if !indexed.table.without_rowid {
        return alloc::vec![Place {
            cid: -1,
            name: None,
            back: false,
            collation: crate::value::Collation::Binary,
            key: false,
        }];
    }
    // An index of a table that keeps its rows in the key's own tree
    // carries the key, which names the row.
    let held: Vec<usize> = indexed
        .index
        .columns
        .iter()
        .filter_map(crate::schema::Keyed::place)
        .collect();
    primary_places(indexed.table)
        .into_iter()
        .filter(|at| !held.contains(at))
        .map(place)
        .collect()
}

/// The rows `PRAGMA index_list` answers: one per index over the table, the
/// one made last first, with its name, whether two rows may share one key,
/// where it came from, and whether it holds fewer entries than the table
/// has rows.
///
/// Reading the table costs O(n) in its indexes.
pub(crate) fn listed_indexes(database: &Database<'_>, name: &[u8]) -> Vec<Vec<Value>> {
    let held = database.indexes(name);
    held.iter()
        .rev()
        .enumerate()
        .map(|(seq, indexed)| {
            alloc::vec![
                Value::Int(i64::try_from(seq).unwrap_or(0)),
                Value::Text(indexed.index.name.clone()),
                Value::Int(i64::from(indexed.index.unique)),
                Value::Text(origin_of(indexed).to_vec()),
                Value::Int(i64::from(indexed.index.filter.is_some())),
            ]
        })
        .collect()
}

/// Where an index came from, which `PRAGMA index_list` writes as a word:
/// `c` for a `CREATE INDEX`, `pk` for a `PRIMARY KEY` and `u` for a
/// `UNIQUE`.
fn origin_of(indexed: &crate::db::Indexed<'_>) -> &'static [u8] {
    if !indexed.sql.is_empty() {
        return b"c";
    }
    if keyed_as_primary(indexed) {
        return b"pk";
    }
    b"u"
}

/// Whether the places of an index are the columns of the primary key, in
/// the order the key holds them.
fn keyed_as_primary(indexed: &crate::db::Indexed<'_>) -> bool {
    let held: Vec<usize> = indexed
        .index
        .columns
        .iter()
        .filter_map(crate::schema::Keyed::place)
        .collect();
    primary_places(indexed.table) == held
}

/// The places of the columns of the primary key, in the order the key
/// holds them.
///
/// Sorting them costs O(n log n) in the columns of the table.
fn primary_places(table: &Table) -> Vec<usize> {
    let mut out: Vec<(u16, usize)> = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.key != 0)
        .map(|(at, column)| (column.key, at))
        .collect();
    out.sort_unstable();
    out.into_iter().map(|(_, at)| at).collect()
}
