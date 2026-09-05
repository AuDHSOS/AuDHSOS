// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The subcommands.

use std::path::Path;

use crate::error::Error;
use crate::image::{boot_image, disk};
use crate::policy::{FUZZ_TARGETS, MIRI_CRATES, Target, crates_for};
use crate::process::Cmd;
use crate::{coverage, deps, fs, layering, spdx, unsafe_budget};

/// `rustfmt --check`, `clippy -D warnings` per target group, SPDX headers.
pub(crate) fn lint(root: &Path) -> Result<(), Error> {
    Cmd::cargo()
        .cwd(root)
        .args(["fmt", "--all", "--", "--check"])
        .run()?;
    let mut host =
        Cmd::cargo()
            .cwd(root)
            .args(["clippy", "--workspace", "--all-targets", "--all-features"]);
    host = exclude_cross(host);
    host.args(["--", "-D", "warnings"]).run()?;
    for target in Target::CROSS {
        let crates = crates_for(target);
        let Some(triple) = target.triple() else {
            continue;
        };
        if crates.is_empty() {
            continue;
        }
        let mut cmd = Cmd::cargo().cwd(root).args(["clippy", "--all-features"]);
        for krate in crates {
            cmd = cmd.arg("-p").arg(krate);
        }
        cmd.arg("--target")
            .arg(triple)
            .args(["--", "-D", "warnings"])
            .run()?;
    }
    let violations = spdx::check(root)?;
    report("SPDX headers", &violations);
    Error::from_violations(violations)
}

/// Adds one `--exclude` per crate that is not built for the host, so that
/// a workspace command stays on the host crates.
pub(crate) fn exclude_cross(mut cmd: Cmd) -> Cmd {
    for target in Target::CROSS {
        for krate in crates_for(target) {
            cmd = cmd.arg("--exclude").arg(krate);
        }
    }
    cmd
}

/// Dependency edges, crate roots, assembly files.
pub(crate) fn check_layering(root: &Path) -> Result<(), Error> {
    let violations = layering::check(root)?;
    report("layering", &violations);
    Error::from_violations(violations)
}

/// No dependency outside the workspace.
pub(crate) fn check_deps(root: &Path) -> Result<(), Error> {
    let violations = deps::check(root)?;
    report("external code", &violations);
    Error::from_violations(violations)
}

/// `unsafe` and `asm!` sites against the budgets.
pub(crate) fn unsafe_budget(root: &Path) -> Result<(), Error> {
    let reports = unsafe_budget::check(root)?;
    eprintln!("{:<18} {:>6} {:>6}", "crate", "unsafe", "asm");
    let mut violations = Vec::new();
    for entry in &reports {
        eprintln!(
            "{:<18} {:>6} {:>6}",
            entry.name, entry.counts.unsafe_keywords, entry.counts.asm_macros
        );
        violations.extend(entry.violations.iter().cloned());
    }
    report("unsafe budget", &violations);
    Error::from_violations(violations)
}

/// The selected test levels.
pub(crate) fn test(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut host = false;
    let mut qemu = false;
    let mut e2e = false;
    for option in options {
        match option.as_str() {
            "--host" => host = true,
            "--qemu" => qemu = true,
            "--e2e" => e2e = true,
            other => return Err(Error::Usage(format!("unknown option `{other}` for test"))),
        }
    }
    if !(host || qemu || e2e) {
        host = true;
    }
    if host {
        let cmd = Cmd::cargo()
            .cwd(root)
            .args(["test", "--workspace", "--all-features"]);
        exclude_cross(cmd).run()?;
    }
    if qemu || e2e {
        return Err(Error::Usage(
            "QEMU and end-to-end tests arrive with Phase 2".to_owned(),
        ));
    }
    Ok(())
}

/// Host coverage against the thresholds.
pub(crate) fn coverage(root: &Path) -> Result<(), Error> {
    let totals = coverage::measure(root)?;
    let (table, violations) = coverage::evaluate(&totals);
    eprint!("{table}");
    report("coverage", &violations);
    Error::from_violations(violations)
}

/// Miri over the host-executable adapter crates.
pub(crate) fn miri(root: &Path) -> Result<(), Error> {
    let mut cmd = Cmd::cargo_plain().cwd(root).args(["miri", "test"]);
    for krate in MIRI_CRATES {
        cmd = cmd.arg("-p").arg(*krate);
    }
    cmd.run()
}

