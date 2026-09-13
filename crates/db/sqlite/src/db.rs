// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A database that answers a statement.
//!
//! The file is read where it lies, its schema is read out of the
//! `CREATE` text `sqlite_schema` holds, and a statement is answered by
//! walking a table's tree once: O(n) in its rows for a scan, O(n log n)
//! where an `ORDER BY` has to sort them, and O(n·g) where a `GROUP BY`
//! has to find which of `g` groups each row belongs to. Nothing is
//! compiled and no index is used yet, so the rows are right and the plan
//! is not — which is the order document 16 puts them in.
//!
//! What is answered is one `SELECT` over one table, or over none, with
//! or without a grouping. Every other shape refuses by name rather than
//! answering something near it.

use alloc::vec::Vec;

use crate::agg::{self, Accumulator, Aggregate};
use crate::ast::{
    Arena, Compound, Distinct, ExprId, JoinKind, Literal, Node, Order, Range, ResultColumn, Select,
    SelectId, SourceKind, Span, UnaryOp,
};
use crate::eval::{self, evaluate_collated, evaluate_row};
use crate::header::Encoding;
use crate::image::Image;
use crate::parse;
use crate::record;
use crate::schema::{self, Generated, Table};
use crate::value::{Affinity, Collation, Value, apply_comparison, compare, compare_affinity};
use crate::{error, number};

/// Why a database could not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file is not one this crate reads.
    Image(error::Error),
    /// The statement is not one this crate reads.
    Parse(parse::Error),
    /// A definition in the schema is not one this crate reads.
    Schema(schema::Error),
    /// An expression could not be answered.
    Eval(eval::Error),
    /// A table the statement names is not in the schema.
    NoTable,
    /// An `ORDER BY` that counts to a column the answer does not have.
    OrderRange,
    /// An aggregate where there is nothing to aggregate over: in a
    /// `WHERE`, in a `GROUP BY`, inside another aggregate, or in an
    /// `ORDER BY` of a statement that does not group.
    Aggregate,
    /// A `HAVING` on a statement that groups nothing.
    Having,
    /// Two sides of a compound that answer different numbers of columns.
    Compound,
    /// An `ORDER BY` of a compound that names none of its columns, which
    /// is the only thing one may name.
    OrderMatch,
    /// A `VALUES` whose rows are not all the same width.
    Values,
    /// A join that cannot be made: a `NATURAL` with a condition written
    /// on it as well, or a `USING` that names a column one of the two
    /// tables does not have.
    Join,
    /// A `*` over two tables of one name, every column of which two
    /// tables would answer.
    Ambiguous,
    /// A table whose rows live in the key's own tree. Reading one is
    /// reading an index, which is a later step.
    WithoutRowid,
    /// A shape of statement this engine does not answer yet: a join, a
    /// grouping, a compound, a statement inside a statement.
    Unsupported,
}

impl From<error::Error> for Error {
    fn from(error: error::Error) -> Self {
        Error::Image(error)
    }
}

impl From<parse::Error> for Error {
    fn from(error: parse::Error) -> Self {
        Error::Parse(error)
    }
}

impl From<schema::Error> for Error {
    fn from(error: schema::Error) -> Self {
        Error::Schema(error)
    }
}

impl From<eval::Error> for Error {
    fn from(error: eval::Error) -> Self {
        Error::Eval(error)
    }
}

/// One table of the schema, and where its rows are.
#[derive(Clone, Debug)]
struct Stored {
    /// The table as its statement describes it.
    table: Table,
    /// The page its tree begins at.
    root: u32,
}

/// One table of a `FROM` clause, and how it attaches to the ones before
/// it.
struct Side<'a> {
    /// The table, and where its rows are.
    stored: &'a Stored,
    /// The name the statement gave it, where it gave one.
    alias: Option<&'a [u8]>,
    /// Which join attaches it.
    kind: JoinKind,
    /// The `ON` condition written on it.
    on: Option<ExprId>,
    /// The columns a `USING` or a `NATURAL` matches it by, which are
    /// also the columns it does not answer a bare name with.
    using: Vec<Vec<u8>>,
}

impl Side<'_> {
    /// What the statement calls this table, which is its alias where it
    /// was given one and its own name otherwise.
    fn calls(&self) -> &[u8] {
        self.alias.unwrap_or(&self.stored.table.name)
    }

    /// Whether `named` is what the statement calls this table.
    fn named(&self, named: &[u8]) -> bool {
        self.calls().eq_ignore_ascii_case(named)
    }

    /// Whether a `*` leaves this table's column of that name out,
    /// which it does for the side a `USING` or a `NATURAL` matched.
    fn hides(&self, column: &[u8]) -> bool {
        self.using
            .iter()
            .any(|name| name.eq_ignore_ascii_case(column))
    }
}

/// A database file, with its schema read.
#[derive(Clone, Debug)]
pub struct Database<'a> {
    /// The file.
    image: Image<'a>,
    /// Its tables.
    tables: Vec<Stored>,
    /// What encoding its text is in.
    encoding: Encoding,
}

/// What a statement answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// The name of each column, as SQLite would name it.
    pub names: Vec<Vec<u8>>,
    /// The rows, each as many values as there are names.
    pub rows: Vec<Vec<Value>>,
}

impl<'a> Database<'a> {
    /// Opens `bytes` and reads its schema.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not read and why.
    pub fn open(bytes: &'a [u8]) -> Result<Self, Error> {
        let image = Image::open(bytes)?;
        let encoding = image.header().encoding;
        let mut tables = Vec::new();
        let mut payload = Vec::new();
        for row in image.schema() {
            let row = row?;
            read_payload(&image, &row.payload, &mut payload)?;
            let record = record::Record::parse(&payload)?;
            let text = |at: usize| -> Result<Vec<u8>, Error> {
                Ok(match record.value(at)? {
                    Some(record::Value::Text(bytes)) => decode(bytes, encoding),
                    _ => Vec::new(),
                })
            };
            if text(0)? != b"table" {
                continue;
            }
            let Some(record::Value::Int(root)) = record.value(3)? else {
                continue;
            };
            let sql = text(4)?;
            if sql.is_empty() {
                continue;
            }
            let (arena, definition) = parse::definition(&sql)?;
            let crate::ast::Definition::Table(written) = definition else {
                continue;
            };
            let mut table = schema::table(&arena, &written, &sql)?;
            // `BINARY` is the same collation under the same name
            // whatever the encoding, and it answers by the bytes the
            // file holds.
            let binary = binary_of(encoding);
            if binary != Collation::Binary {
                for column in &mut table.columns {
                    if column.collation == Collation::Binary {
                        column.collation = binary;
                    }
                }
            }
            tables.push(Stored {
                table,
                root: u32::try_from(root).unwrap_or(0),
            });
        }
        Ok(Database {
            image,
            tables,
            encoding,
        })
    }

