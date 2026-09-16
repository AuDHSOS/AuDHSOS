// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The aggregate functions, and what one group of rows accumulates.
//!
//! `src/func.c` is the document. An aggregate is stepped once per row,
//! which is O(n) over a group, and finished once. `sum` stays an integer
//! until one overflows and then carries on as a Kahan-Babuska-Neumaier
//! sum, which keeps the bits a plain running double would drop; `total`
//! answers the same number as a double and never refuses.
//!
//! `min` and `max` also say whether the row they were just stepped with
//! is the row the group's bare columns come from, which is what
//! `sqlite3SkipAccumulatorLoad` decides in the C.

use alloc::vec::Vec;

use crate::eval::Error;
use crate::value::{Collation, Value, apply_numeric, compare, integer_as_real};

/// A count as the whole number it is, which is what a real is worked
/// out from.
fn count_of(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// Where an integer stops being exact as a double, which is where the
/// Kahan sum splits one in two before adding it.
const EXACT: i64 = 4_503_599_627_370_496;

/// How much of an integer the low half of that split holds.
const LOW: i64 = 16_384;

/// Where a running sum stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Running {
    /// Still an integer, which is exact.
    Whole,
    /// A double, because one of the values was one.
    Double,
    /// A double, because an integer sum overflowed and nothing since
    /// has been a double. `sum` refuses this and `total` does not.
    Overflowed,
}

/// An aggregate this engine has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aggregate {
    /// `count(*)` and `count(X)`.
    Count,
    /// `sum(X)`.
    Sum,
    /// `total(X)`.
    Total,
    /// `avg(X)`.
    Avg,
    /// `min(X)`, which the scalar of the same name is not.
    Min,
    /// `max(X)`, likewise.
    Max,
    /// `group_concat(X)`, `group_concat(X,Y)` and `string_agg(X,Y)`.
    GroupConcat,
    /// `json_group_array(X)`.
    JsonGroupArray,
    /// `json_group_object(L,X)`.
    JsonGroupObject,
    /// `jsonb_group_array(X)`.
    JsonbGroupArray,
    /// `jsonb_group_object(L,X)`.
    JsonbGroupObject,
    /// `median(Y)`, which is `percentile(Y,50)`.
    Median,
    /// `percentile(Y,P)`, where `P` runs from nought to a hundred.
    Percentile,
    /// `percentile_cont(Y,P)`, where `P` runs from nought to one.
    PercentileCont,
    /// `percentile_disc(Y,P)`, which answers a value the group holds
    /// rather than one between two of them.
    PercentileDisc,
}

impl Aggregate {
    /// The name a message writes the aggregate under.
    const fn name(self) -> &'static [u8] {
        match self {
            Aggregate::Median => b"median",
            Aggregate::PercentileCont => b"percentile_cont",
            Aggregate::PercentileDisc => b"percentile_disc",
            _ => b"percentile",
        }
    }

    /// The largest the fraction argument may be, which `percentile`
    /// takes out of a hundred and the other two out of one.
    const fn largest(self) -> f64 {
        match self {
            Aggregate::Percentile => 100.0,
            _ => 1.0,
        }
    }

    /// That largest as a message writes it, which is `%.1f` of it.
    const fn largest_shown(self) -> &'static [u8] {
        match self {
            Aggregate::Percentile => b"100.0",
            _ => b"1.0",
        }
    }

    /// Whether the aggregate answers a value the group holds rather
    /// than one between two of them.
    const fn discrete(self) -> bool {
        matches!(self, Aggregate::PercentileDisc)
    }
}

/// One row of the table: a name, how many arguments the aggregate takes
/// under it, and which aggregate it is.
struct Entry {
    /// The name, in lower case.
    name: &'static [u8],
    /// The fewest arguments it takes.
    least: usize,
    /// The most.
    most: usize,
    /// Which aggregate.
    aggregate: Aggregate,
}

