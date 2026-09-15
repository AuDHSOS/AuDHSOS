// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The expression parser against arbitrary bytes: no input may panic, no
//! input may run the stack out, and a tree that comes back is a tree.
//!
//! A parser is where a stack overflow lives, because the input decides how
//! deep the walk goes. The bound is in the parser and this target is what
//! holds it: a statement of ten thousand brackets is refused rather than
//! followed.

use db_sqlite::ast::{
    Arena, ColumnConstraint, Definition, ExprId, Node, SelectId, TableBody, TriggerStep,
};
use db_sqlite::parse::{definition, expression, statement};

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    // The same bytes as a statement: a `SELECT` is where the tree of
    // statements and the tree of expressions meet, and a subquery is how
    // deep that can go.
    if let Ok((arena, root)) = statement(bytes) {
        walk_select(&arena, root, 0);
    }
    // The same bytes as a definition: what a row of `sqlite_schema`
    // holds, which is text a file decides and not a statement a program
    // wrote.
    if let Ok((arena, definition)) = definition(bytes) {
        walk_definition(&arena, definition);
        // And the table it describes, where it describes one: a schema
        // read off a file is text the file decides.
        if let Definition::Table(written) = definition
            && let Ok(table) = db_sqlite::schema::table(&arena, &written, bytes)
        {
            assert!(
                table.columns.iter().any(|column| column.key == 0)
                    || table.columns.iter().all(|column| column.key > 0),
                "a key that is neither some columns nor all of them"
            );
            if let Some(alias) = table.rowid_alias {
                assert!(alias < table.columns.len(), "a rowid outside the columns");
                assert!(!table.without_rowid, "a rowid on a table with none");
            }
            for (at, column) in table.columns.iter().enumerate() {
                // A key place counts the terms the key was written
                // with, so a column named twice there leaves a place
                // past the count of columns, which is what `PRAGMA
                // table_info` answers. What holds is that no two
                // columns share a place.
                assert!(
                    column.key == 0
                        || !table.columns[..at]
                            .iter()
                            .any(|before| before.key == column.key),
                    "two columns at one key place"
                );
                assert!(
                    !table.columns[..at]
                        .iter()
                        .any(|before| before.name.eq_ignore_ascii_case(&column.name)),
                    "two columns of one name"
                );
            }
        }
    }
    match expression(bytes) {
        Err(error) => {
            assert!(
                error.at <= bytes.len(),
                "a refusal past the end of the input"
            );
            assert!(
                error.at.saturating_add(error.len) <= bytes.len(),
                "a refusal longer than the input"
            );
        }
        Ok((arena, root)) => {
            assert!(!arena.is_empty(), "a tree of no nodes");
            // Every node the tree names is a node the arena holds, and
            // every child of it is one the arena holds as well.
            walk(&arena, root, 0);
        }
    }
});

/// Reads every expression a definition holds, so that a definition is a
/// tree as well.
fn walk_definition(arena: &Arena, definition: Definition) {
    match definition {
        Definition::Table(table) => match table.body {
            TableBody::Select(select) => walk_select(arena, select, 0),
            TableBody::Columns { columns, .. } => {
                for column in arena.columns(columns) {
                    for constraint in arena.column_constraints(column.constraints) {
                        match *constraint {
                            ColumnConstraint::Check { value: expr, .. }
                            | ColumnConstraint::Default { value: expr, .. }
                            | ColumnConstraint::Generated { value: expr, .. } => {
                                walk(arena, expr, 0);
                            }
                            _ => {}
                        }
                    }
                }
            }
        },
        Definition::Index(index) => {
            for term in arena.orders(index.columns) {
                walk(arena, term.expr, 0);
            }
            if let Some(filter) = index.filter {
                walk(arena, filter, 0);
            }
        }
        // A view holds the statement it names, and every expression of
        // that statement with it.
        Definition::View(view) => walk_select(arena, view.select, 0),
        // A column added to a table holds the expressions its
        // constraints are written with, as a column of a `CREATE TABLE`
        // does.
        Definition::AddColumn(added) => {
            for constraint in arena.column_constraints(added.column.constraints) {
                match *constraint {
                    ColumnConstraint::Check { value: expr, .. }
                    | ColumnConstraint::Default { value: expr, .. }
                    | ColumnConstraint::Generated { value: expr, .. } => {
                        walk(arena, expr, 0);
                    }
                    _ => {}
                }
            }
        }
        // A trigger holds the `WHEN` it runs under and every statement
        // of its body.
        Definition::Trigger(trigger) => {
            if let Some(expr) = trigger.condition {
                walk(arena, expr, 0);
            }
            for step in arena.steps(trigger.body) {
                match *step {
                    TriggerStep::Select(select) => walk_select(arena, select, 0),
                    TriggerStep::Insert(insert) => walk_select(arena, insert.select, 0),
                    TriggerStep::Update(update) => {
                        for set in arena.sets(update.sets) {
                            walk(arena, set.value, 0);
                        }
                        if let Some(expr) = update.filter {
                            walk(arena, expr, 0);
                        }
                    }
                    TriggerStep::Delete(delete) => {
                        if let Some(expr) = delete.filter {
                            walk(arena, expr, 0);
                        }
                    }
                }
            }
        }
        // A `DROP` names a table, an index, a view or a trigger and
        // holds no expression.
        Definition::Drop(_) => {}
    }
}

