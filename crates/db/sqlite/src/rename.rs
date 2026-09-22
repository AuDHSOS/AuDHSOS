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

use crate::ast::{Arena, Definition, Node, Span, TriggerStep};

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
        Definition::Table(made) => {
            if same(made.name.text(sql), table) {
                out.push(made.name);
            }
            parents(&arena, &made, sql, table, &mut out);
        }
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
    // `renameTableExprCb` of `research/sqlite/src/alter.c` marks the
    // table a column reference names, so `t.a` and `main.t.a` are
    // written again under the new name. A name a source carries as an
    // alias stands for that source and not for the table.
    let aliased = |name: Span| {
        arena.all_sources().any(|source| {
            source
                .alias
                .is_some_and(|alias| same(alias.text(sql), &crate::schema::dequote(name.text(sql))))
        })
    };
    for id in arena.column_places() {
        let Some(crate::ast::Node::Column {
            table: Some(named), ..
        }) = arena.node(id)
        else {
            continue;
        };
        if same(named.text(sql), table) && !shadowed(named) && !aliased(named) {
            out.push(named);
        }
    }
    out.sort_unstable_by_key(|span| span.start);
    out.dedup_by_key(|span| span.start);
    out
}

/// Where a `CREATE TABLE` names `table` as the parent of a foreign key,
/// which is `renameParentFunc` of `research/sqlite/src/alter.c:848`
/// marking the `REFERENCES` clauses of one statement.
///
/// A column carries one such clause and a table constraint carries one
/// each, so reading them costs O(n) in the columns and the constraints.
fn parents(
    arena: &Arena,
    made: &crate::ast::CreateTable,
    sql: &[u8],
    table: &[u8],
    out: &mut Vec<Span>,
) {
    let crate::ast::TableBody::Columns {
        columns,
        constraints,
    } = made.body
    else {
        return;
    };
    let mut mark = |foreign: crate::ast::Foreign| {
        if same(foreign.table.text(sql), table) {
            out.push(foreign.table);
        }
    };
    for column in arena.columns(columns) {
        for constraint in arena.column_constraints(column.constraints) {
            if let crate::ast::ColumnConstraint::References(foreign) = constraint {
                mark(*foreign);
            }
        }
    }
    for constraint in arena.table_constraints(constraints) {
        if let crate::ast::TableConstraint::ForeignKey { foreign, .. } = constraint {
            mark(*foreign);
        }
    }
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
/// with [`places`], and with quotes around it.
///
/// Writing `n` bytes costs O(n).
#[must_use]
pub fn written(sql: &[u8], places: &[Span], name: &[u8]) -> Vec<u8> {
    written_as(sql, places, &quoted(name))
}

/// `sql` with `written` written at each of `places`, which the caller
/// read with [`places`] or [`column_places`].
///
/// `renameEditSql` of `research/sqlite/src/alter.c:1222` writes the new
/// name as the statement wrote it only where the name the statement
/// wrote carries no quotes and the name it writes over carries none
/// either; every other place carries the name in quotes, because a
/// place that was quoted stays quoted.
///
/// Writing `n` bytes costs O(n).
#[must_use]
pub fn written_as(sql: &[u8], places: &[Span], written: &[u8]) -> Vec<u8> {
    let bare = written.first().is_some_and(|byte| is_name(*byte));
    let held = quoted(&crate::schema::dequote(written));
    let mut out = Vec::new();
    let mut at = 0_usize;
    for place in places {
        out.extend_from_slice(sql.get(at..place.start).unwrap_or_default());
        let over = sql.get(place.start).copied().unwrap_or(0);
        if bare && is_name(over) {
            out.extend_from_slice(written);
        } else {
            out.extend_from_slice(&held);
        }
        at = place.start.saturating_add(place.len);
    }
    out.extend_from_slice(sql.get(at..).unwrap_or_default());
    out
}

/// Whether a byte begins a name that carries no quotes, which is
/// `sqlite3IsIdChar` over the bytes a place may begin with: a letter, a
/// digit, an underscore, or a byte of a character past ASCII.
const fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte >= 0x80
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

/// Where a statement of the schema names the column `column` of the
/// table `table`, earliest first.
///
/// A statement the parser refuses names nothing, because a place can
/// only be marked from the tree.
///
/// Reading the tree costs O(n) in its nodes.
#[must_use]
pub fn column_places(sql: &[u8], table: &[u8], column: &[u8]) -> Vec<Span> {
    let Ok((arena, definition)) = crate::parse::definition(sql) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut found = Finding {
        sql,
        table,
        column,
        arena: &arena,
        out: &mut out,
    };
    match definition {
        Definition::Table(made) => found.table_places(&made),
        Definition::Index(made) => found.index_places(&made),
        Definition::Trigger(made) => found.trigger_places(&made),
        Definition::View(_) => {
            let reads = found.reads_table();
            found.select_places(Stands::bare(reads));
        }
        _ => {}
    }
    out.sort_unstable_by_key(|span| span.start);
    out.dedup_by_key(|span| span.start);
    out
}

/// One walk for the places a column is named at: what is being renamed,
/// the tree that names it, and what the walk has found.
struct Finding<'a> {
    /// The statement the tree points into.
    sql: &'a [u8],
    /// The table whose column is renamed.
    table: &'a [u8],
    /// The column as it stands.
    column: &'a [u8],
    /// The tree of the statement.
    arena: &'a Arena,
    /// The places found so far.
    out: &'a mut Vec<Span>,
}

