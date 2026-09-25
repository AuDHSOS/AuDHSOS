// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::window` and of the window functions the engine
//! answers, against what the C library answers for the same statements
//! over the same rows.

use alloc::string::String;

use crate::change::Writer;
use crate::db::{Answer, Database};
use crate::header::Encoding;
use crate::value::Value;
use crate::window::{Edge, Peers, Span, groups_frame, rows_frame};

/// A table of nine rows, three groups of three under `c`, with one row
/// whose `c` is nothing.
fn rows() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER PRIMARY KEY, b, c)")
        .unwrap();
    writer
        .run(
            b"INSERT INTO t1 VALUES(1,'A','one'),(2,'B','two'),(3,'C','three'),\
              (4,'D','one'),(5,'E','two'),(6,'F','three'),(7,'G','one'),\
              (8,'H',NULL),(9,'I','two')",
        )
        .unwrap();
    writer.written()
}

/// What a statement answers, every value of every row as one list.
fn answered(bytes: &[u8], sql: &str) -> Result<alloc::vec::Vec<String>, crate::db::Error> {
    let database = Database::open(bytes)?;
    let Answer { rows, .. } = database.query(sql.as_bytes())?;
    let mut out = alloc::vec::Vec::new();
    for row in &rows {
        for value in row {
            out.push(match value {
                Value::Null => String::new(),
                Value::Int(number) => alloc::format!("{number}"),
                Value::Real(number) => {
                    String::from_utf8_lossy(&crate::fp::text(*number, 17)).into_owned()
                }
                Value::Text(bytes) | Value::Blob(bytes) => {
                    String::from_utf8_lossy(bytes).into_owned()
                }
            });
        }
    }
    Ok(out)
}

#[test]
fn a_frame_counted_in_rows_reaches_the_rows_the_bounds_name() {
    // Five rows, the walk standing on the third.
    let span = rows_frame(Edge::Start, Edge::Current, 2, 5);
    assert_eq!(span, Span::of(0, 3));
    assert_eq!(
        rows_frame(Edge::Preceding(1), Edge::Following(1), 2, 5),
        Span::of(1, 4)
    );
    assert_eq!(rows_frame(Edge::Start, Edge::End, 2, 5), Span::of(0, 5));
    assert_eq!(
        rows_frame(Edge::Following(1), Edge::End, 2, 5),
        Span::of(3, 5)
    );
    assert_eq!(rows_frame(Edge::End, Edge::End, 2, 5), Span::of(5, 5));
    // A bound that counts further than the rows reach frames none.
    assert_eq!(
        rows_frame(Edge::Start, Edge::Preceding(9), 2, 5),
        Span::of(0, 0)
    );
    assert_eq!(
        rows_frame(Edge::Preceding(1), Edge::Preceding(1), 2, 5),
        Span::of(1, 2)
    );
    assert_eq!(rows_frame(Edge::Start, Edge::Start, 2, 5), Span::of(0, 0));
    assert_eq!(
        rows_frame(Edge::Following(9), Edge::End, 2, 5),
        Span::of(5, 5)
    );
    assert_eq!(rows_frame(Edge::Current, Edge::End, 2, 5), Span::of(2, 5));
    // A frame that ends before it begins holds no row.
    assert_eq!(
        rows_frame(Edge::Current, Edge::Preceding(2), 2, 5),
        Span::of(2, 2)
    );
}

#[test]
fn a_frame_counted_in_groups_reaches_the_groups_the_bounds_name() {
    // Six rows in three groups of two, the walk standing on the third,
    // which is the first row of the second group.
    let peers = Peers::new(6, |at| at % 2 == 0);
    assert_eq!(peers.rows(), 6);
    assert_eq!(peers.of_row(2), Span::of(2, 4));
    assert_eq!(
        groups_frame(Edge::Start, Edge::Current, 2, &peers),
        Span::of(0, 4)
    );
    assert_eq!(
        groups_frame(Edge::Preceding(1), Edge::Following(1), 2, &peers),
        Span::of(0, 6)
    );
    assert_eq!(
        groups_frame(Edge::Start, Edge::End, 2, &peers),
        Span::of(0, 6)
    );
    assert_eq!(
        groups_frame(Edge::End, Edge::End, 2, &peers),
        Span::of(6, 6)
    );
    assert_eq!(
        groups_frame(Edge::Start, Edge::Start, 2, &peers),
        Span::of(0, 0)
    );
    assert_eq!(
        groups_frame(Edge::Start, Edge::Preceding(9), 2, &peers),
        Span::of(0, 0)
    );
    assert_eq!(
        groups_frame(Edge::Following(1), Edge::Following(9), 2, &peers),
        Span::of(4, 6)
    );
    assert_eq!(
        groups_frame(Edge::Preceding(9), Edge::Preceding(1), 2, &peers),
        Span::of(0, 2)
    );
    assert_eq!(
        groups_frame(Edge::Current, Edge::End, 2, &peers),
        Span::of(2, 6)
    );
}

