// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the `CREATE TABLE` and `CREATE INDEX` half of `crate::parse`.
//!
//! `fixtures/schema.corpus` is the shapes a schema is written in and the
//! ways of writing each of them wrong, one statement per line.
//! `fixtures/schema.golden` is what SQLite made of each, written by
//! `tools/sqlite-oracle.c`: the object it created and what the pragmas
//! then answer about it, or the message it refused it with and whether
//! that refusal was a syntax error. A parser alone decides the syntax
//! errors, so those are what it is held to.

#![allow(clippy::arithmetic_side_effects)]

use crate::ast::{
    ColumnConstraint, Conflict, Definition, Literal, Node, Order, TableBody, TableConstraint,
};
use crate::parse::definition;

/// The statements.
fn corpus() -> Vec<&'static str> {
    include_str!("fixtures/schema.corpus").lines().collect()
}

/// What SQLite made of each.
fn golden() -> Vec<&'static str> {
    include_str!("fixtures/schema.golden").lines().collect()
}

/// The parsed statement, where it parses.
fn parsed(sql: &str) -> Option<Definition> {
    definition(sql.as_bytes())
        .map(|(_, definition)| definition)
        .ok()
}

#[test]
fn every_syntax_error_the_c_library_finds_is_found_here() {
    // What SQLite refuses for a reason beyond syntax — a column named
    // twice, an index over a column that is not there — this parser
    // accepts, because those are the schema layer's to refuse. The count
    // is held down so that it cannot grow unnoticed.
    let mut beyond_syntax = 0;
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for (sql, answer) in cases.iter().zip(answers) {
        let mine = parsed(sql);
        if answer.starts_with("!\tsyntax") {
            assert!(mine.is_none(), "{sql} is a syntax error and was accepted");
            continue;
        }
        if answer.starts_with("!\tother") {
            beyond_syntax += usize::from(mine.is_some());
            continue;
        }
        let kind = answer.split('\t').next().unwrap_or_default();
        match kind {
            "table" => assert!(
                matches!(mine, Some(Definition::Table(_))),
                "{sql} makes a table and was not read as one"
            ),
            "index" => assert!(
                matches!(mine, Some(Definition::Index(_))),
                "{sql} makes an index and was not read as one"
            ),
            "view" => assert!(
                matches!(mine, Some(Definition::View(_))),
                "{sql} makes a view and was not read as one"
            ),
            // A `DROP` makes nothing, so the oracle names no kind for
            // it; this parser reads it as the definition that takes one
            // away.
            _ if sql.to_ascii_uppercase().starts_with("ALTER") => assert!(
                matches!(mine, Some(Definition::AddColumn(_))),
                "{sql} adds a column and was not read as one"
            ),
            _ if sql.to_ascii_uppercase().starts_with("DROP") => assert!(
                matches!(mine, Some(Definition::Drop(_))),
                "{sql} takes a table or an index away and was not read as one"
            ),
            "trigger" => assert!(
                matches!(mine, Some(Definition::Trigger(_))),
                "{sql} makes a trigger and was not read as one"
            ),
            // Anything that is not a definition at all: this parser
            // reads five kinds and refuses the rest.
            _ => assert!(
                mine.is_none(),
                "{sql} is not a definition and was read as one"
            ),
        }
    }
    assert_eq!(
        beyond_syntax, 14,
        "what the schema layer has left to refuse"
    );
}

/// The columns of a table, where the statement makes one.
fn table_of(sql: &str) -> (crate::ast::Arena, crate::ast::CreateTable) {
    let (arena, definition) = definition(sql.as_bytes()).expect("a table");
    match definition {
        Definition::Table(table) => (arena, table),
        Definition::Index(_)
        | Definition::View(_)
        | Definition::AddColumn(_)
        | Definition::Rename(_)
        | Definition::DropColumn(_)
        | Definition::Trigger(_)
        | Definition::Drop(_) => {
            panic!("something other than a table was written")
        }
    }
}

