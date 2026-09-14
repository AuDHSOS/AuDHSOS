// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A table, out of the statement that made it.
//!
//! A SQLite file keeps no structures for its schema: `sqlite_schema`
//! holds the `CREATE` text and nothing else, so every column name,
//! declared type, affinity, collation and key in a database is read out
//! of that text. This is `sqlite3AddColumn`, `sqlite3AddPrimaryKey` and
//! the end of `sqlite3EndTable` in `src/build.c`, which is where those
//! rules are and where the surprises are with them: a type name of three
//! letters or more that matches one of six is stored as that one in
//! capitals, a primary key is the rowid only where the type is spelled
//! `INTEGER` and not `INT`, and `STRICT` turns both of those into
//! refusals.
//!
//! Building a table is one pass over its columns: O(n) in them, but for
//! the duplicate-name check, which is O(n²) as SQLite's is.

use alloc::vec::Vec;

use crate::ast::{
    Arena, ColumnConstraint, CreateIndex, CreateTable, ExprId, Literal, Node, Order, Span,
    TableBody, TableConstraint,
};
use crate::value::{Affinity, Collation};

/// Why a definition is not a table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// The columns come from a statement. A file never holds one: what
    /// `CREATE TABLE ... AS SELECT` writes into `sqlite_schema` is the
    /// columns it worked out.
    FromSelect,
    /// Two columns of one name.
    DuplicateColumn,
    /// A collation no engine has.
    NoCollation,
    /// The word after a generated column's expression is neither
    /// `STORED` nor `VIRTUAL`.
    GeneratedWord,
    /// Every column is generated, so there is nothing to store.
    AllGenerated,
    /// A generated column in the primary key.
    GeneratedKey,
    /// More than one primary key.
    ManyKeys,
    /// `AUTOINCREMENT` on a key that is not the rowid.
    Autoincrement,
    /// `AUTOINCREMENT` on a table with no rowid to count.
    AutoincrementWithoutRowid,
    /// `WITHOUT ROWID` with no primary key to take its place.
    MissingKey,
    /// `STRICT` with a column that has no type.
    MissingType,
    /// `STRICT` with a column whose type is not one of the six.
    UnknownType,
    /// A primary key naming a column the table does not have.
    NoSuchColumn,
    /// A primary key whose term is an expression rather than a name.
    KeyExpression,
    /// An index term that is not a column of the table it is over.
    IndexColumn,
    /// An index with a `WHERE` clause, which holds fewer rows than the
    /// table has.
    PartialIndex,
}

/// Whether a column is computed, and whether what it computes is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Generated {
    /// Not computed.
    #[default]
    Never,
    /// `VIRTUAL`: computed every time it is read.
    Virtual,
    /// `STORED`: computed once and written down.
    Stored,
}

/// One column of a table.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    /// The name, with its quotes taken off.
    pub name: Vec<u8>,
    /// The type as the schema reports it: one of the six in capitals
    /// where it is one of the six, and as written otherwise.
    pub declared: Vec<u8>,
    /// What is converted before it is stored.
    pub affinity: Affinity,
    /// How its text is compared.
    pub collation: Collation,
    /// Whether it refuses nothing.
    pub not_null: bool,
    /// What it falls back to, as it was written.
    pub default: Option<Vec<u8>>,
    /// The same, as the tree of the `CREATE` holds it, which is what a
    /// row that was written before the column was added answers.
    pub falls_back: Option<ExprId>,
    /// Where it stands in the primary key, counting from one, or zero
    /// where it is not in it.
    pub key: u16,
    /// Whether it is computed.
    pub generated: Generated,
    /// What it is computed from, where it is computed.
    pub computed: Option<ExprId>,
}

/// A table, as a statement describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// The name, with its quotes taken off.
    pub name: Vec<u8>,
    /// The columns, in the order they were written.
    pub columns: Vec<Column>,
    /// Whether the rows are kept in the primary key's own tree.
    pub without_rowid: bool,
    /// Whether the declared types are enforced.
    pub strict: bool,
    /// The column the rowid is another name for, where there is one.
    pub rowid_alias: Option<usize>,
    /// Whether the rowid counts up rather than filling gaps.
    pub autoincrement: bool,
    /// Where in the statement a column added later is written, which is
    /// `addColOffset`.
    pub add_at: Option<usize>,
    /// The `PRIMARY KEY` and `UNIQUE` constraints an index of the
    /// table's own holds the entries of, in the order they were
    /// written.
    pub keys: Vec<Keys>,
}

