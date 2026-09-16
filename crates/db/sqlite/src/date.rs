// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The date and time functions, which is `src/date.c`.
//!
//! A moment is held as the julian day number times 86 400 000, which is
//! milliseconds, and as the year, month, day, hour, minute and second it
//! was written as. Every function reads its first argument as a moment,
//! applies the modifiers after it in the order they were written, and
//! writes the moment out in its own shape. Reading one moment costs
//! O(m) in the modifiers.
//!
//! What is not here: `now` and `localtime`, which need a clock this
//! crate is given none of, so a statement that names either refuses.

use alloc::vec::Vec;

use crate::value::{Value, integer_as_real, real_as_integer};

/// Milliseconds in a day.
const DAY: i64 = 86_400_000;

/// The julian day of 9999-12-31 23:59:59.999, in milliseconds, which is
/// the largest moment a four-digit year holds.
const LAST: i64 = 464_269_060_799_999;

/// Milliseconds between the julian day and the unix epoch, which is
/// `21086676 * 10000000` of `src/date.c`.
const EPOCH: i64 = 210_866_760_000_000;

/// One date and time, as `DateTime` of `src/date.c` holds it.
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the flags of `DateTime`, which say which of its fields hold the moment"
)]
pub struct Moment {
    /// The julian day number times 86 400 000.
    jd: i64,
    /// The year, the month and the day.
    year: i64,
    month: i64,
    day: i64,
    /// The hour and the minute.
    hour: i64,
    minute: i64,
    /// The seconds, which carry the fraction.
    second: f64,
    /// The offset written after the time, in minutes.
    zone: i64,
    /// Whether `jd` says the moment.
    has_jd: bool,
    /// Whether the year, the month and the day say it.
    has_ymd: bool,
    /// Whether the hour, the minute and the second say it.
    has_hms: bool,
    /// How many days `floor` takes off, which is the day of a month the
    /// month does not have.
    floor: i64,
    /// Whether `second` holds a number the statement wrote rather than
    /// the seconds of a time.
    raw: bool,
    /// Whether the moment ran past what a four-digit year holds.
    error: bool,
    /// Whether the moment is written out with milliseconds.
    subsec: bool,
    /// Whether the moment is known to be UTC.
    utc: bool,
}

/// One field of a date or a time: how many digits it takes, the least
/// and the most it may be, and the byte that follows it.
struct Field {
    /// How many digits.
    digits: usize,
    /// The least it may be.
    least: i64,
    /// The most it may be.
    most: i64,
    /// What follows it, or nothing where it is the last field.
    after: Option<u8>,
}

/// One field, written as `getDigits` of `src/date.c` writes it.
const fn field(digits: usize, least: i64, most: i64, after: Option<u8>) -> Field {
    Field {
        digits,
        least,
        most,
        after,
    }
}

/// The values of the fields, or nothing where a field is not there or
/// stands outside what it may be.
fn read_fields(text: &[u8], fields: &[Field]) -> Option<Vec<i64>> {
    let mut out = Vec::new();
    let mut at = 0;
    for held in fields {
        let mut value: i64 = 0;
        for _ in 0..held.digits {
            let byte = *text.get(at)?;
            if !byte.is_ascii_digit() {
                return None;
            }
            value = value
                .checked_mul(10)?
                .checked_add(i64::from(byte.wrapping_sub(b'0')))?;
            at = at.checked_add(1)?;
        }
        if value < held.least || value > held.most {
            return None;
        }
        out.push(value);
        if let Some(after) = held.after {
            if *text.get(at)? != after {
                return None;
            }
            at = at.checked_add(1)?;
        }
    }
    Some(out)
}

/// The text with the spaces at its front taken off.
fn after_spaces(text: &[u8]) -> &[u8] {
    let mut held = text;
    while held.first().is_some_and(u8::is_ascii_whitespace) {
        held = held.get(1..).unwrap_or_default();
    }
    held
}

impl Moment {
    /// The moment the julian day says, which is 2000-01-01 where
    /// nothing says it.
    fn compute_jd(&mut self) {
        if self.has_jd {
            return;
        }
        let (mut year, mut month, day) = if self.has_ymd {
            (self.year, self.month, self.day)
        } else {
            (2000, 1, 1)
        };
        if !(-4713..=9999).contains(&year) || self.raw {
            *self = Moment {
                error: true,
                ..Moment::default()
            };
            return;
        }
        if month <= 2 {
            year = year.saturating_sub(1);
            month = month.saturating_add(12);
        }
        let a = year.saturating_add(4800).div_euclid(100);
        let b = 38i64.saturating_sub(a).saturating_add(a.div_euclid(4));
        let x1 = 36525i64
            .saturating_mul(year.saturating_add(4716))
            .div_euclid(100);
        let x2 = 306_001i64
            .saturating_mul(month.saturating_add(1))
            .div_euclid(10000);
        let days = integer_as_real(
            x1.saturating_add(x2)
                .saturating_add(day)
                .saturating_add(b)
                .saturating_sub(1524),
        ) - 0.5;
        self.jd = real_as_integer(days * 86_400_000.0);
        self.has_jd = true;
        if !self.has_hms {
            return;
        }
        self.jd = self
            .jd
            .saturating_add(self.hour.saturating_mul(3_600_000))
            .saturating_add(self.minute.saturating_mul(60_000))
            .saturating_add(real_as_integer(self.second * 1000.0 + 0.5));
        if self.zone == 0 {
            return;
        }
        self.jd = self.jd.saturating_sub(self.zone.saturating_mul(60_000));
        self.has_ymd = false;
        self.has_hms = false;
        self.zone = 0;
        self.utc = true;
    }

