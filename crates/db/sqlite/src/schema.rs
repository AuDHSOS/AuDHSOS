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
    Arena, ColumnConstraint, CreateTable, ExprId, Literal, Node, Order, Span, TableBody,
    TableConstraint,
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
    /// Where it stands in the primary key, counting from one, or zero
    /// where it is not in it.
    pub key: u16,
    /// Whether it is computed.
    pub generated: Generated,
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
fn dequote(text: &[u8]) -> Vec<u8> {
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
        doubled = *byte == quote && quote != b']';
    }
    out
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
        rowid_alias: None,
        autoincrement: false,
    };
    let mut key: Option<(Vec<usize>, Order, bool)> = None;
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
            key: 0,
            generated: Generated::Never,
        };
        let mut own_key = None;
        for constraint in arena.column_constraints(written.constraints) {
            match *constraint {
                ColumnConstraint::NotNull(_) => column.not_null = true,
                ColumnConstraint::Collate(name) => {
                    column.collation =
                        Collation::of_name(&dequote(name.text(sql))).ok_or(Error::NoCollation)?;
                }
                ColumnConstraint::Default { text, .. } => {
                    column.default = Some(text.text(sql).to_vec());
                }
                ColumnConstraint::PrimaryKey {
                    order,
                    autoincrement,
                    ..
                } => own_key = Some((order, autoincrement)),
                ColumnConstraint::Generated { kind, .. } => {
                    column.generated = generated_kind(kind, sql)?;
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
        let TableConstraint::PrimaryKey {
            columns,
            autoincrement,
            ..
        } = *constraint
        else {
            continue;
        };
        if key.is_some() {
            return Err(Error::ManyKeys);
        }
        let mut named = Vec::new();
        let mut order = Order::Unspecified;
        for (at, term) in arena.orders(columns).iter().enumerate() {
            if at == 0 {
                order = term.order;
            }
            // A term that is not a name is refused where the key's own
            // index would be built.
            let column = key_name(arena, term.expr).ok_or(Error::KeyExpression)?;
            named.push(index_of(&table, column, sql).ok_or(Error::NoSuchColumn)?);
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