    /// The tables of the schema, in the order the file holds them.
    pub fn tables(&self) -> impl Iterator<Item = &Table> {
        self.tables.iter().map(|stored| &stored.table)
    }

    /// What a comparison uses where nothing writes a collation, which
    /// is `BINARY` over the encoding the file keeps its text in.
    const fn collation(&self) -> Collation {
        binary_of(self.encoding)
    }

    /// The table `name` names.
    fn find(&self, name: &[u8]) -> Option<&Stored> {
        self.tables
            .iter()
            .find(|stored| stored.table.name.eq_ignore_ascii_case(name))
    }

    /// What `sql` answers.
    ///
    /// # Errors
    ///
    /// [`Error`] names what it could not answer and why.
    pub fn query(&self, sql: &[u8]) -> Result<Answer, Error> {
        let (arena, root) = parse::statement(sql)?;
        self.statement(&arena, root, sql)
    }

    /// A statement: one core, or several put together.
    ///
    /// A compound is answered core by core and then merged left to
    /// right, which is the shape `multiSelect` compiles: every operator
    /// but `UNION ALL` answers its rows sorted and each of them once.
    /// Its `ORDER BY` and its `LIMIT` belong to the whole of it; a
    /// statement of one core sorts and limits its own rows, because a
    /// term of that `ORDER BY` may be an expression over the row and the
    /// row is gone by the time the answer is put together.
    fn statement(&self, arena: &Arena, id: SelectId, sql: &[u8]) -> Result<Answer, Error> {
        let first = arena.select(id).ok_or(Error::Unsupported)?;
        if first.compound.is_none() {
            return Ok(self.core(arena, id, sql, true)?.0);
        }
        let mut answers: Vec<Answer> = Vec::new();
        let mut operators: Vec<Compound> = Vec::new();
        let mut written: Vec<Option<Collation>> = Vec::new();
        let mut at = Some(id);
        while let Some(id) = at {
            let core = arena.select(id).ok_or(Error::Unsupported)?;
            let (answer, mine) = self.core(arena, id, sql, false)?;
            match answers.first() {
                None => written = mine,
                Some(first) => {
                    if first.names.len() != answer.names.len() {
                        return Err(Error::Compound);
                    }
                    // A column compares under the collation of the first
                    // core that writes one, which is
                    // `multiSelectCollSeq`.
                    for (slot, one) in written.iter_mut().zip(mine) {
                        *slot = slot.or(one);
                    }
                }
            }
            answers.push(answer);
            at = core.compound.map(|(operator, next)| {
                operators.push(operator);
                next
            });
        }
        let collations: Vec<Collation> = written
            .iter()
            .map(|one| one.unwrap_or(self.collation()))
            .collect();
        let mut rest = answers.into_iter();
        let mut answer = rest.next().ok_or(Error::Unsupported)?;
        for (operator, right) in operators.into_iter().zip(rest) {
            combine(operator, &mut answer, right, &collations);
        }
        let keys = matched(arena, &first, sql, &answer.names)?;
        if !keys.is_empty() {
            sort_by_keys(&mut answer.rows, &keys, &collations);
        }
        limit(arena, &first, sql, &mut answer.rows)?;
        Ok(answer)
    }