    /// The year, the month and the day the julian day says.
    fn compute_ymd(&mut self) {
        if self.has_ymd {
            return;
        }
        if !self.has_jd {
            self.year = 2000;
            self.month = 1;
            self.day = 1;
            self.has_ymd = true;
            return;
        }
        if !whole_day(self.jd) {
            *self = Moment {
                error: true,
                ..Moment::default()
            };
            return;
        }
        let days = self.jd.saturating_add(43_200_000).div_euclid(DAY);
        let alpha =
            real_as_integer((integer_as_real(days) + 32044.75) / 36524.25).saturating_sub(52);
        let shifted = days
            .saturating_add(1)
            .saturating_add(alpha)
            .saturating_sub(alpha.saturating_add(100).div_euclid(4))
            .saturating_add(25);
        let julian = shifted.saturating_add(1524);
        let century = real_as_integer((integer_as_real(julian) - 122.1) / 365.25);
        let counted = 36525i64.saturating_mul(century & 32767).div_euclid(100);
        let months = real_as_integer(integer_as_real(julian.saturating_sub(counted)) / 30.6001);
        let inside = real_as_integer(30.6001 * integer_as_real(months));
        self.day = julian.saturating_sub(counted).saturating_sub(inside);
        self.month = if months < 14 {
            months.saturating_sub(1)
        } else {
            months.saturating_sub(13)
        };
        self.year = if self.month > 2 {
            century.saturating_sub(4716)
        } else {
            century.saturating_sub(4715)
        };
        self.has_ymd = true;
    }

    /// The hour, the minute and the second the julian day says.
    fn compute_hms(&mut self) {
        if self.has_hms {
            return;
        }
        self.compute_jd();
        let day_ms = self.jd.saturating_add(43_200_000).rem_euclid(DAY);
        self.second = integer_as_real(day_ms.rem_euclid(60_000)) / 1000.0;
        let day_min = day_ms.div_euclid(60_000);
        self.minute = day_min.rem_euclid(60);
        self.hour = day_min.div_euclid(60);
        self.raw = false;
        self.has_hms = true;
    }

    /// Both, which is what a moment is written out from.
    fn compute_both(&mut self) {
        self.compute_ymd();
        self.compute_hms();
    }

    /// The year, the month, the day, the hour, the minute and the
    /// second taken back, which the modifiers that move a moment leave
    /// behind.
    const fn clear(&mut self) {
        self.has_ymd = false;
        self.has_hms = false;
        self.zone = 0;
    }

    /// How many days the day of the month runs past the month, which
    /// is `computeFloor`.
    fn compute_floor(&mut self) {
        // The months of 31 days, as a bit each, which is `0x15aa` of
        // `computeFloor`.
        let long = (1i64 << self.month.clamp(0, 12)) & 0x15aa != 0;
        self.floor = if self.day <= 28 || long {
            0
        } else if self.month != 2 {
            i64::from(self.day == 31)
        } else if self.year.rem_euclid(4) != 0
            || (self.year.rem_euclid(100) == 0 && self.year.rem_euclid(400) != 0)
        {
            self.day.saturating_sub(28)
        } else {
            self.day.saturating_sub(29)
        };
    }
}

/// Whether a julian day in milliseconds is one a four-digit year holds.
const fn whole_day(jd: i64) -> bool {
    jd >= 0 && jd <= LAST
}

/// The offset written after a time, which is `(+|-)HH:MM` or `Z`.
///
/// Answers how many bytes it took, or nothing where what follows the
/// time is neither an offset nor the end of the text.
fn read_zone(text: &[u8], moment: &mut Moment) -> Option<()> {
    let held = after_spaces(text);
    moment.zone = 0;
    let sign = match held.first() {
        None => return Some(()),
        Some(b'-') => -1,
        Some(b'+') => 1,
        Some(b'Z' | b'z') => {
            moment.utc = true;
            return after_spaces(held.get(1..).unwrap_or_default())
                .is_empty()
                .then_some(());
        }
        Some(_) => return None,
    };
    let rest = held.get(1..).unwrap_or_default();
    let read = read_fields(rest, &[field(2, 0, 14, Some(b':')), field(2, 0, 59, None)])?;
    let (hours, minutes) = (*read.first()?, *read.get(1)?);
    moment.zone = minutes
        .saturating_add(hours.saturating_mul(60))
        .saturating_mul(sign);
    if moment.zone == 0 {
        moment.utc = true;
    }
    after_spaces(rest.get(5..).unwrap_or_default())
        .is_empty()
        .then_some(())
}

