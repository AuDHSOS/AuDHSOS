// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod artifacts;
mod coverage;
mod deps;
mod error;
mod fs;
mod image;
mod json;
mod layering;
mod linker;
mod out;
mod policy;
mod ppm;
mod process;
mod qemu;
mod qmp;
mod sections;
mod spdx;
mod ssh;
mod symbolize;
mod test_ext;
mod tls;
mod toolchain;
mod unsafe_budget;
mod usage;