/// Documentation with warnings as errors.
pub(crate) fn doc(root: &Path) -> Result<(), Error> {
    Cmd::cargo()
        .cwd(root)
        .args([
            "doc",
            "--workspace",
            "--all-features",
            "--no-deps",
            "--document-private-items",
        ])
        .env("RUSTDOCFLAGS", "-D warnings")
        .run()
}

/// Fuzz targets.
pub(crate) fn fuzz(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut selected: Option<String> = None;
    let mut seconds = 60u64;
    let mut iter = options.iter();
    while let Some(option) = iter.next() {
        match option.as_str() {
            "--target" => selected = iter.next().cloned(),
            "--time" => {
                seconds = iter
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| Error::Usage("--time needs a number of seconds".to_owned()))?;
            }
            other => return Err(Error::Usage(format!("unknown option `{other}` for fuzz"))),
        }
    }
    let targets: Vec<_> = FUZZ_TARGETS
        .iter()
        .filter(|t| selected.as_deref().is_none_or(|s| s == t.name))
        .collect();
    if targets.is_empty() {
        eprintln!("no fuzz targets are registered yet (policy::FUZZ_TARGETS); nothing to run");
        return Ok(());
    }
    for target in targets {
        eprintln!("fuzzing `{}` for {seconds} seconds", target.name);
        Cmd::cargo()
            .cwd(&root.join("fuzz"))
            .args([
                "run",
                "--release",
                "--bin",
                target.name,
                "--",
                &format!("-max_total_time={seconds}"),
            ])
            .env("RUSTFLAGS", "-Zsanitizer=fuzzer")
            .run()?;
    }
    Ok(())
}

/// One step of `check`.
type Step = fn(&Path) -> Result<(), Error>;

/// Everything CI runs, in CI order.
pub(crate) fn check(root: &Path, channel: &str) -> Result<(), Error> {
    eprintln!("toolchain: {channel}");
    let steps: [(&str, Step); 8] = [
        ("lint", lint),
        ("check-layering", check_layering),
        ("check-deps", check_deps),
        ("unsafe-budget", unsafe_budget),
        ("test --host", |root| test(root, &["--host".to_owned()])),
        ("coverage", coverage),
        ("miri", miri),
        ("doc", doc),
    ];
    for (name, step) in steps {
        eprintln!("==> {name}");
        step(root)?;
    }
    eprintln!("==> all checks passed");
    Ok(())
}

fn report(what: &str, violations: &[String]) {
    if violations.is_empty() {
        eprintln!("{what}: ok");
    } else {
        eprintln!("{what}: {} violation(s)", violations.len());
    }
}

/// Writes the boot image and the disk image into `target/`.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option; [`Error::Io`] if the loader or
/// the kernel has not been built, or if a file cannot be written.
pub(crate) fn image(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut profile = "debug";
    for option in options {
        match option.as_str() {
            "--release" => profile = "release",
            other => return Err(Error::Usage(format!("unknown option `{other}` for image"))),
        }
    }
    let target = root.join("target");
    let loader = target
        .join("x86_64-unknown-uefi")
        .join(profile)
        .join("boot-uefi-x86_64.efi");
    let kernel = target
        .join("x86_64-unknown-none")
        .join(profile)
        .join("audhsos-kernel");
    let boot = boot_image::build(&boot_image::placeholder_root_task(), &[], 0)?;
    let files = vec![
        (disk::LOADER_PATH, fs::read_bytes(&loader)?),
        (disk::KERNEL_PATH, fs::read_bytes(&kernel)?),
        (disk::BOOT_IMAGE_PATH, boot.clone()),
    ];
    let image = disk::build(&files)?;
    let boot_path = target.join("boot.img");
    let disk_path = target.join("audhsos.img");
    fs::write_bytes(&boot_path, &boot)?;
    fs::write_bytes(&disk_path, &image)?;
    eprintln!(
        "wrote {} ({} bytes) and {} ({} bytes)",
        boot_path.display(),
        boot.len(),
        disk_path.display(),
        image.len()
    );
    Ok(())
}
