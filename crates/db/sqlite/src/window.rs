// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Window functions: which rows of a partition one call reads, and what
//! the eleven built-in ones answer.
//!
//! `src/window.c` is the document. The rows of a partition are sorted
//! once, which is O(n log n), and the frame of each row is found by
//! halving, which is O(log n); an aggregate over a frame is stepped once
//! per row of the frame, so a statement that aggregates over a moving
//! frame is O(n^2) over a partition where the C library is O(n).

use alloc::vec::Vec;

use crate::agg::Aggregate;
use crate::value::integer_as_real;

/// A window function this engine has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Which {
    /// An aggregate used as a window function, which reads the frame.
    Aggregate(Aggregate),
    /// `row_number()`: which row of the partition, counted from one.
    RowNumber,
    /// `rank()`: one more than the number of rows the row's peers
    /// follow.
    Rank,
    /// `dense_rank()`: which group of peers, counted from one.
    DenseRank,
    /// `percent_rank()`: the rank less one over the rows less one.
    PercentRank,
    /// `cume_dist()`: the rows up to and including the row's peers over
    /// the rows of the partition.
    CumeDist,
    /// `ntile(N)`: which of `N` groups of nearly equal size.
    Ntile,
    /// `lag(X[, offset[, default]])`: the row that far before this one.
    Lag,
    /// `lead(X[, offset[, default]])`: the row that far after this one.
    Lead,
    /// `first_value(X)`: the first row of the frame.
    FirstValue,
    /// `last_value(X)`: the last row of the frame.
    LastValue,
    /// `nth_value(X, N)`: the `N`th row of the frame, counted from one.
    NthValue,
}

impl Which {
    /// Whether a `FILTER` may precede it, which `sqlite3WindowRewrite`
    /// allows for an aggregate and refuses for the rest.
    #[must_use]
    pub const fn filtered(self) -> bool {
        matches!(self, Which::Aggregate(_))
    }
}

/// One row of the table: a name, the fewest and the most arguments it
/// takes, and which function it is.
struct Entry {
    /// The name, in lower case.
    name: &'static [u8],
    /// The fewest arguments it takes.
    least: usize,
    /// The most.
    most: usize,
    /// Which function.
    which: Which,
}

/// The table, which is `aWindowFuncs` of `src/window.c`.
const TABLE: &[Entry] = &[
    Entry {
        name: b"row_number",
        least: 0,
        most: 0,
        which: Which::RowNumber,
    },
    Entry {
        name: b"rank",
        least: 0,
        most: 0,
        which: Which::Rank,
    },
    Entry {
        name: b"dense_rank",
        least: 0,
        most: 0,
        which: Which::DenseRank,
    },
    Entry {
        name: b"percent_rank",
        least: 0,
        most: 0,
        which: Which::PercentRank,
    },
    Entry {
        name: b"cume_dist",
        least: 0,
        most: 0,
        which: Which::CumeDist,
    },
    Entry {
        name: b"ntile",
        least: 1,
        most: 1,
        which: Which::Ntile,
    },
    Entry {
        name: b"lag",
        least: 1,
        most: 3,
        which: Which::Lag,
    },
    Entry {
        name: b"lead",
        least: 1,
        most: 3,
        which: Which::Lead,
    },
    Entry {
        name: b"first_value",
        least: 1,
        most: 1,
        which: Which::FirstValue,
    },
    Entry {
        name: b"last_value",
        least: 1,
        most: 1,
        which: Which::LastValue,
    },
    Entry {
        name: b"nth_value",
        least: 2,
        most: 2,
        which: Which::NthValue,
    },
];

/// Which window function `name` taking `count` arguments is, or nothing
/// where it is neither one of the eleven nor an aggregate.
#[must_use]
pub fn lookup(name: &[u8], count: usize) -> Option<Which> {
    let found = TABLE.iter().find(|entry| {
        name.eq_ignore_ascii_case(entry.name) && count >= entry.least && count <= entry.most
    });
    match found {
        Some(entry) => Some(entry.which),
        None => crate::agg::lookup(name, count).map(Which::Aggregate),
    }
}