#[test]
fn a_column_carries_its_name_its_type_and_what_follows_them() {
    let sql = "CREATE TABLE t(a, b INTEGER, c VARCHAR(10) NOT NULL)";
    let (arena, table) = table_of(sql);
    assert!(!table.temporary);
    assert!(!table.if_not_exists);
    assert_eq!(table.schema, None);
    assert_eq!(table.name.text(sql.as_bytes()), b"t");
    assert!(!table.options.without_rowid);
    assert!(!table.options.strict);
    let TableBody::Columns { columns, .. } = table.body else {
        panic!("a table with no columns written out");
    };
    let columns = arena.columns(columns);
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name.text(sql.as_bytes()), b"a");
    assert_eq!(columns[0].ty, None);
    assert_eq!(
        columns[1].ty.map(|ty| ty.text(sql.as_bytes())),
        Some(&b"INTEGER"[..])
    );
    assert_eq!(
        columns[2].ty.map(|ty| ty.text(sql.as_bytes())),
        Some(&b"VARCHAR(10)"[..])
    );
    assert_eq!(
        arena.column_constraints(columns[2].constraints),
        [ColumnConstraint::NotNull(Conflict::Unspecified)]
    );
}

#[test]
fn a_type_name_is_a_run_of_words_and_a_constraint_ends_it() {
    for (sql, ty) in [
        (
            "CREATE TABLE t(x UNSIGNED BIG INT)",
            Some(&b"UNSIGNED BIG INT"[..]),
        ),
        (
            "CREATE TABLE t(x DOUBLE PRECISION)",
            Some(&b"DOUBLE PRECISION"[..]),
        ),
        (
            "CREATE TABLE t(x DECIMAL(10,5))",
            Some(&b"DECIMAL(10,5)"[..]),
        ),
        (
            "CREATE TABLE t(x \"quoted type\")",
            Some(&b"\"quoted type\""[..]),
        ),
        // Words that may be names are type names; words that may not
        // are where the type ends.
        ("CREATE TABLE t(x generated)", Some(&b"generated"[..])),
        ("CREATE TABLE t(x key)", Some(&b"key"[..])),
        ("CREATE TABLE t(x NOT NULL)", None),
        ("CREATE TABLE t(x PRIMARY KEY)", None),
        ("CREATE TABLE t(x COLLATE NOCASE)", None),
        ("CREATE TABLE t(x AS (1))", None),
        // `GENERATED` is a type name unless `ALWAYS AS` follows it.
        ("CREATE TABLE t(x GENERATED ALWAYS AS (1))", None),
        (
            "CREATE TABLE t(x GENERATED ALWAYS)",
            Some(&b"GENERATED ALWAYS"[..]),
        ),
    ] {
        let (arena, table) = table_of(sql);
        let TableBody::Columns { columns, .. } = table.body else {
            panic!("a table with no columns written out");
        };
        let column = arena.columns(columns).first().copied().expect("a column");
        assert_eq!(
            column.ty.map(|span| span.text(sql.as_bytes())),
            ty,
            "the type of {sql}"
        );
    }
}

#[test]
fn a_default_keeps_the_text_it_was_written_as() {
    // It is what `sqlite_schema` stores and what `PRAGMA table_info`
    // answers with, and the brackets around an expression are not part
    // of it.
    for (sql, text) in [
        ("CREATE TABLE t(x DEFAULT 1)", &b"1"[..]),
        ("CREATE TABLE t(x DEFAULT -1)", &b"-1"[..]),
        ("CREATE TABLE t(x DEFAULT +1)", &b"+1"[..]),
        ("CREATE TABLE t(x DEFAULT 'a')", &b"'a'"[..]),
        ("CREATE TABLE t(x DEFAULT abc)", &b"abc"[..]),
        ("CREATE TABLE t(x DEFAULT x'41')", &b"x'41'"[..]),
        ("CREATE TABLE t(x DEFAULT (1+1))", &b"1+1"[..]),
        (
            "CREATE TABLE t(x DEFAULT CURRENT_TIMESTAMP)",
            &b"CURRENT_TIMESTAMP"[..],
        ),
    ] {
        let (arena, table) = table_of(sql);
        let TableBody::Columns { columns, .. } = table.body else {
            panic!("a table with no columns written out");
        };
        let column = arena.columns(columns).first().copied().expect("a column");
        let [ColumnConstraint::Default { text: written, .. }] =
            arena.column_constraints(column.constraints)
        else {
            panic!("a column with no default: {sql}");
        };
        assert_eq!(written.text(sql.as_bytes()), text, "the default of {sql}");
    }
}