/// One `PRIMARY KEY` or `UNIQUE` as the statement wrote it: the name
/// and the order of each column, the conflict clause, and whether the
/// constraint is the `PRIMARY KEY`.
type Written = (Vec<(Vec<u8>, Order)>, crate::ast::Conflict, bool);

/// One `PRIMARY KEY` or `UNIQUE` an index of the table's own holds the
/// entries of, which is `sqlite_autoindex_<table>_<n>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The columns, in the order the constraint wrote them.
    pub columns: Vec<Keyed>,
    /// What the statement says to do where two rows share a key.
    pub conflict: crate::ast::Conflict,
}

/// The index `at` of the table's own, counting from nought, as a
/// `CREATE INDEX` would have described it.
#[must_use]
pub fn own_index(table: &Table, at: usize) -> Option<Index> {
    let keys = table.keys.get(at)?;
    let mut name = b"sqlite_autoindex_".to_vec();
    name.extend_from_slice(&table.name);
    name.push(b'_');
    name.extend_from_slice(digits(at.saturating_add(1)).as_slice());
    Some(Index {
        name,
        table: table.name.clone(),
        columns: keys.columns.clone(),
        unique: true,
        conflict: keys.conflict,
    })
}

/// Every `PRIMARY KEY` and `UNIQUE` of a table as an index of the
/// table's own, which is `sqlite3CreateIndex` over the constraints of
/// a `CREATE TABLE`.
///
/// Comparing `n` constraints of `k` columns costs O(n^2 k).
fn own_keys(table: &Table, written: &[Written]) -> Result<Vec<Keys>, Error> {
    let mut keys: Vec<Keys> = Vec::new();
    for (columns, conflict, primary) in written {
        let mut keyed = Vec::new();
        for (name, order) in columns {
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(name))
                .ok_or(Error::NoSuchColumn)?;
            let collation = table
                .columns
                .get(at)
                .map_or(Collation::Binary, |column| column.collation);
            keyed.push(Keyed {
                column: at,
                order: *order,
                collation,
            });
        }
        // The `PRIMARY KEY` the rowid is another name for is the
        // table's own tree, so it carries no index of its own.
        let alias = *primary
            && keyed.len() == 1
            && keyed
                .first()
                .is_some_and(|keyed| table.rowid_alias == Some(keyed.column));
        if alias {
            continue;
        }
        // A constraint over the columns of one already there carries no
        // second index, and the one there takes the clause of the
        // constraint where it carries none. The order of a column is
        // not compared, which is what `sqlite3CreateIndex` leaves out,
        // and the collation is the column's, so the columns settle it.
        let held = keys.iter_mut().find(|held| {
            held.columns.len() == keyed.len()
                && held
                    .columns
                    .iter()
                    .zip(&keyed)
                    .all(|(held, new)| held.column == new.column)
        });
        if let Some(held) = held {
            if held.conflict == crate::ast::Conflict::Unspecified {
                held.conflict = *conflict;
            }
            continue;
        }
        keys.push(Keys {
            columns: keyed,
            conflict: *conflict,
        });
    }
    Ok(keys)
}

/// One column of an index, and how it is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyed {
    /// Where the column stands in the table.
    pub column: usize,
    /// Which way the entries run in it.
    pub order: Order,
    /// How its text is compared, which is the table's unless the index
    /// writes another.
    pub collation: Collation,
}

/// An index, as a `CREATE INDEX` describes it.
///
/// An index this crate cannot use is not one it holds: an index over an
/// expression and a partial index both answer fewer entries than their
/// columns say, so `index` refuses them and the statement scans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    /// The name, with its quotes taken off.
    pub name: Vec<u8>,
    /// The table it is over, with its quotes taken off.
    pub table: Vec<u8>,
    /// The columns it holds its entries in the order of.
    pub columns: Vec<Keyed>,
    /// Whether two rows may share one key.
    pub unique: bool,
    /// What the constraint says to do where two rows share one key,
    /// which only an index of the table's own carries.
    pub conflict: crate::ast::Conflict,
}

