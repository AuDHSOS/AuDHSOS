// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Values, and what is done to them before they are compared.
//!
//! `docs/sqlite/datatype3.html` is the document: five storage classes,
//! the affinities that decide what is converted before a comparison, the
//! order NULL, numbers, text and blobs sort in, and the three collations
//! that are built in. What is here is the routines of `src/vdbe.c` and
//! `src/vdbemem.c` those rules are written down in, because the rules as
//! prose leave out which conversion is tried first and which is only
//! tried where it loses nothing.
//!
//! Text is in the database's encoding. Everything here reads it as UTF-8,
//! which is what a statement is; the column layer will say otherwise.

use alloc::vec::Vec;

use crate::fp;
use crate::number::{self, Outcome};

/// A value, as an expression computes with it.
///
/// Text and blobs are owned: a concatenation makes a value that is in no
/// page, and a value that outlives the row it was read from is what an
/// expression answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `NULL`.
    Null,
    /// A 64-bit integer.
    Int(i64),
    /// A double.
    Real(f64),
    /// Text.
    Text(Vec<u8>),
    /// A blob.
    Blob(Vec<u8>),
}

/// The storage classes, in the order they sort in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// `NULL`, which sorts before everything.
    Null,
    /// A number, integer or real, which sort together.
    Number,
    /// Text.
    Text,
    /// A blob, which sorts after everything.
    Blob,
}

/// What a value is converted to before it is compared or stored.
///
/// `None` is the affinity of an expression that is not a column, which is
/// not the same as the `BLOB` affinity of a column with no declared type:
/// one side of a comparison having no affinity is what lets the other
/// side's affinity decide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Affinity {
    /// No affinity, which is every expression that is not a column.
    #[default]
    None,
    /// `BLOB`, which converts nothing.
    Blob,
    /// `TEXT`.
    Text,
    /// `NUMERIC`.
    Numeric,
    /// `INTEGER`.
    Integer,
    /// `REAL`.
    Real,
}

impl Affinity {
    /// Whether the affinity is one of the three that ask for a number.
    #[must_use]
    pub fn numeric(self) -> bool {
        self >= Affinity::Numeric
    }

    /// The affinity a declared type name has.
    ///
    /// This is `sqlite3AffinityType` in `src/build.c`: a rolling window
    /// over the letters, where the first `INT` wins outright and the
    /// others win only over what is still undecided. A name with no
    /// letters at all is `NUMERIC`.
    #[must_use]
    pub fn of_type(name: &[u8]) -> Self {
        let mut window: u32 = 0;
        let mut affinity = Affinity::Numeric;
        for byte in name {
            window = (window << 8) | u32::from(byte.to_ascii_lowercase());
            if window == word(*b"char") || window == word(*b"clob") || window == word(*b"text") {
                affinity = Affinity::Text;
            } else if window == word(*b"blob")
                && matches!(affinity, Affinity::Numeric | Affinity::Real)
            {
                affinity = Affinity::Blob;
            } else if (window == word(*b"real")
                || window == word(*b"floa")
                || window == word(*b"doub"))
                && affinity == Affinity::Numeric
            {
                affinity = Affinity::Real;
            } else if window & 0x00ff_ffff == word(*b"\0int") {
                return Affinity::Integer;
            }
        }
        affinity
    }
}

/// Four letters as the window `Affinity::of_type` compares against.
const fn word(letters: [u8; 4]) -> u32 {
    u32::from_be_bytes(letters)
}

/// The three collations SQLite has built in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Collation {
    /// Byte by byte, then by length.
    #[default]
    Binary,
    /// Byte by byte with the twenty-six letters folded, then by length.
    NoCase,
    /// Byte by byte with trailing spaces ignored.
    Rtrim,
}

/// The collation a name spells, where it spells one.
impl Collation {
    /// The collation `name` names.
    #[must_use]
    pub const fn of_name(name: &[u8]) -> Option<Self> {
        if name.eq_ignore_ascii_case(b"binary") {
            Some(Collation::Binary)
        } else if name.eq_ignore_ascii_case(b"nocase") {
            Some(Collation::NoCase)
        } else if name.eq_ignore_ascii_case(b"rtrim") {
            Some(Collation::Rtrim)
        } else {
            None
        }
    }
}