#[test]
fn the_eleven_built_in_window_functions_answer_what_the_c_library_answers() {
    let bytes = rows();
    for (sql, want) in [
        (
            "SELECT row_number() OVER (ORDER BY a) FROM t1",
            "1|2|3|4|5|6|7|8|9",
        ),
        (
            "SELECT rank() OVER (ORDER BY c), dense_rank() OVER (ORDER BY c) FROM t1",
            "1|1|2|2|2|2|2|2|5|3|5|3|7|4|7|4|7|4",
        ),
        (
            "SELECT percent_rank() OVER (ORDER BY c) FROM t1",
            "0.0|0.125|0.125|0.125|0.5|0.5|0.75|0.75|0.75",
        ),
        (
            "SELECT cume_dist() OVER (ORDER BY c) FROM t1",
            "0.1111111111111111|0.44444444444444442|0.44444444444444442|0.44444444444444442|0.66666666666666663|0.66666666666666663|1.0|1.0|1.0",
        ),
        (
            "SELECT ntile(4) OVER (ORDER BY a) FROM t1",
            "1|1|1|2|2|3|3|4|4",
        ),
        (
            "SELECT lag(a,2,-1) OVER (ORDER BY a), lead(a,2) OVER (ORDER BY a) FROM t1",
            "-1|3|-1|4|1|5|2|6|3|7|4|8|5|9|6||7|",
        ),
        (
            "SELECT first_value(a) OVER w, last_value(a) OVER w, nth_value(a,3) OVER w \
             FROM t1 WINDOW w AS (ORDER BY a ROWS BETWEEN UNBOUNDED PRECEDING \
             AND UNBOUNDED FOLLOWING)",
            "1|9|3|1|9|3|1|9|3|1|9|3|1|9|3|1|9|3|1|9|3|1|9|3|1|9|3",
        ),
    ] {
        let read = answered(&bytes, sql).unwrap();
        assert_eq!(
            read,
            want.split('|').collect::<alloc::vec::Vec<_>>(),
            "{sql}"
        );
    }
}

#[test]
fn an_aggregate_over_a_frame_reads_the_rows_the_frame_holds() {
    let bytes = rows();
    for (sql, want) in [
        (
            "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING) FROM t1",
            "3|6|9|12|15|18|21|24|17",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c RANGE BETWEEN 2 PRECEDING AND CURRENT ROW) FROM t1",
            "8|12|12|12|9|9|16|16|16",
        ),
        (
            "SELECT group_concat(a) OVER (PARTITION BY c ORDER BY a \
             GROUPS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t1",
            "8|1|1,4|4,7|3|3,6|2|2,5|5,9",
        ),
        (
            "SELECT count(*) FILTER (WHERE a%2=0) OVER (ORDER BY a) FROM t1",
            "0|1|1|2|2|3|3|4|4",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN UNBOUNDED PRECEDING \
             AND UNBOUNDED FOLLOWING EXCLUDE CURRENT ROW) FROM t1",
            "44|43|42|41|40|39|38|37|36",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c ROWS BETWEEN UNBOUNDED PRECEDING \
             AND UNBOUNDED FOLLOWING EXCLUDE GROUP) FROM t1",
            "37|33|33|33|36|36|29|29|29",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c ROWS BETWEEN UNBOUNDED PRECEDING \
             AND UNBOUNDED FOLLOWING EXCLUDE TIES) FROM t1",
            "45|34|37|40|39|42|31|34|38",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW \
             AND UNBOUNDED FOLLOWING) FROM t1",
            "45|44|42|39|35|30|24|17|9",
        ),
        (
            "SELECT sum(a) OVER (PARTITION BY c, a ORDER BY a \
             GROUPS BETWEEN CURRENT ROW AND UNBOUNDED FOLLOWING) FROM t1",
            "8|1|4|7|3|6|2|5|9",
        ),
        (
            "SELECT percent_rank() OVER (PARTITION BY a ORDER BY a) FROM t1",
            "0.0|0.0|0.0|0.0|0.0|0.0|0.0|0.0|0.0",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN 1 PRECEDING AND CURRENT ROW \
             EXCLUDE NO OTHERS) FROM t1",
            "1|3|5|7|9|11|13|15|17",
        ),
        (
            "SELECT sum(a) OVER w1, sum(a) OVER w2 FROM t1 \
             WINDOW w1 AS (ORDER BY a), w2 AS (ORDER BY a DESC)",
            "1|45|3|44|6|42|10|39|15|35|21|30|28|24|36|17|45|9",
        ),
        (
            "SELECT sum(a) OVER (), count(*) OVER () FROM t1",
            "45|9|45|9|45|9|45|9|45|9|45|9|45|9|45|9|45|9",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a) FROM t1 \
             WHERE a NOT IN (SELECT a FROM t1 WHERE a<3)",
            "3|7|12|18|25|33|42",
        ),
        (
            "SELECT c, sum(a) OVER () FROM t1 GROUP BY c",
            "|14|one|14|three|14|two|14",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c NULLS LAST \
             RANGE BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) FROM t1",
            "12|12|12|21|21|37|37|37|45",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c NULLS LAST \
             RANGE BETWEEN 1 FOLLOWING AND 2 FOLLOWING) FROM t1",
            "12|12|12|9|9|16|16|16|8",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN CURRENT ROW AND 1 FOLLOWING) FROM t1",
            "3|5|7|9|11|13|15|17|9",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a \
             RANGE BETWEEN 1 PRECEDING AND UNBOUNDED FOLLOWING) FROM t1",
            "45|45|44|42|39|35|30|24|17",
        ),
    ] {
        let read = answered(&bytes, sql).unwrap();
        assert_eq!(
            read,
            want.split('|').collect::<alloc::vec::Vec<_>>(),
            "{sql}"
        );
    }
}