    /// One core of a statement, with the collation each of its columns
    /// compares under. `whole` asks for the `ORDER BY` and the `LIMIT`,
    /// which belong to a core only where it is the statement.
    fn core(
        &self,
        arena: &Arena,
        id: SelectId,
        sql: &[u8],
        whole: bool,
    ) -> Result<(Answer, Vec<Option<Collation>>), Error> {
        let select = arena.select(id).ok_or(Error::Unsupported)?;
        if !select.ctes.is_empty() {
            // A `WITH` names a statement a statement may read, which is
            // a later step.
            return Err(Error::Unsupported);
        }
        if !select.values.is_empty() {
            return listed(arena, &select, sql);
        }
        let sides = self.sides(arena, &select, sql)?;
        let names = names(arena, &select, sql, &sides)?;
        let written = collations(arena, &select, sql, &sides);
        let collations: Vec<Collation> = written
            .iter()
            .map(|one| one.unwrap_or(self.collation()))
            .collect();
        let keys = if whole {
            keys(arena, &select, sql, &names)?
        } else {
            Vec::new()
        };
        let calls = aggregates(arena, &select, sql)?;
        let mut rows: Vec<Sorted> = Vec::new();
        if calls.is_empty() && select.group.is_empty() {
            self.scan(&sides, arena, sql, &mut |cursor: &Cursor<'_>| {
                if keep(arena, select.filter, sql, cursor)? {
                    rows.push(sorted(arena, &select, sql, cursor, &keys)?);
                }
                Ok(())
            })?;
        } else {
            for group in self.groups(arena, &select, sql, &sides, &calls)? {
                rows.push(sorted(arena, &select, sql, &group, &keys)?);
            }
        }
        if !keys.is_empty() {
            rows.sort_by(|left, right| order_of(&left.keys, &right.keys, &keys));
        }
        let mut rows: Vec<Vec<Value>> = rows.into_iter().map(|row| row.values).collect();
        if select.distinct == Distinct::Distinct {
            distinct(&mut rows, &collations);
        }
        if whole {
            limit(arena, &select, sql, &mut rows)?;
        }
        Ok((Answer { names, rows }, written))
    }

    /// The tables of a `FROM` clause, and how each attaches to the ones
    /// before it.
    fn sides<'s>(
        &'s self,
        arena: &Arena,
        select: &Select,
        sql: &'s [u8],
    ) -> Result<Vec<Side<'s>>, Error> {
        let mut out: Vec<Side<'s>> = Vec::new();
        for source in arena.sources(select.from) {
            let SourceKind::Table { schema, name, .. } = source.kind else {
                return Err(Error::Unsupported);
            };
            if schema.is_some_and(|span| !is_main(span.text(sql))) {
                // Only the one schema a file holds is readable, and a
                // name in front of it that is not it names no table
                // rather than another database.
                return Err(Error::NoTable);
            }
            let stored = self.find(name.text(sql)).ok_or(Error::NoTable)?;
            readable(stored)?;
            if matches!(source.join.kind, JoinKind::Right | JoinKind::Full) {
                // A join that keeps the rows of the table read last is
                // a later step.
                return Err(Error::Unsupported);
            }
            let written: Vec<Vec<u8>> = arena
                .names(source.using)
                .iter()
                .map(|span| span.text(sql).to_vec())
                .collect();
            let using = if source.join.natural {
                if source.on.is_some() || !written.is_empty() {
                    return Err(Error::Join);
                }
                // A `NATURAL` join matches by every name both sides
                // hold, in the order the table read last holds them.
                stored
                    .table
                    .columns
                    .iter()
                    .filter(|column| holds(&out, &column.name))
                    .map(|column| column.name.clone())
                    .collect()
            } else {
                for name in &written {
                    if !holds(&out, name) || !has(&stored.table, name) {
                        return Err(Error::Join);
                    }
                }
                written
            };
            out.push(Side {
                stored,
                alias: source.alias.map(|span| span.text(sql)),
                kind: source.join.kind,
                on: source.on,
                using,
            });
        }
        Ok(out)
    }

    /// Walks the rows a statement reads, once, and hands each to `each`.
    ///
    /// One table costs O(n); a join of `k` tables is the loops nested,
    /// which is the product of their rows — no index is used yet, so the
    /// rows are right and the plan is not.
    fn scan<'b>(
        &self,
        sides: &'b [Side<'b>],
        arena: &Arena,
        sql: &[u8],
        each: &mut dyn FnMut(&Cursor<'b>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut cursor = Cursor::new(self.collation(), self.encoding);
        if sides.is_empty() {
            // A statement with no `FROM` reads one row of nothing.
            return each(&cursor);
        }
        self.nest(sides, 0, &mut cursor, arena, sql, each)
    }

    /// One level of the nest: every row of `sides[at]` against what the
    /// levels above it hold.
    fn nest<'b>(
        &self,
        sides: &'b [Side<'b>],
        at: usize,
        cursor: &mut Cursor<'b>,
        arena: &Arena,
        sql: &[u8],
        each: &mut dyn FnMut(&Cursor<'b>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let Some(side) = sides.get(at) else {
            return each(cursor);
        };
        let deeper = at.saturating_add(1);
        let mut any = false;
        let mut payload = Vec::new();
        for row in self.image.rows(side.stored.root) {
            let row = row?;
            read_payload(&self.image, &row.payload, &mut payload)?;
            cursor.held.push(Held {
                table: &side.stored.table,
                alias: side.alias,
                using: &side.using,
                rowid: Some(row.rowid),
                values: values_of(&payload, &side.stored.table, row.rowid, self.encoding)?,
            });
            let attached = attached(arena, side, cursor, sql)?;
            if attached {
                any = true;
                self.nest(sides, deeper, cursor, arena, sql, each)?;
            }
            cursor.held.pop();
        }
        if !any && side.kind == JoinKind::Left {
            // A `LEFT JOIN` answers the row on the left once with
            // nothing on the right where nothing on the right matched.
            cursor
                .held
                .push(Held::empty(&side.stored.table, side.alias, &side.using));
            self.nest(sides, deeper, cursor, arena, sql, each)?;
            cursor.held.pop();
        }
        Ok(())
    }

    /// The groups a statement that aggregates answers: one cursor each,
    /// in the order the `GROUP BY` terms collate in, which is the order
    /// the sorter of `src/select.c` puts them in.
    fn groups<'b>(
        &self,
        arena: &Arena,
        select: &Select,
        sql: &[u8],
        sides: &'b [Side<'b>],
        calls: &[Call],
    ) -> Result<Vec<Cursor<'b>>, Error> {
        let terms = grouping(arena, select, sql, sides)?;
        let mut groups: Vec<Group<'b>> = Vec::new();
        self.scan(sides, arena, sql, &mut |cursor: &Cursor<'b>| {
            if !keep(arena, select.filter, sql, cursor)? {
                return Ok(());
            }
            let mut key = Vec::new();
            for term in &terms {
                key.push(evaluate_collated(arena, *term, sql, cursor)?);
            }
            let found = groups.iter().position(|group| alike(&group.key, &key));
            let group = if let Some(at) = found {
                groups.get_mut(at)
            } else {
                groups.push(Group::new(key, calls));
                groups.last_mut()
            };
            // A place read out of the list, or the end of a list just
            // pushed to: neither is ever nothing.
            group.map_or(Ok(()), |group| group.step(arena, sql, cursor, calls))
        })?;
        if terms.is_empty() && groups.is_empty() {
            // A statement that aggregates over no group answers one row
            // whatever the rows were, which is what `SELECT count(*)`
            // over an empty table is.
            groups.push(Group::new(Vec::new(), calls));
        }
        groups.sort_by(|left, right| order_of_keys(&left.key, &right.key));
        let mut out = Vec::new();
        for group in &groups {
            let mut answers = Vec::new();
            for (call, accumulator) in calls.iter().zip(&group.accumulators) {
                answers.push((call.id, accumulator.finish()?));
            }
            // A group that kept no row answers `NULL` for every column
            // of it, which is the accumulator never loaded.
            let held = group.magnet.clone().unwrap_or_else(|| {
                sides
                    .iter()
                    .map(|side| Held::empty(&side.stored.table, side.alias, &side.using))
                    .collect()
            });
            let cursor = Cursor {
                held,
                collation: self.collation(),
                encoding: self.encoding,
                aggregates: answers,
            };
            if keep(arena, select.having, sql, &cursor)? {
                out.push(cursor);
            }
        }
        Ok(out)
    }
}

