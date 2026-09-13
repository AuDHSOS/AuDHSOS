// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::parse`.
//!
//! Two kinds. The tree of a statement is written out as a bracketed form
//! and compared against what the grammar of `src/parse.y` says it should
//! be, which is how precedence and associativity are held. And the
//! statements of `fixtures/expr.corpus` are compared against
//! `fixtures/expr.golden`, which records, for each of them, whether
//! SQLite's own parser accepted it.

#![allow(clippy::arithmetic_side_effects)]

use crate::ast::{Arena, BinaryOp, CurrentTime, ExprId, LikeOp, Literal, Node, UnaryOp};
use crate::parse::{Error, Expected, expression};

/// The tree, written out so that a test can say what it expects.
#[allow(
    clippy::too_many_lines,
    reason = "one arm per node kind, and the point is that none is missing"
)]
fn write(arena: &Arena, id: ExprId, sql: &[u8], out: &mut String) {
    let Some(node) = arena.node(id) else {
        out.push('?');
        return;
    };
    let text = |span: crate::ast::Span| String::from_utf8_lossy(span.text(sql)).into_owned();
    match node {
        Node::Literal(Literal::Null) => out.push_str("null"),
        Node::Literal(Literal::CurrentTime(which)) => {
            out.push_str(match which {
                CurrentTime::Time => "current_time",
                CurrentTime::Date => "current_date",
                CurrentTime::Timestamp => "current_timestamp",
            });
        }
        Node::Literal(
            Literal::Integer(span)
            | Literal::Float(span)
            | Literal::Text(span)
            | Literal::Blob(span),
        ) => out.push_str(&text(span)),
        Node::Column {
            schema,
            table,
            column,
        } => {
            out.push_str("(col");
            for part in [schema, table].into_iter().flatten() {
                out.push(' ');
                out.push_str(&text(part));
            }
            out.push(' ');
            out.push_str(&text(column));
            out.push(')');
        }
        Node::Variable(span) => {
            out.push_str("(var ");
            out.push_str(&text(span));
            out.push(')');
        }
        Node::Unary { op, operand } => {
            out.push_str(match op {
                UnaryOp::Not => "(not ",
                UnaryOp::Negate => "(neg ",
                UnaryOp::Identity => "(pos ",
                UnaryOp::BitNot => "(bitnot ",
                UnaryOp::IsNull => "(isnull ",
                UnaryOp::NotNull => "(notnull ",
            });
            write(arena, operand, sql, out);
            out.push(')');
        }
        Node::Binary { op, left, right } => {
            out.push('(');
            out.push_str(match op {
                BinaryOp::Or => "or",
                BinaryOp::And => "and",
                BinaryOp::Eq => "eq",
                BinaryOp::Ne => "ne",
                BinaryOp::Lt => "lt",
                BinaryOp::Le => "le",
                BinaryOp::Gt => "gt",
                BinaryOp::Ge => "ge",
                BinaryOp::Is => "is",
                BinaryOp::IsNot => "isnot",
                BinaryOp::Add => "add",
                BinaryOp::Subtract => "sub",
                BinaryOp::Multiply => "mul",
                BinaryOp::Divide => "div",
                BinaryOp::Modulo => "mod",
                BinaryOp::Concat => "cat",
                BinaryOp::Extract => "ptr",
                BinaryOp::ExtractText => "ptrptr",
                BinaryOp::BitAnd => "band",
                BinaryOp::BitOr => "bor",
                BinaryOp::LShift => "shl",
                BinaryOp::RShift => "shr",
            });
            out.push(' ');
            write(arena, left, sql, out);
            out.push(' ');
            write(arena, right, sql, out);
            out.push(')');
        }
        Node::Between {
            value,
            low,
            high,
            negated,
        } => {
            out.push_str(if negated { "(notbetween " } else { "(between " });
            write(arena, value, sql, out);
            out.push(' ');
            write(arena, low, sql, out);
            out.push(' ');
            write(arena, high, sql, out);
            out.push(')');
        }
        Node::InList {
            value,
            list,
            negated,
        } => {
            out.push_str(if negated { "(notin " } else { "(in " });
            write(arena, value, sql, out);
            for item in arena.children(list) {
                out.push(' ');
                write(arena, *item, sql, out);
            }
            out.push(')');
        }
        Node::Like {
            op,
            value,
            pattern,
            escape,
            negated,
        } => {
            out.push('(');
            if negated {
                out.push_str("not");
            }
            out.push_str(match op {
                LikeOp::Like => "like ",
                LikeOp::Glob => "glob ",
                LikeOp::Regexp => "regexp ",
                LikeOp::Match => "match ",
            });
            write(arena, value, sql, out);
            out.push(' ');
            write(arena, pattern, sql, out);
            if let Some(escape) = escape {
                out.push_str(" escape ");
                write(arena, escape, sql, out);
            }
            out.push(')');
        }
        Node::Cast { value, ty } => {
            out.push_str("(cast ");
            write(arena, value, sql, out);
            out.push(' ');
            out.push_str(&text(ty));
            out.push(')');
        }
        Node::Collate { value, name } => {
            out.push_str("(collate ");
            write(arena, value, sql, out);
            out.push(' ');
            out.push_str(&text(name));
            out.push(')');
        }
        Node::Call {
            name,
            args,
            distinct,
            star,
        } => {
            out.push_str("(call ");
            out.push_str(&text(name));
            if distinct {
                out.push_str(" distinct");
            }
            if star {
                out.push_str(" *");
            }
            for arg in arena.children(args) {
                out.push(' ');
                write(arena, *arg, sql, out);
            }
            out.push(')');
        }
        Node::Case {
            operand,
            branches,
            otherwise,
        } => {
            out.push_str("(case");
            if let Some(operand) = operand {
                out.push(' ');
                write(arena, operand, sql, out);
            }
            for branch in arena.children(branches) {
                out.push(' ');
                write(arena, *branch, sql, out);
            }
            if let Some(otherwise) = otherwise {
                out.push_str(" else ");
                write(arena, otherwise, sql, out);
            }
            out.push(')');
        }
        Node::Row(items) => {
            out.push_str("(row");
            for item in arena.children(items) {
                out.push(' ');
                write(arena, *item, sql, out);
            }
            out.push(')');
        }
        Node::Subquery(select) => {
            out.push_str("(sub ");
            write_select(arena, select, sql, out);
            out.push(')');
        }
        Node::Exists(select) => {
            out.push_str("(exists ");
            write_select(arena, select, sql, out);
            out.push(')');
        }
        Node::InSelect {
            value,
            select,
            negated,
        } => {
            out.push_str(if negated { "(notinsel " } else { "(insel " });
            write(arena, value, sql, out);
            out.push(' ');
            write_select(arena, select, sql, out);
            out.push(')');
        }
        Node::InTable {
            value,
            schema,
            table,
            negated,
        } => {
            out.push_str(if negated { "(notintab " } else { "(intab " });
            write(arena, value, sql, out);
            for part in schema.into_iter().chain(core::iter::once(table)) {
                out.push(' ');
                out.push_str(&text(part));
            }
            out.push(')');
        }
    }
}