impl Value {
    /// Which class the value is in.
    #[must_use]
    pub const fn class(&self) -> Class {
        match self {
            Value::Null => Class::Null,
            Value::Int(_) | Value::Real(_) => Class::Number,
            Value::Text(_) => Class::Text,
            Value::Blob(_) => Class::Blob,
        }
    }

    /// The bytes, where the value is text or a blob.
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Text(bytes) | Value::Blob(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// The value as a double, which is `sqlite3VdbeRealValue`.
    #[must_use]
    pub fn to_real(&self) -> f64 {
        match self {
            Value::Real(number) => *number,
            Value::Int(number) => integer_as_real(*number),
            Value::Text(bytes) | Value::Blob(bytes) => number::real(bytes).value,
            Value::Null => 0.0,
        }
    }

    /// The value as an integer, which is `sqlite3VdbeIntValue`.
    #[must_use]
    pub fn to_integer(&self) -> i64 {
        match self {
            Value::Int(number) => *number,
            Value::Real(number) => real_as_integer(*number),
            Value::Text(bytes) | Value::Blob(bytes) => number::integer(bytes).value,
            Value::Null => 0,
        }
    }

    /// The bytes `sqlite3_value_text` answers with: its own where it has
    /// them, the text a number is written as, and nothing for `NULL`.
    #[must_use]
    pub fn text(&self) -> Option<Vec<u8>> {
        match self {
            Value::Null => None,
            Value::Text(bytes) | Value::Blob(bytes) => Some(bytes.clone()),
            other => other.stringify(),
        }
    }

    /// The text a number is written as, which is
    /// `sqlite3VdbeMemStringify`. Nothing for what is not a number.
    #[must_use]
    pub fn stringify(&self) -> Option<Vec<u8>> {
        match self {
            Value::Int(number) => Some(number::integer_text(*number)),
            Value::Real(number) => Some(fp::text(*number, fp::DIGITS)),
            _ => None,
        }
    }

    /// Whether the value counts as true, with `if_null` for `NULL`.
    ///
    /// This is `sqlite3VdbeBooleanValue`.
    #[must_use]
    pub fn truth(&self, if_null: bool) -> bool {
        match self {
            Value::Int(number) => *number != 0,
            Value::Null => if_null,
            _ => self.to_real() != 0.0,
        }
    }

    /// The number the value is for arithmetic: an integer where the text
    /// is one, a double otherwise.
    ///
    /// This is `computeNumericType` in `src/vdbe.c`, which is what makes
    /// `'abc' + 1` answer `1` rather than nothing.
    #[must_use]
    pub fn numeric_type(&self) -> Self {
        let Some(bytes) = self.bytes() else {
            return self.clone();
        };
        let read = number::real(bytes);
        let whole = number::integer(bytes);
        let fits = |limit: bool| {
            !read.fractional()
                && match whole.outcome {
                    Outcome::Exact | Outcome::Empty => true,
                    Outcome::Trailing => limit,
                    Outcome::Overflow | Outcome::Limit => false,
                }
        };
        // Where the text is only a prefix of a number, or none at all,
        // the digits still make an integer unless they overflow.
        let whole_enough = if read.number() && read.complete() {
            fits(false)
        } else {
            fits(true)
        };
        if whole_enough {
            Value::Int(whole.value)
        } else {
            Value::Real(read.value)
        }
    }
}

/// A double as an integer, truncated toward zero and clamped.
///
/// This is `sqlite3RealToI64`.
#[must_use]
pub fn real_as_integer(number: f64) -> i64 {
    if number < -9_223_372_036_854_774_784.0 {
        return i64::MIN;
    }
    if number > 9_223_372_036_854_774_784.0 {
        return i64::MAX;
    }
    truncate(number)
}

/// A double whose magnitude fits, truncated toward zero.
///
/// The mantissa and the exponent are taken apart rather than handed to a
/// conversion, so that nothing rounds and nothing is undefined.
fn truncate(number: f64) -> i64 {
    let bits = number.to_bits();
    let negative = bits >> 63 == 1;
    let exponent = i64::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    // A NaN, which no comparison reaches, and a subnormal, which
    // truncates to zero, are both zero here.
    let (mantissa, shift) = if exponent == 0 || exponent == 0x7ff {
        (0, 0)
    } else {
        (fraction | (1 << 52), exponent.saturating_sub(1075))
    };
    let magnitude = if shift >= 0 {
        mantissa.wrapping_shl(u32::try_from(shift).unwrap_or(0))
    } else {
        mantissa.wrapping_shr(u32::try_from(shift.wrapping_neg()).unwrap_or(64))
    };
    let value = i64::try_from(magnitude).unwrap_or(i64::MIN);
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}

/// An integer as a double, which is exact below 2^53 and rounds above it.
#[must_use]
pub fn integer_as_real(number: i64) -> f64 {
    let high = i32::try_from(number >> 32).unwrap_or(0);
    let low = u32::try_from(number.cast_unsigned() & 0xffff_ffff).unwrap_or(0);
    // The product is exact and the sum rounds once, which is the same
    // answer a conversion instruction gives.
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// Whether a double and an integer are the same number, to the precision
/// the double has.
///
/// This is `sqlite3RealSameAsInt`, and the bound is where a double stops
/// holding every integer with room to spare.
fn same_as_integer(real: f64, whole: i64) -> bool {
    real == 0.0
        || (real.to_bits() == integer_as_real(whole).to_bits()
            && (-2_251_799_813_685_248..2_251_799_813_685_248).contains(&whole))
}

/// Converts text that looks like a number into one, and leaves the rest
/// alone.
///
/// This is `applyNumericAffinity`. `try_for_int` asks for an integer
/// where the double is one, which a comparison does not want and storing
/// does.
pub fn apply_numeric(value: &mut Value, try_for_int: bool) {
    let Value::Text(bytes) = value else {
        return;
    };
    let read = number::real(bytes);
    if !read.number() || !read.complete() {
        return;
    }
    if !read.fractional() {
        let whole = real_as_integer(read.value);
        if same_as_integer(read.value, whole) {
            *value = Value::Int(whole);
            return;
        }
        let read = number::integer(bytes);
        if read.outcome == Outcome::Exact {
            *value = Value::Int(read.value);
            return;
        }
    }
    *value = Value::Real(read.value);
    if try_for_int {
        integer_affinity(value);
    }
}

/// Makes a double an integer where the conversion loses nothing.
///
/// This is `sqlite3VdbeIntegerAffinity`. The two ends of the range are
/// left alone because adding to either of them wraps.
#[expect(
    clippy::float_cmp,
    reason = "the question is whether the conversion is lossless, which is an exact comparison"
)]
pub fn integer_affinity(value: &mut Value) {
    let Value::Real(real) = *value else {
        return;
    };
    let whole = real_as_integer(real);
    if real == integer_as_real(whole) && whole > i64::MIN && whole < i64::MAX {
        *value = Value::Int(whole);
    }
}