/// The index a `CREATE INDEX` describes, read against the table it is
/// over.
///
/// # Errors
///
/// [`Error::IndexColumn`] for a term that is not a column of the table,
/// and [`Error::PartialIndex`] for a `WHERE` clause, which makes the
/// index hold fewer rows than the table has.
pub fn index(
    arena: &Arena,
    definition: &CreateIndex,
    sql: &[u8],
    table: &Table,
) -> Result<Index, Error> {
    if definition.filter.is_some() {
        return Err(Error::PartialIndex);
    }
    let mut columns = Vec::new();
    for term in arena.orders(definition.columns) {
        // A term may be written with a collation of its own, which is
        // the one its entries are held in.
        let (named, written) = collated(arena, term.expr, sql);
        let name = dequote(named.ok_or(Error::IndexColumn)?.text(sql));
        let at = table
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case(&name))
            .ok_or(Error::IndexColumn)?;
        let collation = table
            .columns
            .get(at)
            .map_or(Collation::Binary, |column| column.collation);
        columns.push(Keyed {
            column: at,
            order: term.order,
            collation: written.unwrap_or(collation),
        });
    }
    Ok(Index {
        name: dequote(definition.name.text(sql)),
        table: dequote(definition.table.text(sql)),
        columns,
        unique: definition.unique,
        conflict: crate::ast::Conflict::Unspecified,
    })
}

/// The column an index term names and the collation written on it, where
/// the term is a column and nothing else.
fn collated(arena: &Arena, id: ExprId, sql: &[u8]) -> (Option<Span>, Option<Collation>) {
    match arena.node(id) {
        Some(Node::Collate { value, name }) => (
            collated(arena, value, sql).0,
            Collation::of_name(&dequote(name.text(sql))),
        ),
        Some(Node::Column { column, .. }) => (Some(column), None),
        _ => (None, None),
    }
}

/// The six names a type may be that the schema stores as a code rather
/// than as text, with the affinity of each.
const STANDARD: [(&[u8], Affinity); 6] = [
    (b"ANY", Affinity::Numeric),
    (b"BLOB", Affinity::Blob),
    (b"INT", Affinity::Integer),
    (b"INTEGER", Affinity::Integer),
    (b"REAL", Affinity::Real),
    (b"TEXT", Affinity::Text),
];

/// Which of the six a type is, where it is one.
fn standard(name: &[u8]) -> Option<usize> {
    if name.len() < 3 {
        return None;
    }
    STANDARD
        .iter()
        .position(|(word, _)| name.eq_ignore_ascii_case(word))
}

/// A name with its quotes taken off, which is `sqlite3Dequote`.
pub(crate) fn dequote(text: &[u8]) -> Vec<u8> {
    let quote = match text.first().copied() {
        Some(b'\'') => b'\'',
        Some(b'"') => b'"',
        Some(b'`') => b'`',
        Some(b'[') => b']',
        _ => return text.to_vec(),
    };
    let inner = text
        .get(1..text.len().saturating_sub(1))
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut doubled = false;
    for byte in inner {
        if doubled {
            doubled = false;
            continue;
        }
        out.push(*byte);
        doubled = *byte == quote;
    }
    out
}

/// The column names a table made from a statement carries, which is
/// `sqlite3ColumnsFromExprList`: a name another column already carries
/// gains a number, and `true` or `false` is not a name a column may
/// carry.
pub(crate) fn columns_from(names: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(names.len());
    for (at, written) in names.iter().enumerate() {
        let mut name =
            if written.eq_ignore_ascii_case(b"true") || written.eq_ignore_ascii_case(b"false") {
                let mut held = b"column".to_vec();
                held.extend_from_slice(digits(at.saturating_add(1)).as_slice());
                held
            } else {
                written.clone()
            };
        let mut count: usize = 0;
        while out.iter().any(|held| held.eq_ignore_ascii_case(&name)) {
            count = count.saturating_add(1);
            name = numbered(&name, count);
        }
        out.push(name);
    }
    out
}