/// One statement, written out the same way.
#[allow(
    clippy::too_many_lines,
    reason = "one clause per clause of the grammar, and the point is that none is missing"
)]
fn write_select(arena: &Arena, id: crate::ast::SelectId, sql: &[u8], out: &mut String) {
    use crate::ast::{
        Compound, Distinct, Indexed, JoinKind, Materialized, Nulls, Order, ResultColumn, SourceKind,
    };

    let Some(select) = arena.select(id) else {
        out.push('?');
        return;
    };
    let text = |span: crate::ast::Span| String::from_utf8_lossy(span.text(sql)).into_owned();
    out.push_str("(select");
    if select.recursive {
        out.push_str(" recursive");
    }
    for cte in arena.ctes(select.ctes) {
        out.push_str(" (with ");
        out.push_str(&text(cte.name));
        for column in arena.names(cte.columns) {
            out.push(' ');
            out.push_str(&text(*column));
        }
        match cte.materialized {
            Materialized::Yes => out.push_str(" materialized"),
            Materialized::No => out.push_str(" notmaterialized"),
            Materialized::Unspecified => {}
        }
        out.push(' ');
        write_select(arena, cte.select, sql, out);
        out.push(')');
    }
    match select.distinct {
        Distinct::Distinct => out.push_str(" distinct"),
        Distinct::All => out.push_str(" all"),
        Distinct::Unspecified => {}
    }
    for column in arena.results(select.columns) {
        out.push(' ');
        match *column {
            ResultColumn::Star => out.push('*'),
            ResultColumn::TableStar(table) => {
                out.push_str(&text(table));
                out.push_str(".*");
            }
            ResultColumn::Expr { expr, alias } => {
                write(arena, expr, sql, out);
                if let Some(alias) = alias {
                    out.push_str(" as ");
                    out.push_str(&text(alias));
                }
            }
        }
    }
    for row in arena.children(select.values) {
        out.push_str(" values ");
        write(arena, *row, sql, out);
    }
    for source in arena.sources(select.from) {
        out.push_str(" (from");
        match source.join.kind {
            JoinKind::None => {}
            JoinKind::Inner if source.join.comma => out.push_str(" comma"),
            JoinKind::Inner => out.push_str(" inner"),
            JoinKind::Cross => out.push_str(" cross"),
            JoinKind::Left => out.push_str(" left"),
            JoinKind::Right => out.push_str(" right"),
            JoinKind::Full => out.push_str(" full"),
        }
        if source.join.natural {
            out.push_str(" natural");
        }
        match source.kind {
            SourceKind::Table {
                schema,
                name,
                indexed,
            } => {
                for part in schema.into_iter().chain(core::iter::once(name)) {
                    out.push(' ');
                    out.push_str(&text(part));
                }
                match indexed {
                    Indexed::By(index) => {
                        out.push_str(" indexedby ");
                        out.push_str(&text(index));
                    }
                    Indexed::Not => out.push_str(" notindexed"),
                    Indexed::Unspecified => {}
                }
            }
            SourceKind::Function { schema, name, args } => {
                out.push_str(" (call");
                for part in schema.into_iter().chain(core::iter::once(name)) {
                    out.push(' ');
                    out.push_str(&text(part));
                }
                for arg in arena.children(args) {
                    out.push(' ');
                    write(arena, *arg, sql, out);
                }
                out.push(')');
            }
            SourceKind::Select(inner) => {
                out.push(' ');
                write_select(arena, inner, sql, out);
            }
        }
        if let Some(alias) = source.alias {
            out.push_str(" as ");
            out.push_str(&text(alias));
        }
        if let Some(on) = source.on {
            out.push_str(" on ");
            write(arena, on, sql, out);
        }
        for name in arena.names(source.using) {
            out.push_str(" using ");
            out.push_str(&text(*name));
        }
        out.push(')');
    }
    if let Some(filter) = select.filter {
        out.push_str(" (where ");
        write(arena, filter, sql, out);
        out.push(')');
    }
    for term in arena.children(select.group) {
        out.push_str(" (group ");
        write(arena, *term, sql, out);
        out.push(')');
    }
    if let Some(having) = select.having {
        out.push_str(" (having ");
        write(arena, having, sql, out);
        out.push(')');
    }
    if let Some((operator, next)) = select.compound {
        out.push_str(match operator {
            Compound::Union => " union ",
            Compound::UnionAll => " unionall ",
            Compound::Except => " except ",
            Compound::Intersect => " intersect ",
        });
        write_select(arena, next, sql, out);
    }
    for term in arena.orders(select.order) {
        out.push_str(" (order ");
        write(arena, term.expr, sql, out);
        match term.order {
            Order::Ascending => out.push_str(" asc"),
            Order::Descending => out.push_str(" desc"),
            Order::Unspecified => {}
        }
        match term.nulls {
            Nulls::First => out.push_str(" nullsfirst"),
            Nulls::Last => out.push_str(" nullslast"),
            Nulls::Unspecified => {}
        }
        out.push(')');
    }
    if let Some(limit) = select.limit {
        out.push_str(" (limit ");
        write(arena, limit.count, sql, out);
        if let Some(offset) = limit.offset {
            out.push_str(" offset ");
            write(arena, offset, sql, out);
        }
        out.push(')');
    }
    out.push(')');
}