/// `HH:MM`, `HH:MM:SS` and `HH:MM:SS.FFF`, with the offset after it.
fn read_time(text: &[u8], moment: &mut Moment) -> Option<()> {
    let read = read_fields(text, &[field(2, 0, 24, Some(b':')), field(2, 0, 59, None)])?;
    let (hour, minute) = (*read.first()?, *read.get(1)?);
    let mut rest = text.get(5..).unwrap_or_default();
    let mut second = 0.0;
    if rest.first() == Some(&b':') {
        let after = rest.get(1..).unwrap_or_default();
        let held = read_fields(after, &[field(2, 0, 59, None)])?;
        second = integer_as_real(*held.first()?);
        rest = after.get(2..).unwrap_or_default();
        if rest.first() == Some(&b'.') && rest.get(1).is_some_and(u8::is_ascii_digit) {
            rest = rest.get(1..).unwrap_or_default();
            let mut fraction = 0.0;
            let mut scale = 1.0;
            while let Some(byte) = rest.first().copied().filter(u8::is_ascii_digit) {
                fraction = fraction * 10.0 + f64::from(byte.wrapping_sub(b'0'));
                scale *= 10.0;
                rest = rest.get(1..).unwrap_or_default();
            }
            fraction /= scale;
            // A fraction of a second is written with three digits, so
            // what rounds past them is held back.
            if fraction > 0.999 {
                fraction = 0.999;
            }
            second += fraction;
        }
    }
    moment.has_jd = false;
    moment.raw = false;
    moment.has_hms = true;
    moment.hour = hour;
    moment.minute = minute;
    moment.second = second;
    read_zone(rest, moment)
}

/// `YYYY-MM-DD` with a time after it, which is `parseYyyyMmDd`.
fn read_date(text: &[u8], moment: &mut Moment) -> Option<()> {
    let (held, negative) = match text.first() {
        Some(b'-') => (text.get(1..).unwrap_or_default(), true),
        _ => (text, false),
    };
    let read = read_fields(
        held,
        &[
            field(4, 0, 14712, Some(b'-')),
            field(2, 1, 12, Some(b'-')),
            field(2, 1, 31, None),
        ],
    )?;
    let (year, month, day) = (*read.first()?, *read.get(1)?, *read.get(2)?);
    let mut rest = held.get(10..).unwrap_or_default();
    while rest
        .first()
        .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'T')
    {
        rest = rest.get(1..).unwrap_or_default();
    }
    if read_time(rest, moment).is_none() {
        if !rest.is_empty() {
            return None;
        }
        moment.has_hms = false;
    }
    moment.has_jd = false;
    moment.has_ymd = true;
    moment.year = if negative {
        year.saturating_neg()
    } else {
        year
    };
    moment.month = month;
    moment.day = day;
    moment.compute_floor();
    if moment.zone != 0 {
        moment.compute_jd();
    }
    Some(())
}

/// A number the statement wrote, which is a julian day where it stands
/// in the range of one and a unix time otherwise.
fn read_number(moment: &mut Moment, number: f64) {
    moment.second = number;
    moment.raw = true;
    if (0.0..5_373_484.5).contains(&number) {
        moment.jd = real_as_integer(number * 86_400_000.0 + 0.5);
        moment.has_jd = true;
    }
}

/// The moment a text says, which is a date, a time or a number.
fn read_moment(text: &[u8], moment: &mut Moment) -> Option<()> {
    if read_date(text, moment).is_some() {
        return Some(());
    }
    if read_time(text, moment).is_some() {
        return Some(());
    }
    // `now` and `subsec` without a moment before them both ask for the
    // clock, which this crate is given none of, so neither is read
    // here.
    // `sqlite3AtoF` answers the whole text or nothing, so a date the
    // fields refused is not read as the number it begins with.
    let held = crate::number::real(text);
    if !held.number() || !held.complete() {
        return None;
    }
    read_number(moment, held.value);
    Some(())
}

/// What a `NNN days` modifier moves a moment by: the word, how large
/// the number may be, and how many seconds one of them is, which is
/// `aXformType` of `src/date.c`.
const MOVES: [(&[u8], f64, f64); 6] = [
    (b"second", 4.6427e+14, 1.0),
    (b"minute", 7.7379e+12, 60.0),
    (b"hour", 1.2897e+11, 3600.0),
    (b"day", 5_373_485.0, 86400.0),
    (b"month", 176_546.0, 2_592_000.0),
    (b"year", 14713.0, 31_536_000.0),
];

/// Whether a number is the whole number `whole`, which `weekday N`
/// asks of the count it is given.
#[expect(
    clippy::float_cmp,
    reason = "the question is whether the number is whole, which is an exact comparison"
)]
fn whole_number(number: f64, whole: i64) -> bool {
    integer_as_real(whole) == number
}

