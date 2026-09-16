// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Where a statement of the schema names a table, and that statement
//! written again under another name for it.
//!
//! `sqlite3RenameTableFunc` is the document: `ALTER TABLE ... RENAME TO`
//! reads every statement the schema holds, marks the places that name
//! the table, and writes the name of the new table there. A place is
//! marked from the tree the parser built and not from the text, so a
//! column or a string that reads like the table is left alone.

use alloc::vec::Vec;

use crate::ast::{Definition, Span, TriggerStep};

/// Whether two names are the same name, which is `sqlite3StrICmp` over
/// the text with the quotes taken off.
fn same(one: &[u8], other: &[u8]) -> bool {
    crate::schema::dequote(one).eq_ignore_ascii_case(other)
}

/// Where `sql` names `table`, earliest first.
///
/// A statement the parser refuses names nothing, because a place can
/// only be marked from the tree.
#[must_use]
pub fn places(sql: &[u8], table: &[u8]) -> Vec<Span> {
    let Ok((arena, definition)) = crate::parse::definition(sql) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match definition {
        Definition::Table(made) if same(made.name.text(sql), table) => out.push(made.name),
        Definition::Index(made) if same(made.table.text(sql), table) => out.push(made.table),
        Definition::Trigger(made) if same(made.table.text(sql), table) => out.push(made.table),
        _ => {}
    }
    // A `WITH` term stands for itself, so a name it carries names no
    // table of the schema.
    let terms: Vec<Span> = arena.all_ctes().map(|cte| cte.name).collect();
    let shadowed = |name: Span| {
        terms
            .iter()
            .any(|term| same(term.text(sql), &crate::schema::dequote(name.text(sql))))
    };
    for source in arena.all_sources() {
        if let crate::ast::SourceKind::Table {
            schema: _, name, ..
        } = source.kind
            && same(name.text(sql), table)
            && !shadowed(name)
        {
            out.push(name);
        }
    }
    for step in arena.all_steps() {
        let named = match step {
            TriggerStep::Insert(statement) => statement.name,
            TriggerStep::Update(statement) => statement.name,
            TriggerStep::Delete(statement) => statement.name,
            TriggerStep::Select(_) => continue,
        };
        if same(named.text(sql), table) {
            out.push(named);
        }
    }
    out.sort_unstable_by_key(|span| span.start);
    out.dedup_by_key(|span| span.start);
    out
}

/// The name written as `sqlite3_mprintf("\"%w\"")` writes it: in double
/// quotes, with every double quote inside it doubled.
#[must_use]
pub fn quoted(name: &[u8]) -> Vec<u8> {
    let mut out = alloc::vec![b'"'];
    for byte in name {
        if *byte == b'"' {
            out.push(b'"');
        }
        out.push(*byte);
    }
    out.push(b'"');
    out
}

/// `sql` with `name` written at each of `places`, which the caller read
/// with [`places`].
///
/// Writing `n` bytes costs O(n).
#[must_use]
pub fn written(sql: &[u8], places: &[Span], name: &[u8]) -> Vec<u8> {
    let quoted = quoted(name);
    let mut out = Vec::new();
    let mut at = 0_usize;
    for place in places {
        out.extend_from_slice(sql.get(at..place.start).unwrap_or_default());
        out.extend_from_slice(&quoted);
        at = place.start.saturating_add(place.len);
    }
    out.extend_from_slice(sql.get(at..).unwrap_or_default());
    out
}

/// The name of an index SQLite made for a key of `from`, written for
/// `to`, which is `sqlite3RenameTable` rewriting `sqlite_autoindex_`.
///
/// A name that is not such an index answers nothing.
#[must_use]
pub fn automatic(name: &[u8], from: &[u8], to: &[u8]) -> Option<Vec<u8>> {
    let head = b"sqlite_autoindex_";
    if !name.starts_with(head) {
        return None;
    }
    let rest = name.get(head.len()..)?;
    let tail = rest.get(..from.len())?;
    if !tail.eq_ignore_ascii_case(from) {
        return None;
    }
    let count = rest.get(from.len()..)?;
    if !count.starts_with(b"_") {
        return None;
    }
    let mut out = head.to_vec();
    out.extend_from_slice(to);
    out.extend_from_slice(count);
    Some(out)
}