/// A `VALUES`, which answers its rows and names them by their place.
fn listed(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
) -> Result<(Answer, Vec<Option<Collation>>), Error> {
    let mut rows = Vec::new();
    for row in arena.children(select.values) {
        // The parser builds each row of a `VALUES` as a row node, so
        // what the node names is what the row holds.
        let mut items = Vec::new();
        arena
            .node(*row)
            .into_iter()
            .for_each(|node| arena.under(node, |item| items.push(item)));
        let mut values = Vec::new();
        for item in items {
            values.push(evaluate_row(arena, item, sql, &eval::NoRow)?);
        }
        if rows
            .first()
            .is_some_and(|first: &Vec<Value>| first.len() != values.len())
        {
            return Err(Error::Values);
        }
        rows.push(values);
    }
    let width = rows.first().map_or(0, Vec::len);
    let mut names = Vec::new();
    for at in 1..=width {
        let mut name = b"column".to_vec();
        name.extend_from_slice(&number::integer_text(i64::try_from(at).unwrap_or(0)));
        names.push(name);
    }
    Ok((Answer { names, rows }, alloc::vec![None; width]))
}

/// Whether the row the walk just read attaches to the ones above it:
/// the `ON` condition holds, and every column a `USING` or a `NATURAL`
/// names is equal on both sides.
fn attached(
    arena: &Arena,
    side: &Side<'_>,
    cursor: &Cursor<'_>,
    sql: &[u8],
) -> Result<bool, Error> {
    if let Some(on) = side.on
        && !evaluate_row(arena, on, sql, cursor)?.truth(false)
    {
        return Ok(false);
    }
    for name in &side.using {
        if !equal(cursor, name) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether the column `name` holds the same value on the side the walk
/// just read as on the side before it, which is the `a.x = b.x` a
/// `USING` stands for.
///
/// The side before it is the left of that comparison, so its collation
/// is the one the two are compared under.
fn equal(cursor: &Cursor<'_>, name: &[u8]) -> bool {
    // Both sides hold the column: a `USING` that names one they do not
    // both hold is refused before the walk begins.
    let mine = cursor.held.last().and_then(|held| held.column(name));
    let theirs = cursor
        .held
        .iter()
        .rev()
        .skip(1)
        .find_map(|held| held.column(name));
    mine.zip(theirs).is_some_and(
        |((mut right, right_affinity, _), (mut left, left_affinity, collation))| {
            if left == Value::Null || right == Value::Null {
                // A `NULL` on either side matches nothing, which is
                // what `=` answers and what a join asks of it.
                return false;
            }
            let affinity = compare_affinity(left_affinity, right_affinity);
            apply_comparison(&mut left, &mut right, affinity);
            compare(&left, &right, collation) == core::cmp::Ordering::Equal
        },
    )
}

/// Whether the rows of a table are ones this engine reads.
///
/// This is its own function rather than two tests inside the walk
/// because the walk is written once per shape of statement and this is
/// the same answer for all of them.
fn readable(stored: &Stored) -> Result<(), Error> {
    if stored.table.without_rowid {
        return Err(Error::WithoutRowid);
    }
    if stored
        .table
        .columns
        .iter()
        .any(|column| column.generated == Generated::Virtual)
    {
        // A virtual column is not in the row; computing one is a later
        // step.
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// One aggregate call of a statement.
struct Call {
    /// The node it was written as, which is what answers it: two
    /// `count(*)` in one statement are one column each.
    id: ExprId,
    /// Which aggregate.
    which: Aggregate,
    /// Whether `DISTINCT` precedes its arguments.
    distinct: bool,
    /// Its arguments.
    args: Range,
}

/// One group of rows while it is being accumulated.
struct Group<'a> {
    /// What the `GROUP BY` terms answered for it.
    key: Vec<(Value, Collation)>,
    /// The row its bare columns come from, where it has kept one.
    magnet: Option<Vec<Held<'a>>>,
    /// One accumulator per aggregate call.
    accumulators: Vec<Accumulator>,
}

impl<'a> Group<'a> {
    /// A group that has accumulated nothing.
    fn new(key: Vec<(Value, Collation)>, calls: &[Call]) -> Self {
        Group {
            key,
            magnet: None,
            accumulators: calls
                .iter()
                .map(|call| Accumulator::new(call.which, call.distinct))
                .collect(),
        }
    }

    /// Adds one row to every accumulator, and keeps the row where it is
    /// the one the group's bare columns come from.
    ///
    /// Which row that is: the first of the group, unless a `min` or a
    /// `max` is being accumulated, in which case it is the row the last
    /// of them took, so that `SELECT b, max(a)` answers the `b` beside
    /// the greatest `a`. This is `updateAccumulator` over the register
    /// `sqlite3SkipAccumulatorLoad` sets.
    fn step(
        &mut self,
        arena: &Arena,
        sql: &[u8],
        cursor: &Cursor<'a>,
        calls: &[Call],
    ) -> Result<(), Error> {
        let first = self.magnet.is_none();
        let mut magnet = None;
        for (call, accumulator) in calls.iter().zip(&mut self.accumulators) {
            let mut values = Vec::new();
            let mut collation = cursor.collation;
            for (at, id) in arena.children(call.args).iter().enumerate() {
                // The collation of the first argument is the one the
                // aggregate compares under, which `min`, `max` and a
                // `DISTINCT` are what use.
                if at == 0 {
                    let (value, written) = evaluate_collated(arena, *id, sql, cursor)?;
                    collation = written;
                    values.push(value);
                } else {
                    values.push(evaluate_row(arena, *id, sql, cursor)?);
                }
            }
            accumulator.step(&values, collation);
            if accumulator.magnet() {
                magnet = Some(accumulator.kept());
            }
        }
        if magnet.unwrap_or(first) {
            self.magnet = Some(cursor.held.clone());
        }
        Ok(())
    }
}

/// The expressions a `GROUP BY` groups by.
///
/// A whole number counts the answered columns from one, and a name the
/// table does not hold is looked for among the names the statement
/// answers under. This is `sqlite3ResolveOrderGroupBy` over
/// `resolveAlias`, which is why `GROUP BY 2-1` groups by the number one
/// and `GROUP BY 1` groups by the first column answered.
fn grouping(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
) -> Result<Vec<ExprId>, Error> {
    let results = arena.results(select.columns);
    let mut out = Vec::new();
    for term in arena.children(select.group) {
        if let Some(place) = whole_number(arena, *term, sql) {
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| Error::OrderRange)?;
            match results.get(at) {
                None => return Err(Error::OrderRange),
                // A `*` stands for columns that have no expression to
                // group by; reaching one of them is a later step.
                Some(ResultColumn::Star | ResultColumn::TableStar(_)) => {
                    return Err(Error::Unsupported);
                }
                Some(ResultColumn::Expr { expr, .. }) => out.push(*expr),
            }
            continue;
        }
        // A name is a column of the table where the table has one, and
        // only where it has none is it the name something is answered
        // under.
        let aliased = column_named(arena, *term)
            .filter(|name| !holds(sides, name.text(sql)))
            .and_then(|name| aliased(results, sql, name.text(sql)));
        out.push(aliased.unwrap_or(*term));
    }
    Ok(out)
}

/// Whether any of the tables has a column of this name.
fn holds(sides: &[Side<'_>], name: &[u8]) -> bool {
    sides.iter().any(|side| has(&side.stored.table, name))
}

/// Whether the table has a column of this name.
fn has(table: &Table, name: &[u8]) -> bool {
    table
        .columns
        .iter()
        .any(|column| column.name.eq_ignore_ascii_case(name))
}

/// The expression a statement answers under `name`, where it answers
/// one under that name.
fn aliased(results: &[ResultColumn], sql: &[u8], name: &[u8]) -> Option<ExprId> {
    results.iter().find_map(|column| match *column {
        ResultColumn::Expr {
            expr,
            alias: Some(alias),
            ..
        } if alias.text(sql).eq_ignore_ascii_case(name) => Some(expr),
        _ => None,
    })
}

/// Every aggregate call a statement answers with, and a refusal where
/// one stands somewhere no group has been made yet.
fn aggregates(arena: &Arena, select: &Select, sql: &[u8]) -> Result<Vec<Call>, Error> {
    let mut calls = Vec::new();
    for column in arena.results(select.columns) {
        if let ResultColumn::Expr { expr, .. } = *column {
            gather(arena, expr, sql, &mut calls, false)?;
        }
    }
    // What makes a statement an aggregate one is an aggregate among the
    // columns it answers, or a `GROUP BY`. Nothing else does, which is
    // why a `HAVING` without either is a refusal and not a filter over
    // one group.
    let grouped = !calls.is_empty() || !select.group.is_empty();
    if select.having.is_some() && !grouped {
        return Err(Error::Having);
    }
    if let Some(having) = select.having {
        gather(arena, having, sql, &mut calls, false)?;
    }
    for term in arena.orders(select.order) {
        gather(arena, term.expr, sql, &mut calls, false)?;
    }
    let mut misused = Vec::new();
    for id in select.filter.iter().chain(arena.children(select.group)) {
        gather(arena, *id, sql, &mut misused, false)?;
    }
    if !misused.is_empty() || (!grouped && !calls.is_empty()) {
        return Err(Error::Aggregate);
    }
    Ok(calls)
}

/// Collects the aggregate calls of one expression.
///
/// The walk is as deep as the tree is tall, which the parser has already
/// bounded, so nothing here counts the steps. `inside` is set under an
/// aggregate, where another one is a misuse rather than a call.
fn gather(
    arena: &Arena,
    id: ExprId,
    sql: &[u8],
    out: &mut Vec<Call>,
    inside: bool,
) -> Result<(), Error> {
    arena
        .node(id)
        .map_or(Ok(()), |node| collect(arena, id, node, sql, out, inside))
}

/// The same for one node the arena holds.
fn collect(
    arena: &Arena,
    id: ExprId,
    node: Node,
    sql: &[u8],
    out: &mut Vec<Call>,
    inside: bool,
) -> Result<(), Error> {
    let mut under = inside;
    if let Node::Call {
        name,
        args,
        distinct,
        star,
    } = node
    {
        let count = if star { 0 } else { arena.children(args).len() };
        if let Some(which) = agg::lookup(name.text(sql), count) {
            // `DISTINCT` puts the rows through one column, so there has
            // to be exactly one for them to go through.
            if inside || (distinct && count != 1) {
                return Err(Error::Aggregate);
            }
            out.push(Call {
                id,
                which,
                distinct,
                args,
            });
            under = true;
        }
    }
    let mut deeper = Ok(());
    arena.under(node, |child| {
        if deeper.is_ok() {
            deeper = gather(arena, child, sql, out, under);
        }
    });
    deeper
}

/// The name SQLite gives each answered column.
fn names(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
) -> Result<Vec<Vec<u8>>, Error> {
    let mut names = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            ResultColumn::Star => {
                if sides.is_empty() {
                    return Err(Error::NoTable);
                }
                for (at, side) in sides.iter().enumerate() {
                    // A `*` stands for every column named by its table,
                    // so two tables of one name make every column of
                    // them a name two tables answer.
                    if sides
                        .iter()
                        .take(at)
                        .any(|before| before.named(side.calls()))
                    {
                        return Err(Error::Ambiguous);
                    }
                    // The columns a `USING` or a `NATURAL` matched are
                    // answered once and not twice, which is the side
                    // that hides them leaving them out.
                    for column in &side.stored.table.columns {
                        if !side.hides(&column.name) {
                            names.push(column.name.clone());
                        }
                    }
                }
            }
            ResultColumn::TableStar(span) => {
                let called = span.text(sql);
                let side = sides
                    .iter()
                    .find(|side| side.named(called))
                    .ok_or(Error::NoTable)?;
                for column in &side.stored.table.columns {
                    names.push(column.name.clone());
                }
            }
            ResultColumn::Expr { expr, alias, text } => names.push(match alias {
                Some(span) => span.text(sql).to_vec(),
                // With no name written, a column answers under its
                // own name and everything else under the text it was
                // written as.
                None => match column_named(arena, expr) {
                    Some(name) => answered_name(name.text(sql), sides),
                    None => text.text(sql).to_vec(),
                },
            }),
        }
    }
    Ok(names)
}

/// The values one row answers.
fn project(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<Vec<Value>, Error> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            ResultColumn::Star => {
                for held in &cursor.held {
                    held.each(|name, value| {
                        if !held.hides(name) {
                            out.push(value.clone());
                        }
                    });
                }
            }
            ResultColumn::TableStar(span) => {
                let named = span.text(sql);
                for held in cursor.held.iter().filter(|held| held.named(named)) {
                    held.each(|_, value| out.push(value.clone()));
                }
            }
            ResultColumn::Expr { expr, .. } => {
                out.push(evaluate_row(arena, expr, sql, cursor)?);
            }
        }
    }
    Ok(out)
}