#[test]
fn what_follows_the_columns_belongs_to_the_table() {
    let sql = "CREATE TABLE t(a, b, PRIMARY KEY(a DESC, b) ON CONFLICT REPLACE, \
               UNIQUE(b), CHECK(a>0), FOREIGN KEY(a) REFERENCES base(x))";
    let (arena, table) = table_of(sql);
    let TableBody::Columns {
        columns,
        constraints,
    } = table.body
    else {
        panic!("a table with no columns written out");
    };
    assert_eq!(arena.columns(columns).len(), 2);
    let constraints = arena.table_constraints(constraints);
    assert_eq!(constraints.len(), 4);
    let TableConstraint::PrimaryKey {
        columns: key,
        autoincrement,
        conflict,
    } = constraints[0]
    else {
        panic!("no primary key");
    };
    assert!(!autoincrement);
    assert_eq!(conflict, Conflict::Replace);
    let key = arena.orders(key);
    assert_eq!(key.len(), 2);
    assert_eq!(key[0].order, Order::Descending);
    assert_eq!(key[1].order, Order::Unspecified);
    assert!(matches!(constraints[1], TableConstraint::Unique { .. }));
    assert!(matches!(constraints[2], TableConstraint::Check { .. }));
    let TableConstraint::ForeignKey { columns, foreign } = constraints[3] else {
        panic!("no foreign key");
    };
    assert_eq!(arena.names(columns).len(), 1);
    assert_eq!(foreign.table.text(sql.as_bytes()), b"base");
    assert_eq!(arena.names(foreign.columns).len(), 1);
}

#[test]
fn an_index_carries_its_terms_and_the_rows_it_covers() {
    let sql = "CREATE UNIQUE INDEX IF NOT EXISTS main.i ON base(a DESC, b+1) WHERE a>0";
    let (arena, definition) = definition(sql.as_bytes()).expect("an index");
    let Definition::Index(index) = definition else {
        panic!("a table where an index was written");
    };
    assert!(index.unique);
    assert!(index.if_not_exists);
    assert_eq!(
        index.schema.map(|span| span.text(sql.as_bytes())),
        Some(&b"main"[..])
    );
    assert_eq!(index.name.text(sql.as_bytes()), b"i");
    assert_eq!(index.table.text(sql.as_bytes()), b"base");
    let terms = arena.orders(index.columns);
    assert_eq!(terms.len(), 2);
    assert_eq!(terms[0].order, Order::Descending);
    assert!(matches!(
        arena.node(terms[1].expr),
        Some(Node::Binary { .. })
    ));
    assert!(index.filter.is_some());
}

#[test]
fn the_two_table_options_are_the_only_two() {
    for (sql, without_rowid, strict) in [
        ("CREATE TABLE t(x PRIMARY KEY) WITHOUT ROWID", true, false),
        ("CREATE TABLE t(x INT PRIMARY KEY) STRICT", false, true),
        (
            "CREATE TABLE t(x INT PRIMARY KEY) STRICT, WITHOUT ROWID",
            true,
            true,
        ),
        (
            "CREATE TABLE t(x INT PRIMARY KEY) WITHOUT ROWID, STRICT",
            true,
            true,
        ),
    ] {
        let (_, table) = table_of(sql);
        assert_eq!(table.options.without_rowid, without_rowid, "{sql}");
        assert_eq!(table.options.strict, strict, "{sql}");
    }
    // Anything else is refused where it is written, which is not what
    // SQLite calls a syntax error but is a refusal all the same.
    assert!(parsed("CREATE TABLE t(x) NONSENSE").is_none());
    assert!(parsed("CREATE TABLE t(x) WITHOUT NONSENSE").is_none());
}

#[test]
fn a_table_may_take_its_columns_from_a_statement() {
    let (_, table) = table_of("CREATE TABLE t AS SELECT 1 AS x");
    assert!(matches!(table.body, TableBody::Select(_)));
    let (_, table) = table_of("CREATE TEMP TABLE t(x)");
    assert!(table.temporary);
    let (_, table) = table_of("CREATE TEMPORARY TABLE t(x)");
    assert!(table.temporary);
    // `TEMP` before anything but `TABLE` is a name and not the word.
    assert!(parsed("CREATE TEMP INDEX i ON base(a)").is_none());
    let (arena, table) = table_of("CREATE TABLE temp(x)");
    assert!(!table.temporary);
    let TableBody::Columns { columns, .. } = table.body else {
        panic!("a table with no columns written out");
    };
    assert_eq!(arena.columns(columns).len(), 1);
}