/// How many years a month count carries, which `src/date.c` counts by
/// dividing toward zero as C does.
const fn carried(month: i64) -> i64 {
    if month > 0 {
        month.saturating_sub(1).wrapping_div(12)
    } else {
        month.saturating_sub(12).wrapping_div(12)
    }
}

/// Whether two texts are the same word, whatever their case.
const fn same(one: &[u8], other: &[u8]) -> bool {
    one.eq_ignore_ascii_case(other)
}

/// The moment with one modifier applied, or nothing where the modifier
/// is not one this crate reads.
///
/// `at` is which argument the modifier is, counting the moment as one:
/// `auto`, `julianday` and `unixepoch` are only read as the first.
fn modified(moment: &mut Moment, text: &[u8], at: usize) -> Option<()> {
    let first = text.first().copied()?.to_ascii_lowercase();
    match first {
        b'a' if same(text, b"auto") => {
            if at > 1 {
                return None;
            }
            auto(moment);
            Some(())
        }
        b'c' if same(text, b"ceiling") => {
            moment.compute_jd();
            moment.clear();
            moment.floor = 0;
            Some(())
        }
        b'f' if same(text, b"floor") => {
            moment.compute_jd();
            moment.jd = moment.jd.saturating_sub(moment.floor.saturating_mul(DAY));
            moment.clear();
            Some(())
        }
        b'j' if same(text, b"julianday") => {
            if at > 1 || !moment.has_jd || !moment.raw {
                return None;
            }
            moment.raw = false;
            Some(())
        }
        b'u' if same(text, b"unixepoch") => {
            if at > 1 || !moment.raw {
                return None;
            }
            let held = moment.second * 1000.0 + 210_866_760_000_000.0;
            if !(0.0..464_269_060_800_000.0).contains(&held) {
                return None;
            }
            moment.clear();
            moment.jd = real_as_integer(held + 0.5);
            moment.has_jd = true;
            moment.raw = false;
            Some(())
        }
        b'w' if text.len() > 8 && same(text.get(..8).unwrap_or_default(), b"weekday ") => {
            weekday(moment, text.get(8..).unwrap_or_default())
        }
        b's' if same(text, b"subsec") || same(text, b"subsecond") => {
            moment.subsec = true;
            Some(())
        }
        b's' if text.len() > 9 && same(text.get(..9).unwrap_or_default(), b"start of ") => {
            start_of(moment, text.get(9..).unwrap_or_default())
        }
        b'+' | b'-' | b'0'..=b'9' => moved(moment, text),
        _ => None,
    }
}

/// `weekday N`, which moves the moment forward to the next day `N` of
/// the week, Sunday being nought.
fn weekday(moment: &mut Moment, text: &[u8]) -> Option<()> {
    let read = crate::number::real(text);
    if !read.number() || !read.complete() {
        return None;
    }
    let wanted = real_as_integer(read.value);
    if !(0..=6).contains(&wanted) || !whole_number(read.value, wanted) {
        return None;
    }
    moment.compute_both();
    moment.zone = 0;
    moment.has_jd = false;
    moment.compute_jd();
    let mut held = moment
        .jd
        .saturating_add(129_600_000)
        .div_euclid(DAY)
        .rem_euclid(7);
    if held > wanted {
        held = held.saturating_sub(7);
    }
    moment.jd = moment
        .jd
        .saturating_add(wanted.saturating_sub(held).saturating_mul(DAY));
    moment.clear();
    Some(())
}

/// `start of day`, `start of month` and `start of year`.
fn start_of(moment: &mut Moment, text: &[u8]) -> Option<()> {
    if !moment.has_jd && !moment.has_ymd && !moment.has_hms {
        return None;
    }
    moment.compute_ymd();
    moment.has_hms = true;
    moment.hour = 0;
    moment.minute = 0;
    moment.second = 0.0;
    moment.raw = false;
    moment.zone = 0;
    moment.has_jd = false;
    if same(text, b"month") {
        moment.day = 1;
        return Some(());
    }
    if same(text, b"year") {
        moment.month = 1;
        moment.day = 1;
        return Some(());
    }
    same(text, b"day").then_some(())
}

/// A number the statement wrote, read as a julian day where it is one
/// and as a unix time otherwise, which is `autoAdjustDate`.
fn auto(moment: &mut Moment) {
    if !moment.raw || moment.has_jd {
        moment.raw = false;
        return;
    }
    // The seconds of -4713-11-24 12:00:00 and of 9999-12-31 23:59:59,
    // which is what a unix time may stand between.
    if !(-210_866_760_000.0..=253_402_300_799.0).contains(&moment.second) {
        return;
    }
    let held = moment.second * 1000.0 + 210_866_760_000_000.0;
    moment.clear();
    moment.jd = real_as_integer(held + 0.5);
    moment.has_jd = true;
    moment.raw = false;
}

