// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Decimal arithmetic of any precision, against the answers of the C
//! library's shell with `ext/misc/decimal.c` linked in.

use alloc::string::String;

use crate::decimal::{self, Decimal};

/// What a text answers under one of the functions.
fn shown(held: &[u8]) -> String {
    String::from_utf8_lossy(held).into_owned()
}

/// The text `decimal(X)` answers for a text.
#[test]
fn what_text_one_value_is_read_from_and_written_as() {
    for (text, wanted) in [
        ("1", "1"),
        ("+0", "0"),
        ("-0", "0"),
        ("1.0", "1.0"),
        ("0001.0", "1.0"),
        ("+0001.0", "1.0"),
        ("-0001.0", "-1.0"),
        ("-0000.0", "0.0"),
        (
            "1.0e72",
            "1000000000000000000000000000000000000000000000000000000000000000000000000",
        ),
        (
            "1.0e-72",
            "0.0000000000000000000000000000000000000000000000000000000000000000000000010",
        ),
        ("-123e-4", "-0.0123"),
        ("+123e+4", "1230000"),
        ("e-5", "0.00000"),
        (".e-5", "0.00000"),
        (".", "0"),
        ("", "0"),
        ("abc", "0"),
        ("-", "0"),
        ("1e", "1"),
        ("1e+", "1"),
        ("0.000", "0.000"),
        ("000", "0"),
        ("5e-3", "0.005"),
        ("12.34e-1", "1.234"),
        ("12.34e+1", "123.4"),
        ("0.000e-2", "0.00000"),
        ("  12.5 ", "12.5"),
        (
            "9999e99",
            "9999000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        ),
        ("1.00000000000000001", "1.00000000000000001"),
        // A byte the exponent is not made of is passed over, so the
        // digits around it are read as one number.
        ("1e1x2", "1000000000000"),
    ] {
        let held = Decimal::of_text(text.as_bytes());
        assert_eq!(shown(&held.text()), wanted, "decimal({text})");
    }
}

/// The text `decimal_exp(X)` answers, which is the notation `%+#e`
/// writes.
#[test]
fn what_text_one_value_is_written_as_in_exponential_notation() {
    for (text, wanted) in [
        ("+123e+4", "+1.23e+06"),
        ("0", "+0.0e+00"),
        ("1", "+1.0e+00"),
        ("1.5", "+1.5e+00"),
        ("-1.5", "-1.5e+00"),
        ("0.00001", "+1.0e-05"),
        ("1e300", "+1.0e+300"),
        ("1e-300", "+1.0e-300"),
        ("-0", "+0.0e+00"),
        ("100", "+1.0e+02"),
        ("1.20", "+1.2e+00"),
        // The exponent of a text is read to a million and no further.
        ("1e1000000", "+1.0e+1000000"),
    ] {
        let held = Decimal::of_text(text.as_bytes());
        assert_eq!(shown(&held.scientific(0)), wanted, "decimal_exp({text})");
    }
}

/// The value `decimal(X, N)` answers, which is `X` rounded to `N`
/// significant digits.
#[test]
fn what_one_value_rounded_to_a_count_of_digits_is() {
    for (text, count, wanted) in [
        ("999999999999999", 1, "1000000000000000"),
        ("999999999999999", 2, "1000000000000000"),
        ("999999999999999", 14, "1000000000000000"),
        ("999999999999999", 15, "999999999999999"),
        ("899999999999999", 1, "900000000000000"),
        ("899999999999999", 14, "900000000000000"),
        ("899999999999999", 15, "899999999999999"),
        ("989999999999999", 14, "990000000000000"),
        ("998999999999999", 14, "999000000000000"),
        ("999.999", 3, "1000.000"),
        ("0.00456", 2, "0.00460"),
        ("1.45", 2, "1.50"),
        ("-1.45", 2, "-1.50"),
        ("12345", 3, "12300"),
        // A count of nought leaves the value as it stands, which
        // `decimal_round` answers for every count below one.
        ("12345", 0, "12345"),
        // A value whose leading digits are zeros is rounded past them, so
        // the count reaches beyond the digits and leaves the value as it
        // stands.
        ("0.045", 2, "0.045"),
        ("0.00", 1, "0.00"),
    ] {
        let mut held = Decimal::of_text(text.as_bytes());
        held.round(count);
        assert_eq!(shown(&held.text()), wanted, "decimal({text},{count})");
    }
}

/// The sums `decimal_add(X,Y)` and the differences `decimal_sub(X,Y)`
/// answer.
#[test]
fn what_two_values_add_up_to_and_take_apart_as() {
    for (one, other, sum) in [
        ("1", "2", "3"),
        ("1.5", "2.25", "3.75"),
        ("-1.5", "2.25", "0.75"),
        ("1.5", "-2.25", "-0.75"),
        ("-1.5", "-2.25", "-3.75"),
        ("0", "0", "0"),
        ("999", "1", "1000"),
        ("1000", "-1", "999"),
        ("0.1", "0.2", "0.3"),
        ("1e10", "1", "10000000001"),
        ("1", "1e-10", "1.0000000001"),
        // Two values that cancel leave the sign of the first, which
        // `decimal_result` reads off a value of one digit alone.
        ("-1", "1", "-0"),
        ("1e-1", "0.25", "0.35"),
        ("1", "-1", "0"),
    ] {
        let held = decimal::added(
            Decimal::of_text(one.as_bytes()),
            Decimal::of_text(other.as_bytes()),
        );
        assert_eq!(shown(&held.text()), sum, "decimal_add({one},{other})");
    }
    for (one, other, held) in [
        ("1", "2", "-1"),
        ("2", "1", "1"),
        ("1.5", "1.5", "0.0"),
        ("-1", "-2", "1"),
    ] {
        let answered = decimal::added(
            Decimal::of_text(one.as_bytes()),
            Decimal::of_text(other.as_bytes()).negated(),
        );
        assert_eq!(shown(&answered.text()), held, "decimal_sub({one},{other})");
    }
}

/// The products `decimal_mul(X,Y)` answers, whose fraction is as wide as
/// the two fractions together down to the narrower of the two.
#[test]
fn what_two_values_multiply_out_as() {
    for (one, other, product) in [
        ("1234.00", "2.00", "2468.00"),
        ("1234.00", "2.0000", "2468.00"),
        ("1234.0000", "2.000", "2468.000"),
        ("1234.0000", "2", "2468"),
        ("-2", "3", "-6"),
        ("-2", "-3", "6"),
        ("0", "5", "0"),
        ("0.5", "0.5", "0.25"),
        ("1e5", "1e5", "10000000000"),
        ("-1", "0", "-0"),
    ] {
        let held = decimal::multiplied(
            Decimal::of_text(one.as_bytes()),
            &Decimal::of_text(other.as_bytes()),
        );
        assert_eq!(shown(&held.text()), product, "decimal_mul({one},{other})");
    }
}

/// What `decimal_cmp(X,Y)` answers, which the collation `decimal` orders
/// two texts by.
#[test]
fn how_two_values_compare() {
    for (one, other, wanted) in [
        ("1", "2", core::cmp::Ordering::Less),
        ("2", "1", core::cmp::Ordering::Greater),
        ("1", "1", core::cmp::Ordering::Equal),
        ("1.0", "1", core::cmp::Ordering::Equal),
        ("-1", "1", core::cmp::Ordering::Less),
        ("-1", "-2", core::cmp::Ordering::Greater),
        ("1", "-1", core::cmp::Ordering::Greater),
        ("-0", "+0", core::cmp::Ordering::Equal),
        ("-000.000", "0", core::cmp::Ordering::Equal),
        ("1.2", "1.2000", core::cmp::Ordering::Equal),
        ("12", "1.2", core::cmp::Ordering::Greater),
        ("-12", "-1.2", core::cmp::Ordering::Less),
    ] {
        assert_eq!(
            decimal::collate(one.as_bytes(), other.as_bytes()),
            wanted,
            "decimal_cmp({one},{other})"
        );
    }
}

/// The values `decimal_pow2(N)` answers, and nothing for a power past
/// twenty thousand either way.
#[test]
fn what_a_power_of_two_is() {
    for (power, wanted) in [
        (0, "+1.0e+00"),
        (1, "+2.0e+00"),
        (-1, "+5.0e-01"),
        (10, "+1.024e+03"),
        (-10, "+9.765625e-04"),
        (53, "+9.007199254740992e+15"),
    ] {
        let held = decimal::power_of_two(power).expect("a power of two");
        assert_eq!(shown(&held.scientific(0)), wanted, "decimal_pow2({power})");
    }
    assert!(decimal::power_of_two(20_001).is_none());
    assert!(decimal::power_of_two(-20_001).is_none());
}

/// The values a binary64 number spells exactly, and nothing for a number
/// that is not one.
#[test]
fn what_value_a_binary64_number_spells() {
    for (number, wanted) in [
        (0.0_f64, "0"),
        (-0.0_f64, "0"),
        (1.5_f64, "1.5"),
        (-1.5_f64, "-1.5"),
        (
            0.1_f64,
            "0.1000000000000000055511151231257827021181583404541015625",
        ),
    ] {
        let held = decimal::of_double(number).expect("a value");
        assert_eq!(shown(&held.text()), wanted, "decimal({number})");
    }
    // The smallest number a binary64 holds carries no leading one, so its
    // significand is read one bit over.
    assert_eq!(
        shown(
            &decimal::of_double(f64::from_bits(1))
                .expect("a value")
                .text()
        ),
        alloc::string::String::from_utf8_lossy(SMALLEST)
    );
    assert!(decimal::of_double(f64::NAN).is_none());
    assert!(decimal::of_double(f64::INFINITY).is_none());
    assert!(decimal::of_double(f64::NEG_INFINITY).is_none());
}

/// Nought is nought whatever sign it carries, which is what the step of
/// `decimal_sum` begins a group with.
#[test]
fn what_nought_answers() {
    let held = Decimal::zero();
    assert_eq!(shown(&held.text()), "0");
    // Nought with the sign turned over is still nought, which
    // `decimal_result` reads the sign off a value of one digit for.
    assert_eq!(shown(&Decimal::zero().negated().text()), "0");
    assert_eq!(shown(&Decimal::of_text(b"-0").text()), "0");
    assert_eq!(shown(&Decimal::of_text(b"-1").text()), "-1");
    assert_eq!(
        shown(&decimal::added(Decimal::zero(), Decimal::of_text(b"2.5")).text()),
        "2.5"
    );
}

/// The value of the smallest number a binary64 holds, which
/// `decimal.test` reads from `ieee754_from_blob(x'0000000000000001')`.
const SMALLEST: &[u8] = b"0.000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004940656458412465441765687928682213723650598026143247644255856825006755072702087518652998363616359923797965646954457177309266567103559397963987747960107818781263007131903114045278458171678489821036887186360569987307230500063874091535649843873124733972731696151400317153853980741262385655911710266585566867681870395603106249319452715914924553293054565444011274801297099995419319894090804165633245247571478690147267801593552386115501348035264934720193790268107107491703332226844753335720832431936092382893458368060106011506169809753078342277318329247904982524730776375927247874656084778203734469699533647017972677717585125660551199131504891101451037862738167250955837389733598993664809941164205702637090279242767544565229087538682506419718265533447265625";
