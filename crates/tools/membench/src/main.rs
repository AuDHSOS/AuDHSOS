// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the memory of the development machine does: the bandwidth of a
//! sequential stream and the latency of a random walk.
//!
//! The stream is O(N) in the bytes it touches and says how many of them
//! move per second; the walk is O(1) per step and says what one step
//! costs, which is what a cache miss costs. The numbers describe the host,
//! not the system this repository builds: they are here to tell a slow
//! machine from a fast one before a measurement of the kernel is believed.
//!
//! Run it through `cargo xtask membench`, which builds it with
//! optimizations. A benchmark from the `dev` profile measures the bounds
//! checks and nothing else.

#![forbid(unsafe_code)]

mod bench;
mod config;

use std::process::ExitCode;

use crate::config::{Config, USAGE};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match Config::parse(&arguments) {
        Ok(Some(config)) => match bench::run(config) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        },
        Ok(None) => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