/// One row: what it answers, and what it sorts by.
fn sorted(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    cursor: &Cursor<'_>,
    keys: &[Key],
) -> Result<Sorted, Error> {
    let values = project(arena, select, sql, cursor)?;
    let mut sort = Vec::new();
    for key in keys {
        sort.push(match key {
            Key::Place(at, _) => (
                values.get(*at).cloned().unwrap_or(Value::Null),
                cursor.collation,
            ),
            Key::Expr(expr, _) => evaluate_collated(arena, *expr, sql, cursor)?,
        });
    }
    Ok(Sorted { values, keys: sort })
}

/// What each `ORDER BY` term sorts by: a place in the answer, or an
/// expression over the row.
fn keys(arena: &Arena, select: &Select, sql: &[u8], names: &[Vec<u8>]) -> Result<Vec<Key>, Error> {
    let mut keys = Vec::new();
    for term in arena.orders(select.order) {
        let descending = term.order == Order::Descending;
        // A whole number counts the answered columns from one; a
        // name that is one of them names it; anything else is read
        // against the row.
        if let Some(place) = whole_number(arena, term.expr, sql) {
            let at = usize::try_from(place.saturating_sub(1)).map_err(|_| Error::OrderRange)?;
            if at >= names.len() {
                return Err(Error::OrderRange);
            }
            keys.push(Key::Place(at, descending));
            continue;
        }
        if let Some(name) = column_named(arena, term.expr) {
            let name = name.text(sql);
            if let Some(at) = names
                .iter()
                .position(|answered| answered.eq_ignore_ascii_case(name))
            {
                keys.push(Key::Place(at, descending));
                continue;
            }
        }
        keys.push(Key::Expr(term.expr, descending));
    }
    Ok(keys)
}