/// `(+|-)NNN days` and the other ways of moving a moment: `(+|-)HH:MM`,
/// `(+|-)YYYY-MM-DD` and `(+|-)YYYY-MM-DD HH:MM`.
fn moved(moment: &mut Moment, text: &[u8]) -> Option<()> {
    let sign = text.first().copied()?;
    // The number ends at a colon, a space, or the dash of a
    // `(+|-)YYYY-MM-DD`.
    let mut end = 1;
    while let Some(byte) = text.get(end).copied() {
        if byte == b':' || byte.is_ascii_whitespace() {
            break;
        }
        if byte == b'-' {
            let after = text.get(1..).unwrap_or_default();
            if end == 5 && read_fields(after, &[field(4, 0, 14712, None)]).is_some() {
                break;
            }
            if end == 6 && read_fields(after, &[field(5, 0, 14712, None)]).is_some() {
                break;
            }
        }
        end = end.checked_add(1)?;
    }
    let read = crate::number::real(text.get(..end).unwrap_or_default());
    if !read.number() {
        return None;
    }
    let mut number = read.value;
    let mut rest = text;
    let mut at = end;
    if text.get(end) == Some(&b'-') {
        at = 2;
        rest = date_moved(moment, text, sign, end)?;
        if rest.is_empty() {
            return Some(());
        }
    }
    if rest.get(at) == Some(&b':') {
        return time_moved(moment, rest, sign);
    }
    let mut word = after_spaces(text.get(end..).unwrap_or_default());
    if word.len() < 3 || word.len() > 10 {
        return None;
    }
    if word
        .last()
        .is_some_and(|byte| byte.eq_ignore_ascii_case(&b's'))
    {
        word = word.get(..word.len().saturating_sub(1)).unwrap_or_default();
    }
    moment.compute_jd();
    let rounder = if number < 0.0 { -0.5 } else { 0.5 };
    moment.floor = 0;
    for (which, (name, limit, scale)) in MOVES.iter().enumerate() {
        if !same(word, name) || number <= -limit || number >= *limit {
            continue;
        }
        if which == 4 {
            moment.compute_both();
            moment.month = moment.month.saturating_add(real_as_integer(number));
            let carry = carried(moment.month);
            moment.year = moment.year.saturating_add(carry);
            moment.month = moment.month.saturating_sub(carry.saturating_mul(12));
            moment.compute_floor();
            moment.has_jd = false;
            number -= integer_as_real(real_as_integer(number));
        }
        if which == 5 {
            moment.compute_both();
            moment.year = moment.year.saturating_add(real_as_integer(number));
            moment.compute_floor();
            moment.has_jd = false;
            number -= integer_as_real(real_as_integer(number));
        }
        moment.compute_jd();
        moment.jd = moment
            .jd
            .saturating_add(real_as_integer(number * 1000.0 * scale + rounder));
        moment.clear();
        return Some(());
    }
    moment.clear();
    None
}

/// `(+|-)YYYY-MM-DD` and `(+|-)YYYYY-MM-DD`, which add or take away
/// years, months and days. Answers the text the time after it begins
/// at, which is empty where the modifier ends with the day.
fn date_moved<'a>(moment: &mut Moment, text: &'a [u8], sign: u8, end: usize) -> Option<&'a [u8]> {
    if sign != b'+' && sign != b'-' {
        return None;
    }
    let after = text.get(1..).unwrap_or_default();
    let (read, held) = if end == 5 {
        (
            read_fields(
                after,
                &[
                    field(4, 0, 14712, Some(b'-')),
                    field(2, 0, 11, Some(b'-')),
                    field(2, 0, 30, None),
                ],
            )?,
            text,
        )
    } else {
        (
            read_fields(
                after,
                &[
                    field(5, 0, 14712, Some(b'-')),
                    field(2, 0, 11, Some(b'-')),
                    field(2, 0, 30, None),
                ],
            )?,
            after,
        )
    };
    let (years, months, mut days) = (*read.first()?, *read.get(1)?, *read.get(2)?);
    moment.compute_both();
    moment.has_jd = false;
    if sign == b'-' {
        moment.year = moment.year.saturating_sub(years);
        moment.month = moment.month.saturating_sub(months);
        days = days.saturating_neg();
    } else {
        moment.year = moment.year.saturating_add(years);
        moment.month = moment.month.saturating_add(months);
    }
    let carry = carried(moment.month);
    moment.year = moment.year.saturating_add(carry);
    moment.month = moment.month.saturating_sub(carry.saturating_mul(12));
    moment.compute_floor();
    moment.compute_jd();
    moment.has_hms = false;
    moment.has_ymd = false;
    moment.jd = moment.jd.saturating_add(days.saturating_mul(DAY));
    if held.get(11).is_none_or(|byte| *byte == 0) {
        return Some(&[]);
    }
    if !held.get(11).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let time = held.get(12..).unwrap_or_default();
    read_fields(time, &[field(2, 0, 24, Some(b':')), field(2, 0, 59, None)])?;
    Some(time)
}

