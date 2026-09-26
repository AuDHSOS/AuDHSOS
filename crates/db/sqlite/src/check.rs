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
    (quick, checked): (bool, bool),
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
        rows_held(database, table, (quick, checked), &mut found)?;
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
///
/// `sqlite3Pragma` of `research/sqlite/src/pragma.c:1822` writes the
/// count of every index before it walks the rows, and holds each row to
/// the columns and then to every index in turn, so the problems of one
/// row stand together.
fn rows_held(
    database: &Database<'_>,
    table: &crate::schema::Table,
    (quick, checked): (bool, bool),
    found: &mut Found,
) -> Result<(), Error> {
    let rows = database.held_rows_of(&table.name)?;
    let mut kept = Vec::new();
    if !quick {
        for indexed in database.indexes(&table.name) {
            kept.push(entries_of(database, indexed)?);
        }
    }
    for held in &kept {
        counted_held(&rows, held, found)?;
    }
    for (at, (key, values)) in rows.iter().enumerate() {
        nulls_held(table, values, found);
        types_held(table, values, found);
        if checked {
            checks_held(database, table, values, found)?;
        }
        for held in &kept {
            row_held(database, table, held, (at, key, values), found)?;
        }
    }
    Ok(())
}

/// A `CHECK` of the table one row does not hold to, which the check
/// names the table in.
///
/// `sqlite3Pragma` of `research/sqlite/src/pragma.c:2050` reads the
/// checks of a table against every row of it, and `PRAGMA
/// ignore_check_constraints` leaves them unread, which the caller says
/// with `checked`. Reading one row costs O(c) in the checks.
fn checks_held(
    database: &Database<'_>,
    table: &crate::schema::Table,
    values: &[Value],
    found: &mut Found,
) -> Result<(), Error> {
    if database.refused_check_of(table, values)?.is_none() {
        return Ok(());
    }
    let mut text = b"CHECK constraint failed in ".to_vec();
    text.extend_from_slice(&table.name);
    found.note_row(&text);
    Ok(())
}

/// Every column of one row that may not be nothing and holds nothing.
fn nulls_held(table: &crate::schema::Table, values: &[Value], found: &mut Found) {
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

/// Every column of one row that holds a value of another type than the
/// column is declared.
///
/// `sqlite3Pragma` of `research/sqlite/src/pragma.c:1986` holds three
/// rules over the types a column holds, each of which `OP_IsType` reads
/// the type the record wrote. Reading one row costs O(c) in its columns.
fn types_held(table: &crate::schema::Table, values: &[Value], found: &mut Found) {
    for (column, value) in table.columns.iter().zip(values) {
        let Some(word) = mistyped(table.strict, column, value) else {
            continue;
        };
        let mut text = word;
        text.extend_from_slice(b" value in ");
        text.extend_from_slice(&table.name);
        text.push(b'.');
        text.extend_from_slice(&column.name);
        found.note_row(&text);
    }
}

/// The words a problem of one value opens with, and nothing where the
/// value is of the type the column is declared.
///
/// A column of a `STRICT` table that is not `ANY` holds a value of its
/// own type or nothing, where `aStdTypeMask` counts an integer as a
/// value of a `REAL` column because a real that is a whole number is
/// stored as one. A column of any other table holds no number where it
/// is declared `TEXT`, and no text that converts to a number where it
/// asks for one.
fn mistyped(strict: bool, column: &crate::schema::Column, value: &Value) -> Option<Vec<u8>> {
    if *value == Value::Null {
        return None;
    }
    if strict {
        let held = match column.declared.as_slice() {
            b"ANY" => return None,
            b"BLOB" => matches!(value, Value::Blob(_)),
            b"INT" | b"INTEGER" => matches!(value, Value::Int(_)),
            b"REAL" => matches!(value, Value::Int(_) | Value::Real(_)),
            // A column of a `STRICT` table is declared one of the six,
            // which `crate::schema` holds it to, so the rest is `TEXT`.
            _ => matches!(value, Value::Text(_)),
        };
        if held {
            return None;
        }
        let mut out = b"non-".to_vec();
        out.extend_from_slice(&column.declared);
        return Some(out);
    }
    if column.affinity == crate::value::Affinity::Text {
        return matches!(value, Value::Int(_) | Value::Real(_)).then(|| b"NUMERIC".to_vec());
    }
    if column.affinity.numeric() {
        let mut held = value.clone();
        crate::value::apply(&mut held, crate::value::Affinity::Numeric);
        return matches!(held, Value::Int(_) | Value::Real(_))
            .then(|| b"TEXT".to_vec())
            .filter(|_| matches!(value, Value::Text(_)));
    }
    None
}

/// One index read once: its entries in the order it holds them, what a
/// comparison of two entries does, and how many entries it holds.
struct Kept<'a> {
    /// The index and where its tree begins.
    indexed: crate::db::Indexed<'a>,
    /// The entries, in order.
    keys: Vec<Vec<Value>>,
    /// How each place of an entry is compared.
    collations: Vec<crate::value::Placing>,
}

