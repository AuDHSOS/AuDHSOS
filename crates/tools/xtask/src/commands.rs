// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The subcommands.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::image::{boot_image, disk};
use crate::out::{self, note, note_raw};
use crate::policy::{FUZZ_TARGETS, MIRI_CRATES, Target, crates_for};
use crate::process::Cmd;
use crate::qemu::{self, Machine, Run};
use crate::symbolize;
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
    note!("{:<18} {:>6} {:>6}", "crate", "unsafe", "asm");
    let mut violations = Vec::new();
    for entry in &reports {
        note!(
            "{:<18} {:>6} {:>6}",
            entry.name,
            entry.counts.unsafe_keywords,
            entry.counts.asm_macros
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
    build_user_tests(root)?;
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
        .env(
            USER_TESTS_VARIABLE,
            root.join(USER_TESTS_DIR).display().to_string(),
        )
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
    report_tests(&name, &run, &machine, Some(kernel))
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
        note!("qemu {name}: {}", outcome.name());
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
fn report_tests(
    name: &str,
    run: &Run,
    machine: &Machine,
    image: Option<&Path>,
) -> Result<(), Error> {
    let report = qemu::parse(&run.output);
    let outcome = run.outcome();
    note!(
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
            if let Some(image) = image {
                symbolize::report(image, &run.output);
            }
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
    note_raw!("{table}");
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

/// What makes a fuzz target's `main` the engine's loop instead of the
/// corpus replay. The coverage instrumentation is not here: it is set per
/// package in `fuzz/Cargo.toml`, so that it lands on the code under test
/// and not on the engine, and `fuzz/.cargo/config.toml` turns on the Cargo
/// feature that allows it. Nothing is linked in from outside the
/// workspace; the engine is `fuzz-support`, so no platform runtime has to
/// carry one.
const FUZZING_FLAGS: &str = "--cfg fuzzing";

/// Fuzz targets. Without `--regression` this fuzzes; with it, every stored
/// corpus file is replayed once, which needs no fuzzer runtime and is what
/// CI runs on every push.
pub(crate) fn fuzz(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut selected: Option<String> = None;
    let mut seconds = 60u64;
    let mut mode = Job::Fuzz;
    let mut iter = options.iter();
    while let Some(option) = iter.next() {
        match option.as_str() {
            "--target" => selected = iter.next().cloned(),
            "--regression" => mode = Job::Regression,
            "--merge" => {
                let from = iter.next().cloned().ok_or_else(|| {
                    Error::Usage("--merge needs a directory to fold in".to_owned())
                })?;
                mode = Job::Merge(from);
            }
            "--minimize" => {
                let file = iter.next().cloned().ok_or_else(|| {
                    Error::Usage("--minimize needs the file to shrink".to_owned())
                })?;
                mode = Job::Minimize(file);
            }
            "--time" => {
                seconds = iter
                    .next()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| Error::Usage("--time needs a number of seconds".to_owned()))?;
            }
            other => return Err(Error::Usage(format!("unknown option `{other}` for fuzz"))),
        }
    }
    if let Job::Minimize(_) = mode
        && selected.is_none()
    {
        return Err(Error::Usage(
            "--minimize needs --target, because a crash belongs to one target".to_owned(),
        ));
    }
    let targets: Vec<_> = FUZZ_TARGETS
        .iter()
        .filter(|t| selected.as_deref().is_none_or(|s| s == t.name))
        .collect();
    if targets.is_empty() {
        note!("no fuzz targets are registered yet (policy::FUZZ_TARGETS); nothing to run");
        return Ok(());
    }
    for target in targets {
        match &mode {
            Job::Fuzz => run_fuzzer(root, target.name, seconds)?,
            Job::Regression => replay_corpus(root, target.name)?,
            Job::Merge(from) => merge_corpus(root, target.name, from)?,
            Job::Minimize(file) => minimize_crash(root, target.name, file, seconds)?,
        }
    }
    Ok(())
}

/// What `fuzz` was asked to do.
enum Job {
    /// Mutate and run.
    Fuzz,
    /// Replay the stored corpus, which needs no instrumentation.
    Regression,
    /// Fold a directory into the stored corpus, keeping what adds
    /// coverage.
    Merge(String),
    /// Shrink one crashing input.
    Minimize(String),
}

/// A path the caller gave, as one the fuzz workspace can use. The fuzzer
/// runs with `fuzz/` as its directory, so a path relative to where the
/// xtask was started has to be made absolute before it is handed over.
fn from_here(root: &Path, path: &str) -> PathBuf {
    let given = Path::new(path);
    if given.is_absolute() {
        given.to_path_buf()
    } else {
        root.join(given)
    }
}

/// Folds `from` into the stored corpus of one target, keeping the files
/// that reach something the corpus does not.
fn merge_corpus(root: &Path, name: &str, from: &str) -> Result<(), Error> {
    note!("merging {from} into the corpus of `{name}`");
    Cmd::cargo()
        .cwd(&root.join("fuzz"))
        .args([
            "run",
            "--release",
            "--bin",
            name,
            "--",
            "-merge=1",
            &corpus_of(root, name).display().to_string(),
            &from_here(root, from).display().to_string(),
        ])
        .env("RUSTFLAGS", FUZZING_FLAGS)
        .run()
}

/// Shrinks one crashing input of a target, for at most `seconds`.
fn minimize_crash(root: &Path, name: &str, file: &str, seconds: u64) -> Result<(), Error> {
    note!("shrinking {file} against `{name}` for {seconds} seconds");
    Cmd::cargo()
        .cwd(&root.join("fuzz"))
        .args([
            "run",
            "--release",
            "--bin",
            name,
            "--",
            "-minimize_crash=1",
            &from_here(root, file).display().to_string(),
            &format!("-max_total_time={seconds}"),
        ])
        .env("RUSTFLAGS", FUZZING_FLAGS)
        .run()
}

/// Runs the fuzzer of one target for `seconds` seconds.
fn run_fuzzer(root: &Path, name: &str, seconds: u64) -> Result<(), Error> {
    note!("fuzzing `{name}` for {seconds} seconds");
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
        note!("`{name}`: no corpus at {}", corpus.display());
        return Ok(());
    }
    note!("replaying the corpus of `{name}`");
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
///
/// `--quiet` reduces a run to one line per step: the output of a step that
/// passes is dropped, and the output of one that fails is printed as it
/// would have been. A full run writes some three thousand lines otherwise,
/// which is worth reading while watching it and worth nothing in a log.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option; the error of the first step
/// that fails.
pub(crate) fn check(root: &Path, channel: &str, options: &[String]) -> Result<(), Error> {
    for option in options {
        match option.as_str() {
            "--quiet" => out::set_quiet(true),
            other => return Err(Error::Usage(format!("unknown option `{other}` for check"))),
        }
    }
    note!("toolchain: {channel}");
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
        note!("==> {name}");
        let result = step(root);
        if out::quiet() {
            eprintln!("{name}: {}", if result.is_ok() { "ok" } else { "failed" });
        }
        result?;
    }
    eprintln!("==> all checks passed");
    Ok(())
}

fn report(what: &str, violations: &[String]) {
    if violations.is_empty() {
        note!("{what}: ok");
    } else {
        eprintln!("{what}: {} violation(s)", violations.len());
    }
}

/// Builds every crate that is not built for the host, for its target.
///
/// # Errors
///
/// [`Error::Usage`] for an unknown option; the errors of the build.
/// The directory the flat user programs are written to, below the target
/// directory.
pub(crate) const USER_TESTS_DIR: &str = "target/user-tests";

/// The variable that tells a test kernel where the flat user programs are.
pub(crate) const USER_TESTS_VARIABLE: &str = "AUDHSOS_USER_TESTS_DIR";

/// The address the user programs are linked at, which their linker script
/// repeats and this checks.
pub(crate) const USER_TEST_BASE: u64 = 0x40_0000;

/// The linker script of the user programs.
const USER_SCRIPT: &str = "crates/user/test-programs/user.ld";

/// Builds the user test programs and turns each into a flat binary in
/// [`USER_TESTS_DIR`].
///
/// A test kernel embeds those bytes: it maps them at [`USER_TEST_BASE`]
/// into a process it creates, so what runs in user mode is exactly what
/// the program is, with no loader in between.
///
/// # Errors
///
/// The errors of the build, of `llvm-objcopy`, and of writing the files;
/// [`Error::Violations`] when the linker script and this disagree on the
/// base address.
pub(crate) fn build_user_tests(root: &Path) -> Result<(), Error> {
    let script = fs::read(&root.join(USER_SCRIPT))?;
    let violations = match linker::constant(&script, "USER_BASE") {
        Some(base) if base == USER_TEST_BASE => Vec::new(),
        Some(base) => vec![format!(
            "{USER_SCRIPT}: USER_BASE is {base:#x}, the xtask says {USER_TEST_BASE:#x}"
        )],
        None => vec![format!("{USER_SCRIPT}: USER_BASE is missing")],
    };
    report("user program base", &violations);
    Error::from_violations(violations)?;

    Cmd::cargo()
        .cwd(root)
        .args([
            "build",
            "-p",
            "user-test-programs",
            "--target",
            "x86_64-unknown-none",
        ])
        .run()?;

    let objcopy = coverage::llvm_tools_dir()?.join("llvm-objcopy");
    // `llvm-objcopy` writes into a directory that has to be there.
    fs::write_bytes(&root.join(USER_TESTS_DIR).join(".keep"), b"")?;
    let out = root.join(USER_TESTS_DIR);
    let built = root.join("target/x86_64-unknown-none/debug");
    for program in user_programs(root)? {
        let elf = built.join(&program);
        let flat = out.join(format!("{program}.bin"));
        Cmd::new(&objcopy)
            .cwd(root)
            .args(["-O", "binary"])
            .arg(elf.display().to_string())
            .arg(flat.display().to_string())
            .run()?;
        note!(
            "user program {program}: {} bytes",
            fs::read_bytes(&flat)?.len()
        );
    }
    Ok(())
}

/// The names of the user test programs, read from their manifest so that
/// the list lives in one place.
fn user_programs(root: &Path) -> Result<Vec<String>, Error> {
    let manifest = fs::read(&root.join("crates/user/test-programs/Cargo.toml"))?;
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line == "[[bin]]" {
            in_bin = true;
            continue;
        }
        if in_bin && let Some(rest) = line.strip_prefix("name = ") {
            names.push(rest.trim_matches('"').to_owned());
            in_bin = false;
        }
    }
    if names.is_empty() {
        return Err(Error::Parse(
            "the manifest of the user test programs names no binary".to_owned(),
        ));
    }
    Ok(names)
}

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
    note!(
        "wrote {} ({} bytes) and {} ({} bytes)",
        boot_path.display(),
        boot.len(),
        disk_path.display(),
        image.len()
    );
    Ok(())
}
