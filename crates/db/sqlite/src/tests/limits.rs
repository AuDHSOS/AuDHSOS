// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The limits a connection holds, which `sqlite3_limit` reads and sets.

use crate::change::Writer;
use crate::db::{Limit, Limits};
use crate::header::Encoding;

/// Every limit, with the number it carries and the largest value the
/// build takes for it.
const HELD: [(i64, Limit, i64); 13] = [
    (0, Limit::Length, 1_000_000_000),
    (1, Limit::SqlLength, 1_000_000_000),
    (2, Limit::Column, 2000),
    (3, Limit::ExprDepth, 200),
    (4, Limit::CompoundSelect, 500),
    (5, Limit::VdbeOp, 250_000_000),
    (6, Limit::FunctionArg, 1000),
    (7, Limit::AttachedDatabases, 10),
    (8, Limit::LikePattern, 50_000),
    (9, Limit::VariableNumber, 32766),
    (10, Limit::TriggerDepth, 1000),
    (11, Limit::WorkerThreads, 0),
    (12, Limit::ParserDepth, 1000),
];

/// The number each limit carries and the value a connection opens with.
#[test]
fn what_limits_a_connection_opens_with() {
    let limits = Limits::default();
    for (number, limit, hard) in HELD {
        assert_eq!(Limit::of_number(number), Some(limit), "{number}");
        assert_eq!(limit.place(), usize::try_from(number).unwrap(), "{number}");
        assert_eq!(limit.hard(), hard, "{number}");
        assert_eq!(limits.of(limit), hard, "{number}");
    }
    // A number no limit carries, which `sqlite3_limit` answers minus one
    // for.
    assert_eq!(Limit::of_number(13), None);
    assert_eq!(Limit::of_number(-1), None);
    // The length of a value is the one limit with a smallest value of
    // its own.
    assert_eq!(Limit::Length.least(), 30);
    assert_eq!(Limit::Column.least(), 0);
}

/// What setting a limit answers and leaves.
#[test]
fn what_setting_a_limit_answers() {
    let mut limits = Limits::new();
    // A value below nought reads the limit and leaves it where it
    // stands.
    assert_eq!(limits.set(Limit::Column, -1), 2000);
    assert_eq!(limits.of(Limit::Column), 2000);
    // A value the build takes is held, and the answer is what the limit
    // was.
    assert_eq!(limits.set(Limit::Column, 100), 2000);
    assert_eq!(limits.set(Limit::Column, 50), 100);
    assert_eq!(limits.of(Limit::Column), 50);
    // A value above the hard limit of the build is held to that.
    assert_eq!(limits.set(Limit::Column, 4000), 50);
    assert_eq!(limits.of(Limit::Column), 2000);
    // A length below thirty is held to thirty.
    assert_eq!(limits.set(Limit::Length, 1), 1_000_000_000);
    assert_eq!(limits.of(Limit::Length), 30);
    // Nought is the hard limit of the threads a sort takes, so every
    // value it is set to is that.
    assert_eq!(limits.set(Limit::WorkerThreads, 99999), 0);
    assert_eq!(limits.of(Limit::WorkerThreads), 0);
}

/// An `ATTACH` past the databases the limit holds the connection to is
/// refused, and names that count.
#[test]
fn what_an_attach_is_held_to() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let mut limits = Limits::new();
    limits.set(Limit::AttachedDatabases, 2);
    writer.limited(limits);
    writer.run(b"ATTACH ':memory:' AS one").unwrap();
    writer.run(b"ATTACH ':memory:' AS two").unwrap();
    assert_eq!(
        writer
            .run(b"ATTACH ':memory:' AS three")
            .expect_err("a refusal")
            .message(),
        "too many attached databases - max 2"
    );
    // The hard limit of the build is what a connection that was told no
    // limit is held to.
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for at in 0..10 {
        let sql = alloc::format!("ATTACH ':memory:' AS held{at}");
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        writer
            .run(b"ATTACH ':memory:' AS past")
            .expect_err("a refusal")
            .message(),
        "too many attached databases - max 10"
    );
}