/// The table, which is the `WAGGREGATE` half of `aBuiltinFunc`.
const TABLE: &[Entry] = &[
    Entry {
        name: b"avg",
        least: 1,
        most: 1,
        aggregate: Aggregate::Avg,
    },
    Entry {
        name: b"count",
        least: 0,
        most: 1,
        aggregate: Aggregate::Count,
    },
    Entry {
        name: b"group_concat",
        least: 1,
        most: 2,
        aggregate: Aggregate::GroupConcat,
    },
    Entry {
        name: b"json_group_array",
        least: 1,
        most: 1,
        aggregate: Aggregate::JsonGroupArray,
    },
    Entry {
        name: b"json_group_object",
        least: 2,
        most: 2,
        aggregate: Aggregate::JsonGroupObject,
    },
    Entry {
        name: b"jsonb_group_array",
        least: 1,
        most: 1,
        aggregate: Aggregate::JsonbGroupArray,
    },
    Entry {
        name: b"jsonb_group_object",
        least: 2,
        most: 2,
        aggregate: Aggregate::JsonbGroupObject,
    },
    Entry {
        name: b"max",
        least: 1,
        most: 1,
        aggregate: Aggregate::Max,
    },
    Entry {
        name: b"median",
        least: 1,
        most: 1,
        aggregate: Aggregate::Median,
    },
    Entry {
        name: b"min",
        least: 1,
        most: 1,
        aggregate: Aggregate::Min,
    },
    Entry {
        name: b"percentile",
        least: 2,
        most: 2,
        aggregate: Aggregate::Percentile,
    },
    Entry {
        name: b"percentile_cont",
        least: 2,
        most: 2,
        aggregate: Aggregate::PercentileCont,
    },
    Entry {
        name: b"percentile_disc",
        least: 2,
        most: 2,
        aggregate: Aggregate::PercentileDisc,
    },
    Entry {
        name: b"string_agg",
        least: 2,
        most: 2,
        aggregate: Aggregate::GroupConcat,
    },
    Entry {
        name: b"sum",
        least: 1,
        most: 1,
        aggregate: Aggregate::Sum,
    },
    Entry {
        name: b"total",
        least: 1,
        most: 1,
        aggregate: Aggregate::Total,
    },
];

/// The aggregate `name` names, taking `count` arguments, or nothing
/// where no aggregate does.
///
/// `min` and `max` are aggregates with one argument and scalars with
/// more, so the count is part of the question and a wrong one is not an
/// error here.
#[must_use]
pub fn lookup(name: &[u8], count: usize) -> Option<Aggregate> {
    TABLE
        .iter()
        .find(|entry| {
            name.eq_ignore_ascii_case(entry.name) && count >= entry.least && count <= entry.most
        })
        .map(|entry| entry.aggregate)
}

/// Whether the table holds an aggregate of that name, whatever number
/// of arguments it takes.
///
/// Reading the table costs O(n) in its rows.
#[must_use]
pub fn named(name: &[u8]) -> bool {
    TABLE
        .iter()
        .any(|entry| name.eq_ignore_ascii_case(entry.name))
}

/// What one group has accumulated.
#[derive(Clone, Debug)]
pub struct Accumulator {
    /// Which aggregate it is.
    which: Aggregate,
    /// The values already stepped, for a `DISTINCT` aggregate.
    seen: Option<Vec<Value>>,
    /// How many rows were stepped with something that is not `NULL`.
    count: i64,
    /// The running sum while it is still an integer.
    whole: i64,
    /// The running sum once it is a double.
    sum: f64,
    /// What that sum has dropped so far, which is added back at the end.
    error: f64,
    /// Where the running sum stands.
    running: Running,
    /// The least or the greatest value so far.
    best: Option<Value>,
    /// What `group_concat` has put together.
    text: Vec<u8>,
    /// The elements `json_group_array` and `json_group_object` have
    /// put together, in the binary form of JSON.
    held: Vec<u8>,
    /// Whether it has put anything together, which is not the same as
    /// the text being empty.
    any: bool,
    /// Whether the last step left the group's bare columns alone.
    skipped: bool,
    /// The values a percentile aggregate has been given, as reals.
    reals: Vec<f64>,
    /// The fraction the first row of the group wrote, which every row
    /// after it writes again.
    fraction: Option<f64>,
}