impl Kept<'_> {
    /// What a statement of the index is read against.
    const fn over(&self, database: &Database<'_>) -> crate::change::Over<'_> {
        crate::change::Over {
            arena: self.indexed.arena,
            sql: self.indexed.sql,
            table: self.indexed.table,
            encoding: database.encoding(),
        }
    }
}

/// Every entry of one index read into order, which costs O(n log n) in
/// the entries so that holding `n` rows to them costs O(n log n) and not
/// O(n²).
fn entries_of<'a>(
    database: &Database<'_>,
    indexed: crate::db::Indexed<'a>,
) -> Result<Kept<'a>, Error> {
    let index = indexed.index;
    let image = database.image();
    let collations = crate::change::collations_of(index, database.schema_format());
    let rows = database.held_rows_of(&index.table)?;
    // The entry of a row ends with the key of that row, which is one
    // value for a rowid and the columns of the `PRIMARY KEY` for a
    // table that keeps its rows in the key's own tree.
    let tail = rows.first().map_or(1, |(key, _)| key.len());
    let mut keys: Vec<Vec<Value>> = Vec::new();
    let mut payload = Vec::new();
    for entry in image.entries(indexed.root) {
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
    }
    keys.sort_by(|one, other| order_of(one, other, &collations));
    Ok(Kept {
        indexed,
        keys,
        collations,
    })
}

/// Whether the index holds as many entries as the rows it is over,
/// counting the rows a partial index leaves out.
fn counted_held(
    rows: &[(Vec<Value>, Vec<Value>)],
    held: &Kept<'_>,
    found: &mut Found,
) -> Result<(), Error> {
    let index = held.indexed.index;
    let over = crate::change::Over {
        arena: held.indexed.arena,
        sql: held.indexed.sql,
        table: held.indexed.table,
        encoding: crate::header::Encoding::Utf8,
    };
    let mut count = 0_usize;
    for (_, values) in rows {
        if crate::change::indexes_row(index, &over, values)? {
            count = count.saturating_add(1);
        }
    }
    if count == held.keys.len() {
        return Ok(());
    }
    let mut text = b"wrong # of entries in index ".to_vec();
    text.extend_from_slice(&index.name);
    found.note(&text);
    Ok(())
}

/// One row held to one index: the index holds an entry for the row, and
/// a unique index holds no other entry under the key of that entry.
///
/// `sqlite3Pragma` counts the rows of the table as it walks them and
/// names a row by that count, which is register 7 of the routine it
/// writes and not the key of the row.
fn row_held(
    database: &Database<'_>,
    table: &crate::schema::Table,
    held: &Kept<'_>,
    row: (usize, &[Value], &[Value]),
    found: &mut Found,
) -> Result<(), Error> {
    let (at, key, values) = row;
    let index = held.indexed.index;
    let over = held.over(database);
    if !crate::change::indexes_row(index, &over, values)? {
        return Ok(());
    }
    // An entry holds its text in the encoding the file names, and the
    // row was read into UTF-8, so the entry this row would make is
    // written into that encoding before the two are compared.
    let wanted = crate::value::written(
        &crate::change::entry_of(index, &over, values, key)?,
        database.encoding(),
    );
    let Ok(place) = held
        .keys
        .binary_search_by(|one| order_of(one, &wanted, &held.collations))
    else {
        let mut text = b"row ".to_vec();
        let counted = i64::try_from(at).unwrap_or(0).saturating_add(1);
        text.extend_from_slice(&crate::number::integer_text(counted));
        text.extend_from_slice(b" missing from index ");
        text.extend_from_slice(&index.name);
        found.note_row(&text);
        return Ok(());
    };
    if shares_next(held, table, (place, &wanted)) {
        let mut text = b"non-unique entry in index ".to_vec();
        text.extend_from_slice(&index.name);
        found.note_row(&text);
    }
    Ok(())
}

/// Whether the entry at `place` of a unique index and the entry after it
/// hold one key.
///
/// `sqlite3Pragma` of `research/sqlite/src/pragma.c:2135` passes over an
/// entry that holds nothing at a place whose column may hold nothing,
/// because nothing stands equal to nothing, and holds a place whose
/// column may not to its key whatever it holds.
fn shares_next(held: &Kept<'_>, table: &crate::schema::Table, row: (usize, &[Value])) -> bool {
    let (place, one) = row;
    let index = held.indexed.index;
    if !index.unique {
        return false;
    }
    for (at, keyed) in index.columns.iter().enumerate() {
        let told = keyed
            .place()
            .and_then(|at| table.columns.get(at))
            .is_some_and(|column| column.not_null);
        if !told && one.get(at) == Some(&Value::Null) {
            return false;
        }
    }
    let Some(other) = held.keys.get(place.saturating_add(1)) else {
        return false;
    };
    order_of_places(one, other, &held.collations, index.columns.len()) == core::cmp::Ordering::Equal
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

/// Where the first `places` of one entry stand against another's, which
/// is how a unique index compares two entries: the key that ends an
/// entry stands outside the places the index holds two rows apart by.
fn order_of_places(
    one: &[Value],
    other: &[Value],
    collations: &[crate::value::Placing],
    places: usize,
) -> core::cmp::Ordering {
    one.iter()
        .zip(other)
        .take(places)
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
