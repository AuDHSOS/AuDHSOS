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
    }
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
        ("1 IN", 4, Expected::OpenParen),
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

    // What the parser cannot read yet: each of them needs the `SELECT`
    // of step Q4's second half, or the window functions after it.
    // `RAISE(...)` is not among them — it reads as a call, which is what
    // it looks like, and only the resolver will care that it is not one.
    let not_yet: [&str; 6] = [
        "EXISTS (SELECT 1)",
        "(SELECT 1)",
        "1 IN (SELECT 1)",
        "1 IN t",
        "count(*) OVER ()",
        "count(*) FILTER (WHERE 1)",
    ];

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