/// Converts the value to a number, however much of it is one.
///
/// This is `sqlite3VdbeMemNumerify`, which is `CAST(x AS NUMERIC)`.
pub fn numerify(value: &mut Value) {
    let Some(bytes) = value.bytes() else {
        return;
    };
    let read = number::real(bytes);
    let whole = number::integer(bytes);
    let rounded = real_as_integer(read.value);
    if !read.fractional() && whole.outcome != Outcome::Overflow && whole.outcome != Outcome::Limit {
        *value = Value::Int(whole.value);
    } else if same_as_integer(read.value, rounded) {
        *value = Value::Int(rounded);
    } else {
        *value = Value::Real(read.value);
    }
}

/// Converts the value the way storing it in a column of this affinity
/// would, which converts only where nothing is lost.
///
/// This is `applyAffinity` in `src/vdbe.c`.
pub fn apply(value: &mut Value, affinity: Affinity) {
    match affinity {
        Affinity::Numeric | Affinity::Integer | Affinity::Real => match value {
            Value::Int(_) => {}
            Value::Real(_) => integer_affinity(value),
            _ => apply_numeric(value, true),
        },
        Affinity::Text => {
            if let Some(text) = value.stringify() {
                *value = Value::Text(text);
            }
        }
        Affinity::None | Affinity::Blob => {}
    }
}

/// Converts the value whatever it costs, which is what `CAST` does.
///
/// This is `sqlite3VdbeMemCast`.
pub fn cast(value: &mut Value, affinity: Affinity) {
    if *value == Value::Null {
        return;
    }
    match affinity {
        Affinity::None | Affinity::Blob => {
            let bytes = take_bytes(value);
            *value = Value::Blob(bytes);
        }
        Affinity::Text => {
            let bytes = take_bytes(value);
            *value = Value::Text(bytes);
        }
        Affinity::Numeric => numerify(value),
        Affinity::Integer => *value = Value::Int(value.to_integer()),
        Affinity::Real => *value = Value::Real(value.to_real()),
    }
}

