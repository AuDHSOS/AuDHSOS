// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The subcommands.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::image::{boot_image, disk};
use crate::policy::{FUZZ_TARGETS, MIRI_CRATES, Target, crates_for};
use crate::process::Cmd;
use crate::qemu::{self, Machine, Run};
use crate::{coverage, deps, fs, layering, linker, spdx, unsafe_budget};

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

/// Dependency edges, crate roots, assembly files, linker constants.
pub(crate) fn check_layering(root: &Path) -> Result<(), Error> {
    let mut violations = layering::check(root)?;
    violations.extend(linker::check(root)?);
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
    if qemu {
        test_qemu(root)?;
    }
    if e2e {
        return Err(Error::Usage(
            "end-to-end tests arrive with Phase 6".to_owned(),
        ));
    }
    Ok(())
}

/// Every test kernel in QEMU, then the three images that must make the
/// loader report a failure.
fn test_qemu(root: &Path) -> Result<(), Error> {
    build(root, &[])?;
    let runner = runner_command()?;
    Cmd::cargo()
        .cwd(root)
        .args([
            "test",
            "-p",
            "audhsos-kernel",
            "--target",
            "x86_64-unknown-none",
        ])
        .env(RUNNER_VARIABLE, runner)
        .env(ROOT_VARIABLE, root.display().to_string())
        .run()?;
    loader_images(root)
}

/// The Cargo configuration variable that names the runner of the kernel
/// target. The xtask sets it to itself, so that Cargo starts no second
/// Cargo while it holds the lock on the build directory.
const RUNNER_VARIABLE: &str = "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUNNER";

/// Names the workspace root for a runner Cargo starts.
pub(crate) const ROOT_VARIABLE: &str = "AUDHSOS_ROOT";

/// `<this binary> qemu-runner`, as Cargo wants the runner: a program and
/// its arguments, separated by spaces.
fn runner_command() -> Result<String, Error> {
    let exe = std::env::current_exe()
        .map_err(|source| Error::io("reading the path of the xtask binary", source))?;
    let text = exe.display().to_string();
    if text.contains(char::is_whitespace) {
        return Err(Error::Usage(format!(
            "the xtask binary {text} lies in a path with a space, \
             which Cargo cannot express as a runner"
        )));
    }
    Ok(format!("{text} qemu-runner"))
}

/// Wraps the kernel image `options[0]` into a disk image and runs it.
/// This is the Cargo runner of the kernel target.
///
/// # Errors
///
/// [`Error::Usage`] without an argument or without a built loader; the
/// errors of the image writers and of the run.
pub(crate) fn qemu_runner(root: &Path, options: &[String]) -> Result<(), Error> {
    let kernel = options
        .first()
        .ok_or_else(|| Error::Usage("qemu-runner needs the path of a kernel image".to_owned()))?;
    let kernel = Path::new(kernel);
    let profile = profile_of(kernel);
    let loader = loader_bytes(root, profile)?;
    let boot = boot_image::build(&boot_image::placeholder_root_task(), &[], 0)?;
    let image = disk_image(loader, Some(fs::read_bytes(kernel)?), Some(boot))?;
    let name = fs::file_name(kernel).to_owned();
    let path = write_run_image(root, &name, &image)?;
    let machine = Machine::locate()?;
    let run = machine.run_captured(&path)?;
    report_tests(&name, &run, &machine)
}

/// What a test image has to write on the serial port beyond the
/// protocol, by the prefix of the image name.
const MARKERS: &[(&str, &str)] = &[("console", "audhsos console marker 0123456789")];

/// The prefix of every diagnostic the loader writes.
const LOADER_PREFIX: &str = "[loader] ";

