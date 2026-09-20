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
use crate::value::{Affinity, Collation, Value};

/// Why a definition is not a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// The columns come from a statement. A file never holds one: what
    /// `CREATE TABLE ... AS SELECT` writes into `sqlite_schema` is the
    /// columns it worked out.
    FromSelect,
    /// Two columns of one name, named.
    DuplicateColumn(Vec<u8>),
    /// A collation no engine has.
    NoCollation(alloc::vec::Vec<u8>),
    /// A `likelihood` whose second argument is not a real between
    /// nought and one.
    Likelihood,
    /// The word after a generated column's expression is neither
    /// `STORED` nor `VIRTUAL`.
    GeneratedWord,
    /// Every column is generated, so there is nothing to store.
    AllGenerated,
    /// A generated column in the primary key.
    GeneratedKey,
    /// More than one primary key, with the name of the table.
    ManyKeys(Vec<u8>),
    /// `AUTOINCREMENT` on a key that is not the rowid.
    Autoincrement,
    /// `AUTOINCREMENT` on a table with no rowid to count.
    AutoincrementWithoutRowid,
    /// `WITHOUT ROWID` with no primary key to take its place, with the
    /// name of the table.
    MissingKey(Vec<u8>),
    /// `STRICT` with a column that has no type, with the table and the
    /// column.
    MissingType(Vec<u8>, Vec<u8>),
    /// `STRICT` with a column whose type is not one of the six, with the
    /// table, the column and the type as the statement wrote it.
    UnknownType(Vec<u8>, Vec<u8>, Vec<u8>),
    /// A primary key naming a column the table does not have, named.
    NoSuchColumn(Vec<u8>),
    /// A `FOREIGN KEY` over a column the table does not hold, named.
    ForeignColumn(Vec<u8>),
    /// A `FOREIGN KEY` that names a different number of columns from
    /// the ones it points at.
    ForeignWidth,
    /// A primary key whose term is an expression rather than a name.
    KeyExpression,
    /// An index term that names a column the table it is over does not
    /// hold.
    IndexColumn(Vec<u8>),
    /// An index term that names a column under a table or a schema,
    /// which `sqlite3ResolveSelfReference` refuses under `NC_IdxExpr`.
    IndexDot,
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
    /// What a row that holds nothing there does, where the constraint
    /// said so: `NOT NULL ON CONFLICT REPLACE` writes what the column
    /// falls back to in its place.
    pub null_conflict: crate::ast::Conflict,
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
    /// The foreign keys the rows are held to, in the order they were
    /// written, which is the order `PRAGMA foreign_key_list` answers
    /// them backwards in.
    pub foreign: Vec<Foreign>,
    /// The `CHECK` constraints every row is held to, in the order they
    /// were written.
    pub checks: Vec<Checked>,
}

impl Table {
    /// A table of the columns a view answers, which carries no
    /// constraint, no key and no tree of its own.
    ///
    /// An `INSTEAD OF` trigger reads `old` and `new` out of such a
    /// table, so a column of it converts nothing and is compared byte
    /// by byte, which is what `sqlite3ViewGetColumnNames` leaves a view
    /// column with where the statement of the view names no table
    /// column.
    ///
    /// Building it costs O(n) in the columns.
    #[must_use]
    pub fn viewed(name: Vec<u8>, columns: &[Vec<u8>]) -> Self {
        Table {
            name,
            columns: columns
                .iter()
                .map(|held| Column::bare(held.clone()))
                .collect(),
            without_rowid: false,
            strict: false,
            rowid_alias: None,
            autoincrement: false,
            add_at: None,
            keys: Vec::new(),
            foreign: Vec::new(),
            checks: Vec::new(),
        }
    }
}

impl Column {
    /// A column of no declared type, which holds what it is given.
    #[must_use]
    pub const fn bare(name: Vec<u8>) -> Self {
        Column {
            name,
            declared: Vec::new(),
            affinity: Affinity::None,
            collation: Collation::Binary,
            not_null: false,
            null_conflict: crate::ast::Conflict::Unspecified,
            default: None,
            falls_back: None,
            key: 0,
            generated: Generated::Never,
            computed: None,
        }
    }
}