/// `(+|-)HH:MM`, `(+|-)HH:MM:SS` and `(+|-)HH:MM:SS.FFF`, which add or
/// take away hours, minutes and seconds.
fn time_moved(moment: &mut Moment, text: &[u8], sign: u8) -> Option<()> {
    let held = if text.first().is_some_and(u8::is_ascii_digit) {
        text
    } else {
        text.get(1..).unwrap_or_default()
    };
    let mut taken = Moment::default();
    read_time(held, &mut taken)?;
    taken.compute_jd();
    taken.jd = taken.jd.saturating_sub(43_200_000);
    let days = taken.jd.div_euclid(DAY);
    taken.jd = taken.jd.saturating_sub(days.saturating_mul(DAY));
    if sign == b'-' {
        taken.jd = taken.jd.saturating_neg();
    }
    moment.compute_jd();
    moment.clear();
    moment.jd = moment.jd.saturating_add(taken.jd);
    Some(())
}

/// The moment the arguments say: the first of them read as a moment,
/// and the ones after it as modifiers, which is `isDate`.
fn moment_of(args: &[Value]) -> Option<Moment> {
    let mut moment = Moment::default();
    let first = args.first()?;
    match first {
        Value::Int(number) => read_number(&mut moment, integer_as_real(*number)),
        Value::Real(number) => read_number(&mut moment, *number),
        Value::Text(text) | Value::Blob(text) => read_moment(text, &mut moment)?,
        Value::Null => return None,
    }
    for (at, held) in args.iter().enumerate().skip(1) {
        let text = match held {
            Value::Text(text) | Value::Blob(text) => text.clone(),
            Value::Null => return None,
            other => other.text()?,
        };
        modified(&mut moment, &text, at)?;
    }
    moment.compute_jd();
    if moment.error || !whole_day(moment.jd) {
        return None;
    }
    // A day of a month the month does not have is written out as the
    // day it runs on to: 2023-02-31 is 2023-03-03.
    if args.len() == 1 && moment.has_ymd && moment.day > 28 {
        moment.has_ymd = false;
    }
    Some(moment)
}

/// A number written with `width` digits, the ones in front of it being
/// nought.
fn padded(number: i64, width: usize) -> Vec<u8> {
    let negative = number < 0;
    let mut digits = crate::number::integer_text(number.saturating_abs());
    // The sign counts toward the width, which is what `%04d` writes:
    // -1 is `-001`.
    let room = if negative {
        width.saturating_sub(1)
    } else {
        width
    };
    while digits.len() < room {
        digits.insert(0, b'0');
    }
    if negative {
        digits.insert(0, b'-');
    }
    digits
}

/// The same, with spaces in front of it rather than noughts.
fn spaced(number: i64, width: usize) -> Vec<u8> {
    let mut digits = crate::number::integer_text(number);
    while digits.len() < width {
        digits.insert(0, b' ');
    }
    digits
}

/// The seconds of a moment written with three decimals, which is
/// `%06.3f` of `src/date.c`.
fn written_seconds(second: f64) -> Vec<u8> {
    written_millis(real_as_integer(second * 1000.0 + 0.5))
}

/// A count of milliseconds written as seconds with three decimals.
fn written_millis(millis: i64) -> Vec<u8> {
    let mut out = padded(millis.div_euclid(1000), 2);
    out.push(b'.');
    out.extend_from_slice(&padded(millis.rem_euclid(1000), 3));
    out
}

/// `YYYY-MM-DD` of a moment.
fn written_date(moment: &Moment) -> Vec<u8> {
    let mut out = Vec::new();
    if moment.year < 0 {
        out.push(b'-');
    }
    out.extend_from_slice(&padded(moment.year.saturating_abs(), 4));
    out.push(b'-');
    out.extend_from_slice(&padded(moment.month, 2));
    out.push(b'-');
    out.extend_from_slice(&padded(moment.day, 2));
    out
}

/// `HH:MM:SS`, with the milliseconds after it where the moment says so.
fn written_time(moment: &Moment) -> Vec<u8> {
    let mut out = padded(moment.hour, 2);
    out.push(b':');
    out.extend_from_slice(&padded(moment.minute, 2));
    out.push(b':');
    if moment.subsec {
        out.extend_from_slice(&written_seconds(moment.second));
    } else {
        out.extend_from_slice(&padded(real_as_integer(moment.second), 2));
    }
    out
}

/// The seconds since 1970, which `unixepoch` and `%s` both write.
const fn unix_seconds(moment: &Moment) -> i64 {
    moment
        .jd
        .div_euclid(1000)
        .saturating_sub(EPOCH.div_euclid(1000))
}

/// `date(TIME, MOD, ...)`.
#[must_use]
pub fn date(args: &[Value]) -> Value {
    let Some(mut moment) = moment_of(args) else {
        return Value::Null;
    };
    moment.compute_ymd();
    Value::Text(written_date(&moment))
}

/// `time(TIME, MOD, ...)`.
#[must_use]
pub fn time(args: &[Value]) -> Value {
    let Some(mut moment) = moment_of(args) else {
        return Value::Null;
    };
    moment.compute_hms();
    Value::Text(written_time(&moment))
}

