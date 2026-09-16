// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the date and time functions, against what the C library
//! answers for the same calls.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// What `SELECT quote(EXPR)` answers, which is one text per shape of
/// value.
fn quoted(sql: &str) -> String {
    let writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let statement = alloc::format!("SELECT quote({sql})");
    let answered = database.query(statement.as_bytes()).unwrap();
    let value = answered
        .rows
        .first()
        .and_then(|row| row.first())
        .cloned()
        .unwrap_or(Value::Null);
    match value {
        Value::Null => String::from("NULL"),
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Value::Int(number) => alloc::format!("{number}"),
        Value::Real(number) => String::from_utf8_lossy(&crate::fp::text(number, 15)).into_owned(),
    }
}

/// Every call, against what the C library answers for it.
fn same_as_the_c_library(cases: &[(&str, &str)]) {
    for (sql, want) in cases {
        assert_eq!(quoted(sql), *want, "{sql}");
    }
}

#[test]
fn every_moment_answers_what_the_c_library_answers() {
    same_as_the_c_library(&[
        (r"date('2023-05-17')", r"'2023-05-17'"),
        (r"date('2023-05-17 12:34:56')", r"'2023-05-17'"),
        (r"date('2023-05-17T12:34:56')", r"'2023-05-17'"),
        (r"date('2023-05-17 12:34:56.789')", r"'2023-05-17'"),
        (r"date('12:34:56')", r"'2000-01-01'"),
        (r"date('2451545.0')", r"'2000-01-01'"),
        (r"date(2451545.0)", r"'2000-01-01'"),
        (r"date(2451545)", r"'2000-01-01'"),
        (r"date('2023-05-17','+1 day')", r"'2023-05-18'"),
        (r"date('2023-05-17','-1 day')", r"'2023-05-16'"),
        (r"date('2023-05-17','+1 month')", r"'2023-06-17'"),
        (r"date('2023-05-17','+1 year')", r"'2024-05-17'"),
        (r"date('2023-05-17','+2 hours')", r"'2023-05-17'"),
        (r"date('2023-05-17','+120 minutes')", r"'2023-05-17'"),
        (r"date('2023-05-17','+3600 seconds')", r"'2023-05-17'"),
        (r"date('2023-05-17','start of month')", r"'2023-05-01'"),
        (r"date('2023-05-17','start of year')", r"'2023-01-01'"),
        (r"date('2023-05-17','start of day')", r"'2023-05-17'"),
        (r"date('2023-05-17','weekday 0')", r"'2023-05-21'"),
        (r"date('2023-05-17','weekday 6')", r"'2023-05-20'"),
        (r"date('2023-05-17','weekday 3')", r"'2023-05-17'"),
        (r"date('2023-05-17','+0000-01-00')", r"'2023-06-17'"),
        (r"date('2023-05-17','-0001-00-00')", r"'2022-05-17'"),
        (r"date('2023-05-17','+00001-00-01')", r"'2024-05-18'"),
        (r"date('2023-01-31','+1 month')", r"'2023-03-03'"),
        (r"date('2023-01-31','+1 month','floor')", r"'2023-02-28'"),
        (r"date('2023-01-31','+1 month','ceiling')", r"'2023-03-03'"),
        (r"date('2023-05-17','bogus')", r"NULL"),
        (r"date('bogus')", r"NULL"),
        (r"date(0,'unixepoch')", r"'1970-01-01'"),
        (r"date(0,'auto')", r"'-4713-11-24'"),
        (r"date(2451545.0,'auto')", r"'2000-01-01'"),
        (r"date('2451545.0','julianday')", r"'2000-01-01'"),
        (r"date(NULL)", r"NULL"),
        (r"date('9999-12-31')", r"'9999-12-31'"),
        (r"date('0000-01-01')", r"'0000-01-01'"),
        (r"date('2023-02-29')", r"'2023-03-01'"),
        (r"date('2023-02-31')", r"'2023-03-03'"),
        (r"time('12:34:56')", r"'12:34:56'"),
        (r"time('12:34')", r"'12:34:00'"),
        (r"time('2023-05-17 12:34:56')", r"'12:34:56'"),
        (r"time('12:34:56.789','subsec')", r"'12:34:56.789'"),
        (r"datetime('2023-05-17 12:34:56')", r"'2023-05-17 12:34:56'"),
        (
            r"datetime('2023-05-17 12:34:56.789','subsec')",
            r"'2023-05-17 12:34:56.789'",
        ),
        (
            r"datetime('2023-05-17 12:34:56','+01:30')",
            r"'2023-05-17 14:04:56'",
        ),
        (
            r"datetime('2023-05-17 12:34:56','-01:30')",
            r"'2023-05-17 11:04:56'",
        ),
        (
            r"datetime('2023-05-17 12:34:56','+0001-01-01 02:03')",
            r"'2024-06-18 14:37:56'",
        ),
        (
            r"datetime('2023-05-17 12:00:00 +05:00')",
            r"'2023-05-17 07:00:00'",
        ),
        (
            r"datetime('2023-05-17 12:00:00 -05:15')",
            r"'2023-05-17 17:15:00'",
        ),
    ]);
}

