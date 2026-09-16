// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `PRAGMA integrity_check`: what the file says against what it holds.
//!
//! `sqlite3Pragma` under `PragTyp_INTEGRITY_CHECK` walks every tree the
//! schema names, holds every row to the columns that may not be
//! nothing, and holds every index to the rows it is over. The answer is
//! one row per problem, and the one word `ok` where the walk found
//! none.
//!
//! Walking the file costs O(p) in its pages, and holding a table of `n`
//! rows to an index costs O(n log n).

use alloc::vec::Vec;

use crate::db::{Database, Error};
use crate::image::Image;
use crate::value::Value;

/// How many problems one run answers, which is `PRAGMA
/// integrity_check(N)` at its default.
const MOST: usize = 100;

/// What one run of the check found: the problems, in the order it found
/// them.
struct Found {
    /// The problems.
    problems: Vec<Vec<u8>>,
}

impl Found {
    /// One problem written down, up to [`MOST`] of them.
    fn note(&mut self, text: &[u8]) {
        if self.problems.len() < MOST {
            self.problems.push(text.to_vec());
        }
    }
}

/// What `PRAGMA integrity_check` answers over `database`: one row per
/// problem, and one row of `ok` where it found none.
///
/// `quick` is `PRAGMA quick_check`, which holds no index to the rows it
/// is over and walks the pages alone.
///
/// # Errors
///
/// [`Error`] names what reading the file refuses, which is a file this
/// check cannot read rather than a file it found a problem in.
pub fn integrity(database: &Database<'_>, quick: bool) -> Result<Vec<Vec<u8>>, Error> {
    let mut found = Found {
        problems: Vec::new(),
    };
    pages_used(database, &mut found)?;
    let tables: Vec<crate::schema::Table> = database.tables().cloned().collect();
    for table in &tables {
        rows_held(database, table, quick, &mut found)?;
    }
    if found.problems.is_empty() {
        found.note(b"ok");
    }
    Ok(found.problems)
}

/// Every page of the file reached once: from a tree the schema names,
/// from the free list, from a pointer map, or as page one, which is the
/// root of the schema's own tree.
///
/// A page already reached is not walked again, so a file whose pages
/// name each other in a ring is walked once round it and no further.
fn pages_used(database: &Database<'_>, found: &mut Found) -> Result<(), Error> {
    let image = database.image();
    let count = usize::try_from(image.pages()).unwrap_or(0);
    let mut seen = alloc::vec![0_u8; count.saturating_add(1)];
    for number in map_pages(image) {
        mark(&mut seen, number, found);
    }
    free_pages(image, &mut seen, found)?;
    for root in roots(database) {
        tree_pages(image, root, &mut seen, found)?;
    }
    for (at, slot) in seen.iter().enumerate().skip(1) {
        if *slot == 0 {
            found.note(&text(
                b"Page ",
                u32::try_from(at).unwrap_or(0),
                b": never used",
            ));
        }
    }
    Ok(())
}

/// Page `number` marked reached, and whether this is the first time it
/// was.
fn mark(seen: &mut [u8], number: u32, found: &mut Found) -> bool {
    let at = usize::try_from(number).unwrap_or(0);
    let mut first = true;
    for slot in seen.iter_mut().skip(at).take(1) {
        first = *slot == 0;
        *slot = 1;
    }
    if !first {
        found.note(&text(b"2nd reference to page ", number, b""));
    }
    first
}

/// The page each tree of the schema begins at, the schema's own first.
fn roots(database: &Database<'_>) -> Vec<u32> {
    let mut out = alloc::vec![crate::image::SCHEMA_ROOT];
    for table in database.tables() {
        out.extend(database.table(&table.name).map(|(_, root)| root));
        out.extend(database.indexes(&table.name).iter().map(|kept| kept.root));
    }
    out
}