/// Where a frame measured in rows or in groups begins or ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// `UNBOUNDED PRECEDING`.
    Start,
    /// `<count> PRECEDING`.
    Preceding(usize),
    /// `CURRENT ROW`.
    Current,
    /// `<count> FOLLOWING`.
    Following(usize),
    /// `UNBOUNDED FOLLOWING`.
    End,
}

/// A run of rows of a partition, the end being one past the last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    /// The first row.
    pub start: usize,
    /// One past the last.
    pub end: usize,
}

impl Span {
    /// The run, with an end before its start read as no rows at all.
    #[must_use]
    pub const fn of(start: usize, end: usize) -> Self {
        Span {
            start,
            end: if end < start { start } else { end },
        }
    }
}

/// Where the rows of one partition that share their order terms begin
/// and end.
#[derive(Clone, Debug, Default)]
pub struct Peers {
    /// Which group each row belongs to, counted from zero.
    of: Vec<usize>,
    /// Where each group begins.
    starts: Vec<usize>,
    /// How many rows the partition holds.
    rows: usize,
}

impl Peers {
    /// The groups of a partition of `rows` rows, where `same(at)` says
    /// whether the row after `at` shares its order terms with `at`.
    pub fn new(rows: usize, same: impl Fn(usize) -> bool) -> Self {
        let mut of = Vec::with_capacity(rows);
        let mut starts = Vec::new();
        let mut group = 0_usize;
        for at in 0..rows {
            if at == 0 {
                starts.push(0);
            } else if !same(at.saturating_sub(1)) {
                group = group.saturating_add(1);
                starts.push(at);
            }
            of.push(group);
        }
        Peers { of, starts, rows }
    }

    /// How many rows the partition holds.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Which group the row at `at` belongs to.
    #[must_use]
    pub fn group(&self, at: usize) -> usize {
        self.of.get(at).copied().unwrap_or(0)
    }

    /// Where a group begins, which is the end of the partition for a
    /// group past the last.
    #[must_use]
    pub fn start(&self, group: usize) -> usize {
        self.starts.get(group).copied().unwrap_or(self.rows)
    }

    /// One past the last row of a group.
    #[must_use]
    pub fn end(&self, group: usize) -> usize {
        self.start(group.saturating_add(1))
    }

    /// The rows that share their order terms with the row at `at`.
    #[must_use]
    pub fn of_row(&self, at: usize) -> Span {
        let group = self.group(at);
        Span::of(self.start(group), self.end(group))
    }
}

/// The frame of the row at `at`, measured in rows.
#[must_use]
pub fn rows_frame(start: Edge, end: Edge, at: usize, rows: usize) -> Span {
    let from = match start {
        Edge::Start => 0,
        Edge::Preceding(count) => at.saturating_sub(count),
        Edge::Current => at,
        Edge::Following(count) => at.saturating_add(count).min(rows),
        Edge::End => rows,
    };
    let to = match end {
        Edge::Start => 0,
        Edge::Preceding(count) if count > at => 0,
        Edge::Preceding(count) => at.saturating_sub(count).saturating_add(1),
        Edge::Current => at.saturating_add(1).min(rows),
        Edge::Following(count) => at.saturating_add(count).saturating_add(1).min(rows),
        Edge::End => rows,
    };
    Span::of(from, to)
}