/// Reads every statement reachable from `id`, and every expression of
/// each, so that a statement is a tree as well.
fn walk_select(arena: &Arena, id: SelectId, depth: u32) {
    assert!(
        depth < 512,
        "a statement nested deeper than the parser walks"
    );
    let select = arena
        .select(id)
        .expect("a statement the arena does not hold");
    let deeper = depth.saturating_add(1);
    for cte in arena.ctes(select.ctes) {
        walk_select(arena, cte.select, deeper);
    }
    for column in arena.results(select.columns) {
        if let db_sqlite::ast::ResultColumn::Expr { expr, .. } = *column {
            walk(arena, expr, 0);
        }
    }
    for source in arena.sources(select.from) {
        match source.kind {
            db_sqlite::ast::SourceKind::Select(inner) => walk_select(arena, inner, deeper),
            db_sqlite::ast::SourceKind::Function { args, .. } => {
                for arg in arena.children(args) {
                    walk(arena, *arg, 0);
                }
            }
            db_sqlite::ast::SourceKind::Table { .. } => {}
        }
        if let Some(on) = source.on {
            walk(arena, on, 0);
        }
    }
    for expr in select
        .filter
        .into_iter()
        .chain(select.having)
        .chain(select.limit.map(|limit| limit.count))
        .chain(select.limit.and_then(|limit| limit.offset))
    {
        walk(arena, expr, 0);
    }
    for expr in arena
        .children(select.group)
        .iter()
        .chain(arena.children(select.values))
    {
        walk(arena, *expr, 0);
    }
    for term in arena.orders(select.order) {
        walk(arena, term.expr, 0);
    }
    // A `WINDOW` clause names windows, and a window names terms and may
    // bound its frame by expressions.
    for named in arena.named_windows(select.windows) {
        let Some(window) = arena.window(named.window) else {
            continue;
        };
        for term in arena.children(window.partition) {
            walk(arena, *term, 0);
        }
        for term in arena.orders(window.order) {
            walk(arena, term.expr, 0);
        }
        if let Some(frame) = window.frame {
            for bound in [frame.start, frame.end] {
                if let db_sqlite::ast::Bound::Preceding(id) | db_sqlite::ast::Bound::Following(id) =
                    bound
                {
                    walk(arena, id, 0);
                }
            }
        }
    }
    if let Some((_, next)) = select.compound {
        walk_select(arena, next, deeper);
    }
}

/// Reads every node reachable from `id`, and holds that the tree is one:
/// a child is always a node the arena has.
fn walk(arena: &Arena, id: ExprId, depth: u32) {
    // The parser's own bound is two hundred; a tree deeper than twice that
    // would mean the bound did not hold.
    assert!(depth < 512, "a tree deeper than the parser walks");
    let node = arena.node(id).expect("a node the arena does not hold");
    let mut child = |id| walk(arena, id, depth.saturating_add(1));
    match node {
        Node::Literal(_) | Node::Column { .. } | Node::Variable(_) => {}
        Node::Unary { operand, .. } => child(operand),
        Node::Binary { left, right, .. } => {
            child(left);
            child(right);
        }
        Node::Between {
            value, low, high, ..
        } => {
            child(value);
            child(low);
            child(high);
        }
        Node::InList { value, list, .. } => {
            child(value);
            for item in arena.children(list) {
                child(*item);
            }
        }
        Node::Like {
            value,
            pattern,
            escape,
            ..
        } => {
            child(value);
            child(pattern);
            if let Some(escape) = escape {
                child(escape);
            }
        }
        Node::Cast { value, .. } | Node::Collate { value, .. } => child(value),
        Node::Call { args, filter, .. } | Node::Over { args, filter, .. } => {
            for arg in arena.children(args) {
                child(*arg);
            }
            if let Some(id) = filter {
                child(id);
            }
        }
        Node::Case {
            operand,
            branches,
            otherwise,
        } => {
            for id in operand.into_iter().chain(otherwise) {
                child(id);
            }
            for branch in arena.children(branches) {
                child(*branch);
            }
        }
        Node::Row(items) => {
            for item in arena.children(items) {
                child(*item);
            }
        }
        // A statement is walked by `walk_select`, which the statement
        // above this expression reached it through.
        Node::Subquery(_) | Node::Exists(_) => {}
        Node::InSelect { value, .. } | Node::InTable { value, .. } => child(value),
        // `RAISE(IGNORE)` holds no message and the other three hold
        // one.
        Node::Raise { message, .. } => {
            if let Some(expr) = message {
                child(expr);
            }
        }
    }
}
