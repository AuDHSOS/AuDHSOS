// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The worker pool.

use std::sync::Mutex;

use crate::pool;

#[test]
fn no_job_means_no_result() {
    let jobs: Vec<u32> = Vec::new();
    let results = pool::map(&jobs, 4, |job| *job);
    assert!(results.is_empty());
}

#[test]
fn every_job_is_done_exactly_once() {
    let jobs: Vec<u32> = (0..200).collect();
    let seen = Mutex::new(Vec::new());
    let results = pool::map(&jobs, 8, |job| {
        if let Ok(mut seen) = seen.lock() {
            seen.push(*job);
        }
        job.saturating_mul(2)
    });
    assert_eq!(results.len(), jobs.len());
    let mut seen = seen.into_inner().unwrap_or_default();
    seen.sort_unstable();
    assert_eq!(seen, jobs);
}

#[test]
fn the_results_come_back_in_the_order_the_jobs_were_given() {
    let jobs: Vec<u32> = (0..64).rev().collect();
    let results = pool::map(&jobs, 8, std::string::ToString::to_string);
    let expected: Vec<String> = jobs.iter().map(u32::to_string).collect();
    assert_eq!(results, expected);
}

#[test]
fn one_worker_gives_the_same_answer_as_many() {
    let jobs: Vec<u32> = (0..50).collect();
    let one = pool::map(&jobs, 1, |job| job.saturating_add(1));
    let many = pool::map(&jobs, 16, |job| job.saturating_add(1));
    assert_eq!(one, many);
}

#[test]
fn more_workers_than_jobs_is_not_more_workers() {
    let jobs: Vec<u32> = vec![1, 2];
    assert_eq!(pool::map(&jobs, 64, |job| *job), vec![1, 2]);
}

#[test]
fn the_machine_reports_at_least_one_worker() {
    assert!(pool::parallelism() >= 1);
}