/// The statement of `sql`, written out, or the refusal it ended in.
fn parsed(sql: &str) -> Result<String, Error> {
    let bytes = sql.as_bytes();
    let (arena, root) = crate::parse::statement(bytes)?;
    let mut out = String::new();
    write_select(&arena, root, bytes, &mut out);
    Ok(out)
}

/// The tree of `sql`, written out, or the refusal it ended in.
fn tree(sql: &str) -> Result<String, Error> {
    let bytes = sql.as_bytes();
    let (arena, root) = expression(bytes)?;
    let mut out = String::new();
    write(&arena, root, bytes, &mut out);
    Ok(out)
}

#[test]
fn the_precedence_of_the_grammar_is_the_precedence_of_the_tree() {
    let cases = [
        ("1+2*3", "(add 1 (mul 2 3))"),
        ("1*2+3", "(add (mul 1 2) 3)"),
        ("(1+2)*3", "(mul (add 1 2) 3)"),
        ("1-2-3", "(sub (sub 1 2) 3)"),
        ("1||2||3", "(cat (cat 1 2) 3)"),
        ("1 AND 2 OR 3", "(or (and 1 2) 3)"),
        ("1 OR 2 AND 3", "(or 1 (and 2 3))"),
        ("NOT 1 AND 2", "(and (not 1) 2)"),
        ("NOT (1 AND 2)", "(not (and 1 2))"),
        ("1 = 2 AND 3", "(and (eq 1 2) 3)"),
        ("1 < 2 = 3", "(eq (lt 1 2) 3)"),
        ("1 & 2 + 3", "(band 1 (add 2 3))"),
        ("1 << 2 * 3", "(shl 1 (mul 2 3))"),
        ("- 1 + 2", "(add (neg 1) 2)"),
        ("~ 1 * 2", "(mul (bitnot 1) 2)"),
        ("1 || 2 COLLATE nocase", "(cat 1 (collate 2 nocase))"),
        ("a -> 'b' ->> 'c'", "(ptrptr (ptr (col a) 'b') 'c')"),
    ];
    for (sql, expected) in cases {
        assert_eq!(tree(sql).as_deref(), Ok(expected), "{sql}");
    }
}