/// Drops the rows a `LIMIT` leaves out.
fn limit(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    rows: &mut Vec<Vec<Value>>,
) -> Result<(), Error> {
    let Some(limit) = select.limit else {
        return Ok(());
    };
    let count = evaluate_row(arena, limit.count, sql, &eval::NoRow)?.to_integer();
    let skip = match limit.offset {
        None => 0,
        Some(offset) => evaluate_row(arena, offset, sql, &eval::NoRow)?.to_integer(),
    };
    let skip = usize::try_from(skip).unwrap_or(0);
    rows.drain(..skip.min(rows.len()));
    if count < 0 {
        return Ok(());
    }
    rows.truncate(usize::try_from(count).unwrap_or(0));
    Ok(())
}

/// Reads a payload into `into`, refusing one longer than the whole file
/// before any room is made for it.
///
/// A payload's length is a number the file gives, and a file may give
/// one no machine holds.
fn read_payload(
    image: &Image<'_>,
    payload: &crate::page::Payload<'_>,
    into: &mut Vec<u8>,
) -> Result<(), Error> {
    if payload.total > image.size() {
        return Err(Error::Image(error::Error::Overrun));
    }
    into.clear();
    into.resize(payload.total, 0);
    image.read_payload(payload, into)?;
    Ok(())
}

/// Whether two rows hold the same values, which is what `DISTINCT` and
/// the set operators ask.
fn same(left: &[Value], right: &[Value], collations: &[Collation]) -> bool {
    // Two rows of one answer always hold the same number of values, and
    // there is one collation per column.
    left.iter()
        .zip(right)
        .zip(collations)
        .all(|((first, second), collation)| {
            compare(first, second, *collation) == core::cmp::Ordering::Equal
        })
}

/// Drops the rows another row already holds.
fn distinct(rows: &mut Vec<Vec<Value>>, collations: &[Collation]) {
    let mut seen: Vec<Vec<Value>> = Vec::new();
    rows.retain(|row| {
        let fresh = !seen.iter().any(|kept| same(kept, row, collations));
        if fresh {
            seen.push(row.clone());
        }
        fresh
    });
}

/// Puts two answers together.
///
/// `UNION ALL` is the rows of one after the rows of the other, in the
/// order they were answered. The other three go through the merge
/// `multiSelectOrderBy` compiles: each side sorted by every column and
/// holding each row once, then walked together. Which of two rows that
/// compare equal but are not the same bytes comes out is decided there
/// and not by chance — the first of them inside one side, and the right
/// side's where a `UNION` finds one on both.
fn combine(operator: Compound, left: &mut Answer, right: Answer, collations: &[Collation]) {
    if operator == Compound::UnionAll {
        left.rows.extend(right.rows);
        return;
    }
    ordered(&mut left.rows, collations);
    let mut theirs = right.rows;
    ordered(&mut theirs, collations);
    let wanted = matches!(operator, Compound::Intersect);
    left.rows.retain(|row| {
        let shared = theirs.iter().any(|other| same(other, row, collations));
        shared == wanted
    });
    if operator == Compound::Union {
        left.rows.extend(theirs);
        arrange(&mut left.rows, collations);
    }
}

/// Sorts rows by every column and drops the ones another row already
/// holds, which is what each side of a set operator goes through.
fn ordered(rows: &mut Vec<Vec<Value>>, collations: &[Collation]) {
    arrange(rows, collations);
    rows.dedup_by(|left, right| same(left, right, collations));
}

/// Sorts rows by every column, which is the order a set operator
/// answers in.
fn arrange(rows: &mut [Vec<Value>], collations: &[Collation]) {
    let every: Vec<(usize, bool)> = (0..collations.len()).map(|at| (at, false)).collect();
    sort_by_keys(rows, &every, collations);
}

/// Sorts the rows of an answer under terms that each count to a column
/// of it, backwards where the flag says so.
fn sort_by_keys(rows: &mut [Vec<Value>], terms: &[(usize, bool)], collations: &[Collation]) {
    rows.sort_by(|left, right| {
        for (at, descending) in terms.iter().copied() {
            let collation = collations.get(at).copied().unwrap_or(Collation::Binary);
            let first = left.get(at).unwrap_or(&Value::Null);
            let second = right.get(at).unwrap_or(&Value::Null);
            let order = compare(first, second, collation);
            if order != core::cmp::Ordering::Equal {
                return if descending { order.reverse() } else { order };
            }
        }
        core::cmp::Ordering::Equal
    });
}