#[test]
fn a_window_reads_the_rows_the_statement_leaves_it() {
    let bytes = rows();
    for (sql, want) in [
        (
            "SELECT c, count(*), sum(count(*)) OVER (ORDER BY c) FROM t1 GROUP BY c",
            "|1|1|one|3|4|three|2|6|two|3|9",
        ),
        ("SELECT count(a) FILTER (WHERE a>3) FROM t1", "6"),
        (
            "SELECT a, sum(a) OVER (ORDER BY c NULLS LAST) FROM t1 ORDER BY 1 NULLS LAST",
            "1|12|2|37|3|21|4|12|5|37|6|21|7|12|8|45|9|37",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY c NULLS LAST \
             RANGE BETWEEN 1 PRECEDING AND 1 FOLLOWING) FROM t1",
            "12|12|12|9|9|16|16|16|8",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a NULLS LAST \
             RANGE BETWEEN 2 PRECEDING AND 1 PRECEDING) FROM t1",
            "|1|3|5|7|9|11|13|15",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a DESC NULLS FIRST \
             RANGE BETWEEN 1 FOLLOWING AND 2 FOLLOWING) FROM t1",
            "15|13|11|9|7|5|3|1|",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN 1 FOLLOWING AND 2 FOLLOWING) FROM t1",
            "5|7|9|11|13|15|17|9|",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN 2 PRECEDING AND 1 PRECEDING) FROM t1",
            "|1|3|5|7|9|11|13|15",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN 1.5 PRECEDING AND CURRENT ROW) FROM t1",
            "1|3|5|7|9|11|13|15|17",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1.0 PRECEDING AND CURRENT ROW) FROM t1",
            "1|3|5|7|9|11|13|15|17",
        ),
        (
            "SELECT lag(a) OVER (ORDER BY a), lead(a) OVER (ORDER BY a) FROM t1",
            "|2|1|3|2|4|3|5|4|6|5|7|6|8|7|9|8|",
        ),
        (
            "SELECT first_value(a) OVER (ORDER BY a \
             ROWS BETWEEN 3 PRECEDING AND 2 PRECEDING) FROM t1",
            "||1|1|2|3|4|5|6",
        ),
        (
            "SELECT group_concat(b,'.') OVER (ORDER BY a \
             ROWS BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t1",
            "A|A.B|B.C|C.D|D.E|E.F|F.G|G.H|H.I",
        ),
        (
            "SELECT sum(a) OVER (ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM t1",
            "1|3|6|10|15|21|28|36|45",
        ),
        (
            "SELECT sum(a) OVER (RANGE BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM t1",
            "45|45|45|45|45|45|45|45|45",
        ),
        (
            "SELECT sum(a) OVER (GROUPS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) FROM t1",
            "45|45|45|45|45|45|45|45|45",
        ),
        (
            "SELECT sum(a) OVER (PARTITION BY c, a%2 ORDER BY a) FROM t1",
            "8|4|1|8|6|3|2|5|14",
        ),
        (
            "SELECT sum(a) OVER (ORDER BY a DESC RANGE BETWEEN 2 PRECEDING \
             AND 1 FOLLOWING) FROM t1",
            "17|24|30|26|22|18|14|10|6",
        ),
    ] {
        let read = answered(&bytes, sql).unwrap();
        assert_eq!(
            read,
            want.split('|').collect::<alloc::vec::Vec<_>>(),
            "{sql}"
        );
    }
}