#[test]
fn every_shape_of_expression_reads_as_the_shape_it_is() {
    let cases = [
        ("NULL", "null"),
        ("1", "1"),
        ("1.5e3", "1.5e3"),
        ("'a''b'", "'a''b'"),
        ("x'0a'", "x'0a'"),
        ("?", "(var ?)"),
        (":name", "(var :name)"),
        ("a", "(col a)"),
        ("a.b", "(col a b)"),
        ("a.b.c", "(col a b c)"),
        ("\"quoted\"", "(col \"quoted\")"),
        ("[bracketed]", "(col [bracketed])"),
        ("`quoted`", "(col `quoted`)"),
        ("abs(-1)", "(call abs (neg 1))"),
        ("count(*)", "(call count *)"),
        ("count(DISTINCT a)", "(call count distinct (col a))"),
        ("count(ALL a)", "(call count (col a))"),
        ("f()", "(call f)"),
        ("CAST(1 AS INTEGER)", "(cast 1 INTEGER)"),
        ("CAST(1 AS VARCHAR(10))", "(cast 1 VARCHAR(10))"),
        ("CAST(1 AS DOUBLE PRECISION)", "(cast 1 DOUBLE PRECISION)"),
        // The grammar allows a cast to no type at all.
        ("CAST(1 AS)", "(cast 1 )"),
        ("CASE WHEN 1 THEN 2 END", "(case 1 2)"),
        ("CASE 0 WHEN 1 THEN 2 ELSE 3 END", "(case 0 1 2 else 3)"),
        ("(1,2)", "(row 1 2)"),
        ("(1)", "1"),
        ("1 BETWEEN 2 AND 3", "(between 1 2 3)"),
        ("1 NOT BETWEEN 2 AND 3", "(notbetween 1 2 3)"),
        ("1 IN (1,2)", "(in 1 1 2)"),
        ("1 IN ()", "(in 1)"),
        ("1 NOT IN (1)", "(notin 1 1)"),
        ("1 LIKE 'a'", "(like 1 'a')"),
        ("1 NOT LIKE 'a' ESCAPE 'b'", "(notlike 1 'a' escape 'b')"),
        ("1 GLOB 'a'", "(glob 1 'a')"),
        ("1 REGEXP 'a'", "(regexp 1 'a')"),
        ("1 MATCH 'a'", "(match 1 'a')"),
        ("1 NOT REGEXP 'a'", "(notregexp 1 'a')"),
        ("1 NOT MATCH 'a'", "(notmatch 1 'a')"),
        ("1 NOT GLOB 'a'", "(notglob 1 'a')"),
        ("1 ISNULL", "(isnull 1)"),
        ("1 NOTNULL", "(notnull 1)"),
        ("1 NOT NULL", "(notnull 1)"),
        ("1 IS NULL", "(is 1 null)"),
        ("1 IS NOT 2", "(isnot 1 2)"),
        ("1 IS DISTINCT FROM 2", "(isnot 1 2)"),
        ("1 IS NOT DISTINCT FROM 2", "(is 1 2)"),
        ("CURRENT_TIMESTAMP", "current_timestamp"),
        ("current_date", "current_date"),
        ("CURRENT_TIME", "current_time"),
        ("abort", "(col abort)"),
        ("key(1)", "(call key 1)"),
    ];
    for (sql, expected) in cases {
        assert_eq!(tree(sql).as_deref(), Ok(expected), "{sql}");
    }
}

#[test]
fn a_statement_that_is_not_an_expression_says_where_it_stopped() {
    let cases = [
        ("", 0, Expected::Expression),
        ("1+", 2, Expected::Expression),
        ("+", 1, Expected::Expression),
        ("1 2", 2, Expected::Eof),
        ("(1", 2, Expected::CloseParen),
        ("CAST(1)", 6, Expected::As),
        // `END` is a word that may be a name, so the parser reads it as
        // the value a `CASE` compares against and stops at the end of the
        // statement. SQLite's table-driven parser knows that `END` cannot
        // be one there and stops at the word itself; the two agree that it
        // is not a statement, and differ over where to point.
        ("CASE END", 8, Expected::Expression),
        ("CASE WHEN 1 END", 12, Expected::Then),
        ("CASE WHEN 1 THEN 2", 18, Expected::End),
        ("1 BETWEEN 2", 11, Expected::And),
        ("1 IN", 4, Expected::Name),
        ("1 COLLATE", 9, Expected::Name),
        ("1 NOT 2", 6, Expected::Expression),
    ];
    for (sql, at, expected) in cases {
        let error = tree(sql).expect_err(sql);
        assert_eq!((error.at, error.expected), (at, expected), "{sql}");
    }
}

#[test]
fn a_statement_that_nests_deeper_than_the_walk_is_refused() {
    let deep = "(".repeat(500) + "1" + &")".repeat(500);
    let error = tree(&deep).expect_err("a statement of five hundred brackets");
    assert_eq!(error.expected, Expected::Depth);
    // One that fits is read.
    let shallow = "(".repeat(50) + "1" + &")".repeat(50);
    assert_eq!(tree(&shallow).as_deref(), Ok("1"));
}