/// One `CHECK` of a table: what every row is held to, and what the
/// message names it by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The expression, as the tree of the `CREATE` holds it.
    pub value: ExprId,
    /// The name the constraint was given, or the text of the
    /// expression where it was given none, which is what `CHECK
    /// constraint failed:` writes.
    pub shown: Vec<u8>,
}

/// One `REFERENCES` of a table: which of its columns point at which
/// columns of which table, and what happens to a row of this table
/// when the row they point at goes or changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Foreign {
    /// The columns of this table, by the place each takes in it.
    pub columns: Vec<usize>,
    /// The table they point at, with its quotes taken off.
    pub table: Vec<u8>,
    /// The columns of that table, by name; none where the statement
    /// named none, which points at that table's primary key.
    pub parent: Vec<Vec<u8>>,
    /// What happens to this row when the one it points at goes.
    pub on_delete: crate::ast::Action,
    /// What happens to it when that row changes.
    pub on_update: crate::ast::Action,
    /// Whether the check waits for the end of the transaction.
    pub deferred: bool,
}

/// One `PRIMARY KEY` or `UNIQUE` as the statement wrote it: the name
/// and the order of each column, the conflict clause, and whether the
/// constraint is the `PRIMARY KEY`.
type Written = (Vec<(usize, Order)>, crate::ast::Conflict, bool);

/// One `PRIMARY KEY` or `UNIQUE` an index of the table's own holds the
/// entries of, which is `sqlite_autoindex_<table>_<n>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    /// The columns, in the order the constraint wrote them.
    pub columns: Vec<Keyed>,
    /// What the statement says to do where two rows share a key.
    pub conflict: crate::ast::Conflict,
    /// Whether the constraint is the `PRIMARY KEY`.
    pub primary: bool,
}

/// The columns of the `PRIMARY KEY`, by the place each takes in the
/// table, in the order the key names them, which is the order a table
/// that keeps its rows in the key's own tree places them under.
///
/// Sorting `k` columns costs O(k log k).
#[must_use]
pub fn key_places(table: &Table) -> Vec<usize> {
    let mut named: Vec<usize> = (0..table.columns.len())
        .filter(|at| table.columns.get(*at).is_some_and(|column| column.key > 0))
        .collect();
    named.sort_by_key(|at| table.columns.get(*at).map_or(0, |column| column.key));
    named
}

/// The value of each column of the `PRIMARY KEY` of one row, in the
/// order the key names them.
#[must_use]
pub fn key_of(table: &Table, values: &[Value]) -> Vec<Value> {
    key_places(table)
        .iter()
        .map(|at| values.get(*at).cloned().unwrap_or(Value::Null))
        .collect()
}

/// The collation each column of the `PRIMARY KEY` is held in.
#[must_use]
pub fn key_collations(table: &Table) -> Vec<Collation> {
    key_places(table)
        .iter()
        .map(|at| {
            table
                .columns
                .get(*at)
                .map_or(Collation::Binary, |column| column.collation)
        })
        .collect()
}

/// The index `at` of the table's own, counting from nought, as a
/// `CREATE INDEX` would have described it, and nothing for the
/// `PRIMARY KEY` of a table that keeps its rows in the key's own tree,
/// which that tree holds rather than an index beside it.
#[must_use]
pub fn own_index(table: &Table, at: usize) -> Option<Index> {
    let keys = table.keys.get(at)?;
    if table.without_rowid && keys.primary {
        return None;
    }
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
        filter: None,
    })
}

/// Every `PRIMARY KEY` and `UNIQUE` of a table as an index of the
/// table's own, which is `sqlite3CreateIndex` over the constraints of
/// a `CREATE TABLE`.
///
/// Every column of every constraint was read against the columns of the
/// table before this, so each carries the place it stands at.
///
/// Comparing `n` constraints of `k` columns costs O(n^2 k).
fn own_keys(table: &Table, written: &[Written]) -> Vec<Keys> {
    let mut keys: Vec<Keys> = Vec::new();
    for (columns, conflict, primary) in written {
        let mut keyed = Vec::new();
        for (at, order) in columns {
            let collation = table
                .columns
                .get(*at)
                .map_or(Collation::Binary, |column| column.collation);
            keyed.push(Keyed {
                of: Of::Place(*at),
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
                .is_some_and(|keyed| table.rowid_alias == keyed.place());
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
                    .all(|(held, new)| held.of == new.of)
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
            primary: *primary,
        });
    }
    keys
}

/// What one place of an index entry holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Of {
    /// A column of the table, by the place it takes in it.
    Place(usize),
    /// An expression over the row, as the tree of the `CREATE INDEX`
    /// holds it, which is `Index.aColExpr` of `sqlite3CreateIndex`.
    Term(ExprId),
}