/// A name with the number it collided over put after it, and the number
/// a collision before it left taken back off.
fn numbered(name: &[u8], count: usize) -> Vec<u8> {
    let mut at = name.len().saturating_sub(1);
    while at > 0 && name.get(at).is_some_and(u8::is_ascii_digit) {
        at = at.saturating_sub(1);
    }
    let base = if name.get(at) == Some(&b':') {
        name.get(..at).unwrap_or_default()
    } else {
        name
    };
    let mut out = base.to_vec();
    out.push(b':');
    out.extend_from_slice(digits(count).as_slice());
    out
}

/// A whole number as the digits it is written with.
fn digits(number: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut left = number;
    loop {
        out.push(b'0'.saturating_add(u8::try_from(left % 10).unwrap_or(0)));
        left /= 10;
        if left == 0 {
            break;
        }
    }
    out.reverse();
    out
}

/// The statement a table made from another statement is written as,
/// which is `createTableStmt`: the name, then every column with the
/// type its affinity is written as, over one line where the names are
/// short and one line per column where they are not.
pub(crate) fn created(name: &[u8], columns: &[Vec<u8>], affinities: &[Affinity]) -> Vec<u8> {
    let mut length = ident_length(name);
    for column in columns {
        length = length
            .saturating_add(ident_length(column))
            .saturating_add(5);
    }
    let (first, between, end): (&[u8], &[u8], &[u8]) = if length < 50 {
        (b"", b",", b")")
    } else {
        (b"\n  ", b",\n  ", b"\n)")
    };
    let mut out = b"CREATE TABLE ".to_vec();
    ident_put(&mut out, name);
    out.push(b'(');
    for (at, column) in columns.iter().enumerate() {
        out.extend_from_slice(if at == 0 { first } else { between });
        ident_put(&mut out, column);
        let affinity = affinities.get(at).copied().unwrap_or_default();
        out.extend_from_slice(type_written(affinity));
    }
    out.extend_from_slice(end);
    out
}

/// The type an affinity is written as, which is the table
/// `createTableStmt` keeps.
const fn type_written(affinity: Affinity) -> &'static [u8] {
    match affinity {
        Affinity::None | Affinity::Blob => b"",
        Affinity::Text => b" TEXT",
        Affinity::Numeric => b" NUM",
        Affinity::Integer => b" INT",
        Affinity::Real => b" REAL",
    }
}

/// How many bytes a name takes written out, which is `identLength`.
fn ident_length(name: &[u8]) -> usize {
    name.iter()
        .fold(name.len().saturating_add(2), |count, byte| {
            count.saturating_add(usize::from(*byte == b'"'))
        })
}

/// A name written out, in quotes where it needs them, which is
/// `identPut`: a name that begins with a digit, that is a keyword, that
/// carries a byte other than a letter, a digit or an underscore, or
/// that is empty.
fn ident_put(out: &mut Vec<u8>, name: &[u8]) {
    let plain = name
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
    let quote = !plain
        || name.is_empty()
        || name.first().is_some_and(u8::is_ascii_digit)
        || crate::keyword::lookup(name).is_some();
    if quote {
        out.push(b'"');
    }
    for byte in name {
        out.push(*byte);
        if *byte == b'"' {
            out.push(b'"');
        }
    }
    if quote {
        out.push(b'"');
    }
}

/// The type of a column, with the words the parser could not tell from
/// one taken back off.
///
/// `GENERATED ALWAYS` reads as two words of a type name until the `AS`
/// after it says otherwise, and SQLite trims them here rather than
/// there.
fn type_text(text: &[u8]) -> Vec<u8> {
    let mut end = text.len();
    // A slice shorter than the word never equals it, so no length is
    // checked here.
    let ends_with = |end: usize, word: &[u8]| {
        text.get(end.saturating_sub(word.len())..end)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(word))
    };
    let trim = |end: usize| {
        let mut end = end;
        while text
            .get(end.saturating_sub(1))
            .is_some_and(u8::is_ascii_whitespace)
        {
            end = end.saturating_sub(1);
        }
        end
    };
    if text.len() >= 16 && ends_with(end, b"always") {
        end = trim(end.saturating_sub(6));
        if ends_with(end, b"generated") {
            end = trim(end.saturating_sub(9));
        }
    }
    dequote(text.get(..end).unwrap_or_default())
}