#[test]
fn what_sqlite_refuses_this_parser_refuses_too() {
    let corpus: &[u8] = include_bytes!("fixtures/expr.corpus");
    let golden: &str = include_str!("fixtures/expr.golden");
    let mut cases: Vec<&[u8]> = corpus.split(|byte| *byte == 0).collect();
    cases.pop();
    let answers: Vec<&str> = golden.lines().collect();
    assert_eq!(cases.len(), answers.len());

    // What the parser cannot read yet: the two window clauses.
    // `RAISE(...)` is not among them — it reads as a call, which is what
    // it looks like, and only the resolver will care that it is not one.
    let not_yet: [&str; 2] = ["count(*) OVER ()", "count(*) FILTER (WHERE 1)"];

    let mut waiting = 0;
    for (case, answer) in cases.iter().zip(&answers) {
        let sql = String::from_utf8_lossy(case).into_owned();
        let read = tree(&sql);
        match *answer {
            "accept" => {
                if not_yet.contains(&sql.as_str()) {
                    assert!(read.is_err(), "`{sql}` reads, and was on the waiting list");
                    waiting += 1;
                } else {
                    assert!(read.is_ok(), "`{sql}` is SQL and was refused: {read:?}");
                }
            }
            _ => assert!(read.is_err(), "`{sql}` is not SQL and was read as {read:?}"),
        }
    }
    assert_eq!(waiting, not_yet.len(), "the waiting list is out of date");
}

#[test]
fn a_refusal_inside_a_construct_is_the_refusal_of_the_statement() {
    // Every place the parser reads an expression inside something else,
    // with something that is not one in it.
    let cases = [
        "CAST(1+ AS INTEGER)",
        "f(1+)",
        "f(1,2+)",
        "CASE WHEN 1+ THEN 2 END",
        "CASE WHEN 1 THEN 2+ END",
        "CASE 1+ WHEN 2 THEN 3 END",
        "CASE WHEN 1 THEN 2 ELSE 3+ END",
        "1 BETWEEN 2+ AND 3",
        "1 BETWEEN 2 AND 3+",
        "1 IN (1+)",
        "1 IN (1,2+)",
        "1 LIKE 2+",
        "1 LIKE 'a' ESCAPE 2+",
        "-2+",
        "~2+",
        "+2+",
        "NOT 2+",
        "(1,2+)",
        "(1+)",
        "1 IS DISTINCT 2",
        "1 IS NOT DISTINCT 2",
        "a.b.2",
        "a.2",
        "1 AND",
        "1 COLLATE 2",
        "1 + ",
        "-",
        "~",
        "+ ",
        "- +",
        "1 IN (1",
        "f(1",
        "(1,2",
        "a.b.+",
        "CAST 1 AS INTEGER",
        "CASE WHEN 1 THEN , END",
        "CASE WHEN 1 THEN 2 ELSE , END",
    ];
    for sql in cases {
        assert!(tree(sql).is_err(), "`{sql}` is not an expression");
    }
}

#[test]
fn a_type_name_reads_as_far_as_the_type_goes() {
    let cases = [
        ("CAST(1 AS)", "(cast 1 )"),
        ("CAST(1 AS KEY)", "(cast 1 KEY)"),
        ("CAST(1 AS INT KEY)", "(cast 1 INT KEY)"),
        ("CAST(1 AS 'text')", "(cast 1 'text')"),
        ("CAST(1 AS INT UNSIGNED BIG)", "(cast 1 INT UNSIGNED BIG)"),
        ("CAST(1 AS DECIMAL(10,5))", "(cast 1 DECIMAL(10,5))"),
        ("CAST(1 AS FLOAT(+1))", "(cast 1 FLOAT(+1))"),
        ("CAST(1 AS FLOAT(-1.5))", "(cast 1 FLOAT(-1.5))"),
    ];
    for (sql, expected) in cases {
        assert_eq!(tree(sql).as_deref(), Ok(expected), "{sql}");
    }
    // And where it does not end.
    for sql in [
        "CAST(1 AS INTEGER",
        "CAST(1 AS VARCHAR(10",
        "CAST(1 AS VARCHAR(a))",
        "CAST(1 AS",
        "CAST(1 AS VARCHAR(10) EXTRA)",
        // A word that cannot be a name ends the type, and then the
        // bracket is missing.
        "CAST(1 AS INT SELECT)",
    ] {
        assert!(tree(sql).is_err(), "`{sql}` is not a cast");
    }
}

#[test]
fn the_arena_says_how_many_nodes_it_holds() {
    use crate::parse::Parser;

    let mut parser = Parser::new(b"1 + 2");
    assert!(parser.arena().is_empty());
    assert_eq!(parser.arena().len(), 0);
    let root = parser.only_expression().unwrap();
    // Two constants and the operator over them.
    assert_eq!(parser.arena().len(), 3);
    assert!(!parser.arena().is_empty());
    assert!(parser.arena().node(root).is_some());
    let arena = parser.into_arena();
    assert_eq!(arena.len(), 3);
}