/// One image the loader has to reject: a name, the kernel file the volume
/// carries, and the boot image it carries.
type LoaderCase = (&'static str, Option<Vec<u8>>, Option<Vec<u8>>);

/// The three images the loader has to reject, each run once.
fn loader_images(root: &Path) -> Result<(), Error> {
    let profile = "debug";
    let loader = loader_bytes(root, profile)?;
    let kernel = fs::read_bytes(&kernel_binary(root, profile))?;
    let boot = boot_image::build(&boot_image::placeholder_root_task(), &[], 0)?;
    let cases: [LoaderCase; 3] = [
        ("bad-kernel", Some(corrupt(&kernel)), Some(boot.clone())),
        ("missing-kernel", None, Some(boot)),
        ("missing-boot-image", Some(kernel), None),
    ];
    let machine = Machine::locate()?;
    let mut violations = Vec::new();
    for (name, kernel, boot) in cases {
        let image = disk_image(loader.clone(), kernel, boot)?;
        let path = write_run_image(root, name, &image)?;
        let run = machine.run_captured(&path)?;
        let outcome = run.outcome();
        eprintln!("qemu {name}: {}", outcome.name());
        if outcome != qemu::Outcome::LoaderFailure {
            eprint!("{}", run.output);
            violations.push(format!(
                "the image `{name}` produced a {}, not a loader failure",
                outcome.name()
            ));
            continue;
        }
        if !run.output.contains(LOADER_PREFIX) {
            eprint!("{}", run.output);
            violations.push(format!("the image `{name}` wrote no loader diagnostic"));
        }
    }
    report("loader failure images", &violations);
    Error::from_violations(violations)
}

/// A kernel image whose ELF magic is broken.
fn corrupt(kernel: &[u8]) -> Vec<u8> {
    let mut bytes = kernel.to_vec();
    if let Some(byte) = bytes.first_mut() {
        *byte = 0;
    }
    bytes
}

/// Reads the loader that `build` wrote.
fn loader_bytes(root: &Path, profile: &str) -> Result<Vec<u8>, Error> {
    let path = loader_binary(root, profile);
    fs::read_bytes(&path).map_err(|_| {
        Error::Usage(format!(
            "{} does not exist; run `cargo xtask build` first",
            path.display()
        ))
    })
}

/// Where the loader lands.
fn loader_binary(root: &Path, profile: &str) -> PathBuf {
    root.join("target")
        .join("x86_64-unknown-uefi")
        .join(profile)
        .join("boot-uefi-x86_64.efi")
}

/// Where the kernel lands.
fn kernel_binary(root: &Path, profile: &str) -> PathBuf {
    root.join("target")
        .join("x86_64-unknown-none")
        .join(profile)
        .join("audhsos-kernel")
}

/// The profile a built artefact belongs to, read from its path.
fn profile_of(binary: &Path) -> &'static str {
    if binary
        .components()
        .any(|component| component.as_os_str() == "release")
    {
        "release"
    } else {
        "debug"
    }
}

/// The disk image holding the loader, and the kernel and the boot image
/// where they are given. A missing file is what the loader has to survive.
fn disk_image(
    loader: Vec<u8>,
    kernel: Option<Vec<u8>>,
    boot: Option<Vec<u8>>,
) -> Result<Vec<u8>, Error> {
    let mut files = vec![(disk::LOADER_PATH, loader)];
    if let Some(kernel) = kernel {
        files.push((disk::KERNEL_PATH, kernel));
    }
    if let Some(boot) = boot {
        files.push((disk::BOOT_IMAGE_PATH, boot));
    }
    disk::build(&files)
}

/// Writes one image of a run into `target/qemu/`.
fn write_run_image(root: &Path, name: &str, image: &[u8]) -> Result<PathBuf, Error> {
    let path = root.join("target").join("qemu").join(format!("{name}.img"));
    fs::write_bytes(&path, image)?;
    Ok(path)
}

/// Reports what one test kernel did and fails if it did not pass.
fn report_tests(name: &str, run: &Run, machine: &Machine) -> Result<(), Error> {
    let report = qemu::parse(&run.output);
    let outcome = run.outcome();
    eprintln!(
        "qemu {name}: {} ({} passed, {} failed)",
        outcome.name(),
        report.passed(),
        report.failed()
    );
    if run.timed_out {
        eprintln!(
            "the run was killed after {} seconds",
            machine.timeout().as_secs()
        );
    }
    let mut missing = Vec::new();
    for (prefix, marker) in MARKERS {
        if name.starts_with(prefix) && !run.output.contains(marker) {
            missing.push(format!("the image `{name}` did not write `{marker}`"));
        }
    }
    match qemu::check(&report, outcome).and_then(|()| Error::from_violations(missing)) {
        Ok(()) => Ok(()),
        Err(error) => {
            eprintln!("--- serial output of {name} ---");
            eprint!("{}", run.output);
            eprintln!("--- end of {name} ---");
            Err(error)
        }
    }
}