#[test]
fn every_format_answers_what_the_c_library_answers() {
    same_as_the_c_library(&[
        (
            r"datetime('2023-05-17T12:00:00Z')",
            r"'2023-05-17 12:00:00'",
        ),
        (
            r"datetime('2023-05-17 12:00:00z')",
            r"'2023-05-17 12:00:00'",
        ),
        (r"julianday('2000-01-01')", r"2451544.5"),
        (r"julianday('2000-01-01 12:00:00')", r"2451545.0"),
        (r"julianday('1970-01-01')", r"2440587.5"),
        (r"julianday('bogus')", r"NULL"),
        (r"unixepoch('1970-01-01')", r"0"),
        (r"unixepoch('2023-05-17 12:34:56')", r"1684326896"),
        (
            r"unixepoch('2023-05-17 12:34:56.500','subsec')",
            r"1684326896.5",
        ),
        (r"unixepoch(2451545.0,'julianday')", r"946728000"),
        (r"strftime('%d','2023-05-17')", r"'17'"),
        (r"strftime('%e','2023-05-07')", r"' 7'"),
        (r"strftime('%f','2023-05-17 12:34:56.789')", r"'56.789'"),
        (r"strftime('%F','2023-05-17')", r"'2023-05-17'"),
        (r"strftime('%G','2023-01-01')", r"'2022'"),
        (r"strftime('%g','2023-01-01')", r"'22'"),
        (r"strftime('%H','2023-05-17 05:00')", r"'05'"),
        (r"strftime('%k','2023-05-17 05:00')", r"' 5'"),
        (r"strftime('%I','2023-05-17 13:00')", r"'01'"),
        (r"strftime('%I','2023-05-17 00:30')", r"'12'"),
        (r"strftime('%l','2023-05-17 13:00')", r"' 1'"),
        (r"strftime('%j','2023-05-17')", r"'137'"),
        (r"strftime('%J','2000-01-01')", r"'2451544.5'"),
        (r"strftime('%m','2023-05-17')", r"'05'"),
        (r"strftime('%M','2023-05-17 12:34')", r"'34'"),
        (r"strftime('%p','2023-05-17 13:00')", r"'PM'"),
        (r"strftime('%p','2023-05-17 01:00')", r"'AM'"),
        (r"strftime('%P','2023-05-17 13:00')", r"'pm'"),
        (r"strftime('%P','2023-05-17 01:00')", r"'am'"),
        (r"strftime('%R','2023-05-17 12:34')", r"'12:34'"),
        (r"strftime('%s','2000-01-01')", r"'946684800'"),
        (
            r"strftime('%s','2000-01-01 00:00:00.500','subsec')",
            r"'946684800.500'",
        ),
        (r"strftime('%S','2023-05-17 12:34:56')", r"'56'"),
        (r"strftime('%T','2023-05-17 12:34:56')", r"'12:34:56'"),
        (r"strftime('%u','2023-05-21')", r"'7'"),
        (r"strftime('%w','2023-05-21')", r"'0'"),
        (r"strftime('%U','2023-05-17')", r"'20'"),
        (r"strftime('%V','2023-05-17')", r"'20'"),
        (r"strftime('%W','2023-05-17')", r"'20'"),
        (r"strftime('%Y','2023-05-17')", r"'2023'"),
        (r"strftime('%%','2023-05-17')", r"'%'"),
        (r"strftime('%Q','2023-05-17')", r"NULL"),
        (r"strftime('x%dy','2023-05-17')", r"'x17y'"),
        (r"strftime('%d','bogus')", r"NULL"),
        (
            r"timediff('2023-05-17','2022-01-01')",
            r"'+0001-04-16 00:00:00.000'",
        ),
        (
            r"timediff('2022-01-01','2023-05-17')",
            r"'-0001-04-16 00:00:00.000'",
        ),
        (
            r"timediff('2023-05-17 12:34:56.500','2023-05-17 12:34:56.000')",
            r"'+0000-00-00 00:00:00.500'",
        ),
        (r"timediff('2023-05-17','bogus')", r"NULL"),
        (r"date('2023-05-17','subsec')", r"'2023-05-17'"),
    ]);
}

