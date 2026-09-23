// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The `ORDER BY` written inside the brackets of an aggregate, which
//! says which order it reads the rows of its group in.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over a table of four rows.
fn written() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a TEXT, b INT, c INT)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES('x',3,1),('y',1,1),('z',2,2),('w',NULL,2),('v',NULL,3),('u',1,3)")
        .unwrap();
    writer.written()
}

/// What one statement answers, every value of every row with a bar
/// after it.
fn answered(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    let mut out = String::new();
    for row in &database.query(sql).expect("rows").rows {
        for value in row {
            out.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            out.push('|');
        }
    }
    out
}

/// What a statement is refused with.
fn refused(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database.query(sql).unwrap_err().message()
}

/// An aggregate reads the rows of its group in the order the `ORDER BY`
/// inside its brackets says.
#[test]
fn what_order_an_aggregate_reads_its_group_in() {
    let image = written();
    // One term, a term that sorts backwards, and two terms.
    assert_eq!(
        answered(&image, b"SELECT group_concat(a ORDER BY b) FROM t"),
        "w,v,y,u,z,x|"
    );
    assert_eq!(
        answered(&image, b"SELECT group_concat(a ORDER BY b DESC) FROM t"),
        "x,z,y,u,w,v|"
    );
    assert_eq!(
        answered(&image, b"SELECT group_concat(a ORDER BY c, b) FROM t"),
        "y,x,w,z,v,u|"
    );
    // `NULLS FIRST` and `NULLS LAST` say which end the nulls of a term
    // go to, whichever way the sort runs.
    assert_eq!(
        answered(
            &image,
            b"SELECT group_concat(a ORDER BY b NULLS LAST) FROM t"
        ),
        "y,u,z,x,w,v|"
    );
    assert_eq!(
        answered(
            &image,
            b"SELECT group_concat(a ORDER BY b DESC NULLS FIRST) FROM t"
        ),
        "w,v,x,z,y,u|"
    );
    // One order per group, and a `DISTINCT` that reads the arguments and
    // not the terms.
    assert_eq!(
        answered(
            &image,
            b"SELECT group_concat(a ORDER BY b) FROM t GROUP BY c ORDER BY c"
        ),
        "y,x|w,z|v,u|"
    );
    assert_eq!(
        answered(&image, b"SELECT group_concat(DISTINCT a ORDER BY a) FROM t"),
        "u,v,w,x,y,z|"
    );
    // The row a `max` took is the one the bare columns of the group come
    // from, whether the rows were read in an order of their own or not.
    assert_eq!(
        answered(&image, b"SELECT c, max(b ORDER BY b) FROM t"),
        "1|3|"
    );
    // An aggregate that reads no argument steps once per row all the
    // same.
    assert_eq!(answered(&image, b"SELECT count(ORDER BY b) FROM t"), "6|");
    // An `ORDER BY` inside the brackets of a call that is no aggregate
    // is a misuse of one, and an aggregate inside the terms is a misuse
    // of that.
    assert_eq!(
        refused(&image, b"SELECT abs(b ORDER BY c) FROM t"),
        "ORDER BY may not be used with non-aggregate abs()"
    );
    assert_eq!(
        refused(&image, b"SELECT group_concat(a ORDER BY max(b)) FROM t"),
        "misuse of aggregate function max()"
    );
}
