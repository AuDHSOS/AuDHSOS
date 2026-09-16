// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Numbers written with digit separators, against what the shell
//! answers for the same statements.

use alloc::string::String;

use crate::db::Database;

/// What a statement answers, as `type value|` per value.
fn shown(sql: &[u8]) -> Result<String, String> {
    let database = Database::open(super::SMALL).expect("a database");
    let answered = database.query(sql).map_err(|error| error.message())?;
    let mut out = String::new();
    for row in &answered.rows {
        for value in row {
            out.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            out.push('|');
        }
    }
    Ok(out)
}

/// A separator between two digits is taken out, and a `.` or an `e`
/// left behind makes the number a real, which is
/// `sqlite3DequoteNumber`.
#[test]
fn a_separator_between_two_digits_is_taken_out_of_the_number() {
    for (sql, answer) in [
        (b"SELECT typeof(1_000), 1_000".as_slice(), "integer|1000|"),
        (b"SELECT typeof(1.1_1), 1.1_1", "real|1.11|"),
        (b"SELECT typeof(1_0.1_1), 1_0.1_1", "real|10.11|"),
        (b"SELECT typeof(1e1_000), 1e1_000", "real|Inf|"),
        (
            b"SELECT typeof(12_3_456.7_8_9), 12_3_456.7_8_9",
            "real|123456.789|",
        ),
        (
            b"SELECT typeof(9_223_372_036_854_775_807), 9_223_372_036_854_775_807",
            "integer|9223372036854775807|",
        ),
        (
            b"SELECT typeof(9_223_372_036_854_775_808), 9_223_372_036_854_775_808",
            "real|9.2233720368547758e+18|",
        ),
        (b"SELECT typeof(0x1_2), 0x1_2", "integer|18|"),
    ] {
        assert_eq!(shown(sql), Ok(String::from(answer)), "{sql:?}");
    }
}

/// A separator anywhere but between two digits makes the token one the
/// tokenizer reads as no token at all.
#[test]
fn a_separator_anywhere_else_makes_the_token_unrecognized() {
    for (sql, token) in [
        (b"SELECT 1_".as_slice(), "1_"),
        (b"SELECT 1_.4", "1_.4"),
        (b"SELECT 1e_4", "1e_4"),
        (b"SELECT 1_e4", "1_e4"),
        (b"SELECT 1.4_e4", "1.4_e4"),
        (b"SELECT 1.4e4_", "1.4e4_"),
        (b"SELECT 12__34", "12__34"),
        (b"SELECT 12._34", "12._34"),
        (b"SELECT 12_.34", "12_.34"),
        (b"SELECT 12.34_", "12.34_"),
        // A letter stuck to a number is one token as well.
        (b"SELECT 123a456", "123a456"),
        (b"SELECT 1.4e+_4", "1.4e"),
    ] {
        assert_eq!(
            shown(sql),
            Err(alloc::format!("unrecognized token: \"{token}\"")),
            "{sql:?}"
        );
    }
}

/// A hex literal is a whole number whatever its digits are, and `e` is
/// one of them.
#[test]
fn a_hex_literal_is_a_whole_number_whatever_its_digits_are() {
    assert_eq!(
        shown(b"SELECT typeof(0x1_e), 0x1_e"),
        Ok(String::from("integer|30|"))
    );
}

/// A number written with separators stands where a literal stands as
/// well as where an expression does.
#[test]
fn a_separated_number_stands_where_a_literal_stands() {
    use crate::change::Writer;
    use crate::header::Encoding;
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a DEFAULT 1_0)".as_slice(),
        b"INSERT INTO t DEFAULT VALUES",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(b"SELECT typeof(a), a FROM t").unwrap();
    assert_eq!(
        answered.rows,
        alloc::vec![alloc::vec![
            crate::value::Value::Text(b"integer".to_vec()),
            crate::value::Value::Int(10)
        ]]
    );
}

/// The term a message counts is the one out of range, with the two
/// letters that name its place, which is `%r` of `sqlite3_mprintf`.
#[test]
fn the_term_out_of_range_is_named_by_its_place() {
    for (sql, message) in [
        (
            b"SELECT 1,2,3 ORDER BY 1, 9".as_slice(),
            "2nd ORDER BY term out of range - should be between 1 and 3",
        ),
        (
            b"SELECT 1,2,3 ORDER BY 1,2,9",
            "3rd ORDER BY term out of range - should be between 1 and 3",
        ),
        (
            b"SELECT 1,2,3 ORDER BY 1,1,1,1,1,1,1,1,1,1,9",
            "11th ORDER BY term out of range - should be between 1 and 3",
        ),
        (
            b"SELECT 1,2,3 ORDER BY 1,1,1,9",
            "4th ORDER BY term out of range - should be between 1 and 3",
        ),
        (
            b"SELECT 1 GROUP BY 2",
            "1st GROUP BY term out of range - should be between 1 and 1",
        ),
    ] {
        assert_eq!(shown(sql), Err(String::from(message)), "{sql:?}");
    }
}