impl Accumulator {
    /// A group that has seen nothing yet.
    #[must_use]
    pub const fn new(which: Aggregate, distinct: bool) -> Self {
        Accumulator {
            which,
            seen: if distinct { Some(Vec::new()) } else { None },
            count: 0,
            whole: 0,
            sum: 0.0,
            error: 0.0,
            running: Running::Whole,
            best: None,
            text: Vec::new(),
            held: Vec::new(),
            any: false,
            skipped: false,
            reals: Vec::new(),
            fraction: None,
        }
    }

    /// Whether this aggregate decides where the group's bare columns
    /// come from, which `min` and `max` do and nothing else does.
    #[must_use]
    pub const fn magnet(&self) -> bool {
        matches!(self.which, Aggregate::Min | Aggregate::Max)
    }

    /// Whether the last step told the group to keep the row it was
    /// stepped with.
    #[must_use]
    pub const fn kept(&self) -> bool {
        !self.skipped
    }

    /// Adds one row, whose arguments are `args` and whose first argument
    /// compares under `collation`.
    ///
    /// # Errors
    ///
    /// [`Error::Json`] where a JSON aggregate was given a value JSON
    /// cannot hold.
    pub fn step(
        &mut self,
        args: &[Value],
        carried: &[bool],
        collation: Collation,
    ) -> Result<(), Error> {
        let first = args.first().cloned().unwrap_or(Value::Null);
        if let Some(seen) = &mut self.seen {
            // A row a `DISTINCT` aggregate has already seen jumps over
            // the step, and over what the step would have said about
            // the bare columns.
            if seen
                .iter()
                .any(|kept| compare(kept, &first, collation) == core::cmp::Ordering::Equal)
            {
                return Ok(());
            }
            seen.push(first.clone());
        }
        self.skipped = false;
        match self.which {
            Aggregate::Count => {
                if args.is_empty() || first != Value::Null {
                    self.count = self.count.saturating_add(1);
                }
            }
            Aggregate::Sum | Aggregate::Total | Aggregate::Avg => self.add(&first),
            Aggregate::Min | Aggregate::Max => self.against(first, collation),
            Aggregate::GroupConcat => self.append(&first, args),
            Aggregate::JsonGroupArray
            | Aggregate::JsonGroupObject
            | Aggregate::JsonbGroupArray
            | Aggregate::JsonbGroupObject => self.collect(args, carried)?,
            Aggregate::Median
            | Aggregate::Percentile
            | Aggregate::PercentileCont
            | Aggregate::PercentileDisc => self.percentile(args)?,
        }
        Ok(())
    }

    /// One row of a percentile aggregate, which is `percentStep`: the
    /// fraction is read first and is the same for every row of the
    /// group, and the value is kept where it is a number.
    ///
    /// # Errors
    ///
    /// [`Error::Fraction`] where the fraction is not a number between
    /// nought and the largest the aggregate takes,
    /// [`Error::Fractions`] where two rows of the group wrote different
    /// ones, [`Error::NotNumeric`] for a value that is not a number,
    /// and [`Error::Infinite`] for an infinity.
    fn percentile(&mut self, args: &[Value]) -> Result<(), Error> {
        let name = self.which.name().to_vec();
        let largest = self.which.largest();
        let shown = self.which.largest_shown().to_vec();
        // `median(Y)` is `percentile(Y,50)`, which writes the fraction
        // itself.
        let fraction = if self.which == Aggregate::Median {
            0.5
        } else {
            let mut given = args.get(1).cloned().unwrap_or(Value::Null);
            apply_numeric(&mut given, true);
            let number = match given {
                Value::Int(whole) => integer_as_real(whole),
                Value::Real(number) => number,
                _ => return Err(Error::Fraction(name, shown)),
            };
            let held = number / largest;
            if !(0.0..=1.0).contains(&held) {
                return Err(Error::Fraction(name, shown));
            }
            held
        };
        match self.fraction {
            None => self.fraction = Some(fraction),
            // Two fractions that differ by a thousandth or less are one
            // fraction, which is `percentSameValue`.
            Some(held) if (held - fraction).abs() <= 0.001 => {}
            Some(_) => return Err(Error::Fractions(name)),
        }
        let value = args.first().cloned().unwrap_or(Value::Null);
        if value == Value::Null {
            return Ok(());
        }
        let number = match value {
            Value::Int(whole) => integer_as_real(whole),
            Value::Real(number) => number,
            _ => return Err(Error::NotNumeric(name)),
        };
        if number.is_infinite() {
            return Err(Error::Infinite(name));
        }
        self.reals.push(number);
        Ok(())
    }