#[test]
fn a_window_that_names_another_takes_what_that_window_says() {
    let bytes = rows();
    for (sql, want) in [
        (
            "SELECT sum(a) OVER w FROM t1 WINDOW w AS (PARTITION BY c ORDER BY a)",
            "8|1|5|12|3|9|2|7|16",
        ),
        (
            "SELECT sum(a) OVER (w ORDER BY a) FROM t1 WINDOW w AS (PARTITION BY c)",
            "8|1|5|12|3|9|2|7|16",
        ),
        (
            "SELECT sum(a) OVER (w) FROM t1 WINDOW w AS (PARTITION BY c)",
            "8|12|12|12|9|9|16|16|16",
        ),
    ] {
        let read = answered(&bytes, sql).unwrap();
        assert_eq!(
            read,
            want.split('|').collect::<alloc::vec::Vec<_>>(),
            "{sql}"
        );
    }
}

#[test]
fn what_a_window_refuses() {
    let bytes = rows();
    for sql in [
        // `DISTINCT` is not answered over a window.
        "SELECT sum(DISTINCT a) OVER (ORDER BY a) FROM t1",
        // A `FILTER` belongs to an aggregate and to nothing else.
        "SELECT row_number() FILTER (WHERE a>1) OVER (ORDER BY a) FROM t1",
        // A function no table holds, and one of the eleven called
        // with more arguments than it takes.
        "SELECT nosuch() OVER () FROM t1",
        "SELECT row_number(a) OVER (ORDER BY a) FROM t1",
        "SELECT lag() OVER (ORDER BY a) FROM t1",
        // A window that names itself.
        "SELECT sum(a) OVER (w) FROM t1 WINDOW w AS (w)",
        // A frame offset that is a real with a fraction, and one that
        // is nothing at all.
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1.5 PRECEDING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN NULL PRECEDING AND CURRENT ROW) FROM t1",
        // A window no `WINDOW` clause defines.
        "SELECT sum(a) OVER nosuch FROM t1",
        // A window that overrides what the window it names says.
        "SELECT sum(a) OVER (w PARTITION BY a) FROM t1 WINDOW w AS (PARTITION BY c)",
        "SELECT sum(a) OVER (w ORDER BY a) FROM t1 WINDOW w AS (ORDER BY c)",
        "SELECT sum(a) OVER (w ROWS CURRENT ROW) FROM t1 \
         WINDOW w AS (ORDER BY c ROWS CURRENT ROW)",
        // A frame offset that is not a whole number of no sign.
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 'x' PRECEDING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN -1 PRECEDING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN 'x' PRECEDING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN -1.5 PRECEDING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a RANGE BETWEEN -1 PRECEDING AND CURRENT ROW) FROM t1",
        // A refusal under one window stops the walk before the next.
        "SELECT (nosuch() OVER ()) + (nosuch() OVER ()) FROM t1",
        // A `RANGE` that counts by an offset over other than one term.
        "SELECT sum(a) OVER (ORDER BY a, c RANGE BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t1",
        // A count that is not one or more.
        "SELECT ntile(0) OVER (ORDER BY a) FROM t1",
        "SELECT nth_value(a,0) OVER (ORDER BY a) FROM t1",
        // A window function written where no window is worked out.
        "SELECT a FROM t1 WHERE sum(a) OVER ()",
        // A refusal while the rows a window reads are gathered.
        "SELECT sum(a) OVER (ORDER BY a) FROM t1 WHERE nosuch(a)",
        // A scalar function carrying a `FILTER`.
        "SELECT abs(a) FILTER (WHERE a>1) FROM t1",
    ] {
        assert!(answered(&bytes, sql).is_err(), "{sql}");
    }
}