/// The second moment walked to the first a month at a time, which is
/// `timediffFunc`.
#[test]
fn the_months_timediff_counts_are_the_months_it_walks() {
    same_as_the_c_library(&[
        // A month the moment it is walked to does not reach, so the
        // walk takes a month off and the year with it.
        (
            r"timediff('2000-03-01','2000-01-31')",
            r"'+0000-00-30 00:00:00.000'",
        ),
        (
            r"timediff('2000-01-31','2000-03-01')",
            r"'-0000-01-01 00:00:00.000'",
        ),
        (
            r"timediff('2024-02-29','2023-03-31')",
            r"'+0000-10-29 00:00:00.000'",
        ),
        (
            r"timediff('0000-01-01','0000-01-03')",
            r"'-0000-00-02 00:00:00.000'",
        ),
        (
            r"timediff('1066-10-14 00:00:00','-4713-11-24 12:00:00')",
            r"'+5778-10-19 12:00:00.000'",
        ),
        (
            r"timediff('2000-01-01','2001-03-15 06:07:08.9')",
            r"'-0001-02-14 06:07:08.900'",
        ),
        // The walk crosses the end of a year in both directions.
        (
            r"timediff('2000-12-15','2001-01-10')",
            r"'-0000-00-26 00:00:00.000'",
        ),
        (
            r"timediff('2001-01-10','2000-12-15')",
            r"'+0000-00-26 00:00:00.000'",
        ),
    ]);
}

#[test]
fn every_shape_the_c_library_refuses_is_refused() {
    same_as_the_c_library(&[
        (r"date('9999-99-99')", r"NULL"),
        (r"date('2023-05-17 25:00')", r"NULL"),
        (r"date('2023-05-17 12:60')", r"NULL"),
        (r"date('2023-05-17 12:00:60')", r"NULL"),
        (r"date('2023-05-17 12:00:00+5:00')", r"NULL"),
        (r"date('2023-05-17 12:00:00+05:00x')", r"NULL"),
        (r"date('2023-05-17 12:00:00 Zx')", r"NULL"),
        (r"date('-4713-01-01')", r"NULL"),
        (r"date('-0001-06-15')", r"'-0001-06-15'"),
        (r"date('10000-01-01')", r"NULL"),
        (r"date('14712-12-31')", r"NULL"),
        (
            r"datetime('2024-01-31','+1 month')",
            r"'2024-03-02 00:00:00'",
        ),
        (
            r"datetime('2024-01-31','+1 month','floor')",
            r"'2024-02-29 00:00:00'",
        ),
        (
            r"datetime('2023-03-31','+1 month')",
            r"'2023-05-01 00:00:00'",
        ),
        (
            r"datetime('2023-03-31','+1 month','floor')",
            r"'2023-04-30 00:00:00'",
        ),
        (
            r"datetime('2023-02-28','+1 month','floor')",
            r"'2023-03-28 00:00:00'",
        ),
        (r"date('2023-05-17','weekday x')", r"NULL"),
        (r"date('2023-05-17','weekday 7')", r"NULL"),
        (r"date('2023-05-17','weekday -1')", r"NULL"),
        (r"date('2023-05-17','weekday 2.5')", r"NULL"),
        (r"date('2023-05-17','start of week')", r"NULL"),
        (r"date('2023-05-17','+1 fortnight')", r"NULL"),
        (r"date('2023-05-17','+99999999 years')", r"NULL"),
        (r"date('2023-05-17','+1e400 days')", r"NULL"),
        (r"date('2023-05-17','+ 1 day')", r"NULL"),
        (r"date('2023-05-17','1 day')", r"'2023-05-18'"),
        (r"date('2023-05-17','+1.5 days')", r"'2023-05-18'"),
        (r"date('2023-05-17','-1.5 days')", r"'2023-05-15'"),
        (
            r"datetime('2023-05-17 12:00','+12:30')",
            r"'2023-05-18 00:30:00'",
        ),
        (
            r"datetime('2023-05-17 12:00','-12:30')",
            r"'2023-05-16 23:30:00'",
        ),
        (
            r"datetime('2023-05-17 12:00','+12:30:30')",
            r"'2023-05-18 00:30:30'",
        ),
        (
            r"datetime('2023-05-17 12:00','+12:30:30.500','subsec')",
            r"'2023-05-18 00:30:30.500'",
        ),
        (
            r"datetime('2023-05-17 12:00','+0001-02-03')",
            r"'2024-07-20 12:00:00'",
        ),
        (
            r"datetime('2023-05-17 12:00','-0001-02-03')",
            r"'2022-03-14 12:00:00'",
        ),
    ]);
}