/// The bytes of a value: its own where it has them, and the text a
/// number is written as otherwise.
fn take_bytes(value: &mut Value) -> Vec<u8> {
    match value {
        Value::Text(bytes) | Value::Blob(bytes) => core::mem::take(bytes),
        other => other.stringify().unwrap_or_default(),
    }
}

/// The affinity a comparison between two sides of these affinities uses.
///
/// This is `sqlite3CompareAffinity`: where both sides are columns a
/// number beats text, and where only one is a column that one decides.
#[must_use]
pub fn compare_affinity(left: Affinity, right: Affinity) -> Affinity {
    if left > Affinity::None && right > Affinity::None {
        if left.numeric() || right.numeric() {
            Affinity::Numeric
        } else {
            Affinity::Blob
        }
    } else if left == Affinity::None {
        right
    } else {
        left
    }
}

/// Converts both sides of a comparison the way the affinity asks.
///
/// This is what `OP_Eq` and the five operators beside it do before they
/// compare: only a value that is nothing but text is made a number, and
/// only where one side is text is a number made text.
pub fn apply_comparison(left: &mut Value, right: &mut Value, affinity: Affinity) {
    let text = matches!(left, Value::Text(_)) || matches!(right, Value::Text(_));
    if affinity.numeric() {
        if text {
            apply_numeric(left, false);
            apply_numeric(right, false);
        }
    } else if affinity == Affinity::Text && text {
        for value in [left, right] {
            if let Some(written) = value.stringify() {
                *value = Value::Text(written);
            }
        }
    }
}

/// Where `left` sits against `right`.
///
/// This is `sqlite3MemCompare`: NULL before numbers before text before
/// blobs, integers against doubles without either losing precision, and
/// text under the collation.
#[must_use]
#[expect(
    clippy::match_same_arms,
    reason = "the arms are the sort order in the order it goes, which folding them together would hide"
)]
pub fn compare(left: &Value, right: &Value, collation: Collation) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (left, right) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Int(left), Value::Int(right)) => left.cmp(right),
        (Value::Real(left), Value::Real(right)) => order(*left, *right),
        (Value::Int(left), Value::Real(right)) => integer_against_real(*left, *right),
        (Value::Real(left), Value::Int(right)) => integer_against_real(*right, *left).reverse(),
        (Value::Int(_) | Value::Real(_), _) => Ordering::Less,
        (_, Value::Int(_) | Value::Real(_)) => Ordering::Greater,
        (Value::Text(_), Value::Blob(_)) => Ordering::Less,
        (Value::Blob(_), Value::Text(_)) => Ordering::Greater,
        (Value::Text(left), Value::Text(right)) => collate(left, right, collation),
        (Value::Blob(left), Value::Blob(right)) => binary(left, right),
    }
}

/// Two doubles, where neither is a NaN.
fn order(left: f64, right: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if left < right {
        Ordering::Less
    } else if left > right {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// An integer against a double, without either losing precision.
///
/// This is `sqlite3IntFloatCompare`: the double is truncated and the
/// integers compared first, and only where those agree does the integer
/// become a double.
fn integer_against_real(left: i64, right: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if right.is_nan() {
        // SQLite reads a NaN as a NULL, and every integer is above one.
        return Ordering::Greater;
    }
    if right < -9_223_372_036_854_775_808.0 {
        return Ordering::Greater;
    }
    if right >= 9_223_372_036_854_775_808.0 {
        return Ordering::Less;
    }
    let whole = truncate(right);
    if left != whole {
        return left.cmp(&whole);
    }
    order(integer_as_real(left), right)
}

/// The bytes with trailing spaces dropped.
fn trimmed(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while bytes.get(end.wrapping_sub(1)) == Some(&b' ') {
        end = end.saturating_sub(1);
    }
    bytes.get(..end).unwrap_or_default()
}

/// Byte by byte, then by length.
fn binary(left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    left.cmp(right)
}

/// Text under a collation.
fn collate(left: &[u8], right: &[u8], collation: Collation) -> core::cmp::Ordering {
    match collation {
        Collation::Binary => binary(left, right),
        Collation::NoCase => {
            let fold =
                |bytes: &[u8]| -> Vec<u8> { bytes.iter().map(u8::to_ascii_lowercase).collect() };
            binary(&fold(left), &fold(right))
        }
        Collation::Rtrim => binary(trimmed(left), trimmed(right)),
    }
}