/// `datetime(TIME, MOD, ...)`.
#[must_use]
pub fn datetime(args: &[Value]) -> Value {
    let Some(mut moment) = moment_of(args) else {
        return Value::Null;
    };
    moment.compute_both();
    let mut out = written_date(&moment);
    out.push(b' ');
    out.extend_from_slice(&written_time(&moment));
    Value::Text(out)
}

/// `julianday(TIME, MOD, ...)`.
#[must_use]
pub fn julianday(args: &[Value]) -> Value {
    let Some(mut moment) = moment_of(args) else {
        return Value::Null;
    };
    moment.compute_jd();
    Value::Real(integer_as_real(moment.jd) / 86_400_000.0)
}

/// `unixepoch(TIME, MOD, ...)`.
#[must_use]
pub fn unixepoch(args: &[Value]) -> Value {
    let Some(mut moment) = moment_of(args) else {
        return Value::Null;
    };
    moment.compute_jd();
    if moment.subsec {
        return Value::Real(integer_as_real(moment.jd.saturating_sub(EPOCH)) / 1000.0);
    }
    Value::Int(unix_seconds(&moment))
}

/// How many days the moment stands after the first of January, which
/// is `daysAfterJan01`.
fn after_january(moment: &Moment) -> i64 {
    let mut first = *moment;
    first.has_jd = false;
    first.month = 1;
    first.day = 1;
    first.compute_jd();
    moment
        .jd
        .saturating_sub(first.jd)
        .saturating_add(43_200_000)
        .div_euclid(DAY)
}

/// How many days the moment stands after the most recent Monday.
const fn after_monday(moment: &Moment) -> i64 {
    moment
        .jd
        .saturating_add(43_200_000)
        .div_euclid(DAY)
        .rem_euclid(7)
}

/// How many days the moment stands after the most recent Sunday.
const fn after_sunday(moment: &Moment) -> i64 {
    moment
        .jd
        .saturating_add(129_600_000)
        .div_euclid(DAY)
        .rem_euclid(7)
}

/// The moment of the Thursday of the same week, which the ISO year and
/// the ISO week are counted from.
fn thursday(moment: &Moment) -> Moment {
    let mut held = *moment;
    held.jd = held.jd.saturating_add(
        3i64.saturating_sub(after_monday(moment))
            .saturating_mul(DAY),
    );
    held.has_ymd = false;
    held.compute_ymd();
    held
}

/// `strftime(FORMAT, TIME, MOD, ...)`.
#[must_use]
pub fn strftime(args: &[Value]) -> Value {
    let Some(format) = args.first().and_then(Value::text) else {
        return Value::Null;
    };
    let Some(mut moment) = moment_of(args.get(1..).unwrap_or_default()) else {
        return Value::Null;
    };
    moment.compute_jd();
    moment.compute_both();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(byte) = format.get(at).copied() {
        at = at.saturating_add(1);
        if byte != b'%' {
            out.push(byte);
            continue;
        }
        // A format that ends with a `%` is one `strftime` answers
        // nothing for, which the `default` of its switch does.
        let Some(what) = format.get(at).copied() else {
            return Value::Null;
        };
        at = at.saturating_add(1);
        let Some(written) = converted(&moment, what) else {
            return Value::Null;
        };
        out.extend_from_slice(&written);
    }
    Value::Text(out)
}

/// What one conversion of a format writes, or nothing where the
/// conversion is not one `strftime` has.
fn converted(moment: &Moment, what: u8) -> Option<Vec<u8>> {
    let held = match what {
        b'd' => padded(moment.day, 2),
        b'e' => spaced(moment.day, 2),
        b'f' => written_seconds(moment.second),
        b'F' => written_date(moment),
        b'G' => padded(thursday(moment).year, 4),
        b'g' => padded(thursday(moment).year.rem_euclid(100), 2),
        b'H' => padded(moment.hour, 2),
        b'k' => spaced(moment.hour, 2),
        b'I' | b'l' => {
            let hour = match moment.hour {
                0 => 12,
                hour if hour > 12 => hour.saturating_sub(12),
                hour => hour,
            };
            if what == b'I' {
                padded(hour, 2)
            } else {
                spaced(hour, 2)
            }
        }
        b'j' => padded(after_january(moment).saturating_add(1), 3),
        b'J' => crate::fp::text(integer_as_real(moment.jd) / 86_400_000.0, 16),
        b'm' => padded(moment.month, 2),
        b'M' => padded(moment.minute, 2),
        b'p' => b"AM".to_vec(),
        b'P' => b"am".to_vec(),
        b'R' => {
            let mut written = padded(moment.hour, 2);
            written.push(b':');
            written.extend_from_slice(&padded(moment.minute, 2));
            written
        }
        b's' => {
            if moment.subsec {
                let millis = moment.jd.saturating_sub(EPOCH);
                let mut written = crate::number::integer_text(millis.div_euclid(1000));
                written.push(b'.');
                written.extend_from_slice(&padded(millis.rem_euclid(1000), 3));
                written
            } else {
                crate::number::integer_text(unix_seconds(moment))
            }
        }
        b'S' => padded(real_as_integer(moment.second), 2),
        b'T' => {
            let mut written = padded(moment.hour, 2);
            written.push(b':');
            written.extend_from_slice(&padded(moment.minute, 2));
            written.push(b':');
            written.extend_from_slice(&padded(real_as_integer(moment.second), 2));
            written
        }
        b'u' | b'w' => {
            let day = after_sunday(moment);
            let shown = if day == 0 && what == b'u' { 7 } else { day };
            crate::number::integer_text(shown)
        }
        b'U' => padded(
            after_january(moment)
                .saturating_sub(after_sunday(moment))
                .saturating_add(7)
                .div_euclid(7),
            2,
        ),
        b'V' => padded(
            after_january(&thursday(moment))
                .div_euclid(7)
                .saturating_add(1),
            2,
        ),
        b'W' => padded(
            after_january(moment)
                .saturating_sub(after_monday(moment))
                .saturating_add(7)
                .div_euclid(7),
            2,
        ),
        b'Y' => padded(moment.year, 4),
        b'%' => b"%".to_vec(),
        _ => return None,
    };
    Some(match what {
        b'p' if moment.hour >= 12 => b"PM".to_vec(),
        b'P' if moment.hour >= 12 => b"pm".to_vec(),
        _ => held,
    })
}