#[test]
fn every_shape_of_zone_and_month_answers_the_same() {
    same_as_the_c_library(&[
        (
            r"datetime('2023-05-17 12:00','+00001-02-03')",
            r"'2024-07-20 12:00:00'",
        ),
        (
            r"datetime('2023-05-17 12:00','+0001-02-03 04:05')",
            r"'2024-07-20 16:05:00'",
        ),
        (r"datetime('2023-05-17 12:00','+0001-13-03')", r"NULL"),
        (r"datetime('2023-05-17 12:00','+0001-02-31')", r"NULL"),
        (r"datetime('2023-05-17 12:00','0001-02-03')", r"NULL"),
        (r"datetime('12:34:56.9999')", r"'2000-01-01 12:34:56'"),
        (
            r"datetime('12:34:56.9999','subsec')",
            r"'2000-01-01 12:34:56.999'",
        ),
        (r"julianday(0,'auto')", r"0.0"),
        (r"julianday(2451545,'auto')", r"2451545.0"),
        (r"julianday(1684324496,'auto')", r"2460081.9964814815"),
        (r"julianday('2451545','julianday')", r"2451545.0"),
        (r"julianday(1684324496,'julianday')", r"NULL"),
        (r"julianday('2023-05-17','unixepoch')", r"NULL"),
        (r"julianday(-1,'unixepoch')", r"2440587.499988426"),
        (r"julianday(1e20,'unixepoch')", r"NULL"),
        (r"julianday(1684324496,'unixepoch')", r"2460081.9964814815"),
        (r"date('2023-05-17','+1 day','+1 day')", r"'2023-05-19'"),
        (r"date('2023-05-17','auto')", r"'2023-05-17'"),
        (r"date(1684324496,'unixepoch','auto')", r"NULL"),
        (r"strftime('%f','2023-05-17 12:34:00')", r"'00.000'"),
        (r"strftime('%G %V','2021-01-01')", r"'2020 53'"),
        (r"strftime('%G %V','2020-12-31')", r"'2020 53'"),
        (r"strftime('%g','1999-01-01')", r"'98'"),
        (r"strftime('%U %W','2023-01-01')", r"'01 00'"),
        (r"strftime('%U %W','2023-01-02')", r"'01 01'"),
        (
            r"strftime('%I %l %p %P','2023-05-17 00:30')",
            r"'12 12 AM am'",
        ),
        (r"strftime('%I %l','2023-05-17 12:30')", r"'12 12'"),
        (r"strftime('%e %k','2023-05-07 05:00')", r"' 7  5'"),
        (
            r"timediff('2023-01-31','2023-03-31')",
            r"'-0000-02-00 00:00:00.000'",
        ),
        (
            r"timediff('2023-03-31','2023-01-31')",
            r"'+0000-02-00 00:00:00.000'",
        ),
        (
            r"timediff('2000-01-01','2000-01-01')",
            r"'+0000-00-00 00:00:00.000'",
        ),
        (
            r"timediff('2023-05-17 00:00:00.250','2023-05-16 23:00:00.000')",
            r"'+0000-00-00 01:00:00.250'",
        ),
        (r"unixepoch('2023-05-17','subsec')", r"1684281600.0"),
        (r"unixepoch('1900-01-01')", r"-2208988800"),
        (r"time('2023-05-17 12:34:56','subsec')", r"'12:34:56.000'"),
    ]);
}