#[test]
fn a_run_of_children_knows_its_own_length() {
    let (arena, root) = expression(b"f(1,2,3)").unwrap();
    let Some(Node::Call { args, .. }) = arena.node(root) else {
        panic!("a call");
    };
    assert_eq!(args.len(), 3);
    assert!(!args.is_empty());
    assert_eq!(arena.children(args).len(), 3);
    let (arena, root) = expression(b"f()").unwrap();
    let Some(Node::Call { args, .. }) = arena.node(root) else {
        panic!("a call");
    };
    assert!(args.is_empty());
    assert_eq!(args.len(), 0);
    assert!(arena.children(args).is_empty());
}

#[test]
fn a_tree_that_grows_too_tall_is_refused_even_where_the_parser_never_recursed() {
    // A chain of left-associative operators: the parser reads it in a
    // loop and never nests, and the tree it would build is as tall as the
    // chain is long. This is the case `sqlite3ExprCheckHeight` exists for,
    // and the one a fuzz run found here.
    let long = "1".to_owned() + &"+1".repeat(500);
    let error = tree(&long).expect_err("a chain of five hundred additions");
    assert_eq!(error.expected, Expected::Depth);

    // One that fits is read, and its height is what it looks like.
    let short = "1".to_owned() + &"+1".repeat(10);
    let (arena, root) = expression(short.as_bytes()).unwrap();
    assert_eq!(arena.height(root), 11);
    assert_eq!(
        arena.height(arena.node(root).map_or(root, |node| match node {
            Node::Binary { left, .. } => left,
            _ => root,
        })),
        10
    );
}

#[test]
fn the_height_of_a_leaf_is_one_and_of_a_node_one_more_than_its_tallest_child() {
    let (arena, root) = expression(b"1").unwrap();
    assert_eq!(arena.height(root), 1);
    let (arena, root) = expression(b"f(1, 1+1, 1)").unwrap();
    // The tallest argument is two nodes tall, so the call is three.
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"CASE WHEN 1+1 THEN 1 END").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"1 BETWEEN 1+1 AND 1").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"1 IN (1+1)").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"(1, 1+1)").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"1 LIKE 1 ESCAPE 1+1").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"CAST(1+1 AS INT)").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"(1+1) COLLATE nocase").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"-(1+1)").unwrap();
    assert_eq!(arena.height(root), 3);
    let (arena, root) = expression(b"CASE 1+1 WHEN 1 THEN 1 ELSE 1 END").unwrap();
    assert_eq!(arena.height(root), 3);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one case per clause of the grammar, and the point is that none is missing"
)]
fn every_clause_of_a_statement_reads_as_the_clause_it_is() {
    let cases = [
        ("SELECT 1", "(select 1)"),
        ("SELECT 1;", "(select 1)"),
        ("SELECT *", "(select *)"),
        ("SELECT t.* FROM t", "(select t.* (from t))"),
        ("SELECT DISTINCT a", "(select distinct (col a))"),
        ("SELECT ALL a", "(select all (col a))"),
        ("SELECT a AS b", "(select (col a) as b)"),
        ("SELECT a b", "(select (col a) as b)"),
        ("SELECT a, b", "(select (col a) (col b))"),
        (
            "SELECT a FROM t WHERE a=1",
            "(select (col a) (from t) (where (eq (col a) 1)))",
        ),
        (
            "SELECT a FROM t GROUP BY a",
            "(select (col a) (from t) (group (col a)))",
        ),
        (
            "SELECT a FROM t GROUP BY a HAVING b",
            "(select (col a) (from t) (group (col a)) (having (col b)))",
        ),
        ("SELECT a ORDER BY a", "(select (col a) (order (col a)))"),
        (
            "SELECT a ORDER BY a DESC",
            "(select (col a) (order (col a) desc))",
        ),
        (
            "SELECT a ORDER BY a ASC NULLS FIRST",
            "(select (col a) (order (col a) asc nullsfirst))",
        ),
        (
            "SELECT a ORDER BY a NULLS LAST",
            "(select (col a) (order (col a) nullslast))",
        ),
        ("SELECT a LIMIT 1", "(select (col a) (limit 1))"),
        (
            "SELECT a LIMIT 1 OFFSET 2",
            "(select (col a) (limit 1 offset 2))",
        ),
        // `LIMIT a, b` counts b rows after skipping a.
        ("SELECT a LIMIT 1, 2", "(select (col a) (limit 2 offset 1))"),
        (
            "SELECT 1 FROM t1, t2",
            "(select 1 (from t1) (from comma t2))",
        ),
        (
            "SELECT 1 FROM t1 JOIN t2",
            "(select 1 (from t1) (from inner t2))",
        ),
        (
            "SELECT 1 FROM t1 LEFT JOIN t2 ON 1",
            "(select 1 (from t1) (from left t2 on 1))",
        ),
        (
            "SELECT 1 FROM t1 NATURAL LEFT OUTER JOIN t2",
            "(select 1 (from t1) (from left natural t2))",
        ),
        (
            "SELECT 1 FROM t1 CROSS JOIN t2",
            "(select 1 (from t1) (from cross t2))",
        ),
        (
            "SELECT 1 FROM t1 JOIN t2 USING (a, b)",
            "(select 1 (from t1) (from inner t2 using a using b))",
        ),
        ("SELECT 1 FROM main.t AS x", "(select 1 (from main t as x))"),
        (
            "SELECT 1 FROM t INDEXED BY i",
            "(select 1 (from t indexedby i))",
        ),
        (
            "SELECT 1 FROM t NOT INDEXED",
            "(select 1 (from t notindexed))",
        ),
        (
            "SELECT 1 FROM (SELECT 2) AS x",
            "(select 1 (from (select 2) as x))",
        ),
        ("SELECT 1 FROM f(1,2)", "(select 1 (from (call f 1 2)))"),
        ("VALUES (1)", "(select values (row 1))"),
        (
            "VALUES (1,2),(3)",
            "(select values (row 1 2) values (row 3))",
        ),
        ("SELECT 1 UNION SELECT 2", "(select 1 union (select 2))"),
        (
            "SELECT 1 UNION ALL SELECT 2",
            "(select 1 unionall (select 2))",
        ),
        ("SELECT 1 EXCEPT SELECT 2", "(select 1 except (select 2))"),
        (
            "SELECT 1 INTERSECT SELECT 2",
            "(select 1 intersect (select 2))",
        ),
        (
            "SELECT 1 UNION SELECT 2 UNION SELECT 3",
            "(select 1 union (select 2 union (select 3)))",
        ),
        (
            "SELECT 1 UNION SELECT 2 ORDER BY 1 LIMIT 2",
            "(select 1 union (select 2) (order 1) (limit 2))",
        ),
        (
            "WITH c AS (SELECT 1) SELECT 2",
            "(select (with c (select 1)) 2)",
        ),
        (
            "WITH RECURSIVE c(i,j) AS (SELECT 1) SELECT 2",
            "(select recursive (with c i j (select 1)) 2)",
        ),
        (
            "WITH c AS MATERIALIZED (SELECT 1) SELECT 2",
            "(select (with c materialized (select 1)) 2)",
        ),
        (
            "WITH c AS NOT MATERIALIZED (SELECT 1) SELECT 2",
            "(select (with c notmaterialized (select 1)) 2)",
        ),
        (
            "WITH a AS (SELECT 1), b AS (SELECT 2) SELECT 3",
            "(select (with a (select 1)) (with b (select 2)) 3)",
        ),
        ("SELECT (SELECT 1)", "(select (sub (select 1)))"),
        ("SELECT EXISTS (SELECT 1)", "(select (exists (select 1)))"),
        (
            "SELECT NOT EXISTS (SELECT 1)",
            "(select (not (exists (select 1))))",
        ),
        (
            "SELECT 1 WHERE 1 IN (SELECT 2)",
            "(select 1 (where (insel 1 (select 2))))",
        ),
        (
            "SELECT 1 WHERE 1 NOT IN (SELECT 2)",
            "(select 1 (where (notinsel 1 (select 2))))",
        ),
        ("SELECT 1 WHERE 1 IN t", "(select 1 (where (intab 1 t)))"),
        (
            "SELECT 1 WHERE 1 IN main.t",
            "(select 1 (where (intab 1 main t)))",
        ),
        (
            "SELECT 1 WHERE 1 NOT IN t",
            "(select 1 (where (notintab 1 t)))",
        ),
    ];
    for (sql, expected) in cases {
        assert_eq!(parsed(sql).as_deref(), Ok(expected), "{sql}");
    }
}

