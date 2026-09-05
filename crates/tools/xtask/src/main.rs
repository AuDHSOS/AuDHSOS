// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The build automation of `AuDHSOS`.

#![forbid(unsafe_code)]

mod commands;
mod coverage;
mod deps;
mod error;
mod fs;
mod image;
mod layering;
mod linker;
mod policy;
mod process;
mod spdx;
mod toolchain;
mod unsafe_budget;

use std::path::PathBuf;
use std::process::ExitCode;

use crate::error::Error;

const USAGE: &str = "\
usage: cargo xtask <subcommand> [options]

subcommands:
  lint             rustfmt --check, clippy with -D warnings, SPDX headers
  check-layering   dependency edges, forbid(unsafe_code), assembly files
  check-deps       no dependency outside the workspace
  unsafe-budget    unsafe blocks and asm! sites per adapter crate
  test [--host] [--qemu] [--e2e]
                   run the selected test levels (default: all available)
  coverage         host coverage with thresholds
  miri             run the host-executable adapter crates under Miri
  doc              build documentation with warnings as errors
  fuzz [--target <name>] [--time <seconds>]
                   run fuzz targets
  build [--release]
                   build the loader and the kernel for their targets
  image [--release]
                   write the boot image and the disk image into target/
  check            everything CI runs, in CI order
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::Usage(message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(subcommand) = args.first() else {
        return Err(Error::Usage("missing subcommand".to_owned()));
    };
    if subcommand == "--help" || subcommand == "-h" || subcommand == "help" {
        print!("{USAGE}");
        return Ok(());
    }
    let root = workspace_root()?;
    let channel = toolchain::verify(&root)?;
    let options = args.get(1..).unwrap_or_default();
    match subcommand.as_str() {
        "lint" => commands::lint(&root),
        "check-layering" => commands::check_layering(&root),
        "check-deps" => commands::check_deps(&root),
        "unsafe-budget" => commands::unsafe_budget(&root),
        "test" => commands::test(&root, options),
        "coverage" => commands::coverage(&root),
        "miri" => commands::miri(&root),
        "doc" => commands::doc(&root),
        "fuzz" => commands::fuzz(&root, options),
        "build" => commands::build(&root, options),
        "image" => commands::image(&root, options),
        "check" => commands::check(&root, &channel),
        other => Err(Error::Usage(format!("unknown subcommand `{other}`"))),
    }
}

/// The workspace root: the directory Cargo ran the xtask from.
fn workspace_root() -> Result<PathBuf, Error> {
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        let xtask_dir = PathBuf::from(manifest_dir);
        if let Some(root) = xtask_dir.ancestors().nth(3) {
            return Ok(root.to_path_buf());
        }
    }
    std::env::current_dir().map_err(|source| Error::io("reading the current directory", source))
}

#[cfg(test)]
mod tests;