#[test]
fn every_edge_answers_what_the_c_library_answers() {
    same_as_the_c_library(&[
        (r"date('9999-12-31','+1 year')", r"NULL"),
        (r"julianday(1684324496)", r"NULL"),
        (r"date('2023-05-17 12:00:00x')", r"NULL"),
        (
            r"datetime('2023-05-17 12:00:00+00:00')",
            r"'2023-05-17 12:00:00'",
        ),
        (
            r"datetime('2003-10-22 12:34','-13 month')",
            r"'2002-09-22 12:34:00'",
        ),
        (
            r"datetime('2003-10-22 12:34','-25 month')",
            r"'2001-09-22 12:34:00'",
        ),
        (r"date('2023-05-17',5)", r"NULL"),
        (r"date(5,'unixepoch')", r"'1970-01-01'"),
        (r"strftime('%Y','-0001-06-15')", r"'-001'"),
        (r"strftime('%G','-0001-06-15')", r"'-001'"),
        (r"time('bogus')", r"NULL"),
        (r"datetime('bogus')", r"NULL"),
        (r"unixepoch('bogus')", r"NULL"),
        (r"timediff('bogus','2023-05-17')", r"NULL"),
        (
            r"datetime('2023-05-17 12:00','+00001-02-03 04:05')",
            r"'2024-07-20 16:05:00'",
        ),
        (r"datetime('2023-05-17 12:00','+0001-02-03x')", r"NULL"),
        (r"datetime('2023-05-17 12:00','+0001-02-03 99:99')", r"NULL"),
        (r"datetime('2023-05-17 12:00','+0001-02-03 0405')", r"NULL"),
        (r"date(NULL,'+1 day')", r"NULL"),
        (r"date('2023-05-17',NULL)", r"NULL"),
        (r"time('2023-05-17 12:34:56.999999')", r"'12:34:56'"),
        (r"date('0000-01-01','-1 day')", r"'-0001-12-31'"),
        (r"julianday('0000-01-01')", r"1721059.5"),
        (r"date('2023-05-17','+0 day')", r"'2023-05-17'"),
        (r"strftime('','2023-05-17')", r"''"),
        (r"strftime('%','2023-05-17')", r"NULL"),
        (r"datetime('2023-05-17 12:00','+1:30')", r"NULL"),
    ]);
}

#[test]
fn every_moment_a_modifier_makes_answers_the_same() {
    same_as_the_c_library(&[
        (
            r"datetime('12:34','start of day')",
            r"'2000-01-01 00:00:00'",
        ),
        (r"date('12:34','start of month')", r"'2000-01-01'"),
        (r"date(1684324496,'start of day')", r"NULL"),
        (r"datetime('2023-05-17 12:00','00001-02-03')", r"NULL"),
        (r"datetime('2023-05-17 12:00','+00001-13-03')", r"NULL"),
        (
            r"timediff('2023-01-15','2022-03-31')",
            r"'+0000-09-15 00:00:00.000'",
        ),
        (
            r"timediff('2023-01-15','2022-01-31')",
            r"'+0000-11-15 00:00:00.000'",
        ),
        (
            r"date('5373484.0','julianday','start of day')",
            r"'9999-12-31'",
        ),
        (r"date('5373484.0','julianday')", r"'9999-12-31'"),
        (r"julianday('5373484.4')", r"5373484.4"),
        (r"date(5373484.4,'julianday','+1 day')", r"NULL"),
    ]);
}

#[test]
fn a_moment_that_runs_past_the_years_a_date_holds_answers_nothing() {
    same_as_the_c_library(&[
        (r"strftime(NULL,'2023-05-17')", r"NULL"),
        (r"date('9999-12-31','+1 day','+1 month')", r"NULL"),
        (r"date('9999-12-31','+2 day','start of year')", r"NULL"),
        (r"date(5373484.4,'julianday','+1 day','+1 month')", r"NULL"),
    ]);
}

