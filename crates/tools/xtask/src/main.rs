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
mod out;
mod policy;
mod process;
mod qemu;
mod session;
mod spdx;
mod symbolize;
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
  test [--host] [--qemu] [--e2e] [--release]
                   run the selected test levels (default: all available);
                   --release builds the end-to-end run from the release
                   profile
  pdf [options]    every Markdown document and every RFC as PDF, under
                   target/pdf/; options are passed to the tool, which
                   explains them with --help
  coverage         host coverage with thresholds
  miri             run the host-executable adapter crates under Miri
  doc              build documentation with warnings as errors
  fuzz [--target <name>] [--time <seconds>] [--regression]
       [--merge <directory>] [--minimize <file>]
                   run fuzz targets
  build [--release]
                   build the loader and the kernel for their targets
  build-user-tests build the user programs of the test images and turn
                   each into a flat binary under target/user-tests/
  image [--release]
                   write the boot image and the disk image into target/
  qemu-runner <elf>
                   Cargo's runner for the kernel target: wrap a test kernel
                   into a disk image, run it, and read the serial protocol
  symbolize <elf> <address>...
                   the function, file, and line of every address
  run [--release] [--display]
                   boot the system in QEMU with the console on the terminal
  check [--quiet]  everything CI runs, in CI order; --quiet leaves one
                   line per step and prints the output of a step only
                   when it fails
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
        "lint" => none(subcommand, options).and_then(|()| commands::lint(&root)),
        "check-layering" => {
            none(subcommand, options).and_then(|()| commands::check_layering(&root))
        }
        "check-deps" => none(subcommand, options).and_then(|()| commands::check_deps(&root)),
        "unsafe-budget" => none(subcommand, options).and_then(|()| commands::unsafe_budget(&root)),
        "test" => commands::test(&root, options),
        "pdf" => commands::pdf(&root, options),
        "coverage" => none(subcommand, options).and_then(|()| commands::coverage(&root)),
        "miri" => none(subcommand, options).and_then(|()| commands::miri(&root)),
        "doc" => none(subcommand, options).and_then(|()| commands::doc(&root)),
        "fuzz" => commands::fuzz(&root, options),
        "build" => commands::build(&root, options),
        "build-user-tests" => {
            none(subcommand, options).and_then(|()| commands::build_user_tests(&root))
        }
        "image" => commands::image(&root, options),
        "qemu-runner" => commands::qemu_runner(&root, options),
        "run" => commands::run(&root, options),
        "symbolize" => symbolize::command(options),
        "check" => commands::check(&root, &channel, options),
        other => Err(Error::Usage(format!("unknown subcommand `{other}`"))),
    }
}

/// The option check of a subcommand that takes none. Without it an
/// argument of such a subcommand is read by nobody, and a run that was
/// asked for something it cannot do reports that it did it.
fn none(subcommand: &str, options: &[String]) -> Result<(), Error> {
    match options.first() {
        Some(option) => Err(Error::Usage(format!(
            "unknown option `{option}` for {subcommand}"
        ))),
        None => Ok(()),
    }
}

/// The workspace root: the directory Cargo ran the xtask from.
fn workspace_root() -> Result<PathBuf, Error> {
    if let Some(root) = std::env::var_os(commands::ROOT_VARIABLE) {
        return Ok(PathBuf::from(root));
    }
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