/// Builds everything, writes the images, and boots the system with the
/// serial console on the terminal and no time limit.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option or a machine that did not report
/// success; the errors of the build and of the image writers.
pub(crate) fn run(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut display = false;
    let mut build_options = Vec::new();
    for option in options {
        match option.as_str() {
            "--display" => display = true,
            "--release" => build_options.push("--release".to_owned()),
            other => return Err(Error::Usage(format!("unknown option `{other}` for run"))),
        }
    }
    build(root, &build_options)?;
    image(root, &build_options)?;
    let machine = Machine::locate()?;
    let path = root.join("target").join("audhsos.img");
    let status = machine.run_attached(&path, display)?;
    match qemu::outcome_of(status, false) {
        qemu::Outcome::Success => Ok(()),
        outcome => Err(Error::Usage(format!(
            "the machine reported a {}",
            outcome.name()
        ))),
    }
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

/// Documentation with warnings as errors, per target group.
pub(crate) fn doc(root: &Path) -> Result<(), Error> {
    let host = Cmd::cargo().cwd(root).args([
        "doc",
        "--workspace",
        "--all-features",
        "--no-deps",
        "--document-private-items",
    ]);
    exclude_cross(host)
        .env("RUSTDOCFLAGS", "-D warnings")
        .run()?;
    for target in Target::CROSS {
        let crates = crates_for(target);
        let Some(triple) = target.triple() else {
            continue;
        };
        if crates.is_empty() {
            continue;
        }
        let mut cmd = Cmd::cargo().cwd(root).args([
            "doc",
            "--all-features",
            "--no-deps",
            "--document-private-items",
        ]);
        for krate in crates {
            cmd = cmd.arg("-p").arg(krate);
        }
        cmd.arg("--target")
            .arg(triple)
            .env("RUSTDOCFLAGS", "-D warnings")
            .run()?;
    }
    Ok(())
}

/// The flags that turn a fuzz target into a libFuzzer binary: the coverage
/// instrumentation the fuzzer steers by, `--cfg fuzzing` so that the target
/// leaves its `main` to the runtime, and the link flag that pulls that
/// runtime in. The runtime comes from the platform's clang, not from the
/// workspace, which has no dependency outside itself; a machine whose clang
/// carries no libFuzzer fails at the link step with an undefined `main`.
/// Without these flags the same source builds an ordinary program that
/// replays a corpus, which is what `--regression` uses.
const FUZZING_FLAGS: &str = "-Cpasses=sancov-module \
 -Cllvm-args=-sanitizer-coverage-level=4 \
 -Cllvm-args=-sanitizer-coverage-inline-8bit-counters \
 -Cllvm-args=-sanitizer-coverage-pc-table \
 -Cllvm-args=-sanitizer-coverage-trace-compares \
 --cfg fuzzing \
 -Clink-arg=-fsanitize=fuzzer";

/// Fuzz targets. Without `--regression` this fuzzes; with it, every stored
/// corpus file is replayed once, which needs no fuzzer runtime and is what
/// CI runs on every push.
pub(crate) fn fuzz(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut selected: Option<String> = None;
    let mut seconds = 60u64;
    let mut regression = false;
    let mut iter = options.iter();
    while let Some(option) = iter.next() {
        match option.as_str() {
            "--target" => selected = iter.next().cloned(),
            "--regression" => regression = true,
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
        if regression {
            replay_corpus(root, target.name)?;
        } else {
            run_fuzzer(root, target.name, seconds)?;
        }
    }
    Ok(())
}

/// Runs the fuzzer of one target for `seconds` seconds.
fn run_fuzzer(root: &Path, name: &str, seconds: u64) -> Result<(), Error> {
    eprintln!("fuzzing `{name}` for {seconds} seconds");
    Cmd::cargo()
        .cwd(&root.join("fuzz"))
        .args([
            "run",
            "--release",
            "--bin",
            name,
            "--",
            &corpus_of(root, name).display().to_string(),
            &format!("-max_total_time={seconds}"),
        ])
        .env("RUSTFLAGS", FUZZING_FLAGS)
        .run()
}

/// Replays the stored corpus of one target. A target whose corpus
/// directory does not exist is reported and skipped, so that a target that
/// has found nothing yet does not fail the run.
fn replay_corpus(root: &Path, name: &str) -> Result<(), Error> {
    let corpus = corpus_of(root, name);
    if !corpus.is_dir() {
        eprintln!("`{name}`: no corpus at {}", corpus.display());
        return Ok(());
    }
    eprintln!("replaying the corpus of `{name}`");
    Cmd::cargo()
        .cwd(&root.join("fuzz"))
        .args([
            "run",
            "--quiet",
            "--bin",
            name,
            "--",
            &corpus.display().to_string(),
        ])
        .run()
}

/// The corpus directory of one target.
fn corpus_of(root: &Path, name: &str) -> PathBuf {
    root.join("fuzz").join("corpus").join(name)
}

/// One step of `check`.
type Step = fn(&Path) -> Result<(), Error>;

/// Everything CI runs, in CI order.
pub(crate) fn check(root: &Path, channel: &str) -> Result<(), Error> {
    eprintln!("toolchain: {channel}");
    let steps: [(&str, Step); 10] = [
        ("lint", lint),
        ("check-layering", check_layering),
        ("check-deps", check_deps),
        ("unsafe-budget", unsafe_budget),
        ("test --host", |root| test(root, &["--host".to_owned()])),
        ("coverage", coverage),
        ("miri", miri),
        ("doc", doc),
        ("test --qemu", |root| test(root, &["--qemu".to_owned()])),
        ("fuzz --regression", |root| {
            fuzz(root, &["--regression".to_owned()])
        }),
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

/// Builds every crate that is not built for the host, for its target.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option; the errors of the build.
pub(crate) fn build(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut release = false;
    for option in options {
        match option.as_str() {
            "--release" => release = true,
            other => return Err(Error::Usage(format!("unknown option `{other}` for build"))),
        }
    }
    for target in Target::CROSS {
        let crates = crates_for(target);
        let Some(triple) = target.triple() else {
            continue;
        };
        if crates.is_empty() {
            continue;
        }
        let mut cmd = Cmd::cargo().cwd(root).arg("build");
        for krate in crates {
            cmd = cmd.arg("-p").arg(krate);
        }
        cmd = cmd.arg("--target").arg(triple);
        if release {
            cmd = cmd.arg("--release");
        }
        cmd.run()?;
    }
    Ok(())
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
