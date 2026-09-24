// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::Error;

#[test]
fn every_refusal_says_which_rule_of_the_format_was_broken() {
    let cases = [
        (Error::Magic, "not a SQLite database"),
        (Error::Truncated, "hundred-byte header"),
        (Error::PageSize(300), "page size 300"),
        (Error::Reserved(200), "200 reserved bytes"),
        (Error::Fractions, "64, 32 and 32"),
        (Error::Encoding(9), "text encoding 9"),
        (Error::Page(7), "page 7"),
        (Error::PageKind(3), "3 is not a b-tree page type"),
        (Error::Overrun, "reaches past its page"),
        (Error::Varint, "varint runs past"),
        (Error::SerialType(10), "serial type 10"),
        (Error::Depth, "deeper than this crate walks"),
        (Error::Overflow(5), "overflow chain at page 5"),
        (Error::FreeBlock, "free space of a page"),
        (Error::Balance, "balance this crate does not write"),
        (Error::Full, "database or disk is full"),
    ];
    for (error, expected) in cases {
        let said = format!("{error}");
        assert!(said.contains(expected), "{error:?} said `{said}`");
    }
}

#[test]
fn a_refusal_is_compared_by_what_it_refuses() {
    assert_eq!(Error::Page(1), Error::Page(1));
    assert_ne!(Error::Page(1), Error::Page(2));
    assert_ne!(Error::Overrun, Error::Varint);
    // Copied rather than moved, so a refusal can be answered twice.
    let error = Error::Magic;
    let copy = error;
    assert_eq!(format!("{error:?}"), format!("{copy:?}"));
}

/// The result code and the message every refusal of a file carries,
/// which are the ones `sqlite3ErrStr` and `sqlite3ErrName` write.
#[test]
fn what_code_and_message_a_refusal_of_a_file_carries() {
    use crate::db::Error as Refused;
    for (error, code, name, message) in [
        (Error::Magic, 26, "SQLITE_NOTADB", "file is not a database"),
        (
            Error::Truncated,
            26,
            "SQLITE_NOTADB",
            "file is not a database",
        ),
        (
            Error::PageSize(300),
            26,
            "SQLITE_NOTADB",
            "file is not a database",
        ),
        (
            Error::Reserved(200),
            26,
            "SQLITE_NOTADB",
            "file is not a database",
        ),
        (
            Error::Fractions,
            26,
            "SQLITE_NOTADB",
            "file is not a database",
        ),
        (
            Error::Encoding(4),
            26,
            "SQLITE_NOTADB",
            "file is not a database",
        ),
        (
            Error::Overrun,
            11,
            "SQLITE_CORRUPT",
            "database disk image is malformed",
        ),
        (
            Error::Varint,
            11,
            "SQLITE_CORRUPT",
            "database disk image is malformed",
        ),
        (Error::Full, 13, "SQLITE_FULL", "database or disk is full"),
    ] {
        let refused = Refused::Image(error);
        let held = refused.code();
        assert_eq!(held.number, code);
        assert_eq!(held.name, name.as_bytes());
        assert_eq!(held.extended, code);
        assert_eq!(held.extended_name, name.as_bytes());
        assert_eq!(refused.message(), message);
    }
}