/// Every page of the tree at `root` marked reached, the pages of the
/// chains its cells run onto among them.
fn tree_pages(
    image: &Image<'_>,
    root: u32,
    seen: &mut [u8],
    found: &mut Found,
) -> Result<(), Error> {
    let mut stack = alloc::vec![root];
    while let Some(number) = stack.pop() {
        if !mark(seen, number, found) {
            continue;
        }
        let page = image.page(number)?;
        let interior = page.kind().is_interior();
        for at in 0..page.cells() {
            if interior {
                stack.push(page.child(at)?);
            }
            let chain = match page.cell(at)? {
                crate::page::Cell::TableLeaf { payload, .. }
                | crate::page::Cell::IndexLeaf { payload }
                | crate::page::Cell::IndexInterior { payload, .. } => payload.overflow,
                crate::page::Cell::TableInterior { .. } => None,
            };
            chain_pages(image, chain, seen, found)?;
        }
        stack.extend(page.right_most());
    }
    Ok(())
}

/// Every page of one overflow chain marked reached.
fn chain_pages(
    image: &Image<'_>,
    first: Option<u32>,
    seen: &mut [u8],
    found: &mut Found,
) -> Result<(), Error> {
    let mut number = first;
    while let Some(page) = number {
        if !mark(seen, page, found) {
            break;
        }
        let next =
            crate::bytes::u32_at(image.page_bytes(page)?, 0).ok_or(Error::NoTable(Vec::new()))?;
        number = (next != 0).then_some(next);
    }
    Ok(())
}

/// Every page the free list holds marked reached: the trunks and their
/// leaves.
fn free_pages(image: &Image<'_>, seen: &mut [u8], found: &mut Found) -> Result<(), Error> {
    let usable = image
        .header()
        .page_size
        .saturating_sub(u32::from(image.header().reserved));
    let mut trunk = image.header().freelist;
    while trunk != 0 {
        if !mark(seen, trunk, found) {
            break;
        }
        let bytes = image.page_bytes(trunk)?;
        let next = crate::bytes::u32_at(bytes, 0).ok_or(Error::NoTable(Vec::new()))?;
        let leaves = crate::bytes::u32_at(bytes, 4).ok_or(Error::NoTable(Vec::new()))?;
        // The array of leaves begins eight bytes in, so a trunk names
        // at most that many fewer than the page holds words.
        let most = usable.saturating_sub(8).saturating_div(4);
        for at in 0..leaves.min(most) {
            let slot = usize::try_from(at)
                .unwrap_or(0)
                .saturating_mul(4)
                .saturating_add(8);
            mark(seen, crate::bytes::u32_at(bytes, slot).unwrap_or(0), found);
        }
        trunk = next;
    }
    Ok(())
}

/// The pages of the pointer map, which a file that vacuums itself has
/// one of every `usable / 5 + 1` pages.
fn map_pages(image: &Image<'_>) -> Vec<u32> {
    if image.header().largest_root == 0 {
        return Vec::new();
    }
    let usable = usize::try_from(image.header().page_size)
        .unwrap_or(0)
        .saturating_sub(usize::from(image.header().reserved));
    let span = usable.saturating_div(5).saturating_add(1);
    let mut out = Vec::new();
    let mut number = 2_u32;
    while number <= image.pages() {
        out.push(number);
        number = number.saturating_add(u32::try_from(span).unwrap_or(1).saturating_add(1));
    }
    out
}

/// Every row of one table held to the columns that may not be nothing
/// and to the indexes the table carries.
fn rows_held(
    database: &Database<'_>,
    table: &crate::schema::Table,
    quick: bool,
    found: &mut Found,
) -> Result<(), Error> {
    let rows = database.held_rows_of(&table.name)?;
    for (_, values) in &rows {
        for (at, column) in table.columns.iter().enumerate() {
            if !column.not_null || values.get(at) != Some(&Value::Null) {
                continue;
            }
            let mut text = b"NULL value in ".to_vec();
            text.extend_from_slice(&table.name);
            text.push(b'.');
            text.extend_from_slice(&column.name);
            found.note(&text);
        }
    }
    if quick {
        return Ok(());
    }
    for kept in database.indexes(&table.name) {
        entries_held(database, &kept, &rows, found)?;
    }
    Ok(())
}