#[test]
fn a_generated_column_is_the_same_two_ways() {
    for sql in [
        "CREATE TABLE t(x, y AS (x+1))",
        "CREATE TABLE t(x, y GENERATED ALWAYS AS (x+1))",
    ] {
        let (arena, table) = table_of(sql);
        let TableBody::Columns { columns, .. } = table.body else {
            panic!("a table with no columns written out");
        };
        let column = arena.columns(columns).get(1).copied().expect("a column");
        let [ColumnConstraint::Generated { kind: None, .. }] =
            arena.column_constraints(column.constraints)
        else {
            panic!("not a generated column: {sql}");
        };
    }
    let sql = "CREATE TABLE t(x, y AS (x+1) STORED)";
    let (arena, table) = table_of(sql);
    let TableBody::Columns { columns, .. } = table.body else {
        panic!("a table with no columns written out");
    };
    let column = arena.columns(columns).get(1).copied().expect("a column");
    let [
        ColumnConstraint::Generated {
            kind: Some(word), ..
        },
    ] = arena.column_constraints(column.constraints)
    else {
        panic!("not a generated column");
    };
    assert_eq!(word.text(sql.as_bytes()), b"STORED");
}

#[test]
fn a_literal_where_a_value_belongs_reads_as_the_text_it_is() {
    // `DEFAULT abc` is the string 'abc', not a column and not an error.
    let sql = "CREATE TABLE t(x DEFAULT abc)";
    let (arena, table) = table_of(sql);
    let TableBody::Columns { columns, .. } = table.body else {
        panic!("a table with no columns written out");
    };
    let column = arena.columns(columns).first().copied().expect("a column");
    let [ColumnConstraint::Default { value, .. }] = arena.column_constraints(column.constraints)
    else {
        panic!("a column with no default");
    };
    assert!(matches!(
        arena.node(*value),
        Some(Node::Literal(Literal::Text(_)))
    ));
}

#[test]
fn one_definition_is_all_a_row_of_the_schema_holds() {
    // A trailing semicolon belongs to it; a second statement does not.
    assert!(parsed("CREATE TABLE t(x);").is_some());
    assert!(parsed("CREATE TABLE t(x); CREATE TABLE u(y)").is_none());
    assert!(parsed("CREATE INDEX i ON base(a); SELECT 1").is_none());
}

/// The trigger a statement makes.
fn trigger_of(sql: &str) -> (crate::ast::Arena, crate::ast::CreateTrigger) {
    let (arena, definition) = definition(sql.as_bytes()).expect("a trigger");
    match definition {
        Definition::Trigger(trigger) => (arena, trigger),
        Definition::Table(_)
        | Definition::Index(_)
        | Definition::View(_)
        | Definition::AddColumn(_)
        | Definition::Rename(_)
        | Definition::DropColumn(_)
        | Definition::Drop(_) => panic!("something other than a trigger was written"),
    }
}