/// One place of an index, and how it is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keyed {
    /// What the entry holds there.
    pub of: Of,
    /// Which way the entries run in it.
    pub order: Order,
    /// How its text is compared, which is the table's unless the index
    /// writes another.
    pub collation: Collation,
}

impl Keyed {
    /// The place the column takes in the table, or nothing where the
    /// place holds an expression.
    #[must_use]
    pub const fn place(&self) -> Option<usize> {
        match self.of {
            Of::Place(at) => Some(at),
            Of::Term(_) => None,
        }
    }
}

/// An index, as a `CREATE INDEX` describes it.
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
    /// The `WHERE` of a partial index, as the tree of the `CREATE
    /// INDEX` holds it, which is `Index.pPartIdxWhere`.
    pub filter: Option<ExprId>,
}

impl Index {
    /// The first place of the index, with the place in the table of the
    /// column it holds, or nothing where a statement may not be planned
    /// against the index: a partial index answers fewer entries than
    /// the table has rows, and a place over an expression holds a value
    /// no term of a statement names.
    #[must_use]
    pub fn first_keyed(&self) -> Option<(usize, Keyed)> {
        self.filter
            .is_none()
            .then(|| self.columns.first())
            .flatten()
            .and_then(|first| first.place().map(|at| (at, *first)))
    }
}

/// The index a `CREATE INDEX` describes, read against the table it is
/// over.
///
/// A term that is a column of the table takes that column's place and
/// that column's collation; every other term is an expression the entry
/// holds the value of, compared under the collation the term writes or
/// under `BINARY`, which is `sqlite3CreateIndex` filling `aColExpr`.
///
/// # Errors
///
/// [`Error::IndexColumn`] for a term that names a column the table does
/// not hold.
pub fn index(
    arena: &Arena,
    definition: &CreateIndex,
    sql: &[u8],
    table: &Table,
    collating: &[crate::value::Collating],
) -> Result<Index, Error> {
    let mut columns = Vec::new();
    for term in arena.orders(definition.columns) {
        // A term may be written with a collation of its own, which is
        // the one its entries are held in.
        let (named, written) = collated(arena, term.expr, sql, collating);
        let (of, collation) = if let Some(named) = named {
            let name = dequote(named.text(sql));
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| Error::IndexColumn(name.clone()))?;
            let held = table
                .columns
                .get(at)
                .map_or(Collation::Binary, |column| column.collation);
            (Of::Place(at), held)
        } else {
            if qualified(arena, term.expr) {
                return Err(Error::IndexDot);
            }
            (Of::Term(term.expr), Collation::Binary)
        };
        columns.push(Keyed {
            of,
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
        filter: definition.filter,
    })
}

/// Whether a tree names a column under a table or a schema, which
/// `sqlite3ResolveSelfReference` refuses in an index term.
///
/// Reading the tree costs O(n) in its nodes.
fn qualified(arena: &Arena, id: ExprId) -> bool {
    arena.node(id).is_some_and(|node| {
        if matches!(node, Node::Column { table: Some(_), .. }) {
            return true;
        }
        let mut found = false;
        arena.under(node, |under| found |= qualified(arena, under));
        found
    })
}

/// The column an index term names and the collation written on it,
/// naming no column where the term is not a column and nothing else.
///
/// A term written as text names a column, which is `sqlite3StringToId`
/// turning `TK_STRING` into `TK_ID` before `sqlite3CreateIndex` reads
/// the term.
pub(crate) fn collated(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    collating: &[crate::value::Collating],
) -> (Option<Span>, Option<Collation>) {
    match arena.node(id) {
        Some(Node::Collate { value, name }) => (
            collated(arena, value, sql, collating).0,
            crate::value::collation_of(&dequote(name.text(sql)), collating),
        ),
        Some(Node::Column { column, .. }) => (Some(column), None),
        Some(Node::Literal(crate::ast::Literal::Text(text))) => (Some(text), None),
        _ => (None, None),
    }
}