#[test]
fn what_sqlite_refuses_as_a_statement_this_parser_refuses_too() {
    let corpus: &[u8] = include_bytes!("fixtures/stmt.corpus");
    let golden: &str = include_str!("fixtures/stmt.golden");
    let mut cases: Vec<&[u8]> = corpus.split(|byte| *byte == 0).collect();
    cases.pop();
    let answers: Vec<&str> = golden.lines().collect();
    assert_eq!(cases.len(), answers.len());

    let mut waiting = 0;
    for (case, answer) in cases.iter().zip(&answers) {
        let sql = String::from_utf8_lossy(case).into_owned();
        let read = parsed(&sql);
        // What is not a `SELECT` at all is refused here, and is not a
        // difference: the rest of the language is a later step.
        let ours = sql.trim_start().to_ascii_uppercase();
        let is_select =
            ours.starts_with("SELECT") || ours.starts_with("WITH") || ours.starts_with("VALUES");
        match *answer {
            "accept" if is_select => {
                if read.is_err() {
                    waiting += 1;
                }
            }
            "accept" => {}
            _ => assert!(read.is_err(), "`{sql}` is not SQL and was read as {read:?}"),
        }
    }
    // What the parser cannot read yet is counted rather than listed: the
    // window clauses, and the statements that carry a clause of a later
    // step. The number falls as the steps land, and a rise fails the test.
    // Thirty-six today: the window clauses, and statements carrying a
    // clause of a later step. The number falls as the steps land, and a
    // rise fails the test.
    assert!(
        waiting <= 36,
        "{waiting} statements of the corpus were refused"
    );
}

