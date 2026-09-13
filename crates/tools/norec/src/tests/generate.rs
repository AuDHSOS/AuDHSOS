// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::generate`. What a random generator can be held to is a
//! property over many seeds, so that is what these check.

use crate::case::CountStyle;
use crate::expr::{Expr, Names};
use crate::generate::case;
use crate::oracle;
use crate::rng::Rng;
use crate::schema::Table;

/// The script of the case of one seed.
fn script(seed: u64) -> String {
    oracle::script(&case(&mut Rng::from_seed(seed)), CountStyle::Rows)
}

#[test]
fn the_same_seed_generates_the_same_case() {
    for seed in 0..20 {
        assert_eq!(script(seed), script(seed));
    }
    assert_ne!(script(1), script(2));
}

#[test]
fn every_case_has_a_table_with_a_column_and_rows_as_wide_as_it() {
    for seed in 0..200 {
        let subject = case(&mut Rng::from_seed(seed));
        assert!(!subject.tables.is_empty());
        for table in &subject.tables {
            assert!(!table.columns.is_empty());
            assert!(
                table
                    .rows
                    .iter()
                    .all(|row| row.len() == table.columns.len())
            );
        }
    }
}

#[test]
fn the_predicate_names_only_tables_the_source_clause_has() {
    for seed in 0..200 {
        let subject = case(&mut Rng::from_seed(seed));
        let visible: Vec<&str> = std::iter::once(subject.source.first)
            .chain(subject.source.joins.iter().map(|join| join.table))
            .filter_map(|at| subject.tables.get(at).map(|table| table.name.as_str()))
            .collect();
        let text = subject.render(&subject.predicate);
        for table in &subject.tables {
            if !visible.contains(&table.name.as_str()) {
                assert!(!text.contains(&format!("{}.", table.name)));
            }
        }
    }
}

#[test]
fn every_index_term_names_a_column() {
    for seed in 0..200 {
        let subject = case(&mut Rng::from_seed(seed));
        for table in &subject.tables {
            for index in &table.indexes {
                assert!(index.terms.iter().all(Expr::mentions_column));
                assert!(index.filter.as_ref().is_none_or(Expr::mentions_column));
            }
        }
    }
}

#[test]
fn a_unique_index_is_only_built_over_one_plain_column_without_a_collation() {
    for seed in 0..400 {
        let subject = case(&mut Rng::from_seed(seed));
        for table in &subject.tables {
            for index in index_of(table, true) {
                let [Expr::Column(reference)] = index.terms.as_slice() else {
                    panic!("a unique index over something that is not one column");
                };
                assert!(
                    table
                        .columns
                        .get(reference.column)
                        .is_some_and(|column| column.collation.is_none())
                );
            }
        }
    }
}

/// The indexes of a table whose uniqueness is `unique`.
fn index_of(table: &Table, unique: bool) -> impl Iterator<Item = &crate::schema::Index> {
    table
        .indexes
        .iter()
        .filter(move |index| index.unique == unique)
}

#[test]
fn no_case_calls_a_function_whose_answer_could_change_between_the_two_queries() {
    let forbidden = [
        "random(",
        "randomblob(",
        "date(",
        "time(",
        "datetime(",
        "julianday(",
        "changes(",
        "last_insert_rowid(",
        "sqlite_version(",
        "total_changes(",
        "DISTINCT",
        "SELECT",
    ];
    for seed in 0..400 {
        let subject = case(&mut Rng::from_seed(seed));
        let predicate = subject.render(&subject.predicate);
        for name in forbidden {
            assert!(!predicate.contains(name), "seed {seed} generated {name}");
        }
    }
}

#[test]
fn an_index_term_is_written_without_a_table_name() {
    for seed in 0..200 {
        let subject = case(&mut Rng::from_seed(seed));
        for table in &subject.tables {
            for statement in table.index_statements(&subject.tables) {
                assert!(!statement.contains(&format!("({}.", table.name)));
            }
            for index in &table.indexes {
                for term in &index.terms {
                    let bare = term.render(&subject.tables, Names::Bare);
                    assert!(!bare.contains(&format!("{}.", table.name)));
                }
            }
        }
    }
}