/// Refuses a statement that names a collation the connection does not
/// hold.
///
/// `sqlite3ResolveCollSeqName` looks a name up where the statement is
/// read, so a statement over an empty table is refused as well as one
/// whose rows a comparison reaches. Costs O(n) over the nodes of the
/// statement.
///
/// # Errors
///
/// [`Error::NoCollation`] names the first one the connection does not
/// hold.
pub(crate) fn collations(
    arena: &Arena,
    sql: &[u8],
    collating: &[crate::value::Collating],
) -> Result<(), Error> {
    for name in arena.collates() {
        let named = dequote(name.text(sql));
        if crate::value::collation_of(&named, collating).is_none() {
            return Err(Error::NoCollation(named));
        }
    }
    Ok(())
}

/// Refuses a statement whose `likelihood` carries a second argument
/// that is not a real between nought and one.
///
/// `sqlite3ExprFunction` under `SQLITE_FUNC_UNLIKELY` reads the
/// argument where the statement is read, so it takes a real written as
/// one and nothing else: a whole number, a text and a sum are each
/// refused. Costs O(n) over the nodes of the statement.
///
/// # Errors
///
/// [`Error::Likelihood`] where the argument is another thing.
pub(crate) fn likelihoods(arena: &Arena, sql: &[u8]) -> Result<(), Error> {
    for (name, args) in arena.calls() {
        if !dequote(name.text(sql)).eq_ignore_ascii_case(b"likelihood") {
            continue;
        }
        // A call of another number of arguments is refused for that,
        // which `sqlite3ExprFunction` reads first.
        let held = arena.children(args);
        let (Some(second), 2) = (held.get(1).copied(), held.len()) else {
            continue;
        };
        let chance = match arena.node(second) {
            Some(Node::Literal(crate::ast::Literal::Float(span))) => {
                crate::number::real(span.text(sql)).value
            }
            _ => return Err(Error::Likelihood),
        };
        if !(0.0..=1.0).contains(&chance) {
            return Err(Error::Likelihood);
        }
    }
    Ok(())
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
pub fn table(
    arena: &Arena,
    definition: &CreateTable,
    sql: &[u8],
    collating: &[crate::value::Collating],
) -> Result<Table, Error> {
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
        foreign: Vec::new(),
        checks: Vec::new(),
    };
    let mut key: Option<(Vec<usize>, Order, bool)> = None;
    // Every `PRIMARY KEY` and `UNIQUE`, in the order they were written,
    // because that is the order `sqlite_autoindex_<table>_<n>` counts
    // in: a column's own constraints as the columns run, then the
    // constraints of the table.
    let mut written_keys: Vec<Written> = Vec::new();
    // Every `REFERENCES`, in the order they were written, each with the
    // columns of this table that point.
    let mut pointed: Vec<(Vec<Vec<u8>>, crate::ast::Foreign)> = Vec::new();
    // Every `CHECK`, in the order they were written, each under the
    // name the `CONSTRAINT` in front of it gave it.
    let mut checks: Vec<Checked> = Vec::new();
    for written in arena.columns(columns) {
        let name = dequote(written.name.text(sql));
        if table
            .columns
            .iter()
            .any(|column| column.name.eq_ignore_ascii_case(&name))
        {
            return Err(Error::DuplicateColumn(name));
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
            null_conflict: crate::ast::Conflict::Unspecified,
            default: None,
            falls_back: None,
            key: 0,
            generated: Generated::Never,
            computed: None,
        };
        let mut own_key = None;
        let mut last_named: Option<Span> = None;
        for constraint in arena.column_constraints(written.constraints) {
            if let ColumnConstraint::Named(span) = *constraint {
                last_named = Some(span);
                continue;
            }
            match *constraint {
                ColumnConstraint::NotNull(conflict) => {
                    column.not_null = true;
                    column.null_conflict = conflict;
                }
                ColumnConstraint::Check { value, text } => {
                    checks.push(Checked {
                        value,
                        shown: named_or(last_named, text, sql),
                    });
                }
                ColumnConstraint::Collate(name) => {
                    let named = dequote(name.text(sql));
                    column.collation = crate::value::collation_of(&named, collating)
                        .ok_or_else(|| Error::NoCollation(named.clone()))?;
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
                    written_keys.push((alloc::vec![(table.columns.len(), order)], conflict, true));
                }
                ColumnConstraint::Unique(conflict) => {
                    written_keys.push((
                        alloc::vec![(table.columns.len(), Order::Unspecified)],
                        conflict,
                        false,
                    ));
                }
                ColumnConstraint::Generated { value, kind } => {
                    column.generated = generated_kind(kind, sql)?;
                    column.computed = Some(value);
                }
                ColumnConstraint::References(foreign) => {
                    pointed.push((alloc::vec![named.clone()], foreign));
                }
                _ => {}
            }
        }
        let at = table.columns.len();
        table.columns.push(column);
        if let Some((order, autoincrement)) = own_key {
            if key.is_some() {
                return Err(Error::ManyKeys(table.name.clone()));
            }
            key = Some((alloc::vec![at], order, autoincrement));
        }
    }
    let mut last_named: Option<Span> = None;
    for constraint in arena.table_constraints(constraints) {
        if let TableConstraint::Named(span) = *constraint {
            last_named = Some(span);
            continue;
        }
        if let TableConstraint::Check { value, text } = *constraint {
            checks.push(Checked {
                value,
                shown: named_or(last_named, text, sql),
            });
        }
        if let TableConstraint::ForeignKey { columns, foreign } = *constraint {
            let mut named = Vec::new();
            for column in arena.names(columns) {
                named.push(dequote(column.text(sql)));
            }
            pointed.push((named, foreign));
        }
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
            let place = index_of(&table, column, sql)
                .ok_or_else(|| Error::NoSuchColumn(dequote(column.text(sql))))?;
            written.push((place, term.order));
            named.push(place);
        }
        let primary = matches!(*constraint, TableConstraint::PrimaryKey { .. });
        written_keys.push((written, conflict, primary));
        let TableConstraint::PrimaryKey { autoincrement, .. } = *constraint else {
            continue;
        };
        if key.is_some() {
            return Err(Error::ManyKeys(table.name.clone()));
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

    table.keys = own_keys(&table, &written_keys);
    table.foreign = pointing(arena, &table, &pointed, sql)?;
    table.checks = checks;

    if table.strict {
        for column in &mut table.columns {
            let Some(at) = standard(&column.declared) else {
                let named = (table.name.clone(), column.name.clone());
                return Err(if column.declared.is_empty() {
                    Error::MissingType(named.0, named.1)
                } else {
                    Error::UnknownType(named.0, named.1, column.declared.clone())
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
            return Err(Error::MissingKey(table.name.clone()));
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

/// What a `CHECK` is named in a message: the name a `CONSTRAINT` in
/// front of it gave it, or the text of the expression.
fn named_or(named: Option<Span>, text: Span, sql: &[u8]) -> Vec<u8> {
    let Some(span) = named else {
        let mut written = text.text(sql);
        while written.last().is_some_and(u8::is_ascii_whitespace) {
            written = written
                .get(..written.len().saturating_sub(1))
                .unwrap_or_default();
        }
        return written.to_vec();
    };
    dequote(span.text(sql))
}

/// The foreign keys of a table, each with the places of the columns
/// that point.
///
/// `sqlite3CreateForeignKey` takes the columns as they were written, so
/// a name no column of the table carries is a refusal.
fn pointing(
    arena: &Arena,
    table: &Table,
    written: &[(Vec<Vec<u8>>, crate::ast::Foreign)],
    sql: &[u8],
) -> Result<Vec<Foreign>, Error> {
    let mut out = Vec::new();
    for (names, foreign) in written {
        let mut columns = Vec::new();
        for name in names {
            let at = table
                .columns
                .iter()
                .position(|column| column.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| Error::ForeignColumn(name.clone()))?;
            columns.push(at);
        }
        let mut parent = Vec::new();
        for name in arena.names(foreign.columns) {
            parent.push(dequote(name.text(sql)));
        }
        // `sqlite3CreateForeignKey` reads the two lists of names
        // against each other, so a key of one width pointing at
        // another is refused where the statement is read and whatever
        // the table it points at holds.
        if !parent.is_empty() && parent.len() != columns.len() {
            return Err(Error::ForeignWidth);
        }
        out.push(Foreign {
            columns,
            table: dequote(foreign.table.text(sql)),
            parent,
            on_delete: foreign.on_delete,
            on_update: foreign.on_update,
            deferred: foreign.deferred,
        });
    }
    Ok(out)
}