/// The table `definition` describes, where `sql` is the statement its
/// spans point into.
///
/// # Errors
///
/// [`Error`] names what is wrong with it, which is what SQLite refuses
/// it with.
#[expect(
    clippy::too_many_lines,
    reason = "the rules of one routine of the C library, in the order it applies them"
)]
pub fn table(arena: &Arena, definition: &CreateTable, sql: &[u8]) -> Result<Table, Error> {
    let TableBody::Columns {
        columns,
        constraints,
    } = definition.body
    else {
        return Err(Error::FromSelect);
    };
    let mut table = Table {
        name: dequote(definition.name.text(sql)),
        columns: Vec::new(),
        without_rowid: definition.options.without_rowid,
        strict: definition.options.strict,
        add_at: definition.add_at,
        rowid_alias: None,
        autoincrement: false,
        keys: Vec::new(),
    };
    let mut key: Option<(Vec<usize>, Order, bool)> = None;
    // Every `PRIMARY KEY` and `UNIQUE`, in the order they were written,
    // because that is the order `sqlite_autoindex_<table>_<n>` counts
    // in: a column's own constraints as the columns run, then the
    // constraints of the table.
    let mut written_keys: Vec<Written> = Vec::new();
    for written in arena.columns(columns) {
        let name = dequote(written.name.text(sql));
        if table
            .columns
            .iter()
            .any(|column| column.name.eq_ignore_ascii_case(&name))
        {
            return Err(Error::DuplicateColumn);
        }
        // A type that trims to nothing is no type at all, which is
        // what `GENERATED ALWAYS` before an `AS` leaves behind.
        let declared = written
            .ty
            .map(|ty| type_text(ty.text(sql)))
            .filter(|text| !text.is_empty());
        let standard = declared.as_deref().and_then(standard);
        let named = name.clone();
        let mut column = Column {
            name,
            affinity: match (standard, declared.as_deref()) {
                (Some(at), _) => STANDARD.get(at).map_or(Affinity::Blob, |entry| entry.1),
                (None, None) => Affinity::Blob,
                (None, Some(text)) => Affinity::of_type(text),
            },
            declared: match standard {
                Some(at) => STANDARD
                    .get(at)
                    .map_or_else(Vec::new, |entry| entry.0.to_vec()),
                None => declared.unwrap_or_default(),
            },
            collation: Collation::Binary,
            not_null: false,
            default: None,
            falls_back: None,
            key: 0,
            generated: Generated::Never,
            computed: None,
        };
        let mut own_key = None;
        for constraint in arena.column_constraints(written.constraints) {
            match *constraint {
                ColumnConstraint::NotNull(_) => column.not_null = true,
                ColumnConstraint::Collate(name) => {
                    column.collation =
                        Collation::of_name(&dequote(name.text(sql))).ok_or(Error::NoCollation)?;
                }
                ColumnConstraint::Default { value, text } => {
                    column.default = Some(text.text(sql).to_vec());
                    column.falls_back = Some(value);
                }
                ColumnConstraint::PrimaryKey {
                    order,
                    autoincrement,
                    conflict,
                } => {
                    own_key = Some((order, autoincrement));
                    written_keys.push((alloc::vec![(named.clone(), order)], conflict, true));
                }
                ColumnConstraint::Unique(conflict) => {
                    written_keys.push((
                        alloc::vec![(named.clone(), Order::Unspecified)],
                        conflict,
                        false,
                    ));
                }
                ColumnConstraint::Generated { value, kind } => {
                    column.generated = generated_kind(kind, sql)?;
                    column.computed = Some(value);
                }
                _ => {}
            }
        }
        let at = table.columns.len();
        table.columns.push(column);
        if let Some((order, autoincrement)) = own_key {
            if key.is_some() {
                return Err(Error::ManyKeys);
            }
            key = Some((alloc::vec![at], order, autoincrement));
        }
    }
    for constraint in arena.table_constraints(constraints) {
        let (TableConstraint::PrimaryKey {
            columns, conflict, ..
        }
        | TableConstraint::Unique { columns, conflict }) = *constraint
        else {
            continue;
        };
        let mut written = Vec::new();
        let mut named = Vec::new();
        let mut order = Order::Unspecified;
        for (at, term) in arena.orders(columns).iter().enumerate() {
            if at == 0 {
                order = term.order;
            }
            // A term that is not a name is refused where the key's own
            // index would be built.
            let column = key_name(arena, term.expr).ok_or(Error::KeyExpression)?;
            written.push((dequote(column.text(sql)), term.order));
            named.push(index_of(&table, column, sql).ok_or(Error::NoSuchColumn)?);
        }
        let primary = matches!(*constraint, TableConstraint::PrimaryKey { .. });
        written_keys.push((written, conflict, primary));
        let TableConstraint::PrimaryKey { autoincrement, .. } = *constraint else {
            continue;
        };
        if key.is_some() {
            return Err(Error::ManyKeys);
        }
        key = Some((named, order, autoincrement));
    }

    if !table
        .columns
        .iter()
        .any(|column| column.generated == Generated::Never)
    {
        return Err(Error::AllGenerated);
    }

    if let Some((named, order, autoincrement)) = key {
        for (at, column) in table.columns.iter_mut().enumerate() {
            let Some(place) = named.iter().position(|named| *named == at) else {
                continue;
            };
            if column.generated != Generated::Never {
                return Err(Error::GeneratedKey);
            }
            column.key = u16::try_from(place.saturating_add(1)).unwrap_or(u16::MAX);
        }
        // The rowid is another name for the key only where the key is
        // one column, spelled `INTEGER` and not `INT`, and not written
        // backwards.
        let single = named.first().copied().filter(|_| named.len() == 1);
        let alias = single.filter(|at| {
            order != Order::Descending
                && table
                    .columns
                    .get(*at)
                    .is_some_and(|column| column.declared == b"INTEGER")
        });
        if let Some(at) = alias {
            table.rowid_alias = Some(at);
            table.autoincrement = autoincrement;
        } else if autoincrement {
            return Err(Error::Autoincrement);
        }
    }

    table.keys = own_keys(&table, &written_keys)?;

    if table.strict {
        for column in &mut table.columns {
            let Some(at) = standard(&column.declared) else {
                return Err(if column.declared.is_empty() {
                    Error::MissingType
                } else {
                    Error::UnknownType
                });
            };
            if at == 0 {
                // `ANY` holds whatever it is given, so nothing is
                // converted on the way in.
                column.affinity = Affinity::Blob;
            }
        }
        let alias = table.rowid_alias;
        for (at, column) in table.columns.iter_mut().enumerate() {
            if column.key > 0 && alias != Some(at) {
                column.not_null = true;
            }
        }
    }

    if table.without_rowid {
        if table.autoincrement {
            return Err(Error::AutoincrementWithoutRowid);
        }
        if !table.columns.iter().any(|column| column.key > 0) {
            return Err(Error::MissingKey);
        }
        table.rowid_alias = None;
        for column in &mut table.columns {
            if column.key > 0 {
                column.not_null = true;
            }
        }
    }
    Ok(table)
}