/// The `ORDER BY` of a compound, whose every term has to count or name
/// a column of the answer: there is no row left to read an expression
/// against.
fn matched(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    names: &[Vec<u8>],
) -> Result<Vec<(usize, bool)>, Error> {
    let mut out = Vec::new();
    for key in keys(arena, select, sql, names)? {
        match key {
            Key::Place(at, descending) => out.push((at, descending)),
            Key::Expr(..) => return Err(Error::OrderMatch),
        }
    }
    Ok(out)
}

/// Whether a schema name is the one a file holds.
const fn is_main(name: &[u8]) -> bool {
    name.eq_ignore_ascii_case(b"main")
}

/// The collation each answered column compares under, which is what a
/// `DISTINCT` and the set operators use.
fn collations(
    arena: &Arena,
    select: &Select,
    sql: &[u8],
    sides: &[Side<'_>],
) -> Vec<Option<Collation>> {
    let mut out = Vec::new();
    for column in arena.results(select.columns) {
        match *column {
            // A `*` this engine refuses has already been refused by the
            // names, which are read first, so what is left here is the
            // columns the names counted.
            ResultColumn::Star => {
                for side in sides {
                    for column in &side.stored.table.columns {
                        if !side.hides(&column.name) {
                            out.push(Some(column.collation));
                        }
                    }
                }
            }
            ResultColumn::TableStar(span) => {
                let named = span.text(sql);
                for side in sides.iter().filter(|side| side.named(named)) {
                    for column in &side.stored.table.columns {
                        out.push(Some(column.collation));
                    }
                }
            }
            ResultColumn::Expr { expr, .. } => out.push(written(arena, expr, sql, sides)),
        }
    }
    out
}

/// The collation written on an expression, where one is, which is
/// `sqlite3ExprCollSeq` over the two nodes that carry one.
fn written(arena: &Arena, id: ExprId, sql: &[u8], sides: &[Side<'_>]) -> Option<Collation> {
    match arena.node(id)? {
        Node::Collate { name, .. } => Collation::of_name(name.text(sql)),
        Node::Column { column, .. } => sides
            .iter()
            .flat_map(|side| &side.stored.table.columns)
            .find(|held| held.name.eq_ignore_ascii_case(column.text(sql)))
            .map(|held| held.collation),
        _ => None,
    }
}

/// Whether a `WHERE` or a `HAVING` keeps this row.
fn keep(
    arena: &Arena,
    clause: Option<ExprId>,
    sql: &[u8],
    cursor: &Cursor<'_>,
) -> Result<bool, Error> {
    let Some(clause) = clause else {
        return Ok(true);
    };
    Ok(evaluate_row(arena, clause, sql, cursor)?.truth(false))
}

/// The name a column reference is answered under: the column's own
/// name where the table has one, and `rowid` for the three names the
/// key answers to.
fn answered_name(name: &[u8], sides: &[Side<'_>]) -> Vec<u8> {
    if sides.is_empty() {
        return name.to_vec();
    }
    let held = sides
        .iter()
        .flat_map(|side| &side.stored.table.columns)
        .find(|column| column.name.eq_ignore_ascii_case(name));
    if let Some(column) = held {
        return column.name.clone();
    }
    // A key that is the rowid answers under its own name, whichever of
    // the rowid's three names was written.
    sides
        .iter()
        .find_map(|side| {
            side.stored
                .table
                .rowid_alias
                .and_then(|at| side.stored.table.columns.get(at))
        })
        .map_or_else(|| b"rowid".to_vec(), |column| column.name.clone())
}

/// The whole number an expression is, where it is one.
///
/// This is `sqlite3ExprIsInteger`, which reads the sign as part of the
/// number so that `ORDER BY -1` counts to the column before the first.
fn whole_number(arena: &Arena, id: ExprId, sql: &[u8]) -> Option<i64> {
    match arena.node(id)? {
        Node::Literal(Literal::Integer(span)) => Some(number::integer(span.text(sql)).value),
        Node::Unary {
            op: UnaryOp::Negate,
            operand,
        } => whole_number(arena, operand, sql).map(i64::wrapping_neg),
        _ => None,
    }
}

/// The column an expression names, where it is nothing else.
///
/// A collation written around it counts as something else, which is why
/// `SELECT a COLLATE NOCASE` is answered under that whole text and not
/// under `a`.
fn column_named(arena: &Arena, id: ExprId) -> Option<Span> {
    match arena.node(id)? {
        Node::Column { column, .. } => Some(column),
        _ => None,
    }
}

/// One row with the values it sorts by beside it.
struct Sorted {
    /// What the statement answers for the row.
    values: Vec<Value>,
    /// What it sorts by, one per term.
    keys: Vec<(Value, Collation)>,
}

/// What an `ORDER BY` term sorts by.
enum Key {
    /// The column of the answer at this place, backwards where the flag
    /// says so.
    Place(usize, bool),
    /// An expression over the row.
    Expr(ExprId, bool),
}

/// Where two rows stand against each other under the terms.
fn order_of(
    left: &[(Value, Collation)],
    right: &[(Value, Collation)],
    keys: &[Key],
) -> core::cmp::Ordering {
    for ((first, second), key) in left.iter().zip(right).zip(keys) {
        let descending = match key {
            Key::Place(_, descending) | Key::Expr(_, descending) => *descending,
        };
        let order = compare(&first.0, &second.0, first.1);
        if order != core::cmp::Ordering::Equal {
            return if descending { order.reverse() } else { order };
        }
    }
    core::cmp::Ordering::Equal
}

/// Where two groups stand against each other under their keys, which
/// is the order a `GROUP BY` answers them in.
fn order_of_keys(left: &[(Value, Collation)], right: &[(Value, Collation)]) -> core::cmp::Ordering {
    for (first, second) in left.iter().zip(right) {
        let order = compare(&first.0, &second.0, first.1);
        if order != core::cmp::Ordering::Equal {
            return order;
        }
    }
    core::cmp::Ordering::Equal
}

/// Whether two rows belong to the same group.
fn alike(left: &[(Value, Collation)], right: &[(Value, Collation)]) -> bool {
    order_of_keys(left, right) == core::cmp::Ordering::Equal
}

/// The values of one row, with the rowid put where its alias stands.
fn values_of(
    payload: &[u8],
    table: &Table,
    rowid: i64,
    encoding: Encoding,
) -> Result<Vec<Value>, Error> {
    let record = record::Record::parse(payload)?;
    let mut out = Vec::new();
    for (at, column) in table.columns.iter().enumerate() {
        let mut value = match record.value(at)? {
            None | Some(record::Value::Null) => Value::Null,
            Some(record::Value::Int(number)) => Value::Int(number),
            Some(record::Value::Real(number)) => Value::Real(number),
            Some(record::Value::Text(bytes)) => Value::Text(decode(bytes, encoding)),
            Some(record::Value::Blob(bytes)) => Value::Blob(bytes.to_vec()),
        };
        // A real that is a whole number is stored as an integer, and
        // `OP_RealAffinity` is what turns it back on the way out.
        if column.affinity == Affinity::Real
            && let Value::Int(number) = value
        {
            value = Value::Real(crate::value::integer_as_real(number));
        }
        // The column the rowid is another name for holds nothing of its
        // own: the key is what it answers.
        if table.rowid_alias == Some(at) {
            value = Value::Int(rowid);
        }
        out.push(value);
    }
    Ok(out)
}

/// What `BINARY` is over text the file keeps in this encoding.
const fn binary_of(encoding: Encoding) -> Collation {
    match encoding {
        Encoding::Utf8 => Collation::Binary,
        Encoding::Utf16Le => Collation::Binary16Le,
        Encoding::Utf16Be => Collation::Binary16Be,
    }
}

/// Text as the engine holds it, which is UTF-8 whatever the file keeps.
fn decode(bytes: &[u8], encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => bytes.to_vec(),
        Encoding::Utf16Le => crate::utf8::from_utf16(bytes, false),
        Encoding::Utf16Be => crate::utf8::from_utf16(bytes, true),
    }
}

/// One table of the `FROM`, as one row of the walk holds it.
#[derive(Clone)]
struct Held<'a> {
    /// The table.
    table: &'a Table,
    /// What the statement calls it, where it calls it something.
    alias: Option<&'a [u8]>,
    /// The columns it does not answer a bare name with, which are the
    /// ones a `USING` or a `NATURAL` matched it by.
    using: &'a [Vec<u8>],
    /// The rowid of the row, where it stands for one.
    rowid: Option<i64>,
    /// Its values, one per column.
    values: Vec<Value>,
}

impl<'a> Held<'a> {
    /// The table with no row of it, which is what a `LEFT JOIN` holds
    /// where nothing matched.
    fn empty(table: &'a Table, alias: Option<&'a [u8]>, using: &'a [Vec<u8>]) -> Self {
        Held {
            table,
            alias,
            using,
            rowid: None,
            values: alloc::vec![Value::Null; table.columns.len()],
        }
    }

    /// Whether `named` is what the statement calls this table.
    fn named(&self, named: &[u8]) -> bool {
        self.alias.map_or_else(
            || self.table.name.eq_ignore_ascii_case(named),
            |alias| alias.eq_ignore_ascii_case(named),
        )
    }

    /// Whether a bare name reaches this table's column of that name,
    /// which the side a `USING` matched does not answer.
    fn hides(&self, column: &[u8]) -> bool {
        self.using
            .iter()
            .any(|name| name.eq_ignore_ascii_case(column))
    }

    /// Calls `each` with the name and the value of every column.
    fn each(&self, mut each: impl FnMut(&[u8], &Value)) {
        for (column, value) in self.table.columns.iter().zip(&self.values) {
            each(&column.name, value);
        }
    }

    /// What this table answers for `column`, or nothing where it has no
    /// such column.
    fn column(&self, column: &[u8]) -> Option<(Value, Affinity, Collation)> {
        let at = self
            .table
            .columns
            .iter()
            .position(|held| held.name.eq_ignore_ascii_case(column));
        let Some(at) = at else {
            // The three names the rowid answers to. A table with no
            // rowid is refused before a row of it is read, so there is
            // nothing to rule out here.
            let rowid = column.eq_ignore_ascii_case(b"rowid")
                || column.eq_ignore_ascii_case(b"oid")
                || column.eq_ignore_ascii_case(b"_rowid_");
            if rowid {
                let value = self.rowid.map_or(Value::Null, Value::Int);
                return Some((value, Affinity::Integer, Collation::Binary));
            }
            return None;
        };
        let held = self.table.columns.get(at)?;
        let value = self.values.get(at).cloned().unwrap_or(Value::Null);
        Some((value, held.affinity, held.collation))
    }
}

/// Where a walk stands: one row of every table of the `FROM`.
struct Cursor<'a> {
    /// One per table, in the order they were written.
    held: Vec<Held<'a>>,
    /// What a comparison uses where nothing writes a collation.
    collation: Collation,
    /// What encoding the file keeps its text in.
    encoding: Encoding,
    /// What each aggregate call of the statement answered, where the
    /// row stands for a group.
    aggregates: Vec<(ExprId, Value)>,
}

