// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Host coverage: instrumented test binaries, merged profiles, and a
//! per-crate report against the thresholds.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::policy::{COVERAGE, CRATES, find};
use crate::process::Cmd;

/// Line and branch counts of one crate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    /// Instrumented lines.
    pub(crate) lines: u64,
    /// Lines never executed.
    pub(crate) missed_lines: u64,
    /// Instrumented branches.
    pub(crate) branches: u64,
    /// Branches never taken.
    pub(crate) missed_branches: u64,
}

impl Totals {
    const fn add(&mut self, other: Totals) {
        self.lines = self.lines.saturating_add(other.lines);
        self.missed_lines = self.missed_lines.saturating_add(other.missed_lines);
        self.branches = self.branches.saturating_add(other.branches);
        self.missed_branches = self.missed_branches.saturating_add(other.missed_branches);
    }

    /// Percentage of executed lines; 100 when nothing is instrumented.
    pub(crate) fn line_percent(self) -> f64 {
        percent(self.lines, self.missed_lines)
    }

    /// Percentage of taken branches; 100 when nothing is instrumented.
    pub(crate) fn branch_percent(self) -> f64 {
        percent(self.branches, self.missed_branches)
    }
}

fn percent(total: u64, missed: u64) -> f64 {
    if total == 0 {
        return 100.0;
    }
    let covered = f64::from(u32::try_from(total.saturating_sub(missed)).unwrap_or(u32::MAX));
    let all = f64::from(u32::try_from(total).unwrap_or(u32::MAX));
    covered / all * 100.0
}

/// Builds instrumented tests, runs them, and returns the per-crate totals.
pub(crate) fn measure(root: &Path) -> Result<BTreeMap<String, Totals>, Error> {
    let target_dir = root.join("target").join("coverage-build");
    let profile_dir = root.join("target").join("coverage");
    let _ = std::fs::remove_dir_all(&profile_dir);
    std::fs::create_dir_all(&profile_dir)
        .map_err(|source| Error::io("creating the coverage directory", source))?;
    let rustflags = "-C instrument-coverage -Z coverage-options=branch";
    let build = crate::commands::exclude_cross(Cmd::cargo().cwd(root).args([
        "test",
        "--workspace",
        "--all-features",
        "--no-run",
    ]))
    .env("RUSTFLAGS", rustflags)
    .env("CARGO_TARGET_DIR", target_dir.display().to_string())
    .capture_stderr()?;
    let executables = executables_of(&build);
    if executables.is_empty() {
        return Err(Error::Parse(
            "cargo test --no-run reported no executables".to_owned(),
        ));
    }
    for (index, exe) in executables.iter().enumerate() {
        let pattern = profile_dir.join(format!("{index}-%p-%m.profraw"));
        Cmd::new(exe)
            .cwd(root)
            .env("LLVM_PROFILE_FILE", pattern.display().to_string())
            .run()?;
    }
    let tools = llvm_tools_dir()?;
    let merged = profile_dir.join("merged.profdata");
    let mut merge = Cmd::new(tools.join("llvm-profdata"))
        .args(["merge", "-sparse", "-o"])
        .arg(merged.display().to_string());
    for entry in
        std::fs::read_dir(&profile_dir).map_err(|source| Error::io("listing profiles", source))?
    {
        let path = entry
            .map_err(|source| Error::io("listing profiles", source))?
            .path();
        if path.extension().is_some_and(|e| e == "profraw") {
            merge = merge.arg(path.display().to_string());
        }
    }
    merge.run()?;
    let mut export = Cmd::new(tools.join("llvm-cov"))
        .args(["export", "--format=lcov"])
        .arg(format!("--instr-profile={}", merged.display()))
        .arg("--ignore-filename-regex=/rustc/|/\\.rustup/|/\\.cargo/|/target/|/src/tests/");
    for (index, exe) in executables.iter().enumerate() {
        if index > 0 {
            export = export.arg("--object");
        }
        export = export.arg(exe.display().to_string());
    }
    let text = export.capture()?;
    Ok(totals_by_crate(&text, root))
}