#[test]
fn a_trigger_carries_when_it_runs_what_it_runs_on_and_what_it_runs() {
    use crate::ast::{TriggerEvent, TriggerStep, TriggerTime};
    let sql = "CREATE TRIGGER a AFTER INSERT ON t BEGIN INSERT INTO log VALUES(new.a); END";
    let (arena, trigger) = trigger_of(sql);
    assert!(!trigger.temporary);
    assert!(!trigger.if_not_exists);
    assert_eq!(trigger.schema, None);
    assert_eq!(trigger.name.text(sql.as_bytes()), b"a");
    assert_eq!(trigger.time, TriggerTime::After);
    assert_eq!(trigger.event, TriggerEvent::Insert);
    assert_eq!(trigger.table.text(sql.as_bytes()), b"t");
    assert_eq!(trigger.condition, None);
    assert!(matches!(
        arena.steps(trigger.body),
        [TriggerStep::Insert(_)]
    ));

    // A trigger with no time written runs before the row, `FOR EACH
    // ROW` is read and says nothing, and a `WHEN` is kept.
    let sql = "CREATE TRIGGER b BEFORE DELETE ON t FOR EACH ROW WHEN old.a>1 \
               BEGIN DELETE FROM log WHERE a=old.a; UPDATE log SET a=1; END";
    let (arena, trigger) = trigger_of(sql);
    assert_eq!(trigger.time, TriggerTime::Before);
    assert_eq!(trigger.event, TriggerEvent::Delete);
    assert!(trigger.condition.is_some());
    assert!(matches!(
        arena.steps(trigger.body),
        [TriggerStep::Delete(_), TriggerStep::Update(_)]
    ));
    let sql = "CREATE TRIGGER c UPDATE ON t BEGIN SELECT 1; END";
    let (arena, trigger) = trigger_of(sql);
    assert_eq!(trigger.time, TriggerTime::Before);
    assert_eq!(trigger.event, TriggerEvent::Update);
    assert!(matches!(
        arena.steps(trigger.body),
        [TriggerStep::Select(_)]
    ));

    // `INSTEAD OF`, the columns an `UPDATE OF` names, and
    // `IF NOT EXISTS`.
    let sql = "CREATE TRIGGER IF NOT EXISTS d INSTEAD OF UPDATE OF a,b ON v BEGIN SELECT 1; END";
    let (arena, trigger) = trigger_of(sql);
    assert!(trigger.if_not_exists);
    assert_eq!(trigger.time, TriggerTime::InsteadOf);
    assert_eq!(trigger.event, TriggerEvent::Update);
    assert_eq!(arena.names(trigger.columns).len(), 2);
    assert_eq!(trigger.table.text(sql.as_bytes()), b"v");

    // The four shapes a statement of a body is written in.
    let sql = "CREATE TRIGGER e AFTER INSERT ON t BEGIN \
               REPLACE INTO log VALUES(1); VALUES(2); \
               WITH c(i) AS (SELECT 1) SELECT i FROM c; DELETE FROM log; END";
    let (arena, trigger) = trigger_of(sql);
    assert!(matches!(
        arena.steps(trigger.body),
        [
            TriggerStep::Insert(_),
            TriggerStep::Select(_),
            TriggerStep::Select(_),
            TriggerStep::Delete(_),
        ]
    ));

    // A `DROP TRIGGER` takes one away, and `IF EXISTS` allows a name
    // the database does not hold.
    for (sql, if_exists) in [
        ("DROP TRIGGER a", false),
        ("DROP TRIGGER IF EXISTS a", true),
    ] {
        let (_, definition) = definition(sql.as_bytes()).expect("a drop");
        let Definition::Drop(asked) = definition else {
            panic!("something other than a drop was written");
        };
        assert_eq!(asked.kind, crate::ast::Dropped::Trigger);
        assert_eq!(asked.if_exists, if_exists);
    }
}

#[test]
fn what_a_trigger_refuses() {
    use crate::parse::Expected;
    for (sql, expected) in [
        (
            "CREATE TRIGGER a INSTEAD UPDATE ON t BEGIN SELECT 1; END",
            Expected::Of,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT ON t BEGIN CREATE TABLE u(a); END",
            Expected::Step,
        ),
        (
            "CREATE TRIGGER a AFTER TRUNCATE ON t BEGIN SELECT 1; END",
            Expected::Update,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT t BEGIN SELECT 1; END",
            Expected::On,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT ON t FOR ROW BEGIN SELECT 1; END",
            Expected::Row,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT ON t SELECT 1; END",
            Expected::Body,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT ON t BEGIN DROP TABLE t; END",
            Expected::Step,
        ),
        (
            "CREATE TRIGGER a AFTER INSERT ON t BEGIN SELECT 1 END",
            Expected::Semi,
        ),
    ] {
        let read = definition(sql.as_bytes());
        assert_eq!(
            read.err().map(|error| error.expected),
            Some(expected),
            "{sql}"
        );
    }
    // `RAISE` is an expression, and one of its four words is what
    // stands in the brackets.
    for (sql, expected) in [
        ("RAISE(NOTHING, 'x')", Expected::Action),
        ("RAISE(ABORT 'x')", Expected::Expression),
        ("RAISE ABORT", Expected::OpenParen),
        ("RAISE(ABORT, 'x'", Expected::CloseParen),
    ] {
        let read = crate::parse::expression(sql.as_bytes());
        assert_eq!(
            read.err().map(|error| error.expected),
            Some(expected),
            "{sql}"
        );
    }
}