/// The frame of the row at `at`, measured in groups of peers.
#[must_use]
pub fn groups_frame(start: Edge, end: Edge, at: usize, peers: &Peers) -> Span {
    let group = peers.group(at);
    let from = match start {
        Edge::Start => 0,
        Edge::Preceding(count) => peers.start(group.saturating_sub(count)),
        Edge::Current => peers.start(group),
        Edge::Following(count) => peers.start(group.saturating_add(count)),
        Edge::End => peers.rows(),
    };
    let to = match end {
        Edge::Start => 0,
        Edge::Preceding(count) if count > group => 0,
        Edge::Preceding(count) => peers.end(group.saturating_sub(count)),
        Edge::Current => peers.end(group),
        Edge::Following(count) => peers.end(group.saturating_add(count)),
        Edge::End => peers.rows(),
    };
    Span::of(from, to)
}

/// The first row of `rows` for which `holds` is true, the rows before it
/// being the rows for which `holds` is false, found by halving in
/// O(log n).
pub fn first_true(rows: usize, holds: impl Fn(usize) -> bool) -> usize {
    let mut low = 0_usize;
    let mut high = rows;
    while low < high {
        let middle = low.saturating_add(high.saturating_sub(low) / 2);
        if holds(middle) {
            high = middle;
        } else {
            low = middle.saturating_add(1);
        }
    }
    low
}

/// The rows of a frame, with the rows `EXCLUDE` leaves out taken away.
#[must_use]
pub fn kept(span: Span, exclude: crate::ast::Exclude, at: usize, peers: &Peers) -> Vec<usize> {
    let group = peers.of_row(at);
    (span.start..span.end)
        .filter(|row| match exclude {
            crate::ast::Exclude::NoOthers => true,
            crate::ast::Exclude::CurrentRow => *row != at,
            crate::ast::Exclude::Group => *row < group.start || *row >= group.end,
            crate::ast::Exclude::Ties => *row == at || *row < group.start || *row >= group.end,
        })
        .collect()
}

/// `row_number()`.
#[must_use]
pub fn row_number(at: usize) -> i64 {
    i64::try_from(at.saturating_add(1)).unwrap_or(i64::MAX)
}

/// `rank()`.
#[must_use]
pub fn rank(at: usize, peers: &Peers) -> i64 {
    i64::try_from(peers.of_row(at).start.saturating_add(1)).unwrap_or(i64::MAX)
}

/// `dense_rank()`.
#[must_use]
pub fn dense_rank(at: usize, peers: &Peers) -> i64 {
    i64::try_from(peers.group(at).saturating_add(1)).unwrap_or(i64::MAX)
}

/// `percent_rank()`, which is zero for a partition of one row rather
/// than a division by zero.
#[must_use]
pub fn percent_rank(at: usize, peers: &Peers) -> f64 {
    let rows = peers.rows();
    if rows <= 1 {
        return 0.0;
    }
    let below = integer_as_real(rank(at, peers).saturating_sub(1));
    let over = integer_as_real(i64::try_from(rows.saturating_sub(1)).unwrap_or(i64::MAX));
    below / over
}

/// `cume_dist()`.
#[must_use]
pub fn cume_dist(at: usize, peers: &Peers) -> f64 {
    let rows = peers.rows();
    let upto = integer_as_real(i64::try_from(peers.of_row(at).end).unwrap_or(i64::MAX));
    upto / integer_as_real(i64::try_from(rows).unwrap_or(i64::MAX))
}

/// `ntile(tiles)`, where `tiles` is at least one.
#[must_use]
pub fn ntile(at: usize, rows: usize, tiles: usize) -> i64 {
    // The first `wide` groups hold one row more than the rest, which is
    // how `sqlite3WindowCodeStep` splits a partition that does not
    // divide evenly.
    let size = rows.checked_div(tiles).unwrap_or(0);
    let wide = rows.saturating_sub(size.saturating_mul(tiles));
    let held = wide.saturating_mul(size.saturating_add(1));
    let tile = if at < held {
        at.checked_div(size.saturating_add(1)).unwrap_or(0)
    } else {
        let over = at.saturating_sub(held);
        wide.saturating_add(over.checked_div(size).unwrap_or(0))
    };
    i64::try_from(tile.saturating_add(1)).unwrap_or(i64::MAX)
}