#[test]
fn a_refusal_inside_a_clause_is_the_refusal_of_the_statement() {
    let cases = [
        // The `WITH` clause, one refusal per place it reads something.
        "WITH 1 AS (SELECT 1) SELECT 1",
        "WITH c(1) AS (SELECT 1) SELECT 1",
        "WITH c(a AS (SELECT 1) SELECT 1",
        "WITH c SELECT 1",
        "WITH c AS NOT (SELECT 1) SELECT 1",
        "WITH c AS SELECT 1",
        "WITH c AS (SELECT) SELECT 1",
        "WITH c AS (SELECT 1 SELECT 1",
        "WITH c AS (SELECT 1), SELECT 1",
        // The result columns and their names.
        "SELECT 1 AS 2",
        "SELECT 1+",
        // `VALUES`.
        "VALUES 1",
        "VALUES (1+)",
        "VALUES (1",
        "VALUES (1),",
        // `FROM`, and everything a table may carry.
        "SELECT 1 FROM 2",
        "SELECT 1 FROM main.+",
        "SELECT 1 FROM f(1+)",
        "SELECT 1 FROM f(1",
        "SELECT 1 FROM (SELECT 1",
        "SELECT 1 FROM t AS 2",
        "SELECT 1 FROM t INDEXED 2",
        "SELECT 1 FROM t INDEXED BY 2",
        "SELECT 1 FROM (SELECT 1) NOT INDEXED",
        "SELECT 1 FROM t1 JOIN t2 ON 1+",
        "SELECT 1 FROM t1 JOIN t2 USING 1",
        "SELECT 1 FROM t1 JOIN t2 USING (1)",
        "SELECT 1 FROM t1 JOIN t2 USING (a",
        "SELECT 1 FROM t LEFT",
        "SELECT 1 FROM t LEFT SELECT",
        // The clauses after it.
        "SELECT 1 WHERE 1+",
        "SELECT 1 GROUP a",
        "SELECT 1 GROUP BY 1+",
        "SELECT 1 GROUP BY a HAVING 1+",
        "SELECT 1 ORDER 1",
        "SELECT 1 ORDER BY 1+",
        "SELECT 1 ORDER BY a NULLS 2",
        "SELECT 1 LIMIT 1+",
        "SELECT 1 LIMIT 1 OFFSET 1+",
        "SELECT 1 LIMIT 1, 1+",
        // The statements inside expressions.
        "SELECT (SELECT 1",
        "SELECT EXISTS 1",
        "SELECT EXISTS (SELECT 1",
        "SELECT 1 WHERE 1 IN (SELECT 1",
        "SELECT 1 WHERE 1 IN main.+",
        "SELECT 1 WHERE 1 IN (SELECT)",
        // And a compound that never gets its second half.
        "SELECT 1 UNION",
        "SELECT 1 UNION ALL",
        "SELECT 1 EXCEPT",
        "SELECT 1 INTERSECT",
    ];
    for sql in cases {
        assert!(parsed(sql).is_err(), "`{sql}` is not a statement");
    }
}

#[test]
fn the_clauses_that_may_repeat_are_read_to_their_end() {
    let cases = [
        (
            "SELECT 1 GROUP BY a, b",
            "(select 1 (group (col a)) (group (col b)))",
        ),
        ("SELECT 1 FROM f()", "(select 1 (from (call f)))"),
        // A word that is not `INDEXED` after `NOT` is where the table
        // ends, and the statement with it.
        ("SELECT 1 FROM t", "(select 1 (from t))"),
    ];
    for (sql, expected) in cases {
        assert_eq!(parsed(sql).as_deref(), Ok(expected), "{sql}");
    }
    assert!(parsed("SELECT 1 FROM t NOT x").is_err());
}

#[test]
fn a_row_of_values_that_is_too_tall_is_refused_where_it_is_built() {
    // An expression of exactly the greatest height there may be, so that
    // it is the row over it that is one too tall.
    let tall = "VALUES (1".to_owned() + &"+1".repeat(199) + ")";
    let error = parsed(&tall).expect_err("a row one taller than the bound");
    assert_eq!(error.expected, Expected::Depth);
    // One shorter, and the row fits.
    let fits = "VALUES (1".to_owned() + &"+1".repeat(198) + ")";
    assert!(parsed(&fits).is_ok());
}

#[test]
fn the_arena_says_how_many_statements_it_holds() {
    let (arena, _) = crate::parse::statement(b"SELECT (SELECT 1) UNION SELECT 2").unwrap();
    // The subquery, and the two halves of the compound.
    assert_eq!(arena.selects(), 3);
    let (arena, _) = crate::parse::statement(b"SELECT 1").unwrap();
    assert_eq!(arena.selects(), 1);
}

#[test]
fn statements_that_nest_deeper_than_the_walk_are_refused() {
    let deep = "SELECT ".to_owned() + &"(SELECT ".repeat(300) + "1" + &")".repeat(300);
    let error = parsed(&deep).expect_err("three hundred nested statements");
    assert_eq!(error.expected, Expected::Depth);
}