impl Cursor<'_> {
    /// A cursor that holds no table, which is a statement with no
    /// `FROM` before it reads its one row of nothing.
    const fn new(collation: Collation, encoding: Encoding) -> Self {
        Cursor {
            held: Vec::new(),
            collation,
            encoding,
            aggregates: Vec::new(),
        }
    }
}

impl eval::Row for Cursor<'_> {
    fn collation(&self) -> Collation {
        self.collation
    }

    fn encoding(&self) -> Encoding {
        self.encoding
    }

    fn aggregate(&self, id: ExprId) -> Option<Value> {
        self.aggregates
            .iter()
            .find(|(call, _)| *call == id)
            .map(|(_, value)| value.clone())
    }

    fn column(
        &self,
        schema: Option<&[u8]>,
        table: Option<&[u8]>,
        column: &[u8],
    ) -> Option<(Value, Affinity, Collation)> {
        if schema.is_some_and(|named| !is_main(named)) {
            return None;
        }
        let mut found = None;
        for held in &self.held {
            match table {
                Some(named) if !held.named(named) => continue,
                // A bare name does not reach the side a `USING` or a
                // `NATURAL` matched; the side before it answers.
                None if held.hides(column) => continue,
                _ => {}
            }
            let Some(answer) = held.column(column) else {
                continue;
            };
            if found.is_some() {
                // A name two tables answer is ambiguous, which is a
                // refusal and not a choice.
                return None;
            }
            found = Some(answer);
        }
        found
    }
}