/// One index held to the rows it is over: every row has an entry, the
/// index holds no entry more than the rows have, and a unique index
/// holds no key twice.
fn entries_held(
    database: &Database<'_>,
    kept: &crate::db::Indexed<'_>,
    rows: &[(Vec<Value>, Vec<Value>)],
    found: &mut Found,
) -> Result<(), Error> {
    let index = kept.index;
    let root = kept.root;
    let over = crate::change::Over {
        arena: kept.arena,
        sql: kept.sql,
        table: kept.table,
        encoding: database.encoding(),
    };
    let image = database.image();
    let collations: Vec<crate::value::Collation> = index
        .columns
        .iter()
        .map(|column| column.collation)
        .collect();
    // The entry of a row ends with the key of that row, which is one
    // value for a rowid and the columns of the `PRIMARY KEY` for a
    // table that keeps its rows in the key's own tree.
    let tail = rows.first().map_or(1, |(key, _)| key.len());
    let mut keys: Vec<Vec<Value>> = Vec::new();
    let mut payload = Vec::new();
    let mut count = 0_usize;
    for entry in image.entries(root) {
        let entry = entry?;
        payload.resize(entry.total, 0);
        image.read_payload(&entry, &mut payload)?;
        let record = crate::record::Record::parse(&payload)?;
        let mut key = Vec::new();
        let width = index.columns.len().saturating_add(tail);
        for at in 0..width {
            key.push(held(record.value(at)?));
        }
        keys.push(key);
        count = count.saturating_add(1);
    }
    // The entries are put in order once, so holding `n` rows to them
    // costs O(n log n) and not O(n²).
    keys.sort_by(|one, other| order_of(one, other, &collations));
    // `sqlite3Pragma` counts the rows of the table as it walks them and
    // names a row by that count, which is register 7 of the routine it
    // writes and not the key of the row.
    let mut held_rows = 0_usize;
    for (at, (key, values)) in rows.iter().enumerate() {
        if !crate::change::indexes_row(index, &over, values)? {
            continue;
        }
        held_rows = held_rows.saturating_add(1);
        let wanted = crate::change::entry_of(index, &over, values, key)?;
        if keys
            .binary_search_by(|held| order_of(held, &wanted, &collations))
            .is_ok()
        {
            continue;
        }
        let mut text = b"row ".to_vec();
        let counted = i64::try_from(at).unwrap_or(0).saturating_add(1);
        text.extend_from_slice(&crate::number::integer_text(counted));
        text.extend_from_slice(b" missing from index ");
        text.extend_from_slice(&index.name);
        found.note(&text);
    }
    if count != held_rows {
        let mut text = b"wrong # of entries in index ".to_vec();
        text.extend_from_slice(&index.name);
        found.note(&text);
    }
    if index.unique {
        for (one, other) in keys.iter().zip(keys.iter().skip(1)) {
            let columns = index.columns.len();
            if order_of(
                one.get(..columns).unwrap_or_default(),
                other.get(..columns).unwrap_or_default(),
                &collations,
            ) != core::cmp::Ordering::Equal
            {
                continue;
            }
            let mut text = b"non-unique entry in index ".to_vec();
            text.extend_from_slice(&index.name);
            found.note(&text);
            break;
        }
    }
    Ok(())
}

/// One value of a record as a value of its own.
fn held(value: Option<crate::record::Value<'_>>) -> Value {
    match value {
        None | Some(crate::record::Value::Null) => Value::Null,
        Some(crate::record::Value::Int(number)) => Value::Int(number),
        Some(crate::record::Value::Real(number)) => Value::Real(number),
        Some(crate::record::Value::Text(bytes)) => Value::Text(bytes.to_vec()),
        Some(crate::record::Value::Blob(bytes)) => Value::Blob(bytes.to_vec()),
    }
}

/// Where one entry stands against another, column by column under the
/// collation each is held in.
fn order_of(
    one: &[Value],
    other: &[Value],
    collations: &[crate::value::Collation],
) -> core::cmp::Ordering {
    one.iter()
        .zip(other)
        .enumerate()
        .map(|(at, (mine, theirs))| {
            let collation = collations
                .get(at)
                .copied()
                .unwrap_or(crate::value::Collation::Binary);
            crate::value::compare(mine, theirs, collation)
        })
        .find(|order| *order != core::cmp::Ordering::Equal)
        .unwrap_or(core::cmp::Ordering::Equal)
}

/// One message of a word, a number and a word.
fn text(before: &[u8], number: u32, after: &[u8]) -> Vec<u8> {
    let mut out = before.to_vec();
    out.extend_from_slice(&crate::number::integer_text(i64::from(number)));
    out.extend_from_slice(after);
    out
}