impl Finding<'_> {
    /// Marks `span` where it names the column.
    fn mark(&mut self, span: Span) {
        if same(span.text(self.sql), self.column) {
            self.out.push(span);
        }
    }

    /// Marks every name of a run that names the column.
    fn mark_names(&mut self, names: crate::ast::Range) {
        for span in self.arena.names(names) {
            self.mark(*span);
        }
    }

    /// Marks every term of a sort list that names the column.
    fn mark_orders(&mut self, terms: crate::ast::Range, stands: Stands) {
        for term in self.arena.orders(terms) {
            self.mark_expr(term.expr, stands);
        }
    }

    /// Marks every column reference of a tree that names the column of
    /// the table, which is `renameColumnExprCb` over the tree a
    /// statement of the schema was parsed into.
    ///
    /// Reading the tree costs O(n) in its nodes.
    fn mark_expr(&mut self, id: crate::ast::ExprId, stands: Stands) {
        // A place the tree does not hold names nothing, which a literal
        // that carries no name stands for.
        let node = self
            .arena
            .node(id)
            .unwrap_or(Node::Literal(crate::ast::Literal::Null));
        if let Node::Column {
            table: held,
            column,
            ..
        } = node
        {
            // A name written under a table names that table's column;
            // `new` and `old` name the row the trigger stands on, and a
            // name written under nothing names the column of the table
            // the statement reads.
            let mine = match held {
                None => stands.bare,
                Some(held) => {
                    let held = crate::schema::dequote(held.text(self.sql));
                    if held.eq_ignore_ascii_case(b"new") || held.eq_ignore_ascii_case(b"old") {
                        stands.row
                    } else {
                        same(self.table, &held)
                    }
                }
            };
            if mine {
                self.mark(column);
            }
        }
        let mut under = Vec::new();
        self.arena.under(node, |id| under.push(id));
        for id in under {
            self.mark_expr(id, stands);
        }
    }

    /// Whether any `FROM` of the statement names the table, which says
    /// what a name written under nothing reads.
    ///
    /// Reading the sources costs O(n) in them.
    fn reads_table(&self) -> bool {
        self.arena.all_sources().any(|source| {
            let crate::ast::SourceKind::Table { name, .. } = source.kind else {
                return false;
            };
            same(name.text(self.sql), self.table)
        })
    }

    /// Marks the places a `CREATE TABLE` names the column at: the
    /// column's own name, the keys the table carries, every `CHECK`,
    /// every `DEFAULT`, every computed column, and the columns of a
    /// `REFERENCES` that points at the table.
    fn table_places(&mut self, made: &crate::ast::CreateTable) {
        let over = same(made.name.text(self.sql), self.table);
        let crate::ast::TableBody::Columns {
            columns,
            constraints,
        } = made.body
        else {
            return;
        };
        for column in self.arena.columns(columns) {
            if over {
                self.mark(column.name);
            }
            for constraint in self.arena.column_constraints(column.constraints) {
                match constraint {
                    crate::ast::ColumnConstraint::Check { value, .. }
                    | crate::ast::ColumnConstraint::Default { value, .. }
                    | crate::ast::ColumnConstraint::Generated { value, .. } => {
                        self.mark_expr(*value, Stands::bare(over));
                    }
                    crate::ast::ColumnConstraint::References(foreign) => {
                        self.foreign_places(foreign);
                    }
                    _ => {}
                }
            }
        }
        for constraint in self.arena.table_constraints(constraints) {
            match constraint {
                crate::ast::TableConstraint::PrimaryKey { columns, .. }
                | crate::ast::TableConstraint::Unique { columns, .. } => {
                    self.mark_orders(*columns, Stands::bare(over));
                }
                crate::ast::TableConstraint::Check { value, .. } => {
                    self.mark_expr(*value, Stands::bare(over));
                }
                crate::ast::TableConstraint::ForeignKey { columns, foreign } => {
                    if over {
                        self.mark_names(*columns);
                    }
                    self.foreign_places(foreign);
                }
                crate::ast::TableConstraint::Named(_) => {}
            }
        }
    }

    /// Marks the columns of a `REFERENCES` that points at the table.
    fn foreign_places(&mut self, foreign: &crate::ast::Foreign) {
        if same(foreign.table.text(self.sql), self.table) {
            self.mark_names(foreign.columns);
        }
    }

    /// Marks the places a `CREATE INDEX` over the table names the
    /// column at: its terms and its `WHERE`.
    fn index_places(&mut self, made: &crate::ast::CreateIndex) {
        let stands = Stands::bare(same(made.table.text(self.sql), self.table));
        self.mark_orders(made.columns, stands);
        if let Some(filter) = made.filter {
            self.mark_expr(filter, stands);
        }
    }

    /// Marks the places an `ON CONFLICT` of an `INSERT` over the table
    /// names the column at: the terms it names the index by, the `WHERE`
    /// of a partial index, what it writes, and the `WHERE` the write is
    /// held to.
    ///
    /// `sqlite3UpsertAnalyzeTarget` reads the terms against the table
    /// the `INSERT` writes, so a statement over another table names none
    /// of this one's columns.
    fn upsert_places(&mut self, upserts: crate::ast::Range, writes: bool, row: bool) {
        let stands = Stands { bare: writes, row };
        let held: Vec<crate::ast::Upsert> = self.arena.upserts(upserts).to_vec();
        for upsert in held {
            self.mark_orders(upsert.targets, stands);
            if let Some(over) = upsert.over {
                self.mark_expr(over, stands);
            }
            let sets: Vec<crate::ast::Set> = self.arena.sets(upsert.sets).to_vec();
            for set in sets {
                if writes {
                    self.mark(set.column);
                }
                self.mark_expr(set.value, stands);
            }
            if let Some(filter) = upsert.filter {
                self.mark_expr(filter, stands);
            }
        }
    }

    /// Marks the places a `CREATE TRIGGER` names the column at: the
    /// columns of an `UPDATE OF` where the trigger is on the table, the
    /// `WHEN`, and every statement of the body.
    fn trigger_places(&mut self, made: &crate::ast::CreateTrigger) {
        let on = same(made.table.text(self.sql), self.table);
        if on {
            self.mark_names(made.columns);
        }
        // The `WHEN` reads the row the trigger stands on and nothing
        // else, so a name written under nothing names that table.
        let when = Stands { bare: on, row: on };
        if let Some(condition) = made.condition {
            self.mark_expr(condition, when);
        }
        let reads = self.reads_table();
        let steps: Vec<TriggerStep> = self.arena.steps(made.body).to_vec();
        for step in steps {
            match step {
                TriggerStep::Insert(into) => {
                    let writes = same(into.name.text(self.sql), self.table);
                    if writes {
                        self.mark_names(into.columns);
                    }
                    self.select_places(Stands {
                        bare: reads,
                        row: on,
                    });
                    self.upsert_places(into.upserts, writes, on);
                }
                TriggerStep::Update(update) => {
                    // A name written under nothing reads the table the
                    // statement writes, so a statement over another
                    // table names none of this one's columns.
                    let writes = same(update.name.text(self.sql), self.table);
                    let stands = Stands {
                        bare: writes,
                        row: on,
                    };
                    let sets: Vec<crate::ast::Set> = self.arena.sets(update.sets).to_vec();
                    for set in sets {
                        if writes {
                            self.mark(set.column);
                        }
                        self.mark_expr(set.value, stands);
                    }
                    if let Some(filter) = update.filter {
                        self.mark_expr(filter, stands);
                    }
                }
                TriggerStep::Delete(statement) => {
                    let stands = Stands {
                        bare: same(statement.name.text(self.sql), self.table),
                        row: on,
                    };
                    if let Some(filter) = statement.filter {
                        self.mark_expr(filter, stands);
                    }
                }
                TriggerStep::Select(_) => {
                    self.select_places(Stands {
                        bare: reads,
                        row: on,
                    });
                }
            }
        }
    }

    /// Marks the places a statement names the column at, which is every
    /// expression the tree holds.
    fn select_places(&mut self, stands: Stands) {
        let ids: Vec<crate::ast::ExprId> = self.arena.all().map(|(id, _)| id).collect();
        for id in ids {
            self.mark_expr(id, stands);
        }
    }
}

/// What a name written under nothing, and what `new` and `old`, read
/// where a tree is walked.
#[derive(Clone, Copy)]
struct Stands {
    /// Whether a name written under nothing names the column of the
    /// table being altered.
    bare: bool,
    /// Whether `new` and `old` name that table's row.
    row: bool,
}

impl Stands {
    /// A tree no trigger stands over, where a name written under
    /// nothing reads the table when `bare`.
    const fn bare(bare: bool) -> Self {
        Self { bare, row: false }
    }
}