#[test]
fn a_modifier_that_is_not_one_answers_nothing() {
    same_as_the_c_library(&[
        (r"date('2023-05-17','abc')", r"NULL"),
        (r"date('2023-05-17','cbc')", r"NULL"),
        (r"date('2023-05-17','fbc')", r"NULL"),
        (r"date('2023-05-17','jbc')", r"NULL"),
        (r"date('2023-05-17','ubc')", r"NULL"),
        (r"date('2023-05-17','wbc')", r"NULL"),
        (r"date('2023-05-17','sbc')", r"NULL"),
        (r"date('2023-05-17','subsecx')", r"NULL"),
        (r"date('2023-05-17','weekday')", r"NULL"),
        (r"date('2023-05-17','start of')", r"NULL"),
        (r"date('2023-05-17','start ofx')", r"NULL"),
        (r"date(5,'julianday','auto')", r"NULL"),
        (r"date(5,'julianday','julianday')", r"NULL"),
        (r"date(5,'unixepoch','unixepoch')", r"NULL"),
        (r"date('2023-05-17','+1 day','auto')", r"NULL"),
        (r"date('2024-02-29','+1 year')", r"'2025-03-01'"),
        (r"date('2024-02-29','+1 year','floor')", r"'2025-02-28'"),
        (r"date('2000-02-29','+1 year','floor')", r"'2001-02-28'"),
        (r"date('1900-02-28','+1 year','floor')", r"'1901-02-28'"),
        (r"date('2023-00-05')", r"NULL"),
        (r"date('2023-05-00')", r"NULL"),
        (r"time('12:34:56.x')", r"NULL"),
        (r"date('2023-05-17','+1 dy')", r"NULL"),
        (r"date('2023-05-17','+1 dayss')", r"NULL"),
        (r"date('2023-05-17','+1 abcdefghijk')", r"NULL"),
        (r"date('2023-05-17','+99999999999 days')", r"NULL"),
        (r"strftime('%u','2023-05-20')", r"'6'"),
        (r"strftime('%w','2023-05-20')", r"'6'"),
        (r"date(1e30,'auto')", r"NULL"),
        (r"date(-5,'auto')", r"'1969-12-31'"),
        (r"datetime('2023-05-17 12:00','+1-02-03')", r"NULL"),
    ]);
}

#[test]
fn every_word_a_modifier_is_near_answers_the_same() {
    same_as_the_c_library(&[
        (r"date('1900-01-31','+1 month','floor')", r"'1900-02-28'"),
        (r"date('2000-01-31','+1 month','floor')", r"'2000-02-29'"),
        (r"date('2023-05-17 12:00:00+05:00','julianday')", r"NULL"),
        (r"date('2023-05-17','weekdayx 1')", r"NULL"),
        (
            r"datetime('2023-05-17 12:34:56.789','subsecond')",
            r"'2023-05-17 12:34:56.789'",
        ),
        (r"date('2023-05-17','start ofxy')", r"NULL"),
        (r"date('2023-05-17','weekday 1x')", r"NULL"),
        (r"date(-210866760001,'auto')", r"NULL"),
        (r"date(253402300800,'auto')", r"NULL"),
        (r"date(253402300799,'auto')", r"'9999-12-31'"),
        (r"date(-210866760000,'auto')", r"'-4713-11-24'"),
        (
            r"datetime('2023-05-17 12:00','+00001-02-03 04:05:06')",
            r"'2024-07-20 16:05:06'",
        ),
    ]);
}

#[test]
fn a_count_past_what_a_modifier_takes_answers_nothing() {
    same_as_the_c_library(&[
        (r"datetime('2023-05-17 12:00','+1234a-02-03')", r"NULL"),
        (r"date('2023-05-17','-99999999999 days')", r"NULL"),
        (r"date('2023-05-17','-5373485 days')", r"NULL"),
        (r"date('2023-05-17','5373485 days')", r"NULL"),
    ]);
}

#[test]
fn a_call_that_asks_for_the_clock_answers_nothing() {
    // `now`, `localtime` and `utc` all ask for a clock, which this
    // crate is given none of, so every one of them answers nothing.
    for sql in [
        "date()",
        "strftime('%d')",
        "date('now')",
        "datetime('now')",
        "time('now')",
        "julianday('now')",
        "unixepoch('now')",
        "strftime('%Y','now')",
        "date('2023-05-17','localtime')",
        "date('2023-05-17','utc')",
    ] {
        assert_eq!(quoted(sql), "NULL", "{sql}");
    }
}