    /// What a percentile aggregate answers, which is `percentCompute`:
    /// the value the fraction names among the values the group held, in
    /// order, or nothing where the group held none.
    fn percentile_of(&self) -> Value {
        let mut held = self.reals.clone();
        held.sort_by(|left, right| {
            left.partial_cmp(right)
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let Some(last) = held.len().checked_sub(1) else {
            return Value::Null;
        };
        let place = self.fraction.unwrap_or(0.0) * integer_as_real(count_of(last));
        let first = usize::try_from(crate::value::real_as_integer(place)).unwrap_or(0);
        let one = held.get(first).copied().unwrap_or(0.0);
        if self.which.discrete() {
            return Value::Real(one);
        }
        // The value lies between the two the fraction falls between,
        // and on the first where the fraction names it outright, which
        // is what a distance of nought answers.
        let second = if first == last {
            first
        } else {
            first.saturating_add(1)
        };
        let other = held.get(second).copied().unwrap_or(one);
        Value::Real(one + (other - one) * (place - integer_as_real(count_of(first))))
    }

    /// One number of a sum, which is `sumStep`.
    fn add(&mut self, value: &Value) {
        let mut numeric = value.clone();
        // `sqlite3_value_numeric_type` converts text that is a whole
        // number and leaves everything else, a blob included, alone.
        if matches!(numeric, Value::Text(_)) {
            apply_numeric(&mut numeric, false);
        }
        if numeric == Value::Null {
            return;
        }
        self.count = self.count.saturating_add(1);
        let whole = if let Value::Int(number) = numeric {
            Some(number)
        } else {
            None
        };
        match (self.running, whole) {
            (Running::Whole, Some(number)) => {
                if let Some(sum) = self.whole.checked_add(number) {
                    self.whole = sum;
                } else {
                    self.begin(Running::Overflowed);
                    self.integer(number);
                }
            }
            (Running::Whole, None) => {
                self.begin(Running::Double);
                self.real(numeric.to_real());
            }
            (_, Some(number)) => self.integer(number),
            (_, None) => {
                // A double after an overflow is no longer an overflow:
                // the sum is a double from here and answers as one.
                self.running = Running::Double;
                self.real(numeric.to_real());
            }
        }
    }

    /// Starts the double sum from the integer one, which is
    /// `kahanBabuskaNeumaierInit`.
    fn begin(&mut self, running: Running) {
        self.running = running;
        let value = self.whole;
        if value <= -EXACT || value >= EXACT {
            let low = value.wrapping_rem(LOW);
            self.sum = integer_as_real(value.wrapping_sub(low));
            self.error = integer_as_real(low);
        } else {
            self.sum = integer_as_real(value);
            self.error = 0.0;
        }
    }

    /// Adds an integer to the double sum, in two halves where one half
    /// would not be exact.
    fn integer(&mut self, value: i64) {
        if value <= -EXACT || value >= EXACT {
            let low = value.wrapping_rem(LOW);
            self.real(integer_as_real(value.wrapping_sub(low)));
            self.real(integer_as_real(low));
        } else {
            self.real(integer_as_real(value));
        }
    }

    /// Adds a double, keeping what the addition dropped.
    fn real(&mut self, value: f64) {
        let sum = self.sum;
        let total = sum + value;
        self.error += if sum.abs() > value.abs() {
            (sum - total) + value
        } else {
            (value - total) + sum
        };
        self.sum = total;
    }

    /// One value against the best so far, which is `minmaxStep`.
    fn against(&mut self, value: Value, collation: Collation) {
        if value == Value::Null {
            self.skipped = self.best.is_some();
            return;
        }
        let Some(best) = &self.best else {
            self.best = Some(value);
            return;
        };
        let order = compare(best, &value, collation);
        let take = if self.which == Aggregate::Max {
            order == core::cmp::Ordering::Less
        } else {
            order == core::cmp::Ordering::Greater
        };
        if take {
            self.best = Some(value);
        } else {
            self.skipped = true;
        }
    }

    /// One value onto the text, which is `groupConcatStep`.
    ///
    /// The separator goes before the value and not after it, so the
    /// first value is the one that does not carry one.
    fn append(&mut self, value: &Value, args: &[Value]) {
        if *value == Value::Null {
            return;
        }
        if self.any {
            let separator = match args.get(1) {
                None => Some(b",".to_vec()),
                Some(written) => written.text(),
            };
            if let Some(separator) = separator {
                self.text.extend_from_slice(&separator);
            }
        }
        self.any = true;
        // A value that is not `NULL` always has text.
        self.text
            .extend_from_slice(&value.text().unwrap_or_default());
    }

    /// The double sum with what it dropped added back, unless what it
    /// dropped is not a number.
    fn total(&self) -> f64 {
        if self.running == Running::Whole {
            integer_as_real(self.whole)
        } else if self.error.is_finite() {
            self.sum + self.error
        } else {
            self.sum
        }
    }

    /// One row of `json_group_array` or `json_group_object`, written
    /// into what the group holds.
    fn collect(&mut self, args: &[Value], carried: &[bool]) -> Result<(), Error> {
        let object = matches!(
            self.which,
            Aggregate::JsonGroupObject | Aggregate::JsonbGroupObject
        );
        if !self.held.is_empty() {
            self.held.push(b',');
        }
        if object {
            let label = args.first().and_then(Value::text).unwrap_or_default();
            crate::json::write_raw(&label, &mut self.held);
            self.held.push(b':');
        }
        let at = usize::from(object);
        let value = args.get(at).unwrap_or(&Value::Null);
        let json = carried.get(at).copied().unwrap_or(false);
        crate::json::write_value(value, json, &mut self.held).map_err(Error::Json)
    }

    /// The text of what `json_group_array` or `json_group_object` put
    /// together.
    fn collected(&self) -> Result<Value, Error> {
        let object = matches!(
            self.which,
            Aggregate::JsonGroupObject | Aggregate::JsonbGroupObject
        );
        let mut text = Vec::new();
        text.push(if object { b'{' } else { b'[' });
        text.extend_from_slice(&self.held);
        text.push(if object { b'}' } else { b']' });
        if !matches!(
            self.which,
            Aggregate::JsonbGroupArray | Aggregate::JsonbGroupObject
        ) {
            return Ok(Value::Text(text));
        }
        // The binary form is what the text answers when it is read
        // again, which is `jsonArrayCompute` under `JSON_BLOB`.
        let (blob, _) = crate::json::read(&text)
            .ok()
            .ok_or(Error::Json(crate::json::Refused::Malformed))?;
        Ok(Value::Blob(blob))
    }

    /// What the group answers.
    ///
    /// # Errors
    ///
    /// [`Error::Overflow`] where an integer `sum` overflowed and no
    /// double came after it to make the sum a double, and
    /// [`Error::Json`] where what a JSON aggregate put together is not
    /// JSON.
    pub fn finish(&self) -> Result<Value, Error> {
        Ok(match self.which {
            Aggregate::Median
            | Aggregate::Percentile
            | Aggregate::PercentileCont
            | Aggregate::PercentileDisc => self.percentile_of(),
            Aggregate::Count => Value::Int(self.count),
            Aggregate::Sum => match self.running {
                _ if self.count == 0 => Value::Null,
                Running::Whole => Value::Int(self.whole),
                Running::Double => Value::Real(self.total()),
                Running::Overflowed => return Err(Error::Overflow),
            },
            Aggregate::Avg => {
                if self.count == 0 {
                    Value::Null
                } else {
                    Value::Real(self.total() / integer_as_real(self.count))
                }
            }
            Aggregate::Total => Value::Real(self.total()),
            Aggregate::Min | Aggregate::Max => self.best.clone().unwrap_or(Value::Null),
            Aggregate::GroupConcat => {
                if self.any {
                    Value::Text(self.text.clone())
                } else {
                    Value::Null
                }
            }
            Aggregate::JsonGroupArray
            | Aggregate::JsonGroupObject
            | Aggregate::JsonbGroupArray
            | Aggregate::JsonbGroupObject => self.collected()?,
        })
    }
}