/// The julian day the difference of two moments is written against,
/// which is `1486995408 * 100000`: the first day of the year nought.
const NOUGHT: i64 = 148_699_540_800_000;

/// `timediff(ONE, OTHER)`: what must be added to the second moment to
/// reach the first, written as `(+|-)YYYY-MM-DD HH:MM:SS.SSS`.
///
/// `timediffFunc` moves the year of the second moment to the year of
/// the first, then its month to the month of the first, then walks it a
/// month at a time until it stands on the near side of the first; what
/// is left is written against the first day of the year nought, so the
/// day of the month it lands on, less one, is the count of days.
///
/// Walking the months costs O(n) in them.
#[must_use]
pub fn timediff(args: &[Value]) -> Value {
    let (Some(mut one), Some(mut other)) = (
        moment_of(args.get(..1).unwrap_or_default()),
        moment_of(args.get(1..2).unwrap_or_default()),
    ) else {
        return Value::Null;
    };
    one.compute_both();
    other.compute_both();
    let ahead = one.jd >= other.jd;
    let (sign, years, months) = walked(&mut other, &one, ahead);
    let rest = if ahead {
        one.jd.saturating_sub(other.jd)
    } else {
        other.jd.saturating_sub(one.jd)
    };
    // The years and the months are counted against the moment that was
    // walked, so what is left is a run of days and a time of day.
    let mut held = Moment {
        jd: rest.saturating_add(NOUGHT),
        has_jd: true,
        ..Moment::default()
    };
    held.compute_both();
    let mut out = alloc::vec![sign];
    out.extend_from_slice(&padded(years, 4));
    out.push(b'-');
    out.extend_from_slice(&padded(months, 2));
    out.push(b'-');
    out.extend_from_slice(&padded(held.day.saturating_sub(1), 2));
    out.push(b' ');
    out.extend_from_slice(&padded(held.hour, 2));
    out.push(b':');
    out.extend_from_slice(&padded(held.minute, 2));
    out.push(b':');
    out.extend_from_slice(&written_seconds(held.second));
    Value::Text(out)
}

/// The second moment walked to the first a year and then a month at a
/// time, with the sign, the years and the months it took, which is the
/// two halves of `timediffFunc`.
///
/// `ahead` says the first moment stands after the second.
///
/// Walking the months costs O(n) in them.
fn walked(other: &mut Moment, one: &Moment, ahead: bool) -> (u8, i64, i64) {
    let (sign, mut years) = if ahead {
        (b'+', one.year.saturating_sub(other.year))
    } else {
        (b'-', other.year.saturating_sub(one.year))
    };
    if years != 0 {
        other.year = one.year;
        other.has_jd = false;
        other.compute_jd();
    }
    let mut months = if ahead {
        one.month.saturating_sub(other.month)
    } else {
        other.month.saturating_sub(one.month)
    };
    if months < 0 {
        years = years.saturating_sub(1);
        months = months.saturating_add(12);
    }
    if months != 0 {
        other.month = one.month;
        other.has_jd = false;
        other.compute_jd();
    }
    while if ahead {
        one.jd < other.jd
    } else {
        one.jd > other.jd
    } {
        months = months.saturating_sub(1);
        if months < 0 {
            months = 11;
            years = years.saturating_sub(1);
        }
        if ahead {
            other.month = other.month.saturating_sub(1);
            if other.month < 1 {
                other.month = 12;
                other.year = other.year.saturating_sub(1);
            }
        } else {
            other.month = other.month.saturating_add(1);
            if other.month > 12 {
                other.month = 1;
                other.year = other.year.saturating_add(1);
            }
        }
        other.has_jd = false;
        other.compute_jd();
    }
    (sign, years, months)
}