/// The result code every other refusal carries, which is
/// `SQLITE_CONSTRAINT` with the constraint the row broke for the rows a
/// statement could not write and `SQLITE_ERROR` for the statements the
/// engine could not read.
#[test]
fn what_code_a_refusal_of_a_statement_carries() {
    use crate::db::Error as Refused;
    for (refused, number, name, extended, extended_name) in [
        (
            Refused::Unique(b"t.a".to_vec()),
            19,
            "SQLITE_CONSTRAINT",
            2067,
            "SQLITE_CONSTRAINT_UNIQUE",
        ),
        (
            Refused::NotNull(b"t.a".to_vec()),
            19,
            "SQLITE_CONSTRAINT",
            1299,
            "SQLITE_CONSTRAINT_NOTNULL",
        ),
        (
            Refused::Check(b"t".to_vec()),
            19,
            "SQLITE_CONSTRAINT",
            275,
            "SQLITE_CONSTRAINT_CHECK",
        ),
        (
            Refused::Foreign,
            19,
            "SQLITE_CONSTRAINT",
            787,
            "SQLITE_CONSTRAINT_FOREIGNKEY",
        ),
        (
            Refused::ForeignMismatch(b"a".to_vec(), b"b".to_vec()),
            19,
            "SQLITE_CONSTRAINT",
            787,
            "SQLITE_CONSTRAINT_FOREIGNKEY",
        ),
        (
            Refused::StoredType(
                b"t".to_vec(),
                b"a".to_vec(),
                b"INT".to_vec(),
                b"text".to_vec(),
            ),
            19,
            "SQLITE_CONSTRAINT",
            3091,
            "SQLITE_CONSTRAINT_DATATYPE",
        ),
        (
            Refused::Constraint,
            19,
            "SQLITE_CONSTRAINT",
            19,
            "SQLITE_CONSTRAINT",
        ),
        (
            Refused::HeldConstraint(b"t".to_vec()),
            19,
            "SQLITE_CONSTRAINT",
            19,
            "SQLITE_CONSTRAINT",
        ),
        (
            Refused::Auth(crate::auth::Error::Denied),
            23,
            "SQLITE_AUTH",
            23,
            "SQLITE_AUTH",
        ),
        (
            Refused::Mismatch,
            20,
            "SQLITE_MISMATCH",
            20,
            "SQLITE_MISMATCH",
        ),
        (
            Refused::ReadOnlyDatabase,
            8,
            "SQLITE_READONLY",
            8,
            "SQLITE_READONLY",
        ),
        (Refused::Incomplete, 1, "SQLITE_ERROR", 1, "SQLITE_ERROR"),
    ] {
        let held = refused.code();
        assert_eq!(held.number, number, "{refused:?}");
        assert_eq!(held.name, name.as_bytes(), "{refused:?}");
        assert_eq!(held.extended, extended, "{refused:?}");
        assert_eq!(held.extended_name, extended_name.as_bytes(), "{refused:?}");
    }
}

/// The words each result code names, which `sqlite3_errstr` answers.
#[test]
fn what_words_a_result_code_names() {
    for (code, words) in [
        (0, "not an error"),
        (1, "SQL logic error"),
        (3, "access permission denied"),
        (4, "query aborted"),
        (5, "database is locked"),
        (6, "database table is locked"),
        (7, "out of memory"),
        (8, "attempt to write a readonly database"),
        (9, "interrupted"),
        (10, "disk I/O error"),
        (11, "database disk image is malformed"),
        (12, "unknown operation"),
        (13, "database or disk is full"),
        (14, "unable to open database file"),
        (15, "locking protocol"),
        (17, "database schema has changed"),
        (18, "string or blob too big"),
        (19, "constraint failed"),
        (20, "datatype mismatch"),
        (21, "bad parameter or other API misuse"),
        (23, "authorization denied"),
        (25, "column index out of range"),
        (26, "file is not a database"),
        (27, "notification message"),
        (28, "warning message"),
        // The three codes that name words of their own.
        (516, "abort due to ROLLBACK"),
        (100, "another row available"),
        (101, "no more rows available"),
        // An extended code names the words of the primary code in its
        // lowest byte.
        (2067, "constraint failed"),
        (266, "disk I/O error"),
        // The codes the table holds no words for, and one past every
        // code the library holds.
        (2, "unknown error"),
        (16, "unknown error"),
        (22, "unknown error"),
        (24, "unknown error"),
        (200, "unknown error"),
    ] {
        assert_eq!(crate::db::errstr(code), words, "{code}");
    }
}