#[test]
fn what_the_parser_refuses_of_a_frame() {
    for sql in [
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 PRECEDING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1 FOLLOWING AND 1 PRECEDING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1 FOLLOWING AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS 1 FOLLOWING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN UNBOUNDED FOLLOWING \
         AND UNBOUNDED FOLLOWING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN UNBOUNDED PRECEDING \
         AND UNBOUNDED PRECEDING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING \
         EXCLUDE NOTHING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN 1 NEITHER AND CURRENT ROW) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN UNBOUNDED NEITHER \
         AND UNBOUNDED FOLLOWING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN CURRENT NOTAROW \
         AND UNBOUNDED FOLLOWING) FROM t1",
        "SELECT sum(a) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND CURRENT NOTAROW) FROM t1",
        "SELECT sum(a) FILTER (a>1) OVER () FROM t1",
        "SELECT sum(a) OVER (ORDER BY a EXCLUDE TIES) FROM t1",
    ] {
        assert!(crate::parse::statement(sql.as_bytes()).is_err(), "{sql}");
    }
}

#[test]
fn the_three_words_of_a_window_are_names_where_no_window_stands() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    // `WINDOW`, `OVER` and `FILTER` are names outside a window, which
    // is what `analyzeWindowKeyword` and its two neighbours decide.
    writer.run(b"CREATE TABLE window(over, filter)").unwrap();
    writer.run(b"INSERT INTO window VALUES(1,2)").unwrap();
    let bytes = writer.written();
    assert_eq!(
        answered(&bytes, "SELECT over, filter FROM window").unwrap(),
        ["1", "2"]
    );
    assert_eq!(
        answered(&bytes, "SELECT over FROM window AS over").unwrap(),
        ["1"]
    );
    // A name that is a keyword after `WINDOW` ends the clause rather
    // than naming a window, so the word is the alias of the answer.
    assert_eq!(
        answered(&bytes, "SELECT over window FROM window").unwrap(),
        ["1"]
    );
    // `WINDOW` names a window only where `AS` follows the name after
    // it, so a column may be called `window` and carry a type.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t2(window x, b)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(3,4)").unwrap();
    let bytes = writer.written();
    assert_eq!(answered(&bytes, "SELECT window FROM t2").unwrap(), ["3"]);
    // `FILTER` follows the bracket of a call and is followed by one, so
    // the word alone after a call is the alias of the answer.
    assert_eq!(
        answered(&bytes, "SELECT count(*) filter FROM t2").unwrap(),
        ["1"]
    );
}

/// A `RANGE` frame that begins backwards holds the row it stands on
/// whatever the offset does to the term, which is the comparison
/// `windowCodeRangeTest` makes before it moves one term by the offset.
#[test]
fn what_a_range_frame_holds_where_the_offset_rounds_the_term() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a INTEGER)").unwrap();
    // Two integers a double holds no exact value of, so a term moved by
    // a real offset rounds away from the term itself.
    writer
        .run(b"INSERT INTO t1 VALUES(3578824042033200656),(3029012920382354029)")
        .unwrap();
    let bytes = writer.written();
    for frame in [
        "ORDER BY a RANGE BETWEEN 0.3 PRECEDING AND 10 FOLLOWING",
        "ORDER BY a RANGE BETWEEN 0.3 PRECEDING AND 0.1 PRECEDING",
        "ORDER BY a RANGE BETWEEN 0.3 FOLLOWING AND 10 FOLLOWING",
        "ORDER BY a DESC RANGE BETWEEN 0.3 PRECEDING AND 10 FOLLOWING",
        "ORDER BY a NULLS LAST RANGE BETWEEN 0.3 PRECEDING AND 10 FOLLOWING",
        "ORDER BY a RANGE BETWEEN 1.0 PRECEDING AND 2.0 PRECEDING",
    ] {
        let sql = alloc::format!("SELECT total(a) OVER ({frame}) FROM t1 ORDER BY a");
        assert_eq!(
            answered(&bytes, &sql).unwrap(),
            ["3.0290129203823539e+18", "3.5788240420332006e+18"],
            "{sql}"
        );
    }
    // A frame that counts backwards over a term that sorts backwards
    // subtracts the offset, which rounds away from the term and holds
    // no row.
    assert_eq!(
        answered(
            &bytes,
            "SELECT total(a) OVER (ORDER BY a DESC \
             RANGE BETWEEN 0.3 PRECEDING AND 0.1 PRECEDING) FROM t1 ORDER BY a"
        )
        .unwrap(),
        ["0.0", "0.0"]
    );
    // A text is moved by no offset, so the two terms are compared as
    // they stand.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t2(a)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES('x'),('y')").unwrap();
    let bytes = writer.written();
    assert_eq!(
        answered(
            &bytes,
            "SELECT count(*) OVER (ORDER BY a \
             RANGE BETWEEN 1 PRECEDING AND CURRENT ROW) FROM t2 ORDER BY a"
        )
        .unwrap(),
        ["1", "1"]
    );
}
