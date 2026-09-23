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
pub const MOST: usize = 100;

/// What `PRAGMA integrity_check` was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checking {
    /// No argument, or a number, which is how many problems to answer at
    /// most; nought stands for [`MOST`].
    Most(usize),
    /// A name, which is the one table the check is over.
    Table(Vec<u8>),
}

/// Whether the schema of `database` holds an object the pragma may be
/// given: a table of that name, or the schema's own table, which
/// `sqlite3LocateTable` finds and every index of which it does not.
#[must_use]
pub fn holds_object(database: &Database<'_>, name: &[u8]) -> bool {
    crate::db::schema_named(name)
        || database
            .tables()
            .any(|table| table.name.eq_ignore_ascii_case(name))
}

/// How many problems the pragma answers at most, which a name stands for
/// the hundred of.
#[must_use]
pub const fn allowed(asked: &Checking) -> usize {
    match asked {
        Checking::Most(most) => *most,
        Checking::Table(_) => MOST,
    }
}

/// What the pragma was given, read out of the text after the equals
/// sign: a word of digits is a count and every other word is a name,
/// which is why `PRAGMA integrity_check='4'` names a table.
#[must_use]
pub fn checking(written: Option<&[u8]>) -> Checking {
    let Some(text) = written else {
        return Checking::Most(MOST);
    };
    let read = crate::number::integer(text);
    if read.outcome == crate::number::Outcome::Exact {
        let most = usize::try_from(read.value).unwrap_or(MOST);
        return Checking::Most(if most == 0 { MOST } else { most });
    }
    Checking::Table(crate::schema::dequote(text))
}

/// What one run of the check found, which `sqlite3Pragma` writes in
/// three runs: the pages of the file, the counts of the indexes, the
/// rows.
struct Found {
    /// The problems of the pages, which one row carries.
    pages: Vec<Vec<u8>>,
    /// The problems of the counts.
    problems: Vec<Vec<u8>>,
    /// The problems of the rows.
    rows: Vec<Vec<u8>>,
}

impl Found {
    /// One problem of a page written down.
    fn note_page(&mut self, text: &[u8]) {
        self.pages.push(text.to_vec());
    }

    /// One problem of a count written down.
    fn note(&mut self, text: &[u8]) {
        self.problems.push(text.to_vec());
    }

    /// One problem of a row written down.
    fn note_row(&mut self, text: &[u8]) {
        self.rows.push(text.to_vec());
    }

    /// The problems in the order `sqlite3Pragma` writes them, taking
    /// what `left` still allows and counting it down.
    ///
    /// The problems of the pages are one row, which
    /// `sqlite3BtreeIntegrityCheck` writes as one text of lines under
    /// the name of the database, and each line of it counts as one.
    fn taken(self, named: &[u8], left: &mut usize) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let pages = taken_from(self.pages, left);
        if !pages.is_empty() {
            let mut text = b"*** in database ".to_vec();
            text.extend_from_slice(named);
            text.extend_from_slice(b" ***\n");
            text.extend_from_slice(&pages.join(&b'\n'));
            out.push(text);
        }
        out.extend(taken_from(self.problems, left));
        out.extend(taken_from(self.rows, left));
        out
    }
}

/// The first `left` of the problems, with `left` counted down by as many.
fn taken_from(mut held: Vec<Vec<u8>>, left: &mut usize) -> Vec<Vec<u8>> {
    held.truncate(*left);
    *left = left.saturating_sub(held.len());
    held
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
pub fn integrity(
    database: &Database<'_>,
    quick: bool,
    (asked, named): (&Checking, &[u8]),
    left: &mut usize,
) -> Result<Vec<Vec<u8>>, Error> {
    let mut found = Found {
        pages: Vec::new(),
        problems: Vec::new(),
        rows: Vec::new(),
    };
    let tables: Vec<crate::schema::Table> = database.tables().cloned().collect();
    // `tableSkipIntegrityCheck` of `research/sqlite/src/pragma.c:1700`
    // walks the one table the pragma named and no other, and a name no
    // object of the schema carries is refused.
    let over: Vec<&crate::schema::Table> = match asked {
        Checking::Most(_) => tables.iter().collect(),
        Checking::Table(name) => tables
            .iter()
            .filter(|table| table.name.eq_ignore_ascii_case(name))
            .collect(),
    };
    // The pages of the file are walked where the pragma names no table,
    // because a page of a table it does not name is one the walk would
    // count as never used.
    if matches!(asked, Checking::Most(_)) {
        pages_used(database, &mut found)?;
    }
    for table in over {
        rows_held(database, table, quick, &mut found)?;
    }
    Ok(found.taken(named, left))
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
            found.note_page(&text(
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
        found.note_page(&text(b"2nd reference to page ", number, b""));
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
///
/// `ptrmapPageno` of `research/sqlite/src/btree.c` counts one entry per
/// five usable bytes and one page for the map itself, so the map pages
/// lie at page two and every that many pages after it, which
/// [`crate::tree::map_page`] answers for one page.
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
        number = number.saturating_add(u32::try_from(span).unwrap_or(1));
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
            found.note_row(&text);
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
    let collations = crate::change::collations_of(index, database.schema_format());
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
        // An entry holds its text in the encoding the file names, and
        // the row was read into UTF-8, so the entry this row would make
        // is written into that encoding before the two are compared.
        let wanted = crate::value::written(
            &crate::change::entry_of(index, &over, values, key)?,
            database.encoding(),
        );
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
        found.note_row(&text);
    }
    if count != held_rows {
        let mut text = b"wrong # of entries in index ".to_vec();
        text.extend_from_slice(&index.name);
        found.note(&text);
    }
    if index.unique {
        for (one, other) in keys.iter().zip(keys.iter().skip(1)) {
            let columns = index.columns.len();
            // `sqlite3Pragma` of `research/sqlite/src/pragma.c:2132`
            // reads an entry that holds a null at any of its places as
            // one of its own, because a null stands equal to nothing.
            if one
                .get(..columns)
                .unwrap_or_default()
                .contains(&Value::Null)
            {
                continue;
            }
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
            found.note_row(&text);
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

/// Where one entry stands against another, place by place as the index
/// holds them.
fn order_of(
    one: &[Value],
    other: &[Value],
    collations: &[crate::value::Placing],
) -> core::cmp::Ordering {
    one.iter()
        .zip(other)
        .enumerate()
        .map(|(at, (mine, theirs))| {
            let placing = collations.get(at).copied().unwrap_or_default();
            crate::value::compare_placed(mine, theirs, placing)
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