/// Which of `STORED` and `VIRTUAL` was written, and nothing else.
fn generated_kind(kind: Option<Span>, sql: &[u8]) -> Result<Generated, Error> {
    let Some(word) = kind else {
        return Ok(Generated::Virtual);
    };
    let word = dequote(word.text(sql));
    if word.eq_ignore_ascii_case(b"stored") {
        Ok(Generated::Stored)
    } else if word.eq_ignore_ascii_case(b"virtual") {
        Ok(Generated::Virtual)
    } else {
        Err(Error::GeneratedWord)
    }
}

/// The name a term of a primary key names, where it names one.
///
/// `sqlite3ExprSkipCollate` and `sqlite3StringToId`: a collation around
/// it does not count, and a string where a name belongs is a name.
fn key_name(arena: &Arena, id: ExprId) -> Option<Span> {
    let mut node = arena.node(id)?;
    if let Node::Collate { value, .. } = node {
        node = arena.node(value)?;
    }
    match node {
        Node::Column { column, .. } => Some(column),
        Node::Literal(Literal::Text(span)) => Some(span),
        _ => None,
    }
}

/// Where a name stands among the columns.
fn index_of(table: &Table, name: Span, sql: &[u8]) -> Option<usize> {
    let name = dequote(name.text(sql));
    table
        .columns
        .iter()
        .position(|column| column.name.eq_ignore_ascii_case(&name))
}
