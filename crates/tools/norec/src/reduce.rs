// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Shrinking a case that found something to a case a reader can follow.
//!
//! Every candidate is smaller than what it came from and is kept only when
//! the engine still disagrees with itself over it, so what comes out shows
//! the same bug. A candidate that no longer parses — a predicate that
//! names a table a dropped join removed — is answered `Failed` by the
//! engine and is dropped for that reason, with no check of its own.

use crate::case::{Case, CountStyle};
use crate::engine::Engine;
use crate::error::Error;
use crate::oracle;

/// The smallest case reached within `budget` engine runs. O(budget) runs;
/// each accepted candidate restarts the search from the smaller case.
pub(crate) fn reduce(
    engine: &mut dyn Engine,
    case: &Case,
    style: CountStyle,
    budget: u32,
) -> Result<Case, Error> {
    let mut best = case.clone();
    let mut left = budget;
    loop {
        let mut shrank = false;
        for candidate in candidates(&best) {
            if left == 0 {
                return Ok(best);
            }
            if candidate.size() >= best.size() {
                continue;
            }
            left = left.saturating_sub(1);
            if oracle::mismatches(engine, &candidate, style)? {
                best = candidate;
                shrank = true;
                break;
            }
        }
        if !shrank {
            return Ok(best);
        }
    }
}

/// Every one-step simplification, largest first: what costs the engine
/// most to run is what is worth dropping first.
fn candidates(case: &Case) -> Vec<Case> {
    let mut out = Vec::new();
    if !case.source.joins.is_empty() {
        let mut candidate = case.clone();
        candidate.source.joins.pop();
        out.push(candidate);
    }
    if case.analyze {
        let mut candidate = case.clone();
        candidate.analyze = false;
        out.push(candidate);
    }
    for (at, table) in case.tables.iter().enumerate() {
        out.extend(fewer_rows(case, at, table.rows.len()));
        for index in 0..table.indexes.len() {
            let mut candidate = case.clone();
            if let Some(table) = candidate.tables.get_mut(at) {
                table.indexes.remove(index);
            }
            out.push(candidate);
        }
    }
    out.extend(case.predicate.candidates().into_iter().map(|predicate| {
        let mut candidate = case.clone();
        candidate.predicate = predicate;
        candidate
    }));
    out
}

/// The row sets worth trying for one table: each half, then each row on
/// its own. O(rows) candidates.
fn fewer_rows(case: &Case, at: usize, rows: usize) -> Vec<Case> {
    if rows == 0 {
        return Vec::new();
    }
    let half = rows.saturating_div(2);
    let mut keeps: Vec<Vec<usize>> = Vec::new();
    if half > 0 {
        keeps.push((0..half).collect());
        keeps.push((half..rows).collect());
    }
    for skipped in 0..rows {
        keeps.push((0..rows).filter(|row| *row != skipped).collect());
    }
    keeps
        .into_iter()
        .map(|keep| {
            let mut candidate = case.clone();
            if let Some(table) = candidate.tables.get_mut(at) {
                let mut kept = Vec::new();
                for row in keep {
                    if let Some(values) = table.rows.get(row) {
                        kept.push(values.clone());
                    }
                }
                table.rows = kept;
            }
            candidate
        })
        .collect()
}