/// The executables named in the standard error output of
/// `cargo test --no-run`.
pub(crate) fn executables_of(stderr: &str) -> Vec<PathBuf> {
    stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("Executable"))
        .filter_map(|line| {
            let open = line.rfind('(')?;
            let close = line.rfind(')')?;
            line.get(open.saturating_add(1)..close).map(PathBuf::from)
        })
        .collect()
}

/// The directory holding `llvm-profdata` and `llvm-cov` of the toolchain.
pub(crate) fn llvm_tools_dir() -> Result<PathBuf, Error> {
    let rustc = Cmd::toolchain_binary("rustc");
    let sysroot = rustc
        .clone()
        .arg("--print")
        .arg("sysroot")
        .capture()?
        .trim()
        .to_owned();
    let version = rustc.arg("-vV").capture()?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or_else(|| Error::Parse("rustc -vV reports no host".to_owned()))?
        .trim()
        .to_owned();
    Ok(PathBuf::from(sysroot)
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin"))
}

/// Sums the records of an LCOV export per policy crate. A record starts
/// with `SF:<path>` and ends with `end_of_record`; `LF`/`LH` count lines
/// found and hit, `BRF`/`BRH` count branches found and hit.
pub(crate) fn totals_by_crate(lcov: &str, root: &Path) -> BTreeMap<String, Totals> {
    let mut totals: BTreeMap<String, Totals> = BTreeMap::new();
    let mut file: Option<PathBuf> = None;
    let mut record = Totals::default();
    let mut hit = (0u64, 0u64);
    for line in lcov.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            file = Some(root.join(path.trim()));
            record = Totals::default();
            hit = (0, 0);
        } else if let Some(value) = line.strip_prefix("LF:") {
            record.lines = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("LH:") {
            hit.0 = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("BRF:") {
            record.branches = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("BRH:") {
            hit.1 = value.trim().parse().unwrap_or(0);
        } else if line.trim() == "end_of_record" {
            record.missed_lines = record.lines.saturating_sub(hit.0);
            record.missed_branches = record.branches.saturating_sub(hit.1);
            if let Some(krate) = file.take().and_then(|f| crate_of(&f, root)) {
                totals.entry(krate.to_owned()).or_default().add(record);
            }
        }
    }
    totals
}

/// The policy crate whose directory contains `file`; the longest matching
/// path wins.
pub(crate) fn crate_of(file: &Path, root: &Path) -> Option<&'static str> {
    CRATES
        .iter()
        .filter(|c| file.starts_with(root.join(c.path)))
        .max_by_key(|c| c.path.len())
        .map(|c| c.name)
}

/// Formats the table and returns threshold violations.
pub(crate) fn evaluate(totals: &BTreeMap<String, Totals>) -> (String, Vec<String>) {
    let mut table = format!("{:<18} {:>8} {:>8}\n", "crate", "lines", "branches");
    let mut violations = Vec::new();
    for krate in CRATES {
        let Some(total) = totals.get(krate.name) else {
            if krate.coverage_gate {
                violations.push(format!("`{}` has no coverage data", krate.name));
            }
            continue;
        };
        let (lines, branches) = (total.line_percent(), total.branch_percent());
        let gate = if krate.coverage_gate {
            ""
        } else {
            "  (reported only)"
        };
        let _ = writeln!(
            table,
            "{:<18} {lines:>7.2}% {branches:>7.2}%{gate}",
            krate.name
        );
        if krate.coverage_gate && find(krate.name).is_some() {
            if lines < COVERAGE.lines {
                violations.push(format!(
                    "`{}` line coverage {lines:.2}% is below {:.0}%",
                    krate.name, COVERAGE.lines
                ));
            }
            if branches < COVERAGE.branches {
                violations.push(format!(
                    "`{}` branch coverage {branches:.2}% is below {:.0}%",
                    krate.name, COVERAGE.branches
                ));
            }
        }
    }
    (table, violations)
}
