// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The subcommands.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::anchors;
use crate::error::Error;
use crate::image::{archive, boot_image, disk, fat32, loadable};
use crate::out::{self, note, note_raw};
use crate::policy::{FUZZ_TARGETS, FuzzTarget, MIRI_TARGETS, Target, crates_for};
use crate::ppm;
use crate::process::{Cmd, run_parallel, test_jobs};
use crate::qemu::{self, Machine, Run};
use crate::qmp::{Button, Qmp};
use crate::session::Session;
use crate::ssh;
use crate::symbolize;
use crate::tls;
use crate::{artifacts, coverage, deps, fs, layering, linker, spdx, unsafe_budget};

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
    let mut ssh = false;
    let mut tls = false;
    let mut profile = Vec::new();
    for option in options {
        match option.as_str() {
            "--host" => host = true,
            "--qemu" => qemu = true,
            "--e2e" => e2e = true,
            "--ssh" => ssh = true,
            "--tls" => tls = true,
            "--release" => profile.push("--release".to_owned()),
            other => return Err(Error::Usage(format!("unknown option `{other}` for test"))),
        }
    }
    if !(host || qemu || e2e || ssh || tls) {
        host = true;
    }
    if host {
        let jobs = test_jobs()?;
        let command =
            exclude_cross(
                Cmd::cargo()
                    .cwd(root)
                    .args(["test", "--workspace", "--all-features"]),
            );
        let executables = artifacts::build_tests(command.clone())?;
        let jobs = jobs.min(executables.len());
        let commands: Vec<_> = executables
            .iter()
            .map(|exe| exe.test_command(jobs))
            .collect();
        run_parallel(&commands, jobs)?;
        // --no-run does not build or execute doc tests. Keep Cargo in
        // charge of these so the host command retains its test coverage.
        command.arg("--doc").run()?;
    }
    if qemu {
        test_qemu(root)?;
    }
    if e2e {
        test_e2e(root, &profile)?;
    }
    // The Secure Shell run alone, which `--e2e` runs last anyway. It is
    // its own option because it is the one run that needs a server on the
    // development machine, and a person who is changing the client repeats
    // it without the four runs before it.
    if ssh {
        build(root, &profile)?;
        image(root, &profile)?;
        let machine = Machine::locate()?;
        let path = root.join("target").join("audhsos.img");
        test_the_secure_shell_client(&machine, &path, root)?;
    }
    // The TLS run alone, for the same reason as the Secure Shell one.
    if tls {
        build(root, &profile)?;
        image(root, &profile)?;
        let machine = Machine::locate()?;
        let path = root.join("target").join("audhsos.img");
        test_the_tls_client(&machine, &path, root)?;
    }
    Ok(())
}

/// How long an end-to-end run waits for a line before it gives up.
///
/// Every program outside the boot set is read off the volume one message
/// of two kibibytes at a time (D-92), so what a line waits on is often a
/// program being read and not the work behind the line. Phase 14 put two
/// more programs on the volume, and three minutes is what leaves room for
/// that on a machine where the emulator translates every instruction.
const E2E_TIMEOUT: Duration = Duration::from_secs(180);

/// What the run has to see, in this order, for the system to have worked.
///
/// Each line is written by a different part of it, so the first one that
/// does not come says where the boot stopped: the root task read the
/// archive, the memory server answered, the name server answered, the
/// console driver took the port, and the application found it and said
/// something through it.
const E2E_LINES: [(&str, &str); 23] = [
    (
        "[init] started server-memory",
        "the memory server did not start",
    ),
    (
        "[init] started server-name",
        "the name server did not start",
    ),
    (
        "[init] started server-console",
        "the console driver did not start",
    ),
    (
        "[init] started server-display",
        "the display server did not start",
    ),
    (
        "[init] started server-input",
        "the input server did not start",
    ),
    ("[init] started app-hello", "the application did not start"),
    (
        "hello from userland",
        "the application said nothing through the console driver",
    ),
    (
        "[checks] missing name: the requested item does not exist",
        "a lookup of a name nobody registered was not refused with NotFound",
    ),
    (
        "[checks] memory comes back zeroed: ok",
        "memory that was used, given back, and asked for again was not zeroed",
    ),
    (
        "[checks] more than there is: ",
        "the memory server said nothing to a request no machine can meet",
    ),
    (
        "[checks] line 7 of 8, and the whole of it",
        "the last line of the second client did not arrive",
    ),
    (
        "[paint] drawn on ",
        "the program that draws never presented anything",
    ),
    (
        "is gone: surface ",
        "the display server did not take back the surface of the program that ended",
    ),
    (
        "[init] started app-input",
        "the program that listens did not start",
    ),
    (
        "[info] ecam=",
        "the kernel did not report the configuration window of the bus",
    ),
    (
        "[lspci] window segment=",
        "the program that walks the bus was given no window",
    ),
    (
        " 1af4:1041 class=02:00:00",
        "the bus walk did not find the virtio network device of the machine",
    ),
    (
        "[init] started server-net",
        "the network server did not start",
    ),
    (
        "[net] device mac=",
        "the network driver did not bring the device up",
    ),
    (
        "[init] started app-net",
        "the program that uses a socket did not start",
    ),
    (
        "[tls-app] anchors=",
        "the program that reads the trust anchors reported none",
    ),
    (
        "[faulter] about to write to nowhere",
        "the program that faults on purpose never ran",
    ),
    (
        "faulted: PageFault",
        "the root task did not report the fault of its child",
    ),
];

/// The four structures a driver of the network device needs, which the bus
/// walk has to have read off the device itself.
const VIRTIO_STRUCTURES: [&str; 4] = ["common", "notify", "isr", "device"];

/// What the line about the virtio device does not say.
///
/// The order of the names is the device's own order of preference and no
/// business of this check (virtio 4.1.4), so each is looked for on its own.
fn virtio_lines(output: &str) -> Vec<String> {
    let Some(line) = output
        .lines()
        .find(|line| line.contains("[lspci] virtio-net structures="))
    else {
        return vec!["the bus walk read no structures off the network device".to_owned()];
    };
    let mut violations = Vec::new();
    for wanted in VIRTIO_STRUCTURES {
        if !line.contains(wanted) {
            violations.push(format!(
                "the network device published no {wanted} structure"
            ));
        }
    }
    if !line.contains("msix=4") {
        violations.push(format!(
            "the message table of the network device is not the four vectors QEMU gives it: {line}"
        ));
    }
    violations
}

/// What the file system server has to report of the disk it was handed:
/// the capacity the device answered, and the volume it mounted or wrote.
fn block_lines(output: &str) -> Vec<String> {
    let mut violations = Vec::new();
    if output.contains("[files] no disk") {
        violations.push(
            "the root task handed over no block device, though the machine carries one".to_owned(),
        );
        return violations;
    }
    for line in output.lines().filter(|line| line.starts_with("[init] no ")) {
        violations.push(format!("the root task refused the handover: {line}"));
    }
    // Both disks are driven and both volumes are mounted: the one the
    // firmware wrote, and the one the system formatted.
    for name in ["boot", "scratch"] {
        if !output.contains(&format!("[files] {name} volume: clusters=")) {
            violations.push(format!("the file system server mounted no {name} volume"));
        }
    }
    // The scratch volume is 64 MiB of one-sector clusters less what the
    // tables and the reserved sectors take; a volume of the other disk's
    // size would be the boot one twice.
    if !output.contains(&format!(
        "[files] scratch volume: clusters={SCRATCH_CLUSTERS}"
    )) {
        violations.push(format!(
            "the scratch volume is not the {SCRATCH_CLUSTERS} clusters of a 64 MiB disk"
        ));
    }
    violations
}

/// Clusters of the scratch volume: [`SCRATCH_SIZE`] of 512-byte sectors,
/// one sector to a cluster, less the reserved sectors and the two tables
/// the format writes.
const SCRATCH_CLUSTERS: u64 = 128_992;

/// That the volume the server mounted is the boot one and that it was
/// mounted rather than written over.
///
/// The boot volume carries the loader, the kernel and the boot image, so
/// most of its clusters are taken. A disk this server had formatted would
/// report all but one of them free, which is what this refuses.
fn boot_volume_lines(output: &str) -> Vec<String> {
    let Some(line) = output
        .lines()
        .find(|line| line.starts_with("[files] boot volume: clusters="))
    else {
        return vec!["the server said nothing of the volume it mounted".to_owned()];
    };
    let numbers: Vec<u32> = line
        .split(|byte: char| !byte.is_ascii_digit())
        .filter_map(|word| word.parse().ok())
        .collect();
    let (Some(clusters), Some(free)) = (numbers.first(), numbers.get(1)) else {
        return vec![format!("the geometry names no cluster counts: {line}")];
    };
    if free.saturating_mul(2) >= *clusters {
        return vec![format!(
            "the boot volume was written over rather than mounted: {line}"
        )];
    }
    Vec::new()
}

/// How many lines the second client writes while the first writes its own.
///
/// It repeats the number of the run in the text, so a line that lost bytes
/// to another writer cannot be mistaken for one that kept them. `LINES` in
/// `app-checks` says the same number; a disagreement makes this run fail
/// with the line it could not find.
const E2E_INTERLEAVED: usize = 8;

/// Every line of the second client that did not arrive whole and exactly
/// once.
fn torn_lines(output: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for index in 0..E2E_INTERLEAVED {
        let wanted = format!("[checks] line {index} of {E2E_INTERLEAVED}, and the whole of it");
        let seen = output.lines().filter(|line| line.trim() == wanted).count();
        if seen != 1 {
            violations.push(format!(
                "the line `{wanted}` stands {seen} times, not once: two writers tore it"
            ));
        }
    }
    violations
}

/// The end-to-end tests: the whole system, from the loader to a line an
/// application wrote through a driver that runs at ring three.
///
/// # Errors
///
/// [`Error::Violations`] for every line of [`E2E_LINES`] that did not come;
/// the errors of the build, of the images, and of the machine.
fn test_e2e(root: &Path, options: &[String]) -> Result<(), Error> {
    build(root, options)?;
    image(root, options)?;
    let machine = Machine::locate()?;
    let path = root.join("target").join("audhsos.img");
    let socket = qemu::socket_path("audhsos-qmp")?;
    let _ = std::fs::remove_file(&socket);
    let run = qemu::Options {
        qmp: Some(socket.clone()),
        // The end-to-end run carries a disk of its own, under its own
        // name, so that what it writes survives into a second boot and
        // meets no other run (D-136). It starts blank, so that what the
        // second boot finds is what the first boot wrote.
        scratch: Some(blank_scratch_image(root, "e2e")?),
        ..qemu::Options::plain()
    };
    let forwarded = run.network;
    let mut session = Session::start(&machine, &path, &run)?;

    let mut violations = Vec::new();
    for (needle, complaint) in E2E_LINES {
        if !session.wait_for(needle, E2E_TIMEOUT) {
            violations.push((*complaint).to_owned());
            break;
        }
    }
    if violations.is_empty() {
        violations.extend(virtio_lines(&session.output()));
    }
    if violations.is_empty() {
        violations.extend(block_lines(&session.output()));
    }
    // The network, before anything else this run drives: the program of
    // the image takes the connection the forwarded port opens, sends back
    // what it was sent, and then makes an HTTP request over the same
    // connection, which this answers. It waits for the connection with a
    // deadline of its own, so the runner opens it as soon as the program
    // says it is listening.
    if violations.is_empty() {
        violations.extend(exchange_over_the_network(forwarded, &mut session));
    }
    if violations.is_empty() {
        violations.extend(network_lines(&session.output()));
    }
    if violations.is_empty() {
        violations.extend(anchor_lines(root, &session.output())?);
    }
    // The picture, while the machine still runs: `app-hello` is waiting to
    // be typed at, so nothing has ended yet. What is on the screen is
    // checked against what `app-paint` says it drew and against the font
    // this system carries, never against a resolution assumed here.
    if violations.is_empty() {
        match screen_of(&socket, root) {
            Ok(image) => violations.extend(drawn_lines(&image, &session.output())),
            Err(error) => violations.push(format!("no picture of the screen: {error}")),
        }
    }
    // Console input: the bytes go the other way, through the same port, and
    // only once the program on the far side says it is listening — a byte
    // sent earlier would reach a controller whose receive path is not up.
    if violations.is_empty() {
        if session.wait_for("[hello] ready", E2E_TIMEOUT) {
            session.send(b"typed\n")?;
            if !session.wait_for("[hello] echo: typed", E2E_TIMEOUT) {
                violations.push("what was typed did not come back".to_owned());
            }
        } else {
            violations.push("the application never said it was ready".to_owned());
        }
    }
    // Input: the keyboard and the mouse of the machine, driven through the
    // machine protocol. Nothing is injected before the program that listens
    // says it is subscribed — an event sent earlier would reach a server
    // whose client has no ring yet.
    if violations.is_empty() {
        match inject_input(&socket, &mut session) {
            Ok(()) => violations.extend(input_lines(&session.output())),
            Err(error) => violations.push(format!("nothing could be injected: {error}")),
        }
    }
    if violations.is_empty() {
        violations.extend(drive_the_screen(&socket, &mut session, root));
    }
    // Two clients wrote at once and no line of either may be torn: every
    // line the second one wrote has to stand whole and once, which the
    // last one arriving does not by itself say.
    if violations.is_empty() {
        violations.extend(torn_lines(&session.output()));
    }
    // The file the first boot wrote, and a file whose length crosses a
    // cluster, read back byte for byte.
    if violations.is_empty() {
        if session.wait_for(FILES_DONE, E2E_TIMEOUT) {
            violations.extend(file_lines(&session.output(), true));
        } else {
            violations.push("the program that uses the volume did not finish".to_owned());
        }
    }
    // The run ends itself: the application reports to the root task, and
    // the root task writes to the exit device through its `SystemControl`.
    // A run this had to kill would say nothing about whether that works.
    if violations.is_empty() {
        match session.wait_for_end(E2E_TIMEOUT) {
            None => violations.push("the machine did not end by itself".to_owned()),
            status => {
                let outcome = qemu::outcome_of(status, false);
                if outcome != qemu::Outcome::Success {
                    violations.push(format!("the machine reported a {}", outcome.name()));
                }
            }
        }
    }
    let output = session.finish();
    let _ = std::fs::remove_file(&socket);
    note!(
        "qemu end-to-end: {} line(s) of output",
        output.lines().count()
    );
    report("end-to-end", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the end-to-end run ---");
        eprintln!("{output}");
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)?;
    test_the_same_disk_again(&machine, &path, root)?;
    test_without_a_framebuffer(&machine, &path)?;
    test_without_a_network(&machine, &path)?;
    test_the_secure_shell_client(&machine, &path, root)?;
    test_the_tls_client(&machine, &path, root)
}

/// The half of catalog 6.6.65 that the run's anchor is: the root of the
/// chain the server presents reaches the program of the image over the
/// scratch volume, and the program holds it beside the anchors of the
/// boot volume (D-150).
///
/// The handshake over that trust set is step T8 of document 11 and is
/// Phase 15; this run is where its remaining lines go.
fn test_the_tls_client(machine: &Machine, path: &Path, root: &Path) -> Result<(), Error> {
    let material = tls::Material::new()?;
    let server = tls::Server::start(&material)?;
    note!(
        "tls server: port {}, one root for {}",
        server.port(),
        tls::NAME
    );
    let files = tls::scratch_files(server.port(), &material);
    let scratch = written_scratch_image(root, "tls", &files)?;
    let mut session = Session::start(
        machine,
        path,
        &qemu::Options {
            scratch: Some(scratch),
            ..qemu::Options::plain()
        },
    )?;

    let mut violations = Vec::new();
    for (needle, complaint) in tls_lines(server.port(), root)? {
        if !session.wait_for(&needle, E2E_TIMEOUT) {
            violations.push(complaint);
            break;
        }
    }
    let output = session.finish();
    report("tls", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the TLS run ---");
        eprintln!("{output}");
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)
}

/// What the TLS run has to see, in this order: three lines of the boot,
/// then the anchors of the boot volume, where the run's server is, the
/// root of its chain, and one trust set holding both.
///
/// The three boot lines are checkpoints and not assertions of their own.
/// Each line of this list gets [`E2E_TIMEOUT`] to itself, and `app-tls`
/// is the second to last program the root task starts: without them one
/// timeout would have to cover the firmware, the kernel, and the loading
/// of fifteen programs off the volume, one message of two kibibytes at a
/// time (D-92). The run that failed in CI reached `app-ssh` and lost the
/// budget there.
///
/// # Errors
///
/// Whatever reading the anchor directory answers.
fn tls_lines(port: u16, root: &Path) -> Result<Vec<(String, String)>, Error> {
    let carried = anchors::of(root)?.len();
    Ok(vec![
        (
            "[init] started server-net".to_owned(),
            "the machine did not get as far as the network server".to_owned(),
        ),
        (
            "[init] started app-ssh".to_owned(),
            "the machine did not get as far as the program before `app-tls`".to_owned(),
        ),
        (
            "[init] started app-tls".to_owned(),
            "the root task did not start the program that holds the anchors".to_owned(),
        ),
        (
            format!("[tls-app] anchors={carried}"),
            "the program did not read the anchor table of the boot volume".to_owned(),
        ),
        (
            format!("[tls-app] run server=10.0.2.2:{port} name={}", tls::NAME),
            "the run's port and name did not reach the program over the scratch volume".to_owned(),
        ),
        (
            "[tls-app] run anchor subject=".to_owned(),
            "the program refused the root of the run's chain".to_owned(),
        ),
        (
            format!("[tls-app] trust anchors={}", carried.saturating_add(1)),
            "the run's root did not join the anchors of the image".to_owned(),
        ),
    ])
}

/// The acceptance of track S: the client of the image reaches a command on
/// an OpenSSH server and reads what it wrote.
///
/// This is the one check from outside that the exchange hash, the key
/// derivation and the packet layer are what the documents mean
/// (14.12). It is a run of its own, with a scratch disk of its own, so
/// that the end-to-end run above keeps the blank disk its persistence
/// test needs.
fn test_the_secure_shell_client(machine: &Machine, path: &Path, root: &Path) -> Result<(), Error> {
    let material = ssh::material(root)?;
    let account = ssh::account()?;
    let server = ssh::Server::start(root, &material)?;
    note!(
        "sshd: port {}, {} fingerprint(s) trusted",
        server.port(),
        material.fingerprints.len()
    );
    let files = ssh::scratch_files(server.port(), &account, &material);
    let scratch = written_scratch_image(root, "ssh", &files)?;
    let mut session = Session::start(
        machine,
        path,
        &qemu::Options {
            scratch: Some(scratch),
            ..qemu::Options::plain()
        },
    )?;

    let mut violations = Vec::new();
    for (needle, complaint) in ssh_lines() {
        if !session.wait_for(&needle, E2E_TIMEOUT) {
            violations.push(complaint.to_owned());
            break;
        }
    }
    let output = session.finish();
    report("secure shell", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the Secure Shell run ---");
        eprintln!("{output}");
        eprintln!("--- what the server said ---");
        eprintln!("{}", server.log());
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)
}

/// What the client of the image has to reach, in this order.
///
/// The command writes on both streams and exits with a status that is not
/// zero, so a client that reads only the first stream and one that
/// reports no status both fail here. The three needles that quote the
/// command are built from the constants the command itself is built from,
/// so the check cannot drift away from what the guest is told to run.
fn ssh_lines() -> [(String, &'static str); 6] {
    [
        (
            "[init] started app-ssh".to_owned(),
            "the Secure Shell client did not start",
        ),
        (
            "[ssh-app] connected to 10.0.2.2:".to_owned(),
            "the client reached no server: it read no configuration off the volume, or the connection was refused",
        ),
        (
            "[ssh-app] command started".to_owned(),
            "the client did not get through the key exchange, the host key, the authentication and the channel",
        ),
        (
            format!("[ssh-app] stdout: {}", ssh::guest::SAID),
            "the client did not read what the command wrote",
        ),
        (
            format!("[ssh-app] stderr: {}", ssh::guest::COMPLAINED),
            "the client did not read the extended data of the command",
        ),
        (
            format!("[ssh-app] exit status {}", ssh::guest::STATUS),
            "the client did not take the exit status of the command",
        ),
    ]
}

/// What the second boot has to reach before anything is typed at it, in
/// this order. Every program outside the boot set is read off the volume,
/// so the run passes these one at a time rather than waiting once for the
/// last of them.
const SECOND_BOOT_LINES: [(&str, &str); 7] = [
    (
        "[files] boot volume: clusters=",
        "the second boot mounted no boot volume",
    ),
    (
        "[init] started server-display",
        "the second boot read no program off the volume",
    ),
    (
        "[init] started app-canvas",
        "the second boot stopped before the program that draws",
    ),
    (
        "[init] started server-net",
        "the second boot stopped before the network server",
    ),
    (
        "[init] started app-net",
        "the second boot stopped before the network client",
    ),
    (
        "[init] started app-files",
        "the second boot did not read every program off the volume",
    ),
    ("[hello] ready", "the second boot never said it was ready"),
];

/// What the program that reads the trust anchors has to report: as many
/// anchors as the image was given, and one line per anchor, because a
/// table whose records parse one by one is the whole of what the program
/// can say before a handshake exists to use them.
///
/// # Errors
///
/// Whatever reading the anchor directory answers.
fn anchor_lines(root: &Path, output: &str) -> Result<Vec<String>, Error> {
    let carried = anchors::of(root)?.len();
    let mut violations = Vec::new();
    if !output.contains(&format!("[tls-app] anchors={carried}")) {
        violations.push(format!(
            "the image carries {carried} trust anchor(s) and the program reported another number"
        ));
    }
    let read = output.matches("[tls-app] anchor subject=").count();
    if read != carried {
        violations.push(format!(
            "the program read {read} of the {carried} anchor(s) of the image"
        ));
    }
    Ok(violations)
}

/// The line the program that uses the volume writes last.
const FILES_DONE: &str = "[files-app] entries=";

/// What that program has to report.
///
/// `first` says whether this is the boot that finds a blank volume: the
/// first writes the file, the second finds it and compares the bytes.
fn file_lines(output: &str, first: bool) -> Vec<String> {
    let mut violations = Vec::new();
    let wanted = if first {
        "[files-app] first boot: wrote 30 bytes"
    } else {
        "[files-app] second boot: 30 bytes, same=true"
    };
    if !output.contains(wanted) {
        violations.push(format!("the volume did not carry the file: no `{wanted}`"));
    }
    // The length crosses a cluster, which on this volume is one sector.
    if !output.contains("[files-app] long file: wrote 700 read 700 same=true") {
        violations.push(
            "a file whose length crosses a cluster did not come back byte for byte".to_owned(),
        );
    }
    if !output.contains("[files-app] entry BOOT.TXT") {
        violations.push("the listing of the volume names no BOOT.TXT".to_owned());
    }
    violations
}

/// What the runner sends through the forwarded port, which comes back
/// byte for byte.
const ECHO: &[u8] = b"a line from the development machine\n";

/// What the runner answers the request with.
const RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\nConnection: close\r\n\r\nhello, world\n";

/// How many bytes of the body that response carries.
const RESPONSE_BODY: usize = 13;

/// How long the runner waits for one read of the connection.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(30);

/// Opens the forwarded port, exchanges bytes both ways, and answers the
/// request the program of the image makes over the same connection.
///
/// The connection carries two exchanges because the machine has one port
/// forwarded into it: the echo first, then a request and its answer. What
/// is checked here is what crossed the link; what the program made of it
/// is checked in [`network_lines`].
fn exchange_over_the_network(port: Option<u16>, session: &mut Session) -> Vec<String> {
    let Some(port) = port else {
        return vec!["the run forwarded no port".to_owned()];
    };
    if !session.wait_for("[net-app] listening on", E2E_TIMEOUT) {
        return vec!["the program of the image never listened".to_owned()];
    }
    let mut violations = match speak(port) {
        Ok(violations) => violations,
        Err(error) => vec![format!("the forwarded port refused: {error}")],
    };
    // The bytes crossed the link; what the program made of them is a line
    // it writes after the connection is closed, and the checks that read
    // those lines run as soon as this returns.
    if violations.is_empty() && !session.wait_for("[net-app] closed", E2E_TIMEOUT) {
        violations.push("the program of the image never closed the connection".to_owned());
    }
    violations
}

/// The exchange itself, over one connection.
fn speak(port: u16) -> std::io::Result<Vec<String>> {
    use std::io::{Read, Write};
    let mut violations = Vec::new();
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(NETWORK_TIMEOUT))?;
    stream.set_write_timeout(Some(NETWORK_TIMEOUT))?;
    stream.write_all(ECHO)?;
    let mut back = vec![0u8; ECHO.len()];
    stream.read_exact(&mut back)?;
    if back != ECHO {
        violations.push(format!(
            "what came back is not what went out: {:?}",
            String::from_utf8_lossy(&back)
        ));
    }
    // The request the program makes over the same connection, read up to
    // the empty line that ends its head.
    let mut request = Vec::new();
    let mut byte = [0u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        let read = stream.read(&mut byte)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&byte);
        if request.len() > 4096 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&request).into_owned();
    if !head.starts_with("GET / HTTP/1.1\r\n") {
        violations.push(format!("the request is not a GET: {head:?}"));
    }
    if !head.to_ascii_lowercase().contains("host:") {
        violations.push(format!("the request names no host: {head:?}"));
    }
    stream.write_all(RESPONSE)?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(violations)
}

/// What the network server and its client have to report.
fn network_lines(output: &str) -> Vec<String> {
    let mut violations = Vec::new();
    if output.contains("[net] no interface") {
        violations.push(
            "the root task handed over no network device, though the machine carries one"
                .to_owned(),
        );
        return violations;
    }
    for (needle, complaint) in [
        (
            format!("[net] device mac={}", qemu::GUEST_MAC),
            "the driver did not report the address the command line gave the device".to_owned(),
        ),
        (
            format!("[net] lease address={}", qemu::GUEST_ADDRESS),
            "the address configuration client reached no lease".to_owned(),
        ),
        (
            "[net-app] lease=yes".to_owned(),
            "the client was told of no lease".to_owned(),
        ),
        (
            "gateway=10.0.2.2".to_owned(),
            "the gateway of the link never reached the client".to_owned(),
        ),
        (
            "[net-app] resolved example.com:".to_owned(),
            "the name was not resolved".to_owned(),
        ),
        (
            format!("[net-app] echo {} bytes", ECHO.len()),
            "the program did not echo what was sent to it".to_owned(),
        ),
        (
            format!("[net-app] http status=200 body={RESPONSE_BODY}"),
            "the answer to the request was not parsed".to_owned(),
        ),
        (
            "[net-app] closed".to_owned(),
            "the connection was not closed cleanly".to_owned(),
        ),
    ] {
        if !output.contains(&needle) {
            violations.push(format!("{complaint}: no `{needle}`"));
        }
    }
    // A resolution that answered no address is a line with nothing after
    // the colon, which the check above would take for a success.
    if let Some(line) = output
        .lines()
        .find(|line| line.contains("[net-app] resolved example.com:"))
        && line
            .split(':')
            .nth(1)
            .is_none_or(|rest| rest.trim().is_empty())
    {
        violations.push(format!("the resolution named no address: {line}"));
    }
    violations
}

/// Boots the same disk a second time, which is the persistence test and
/// the reason the scratch disk is kept across runs (D-136).
///
/// # Errors
///
/// [`Error::Violations`] when the second boot does not find what the first
/// one wrote; the errors of the machine.
fn test_the_same_disk_again(machine: &Machine, path: &Path, root: &Path) -> Result<(), Error> {
    let socket = qemu::socket_path("audhsos-qmp-again")?;
    let _ = std::fs::remove_file(&socket);
    let mut session = Session::start(
        machine,
        path,
        &qemu::Options {
            qmp: Some(socket.clone()),
            scratch: Some(scratch_image(root, "e2e")?),
            ..qemu::Options::plain()
        },
    )?;
    let mut violations = Vec::new();
    // The programs outside the boot set are read off the volume one
    // message at a time, so the boot takes minutes rather than seconds and
    // each line is waited for on its own. What is typed goes first: the
    // line the program that reads the volume writes last comes after
    // `[hello] ready`, and a wait for a line that has already gone past
    // never ends.
    for (needle, complaint) in SECOND_BOOT_LINES {
        if !session.wait_for(needle, E2E_TIMEOUT) {
            violations.push((*complaint).to_owned());
            break;
        }
    }
    if violations.is_empty() {
        session.send(b"typed\n")?;
    }
    if violations.is_empty() {
        if session.wait_for(FILES_DONE, E2E_TIMEOUT) {
            violations.extend(file_lines(&session.output(), false));
        } else {
            violations.push("the second boot did not reach the volume".to_owned());
        }
    }
    // The programs that listen report only once something was typed or
    // moved at them, and the machine ends when every program that reports
    // has reported. So the same events go in here as in the first boot.
    if violations.is_empty() {
        match inject_input(&socket, &mut session) {
            Ok(()) => {}
            Err(error) => violations.push(format!("nothing could be injected: {error}")),
        }
    }
    if violations.is_empty() {
        violations.extend(drive_the_screen(&socket, &mut session, root));
    }
    if violations.is_empty() {
        match session.wait_for_end(E2E_TIMEOUT) {
            None => violations.push("the second boot did not end by itself".to_owned()),
            status => {
                let outcome = qemu::outcome_of(status, false);
                if outcome != qemu::Outcome::Success {
                    violations.push(format!("the machine reported a {}", outcome.name()));
                }
            }
        }
    }
    let output = session.finish();
    let _ = std::fs::remove_file(&socket);
    note!(
        "qemu the same disk again: {} line(s)",
        output.lines().count()
    );
    report("the same disk again", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the second boot ---");
        eprintln!("{output}");
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)
}

/// Where `app-paint` fills its rectangle and what it fills it with. It says
/// so itself in `app_paint.rs`; a disagreement makes this run fail with the
/// pixel it did not find.
const PAINT_BOX: (u32, u32, u32, u32) = (64, 64, 96, 48);
const PAINT_COLOR: (u8, u8, u8) = (0x20, 0xC0, 0x40);

/// Where it writes its line, and what it writes.
const PAINT_TEXT: (u32, u32) = (64, 128);
const PAINT_STRING: &str = "AuDHSOS";

/// Takes a picture of the screen of the running machine.
fn screen_of(socket: &Path, root: &Path) -> Result<ppm::Image, Error> {
    let mut qmp = Qmp::connect(socket, E2E_TIMEOUT)?;
    qmp.screendump(&root.join("target").join("screen.ppm"))
}

/// What the picture does not show of what `app-paint` said it drew.
fn drawn_lines(image: &ppm::Image, output: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let Some((width, height)) = screen_size(output) else {
        violations.push("the display server never said what the screen is".to_owned());
        return violations;
    };
    if (image.width(), image.height()) != (width, height) {
        violations.push(format!(
            "the picture is {}x{} and the display server said {width}x{height}",
            image.width(),
            image.height()
        ));
        return violations;
    }
    let (x, y, box_width, box_height) = PAINT_BOX;
    match image.count_of(x, y, box_width, box_height, PAINT_COLOR) {
        Ok(count) if count == box_width.saturating_mul(box_height) => {}
        Ok(count) => violations.push(format!(
            "{count} of {} pixels of the rectangle carry its color",
            box_width.saturating_mul(box_height)
        )),
        Err(error) => violations.push(format!("the rectangle is not on the screen: {error}")),
    }
    for (at_x, at_y) in [
        (x.saturating_sub(1), y),
        (x, y.saturating_sub(1)),
        (x.saturating_add(box_width), y),
    ] {
        match image.pixel(at_x, at_y) {
            Ok((0, 0, 0)) => {}
            Ok(color) => violations.push(format!(
                "the pixel at {at_x},{at_y} is {color:?} and should be black"
            )),
            Err(error) => violations.push(format!("the pixel at {at_x},{at_y}: {error}")),
        }
    }
    violations.extend(text_lines(image));
    violations
}

/// What the picture does not show of the line `app-paint` wrote, glyph by
/// glyph against the font this system carries.
fn text_lines(image: &ppm::Image) -> Vec<String> {
    let (left, top) = PAINT_TEXT;
    for (index, character) in PAINT_STRING.chars().enumerate() {
        let rows = gfx::glyph(character);
        let cell = u32::try_from(index)
            .unwrap_or(0)
            .saturating_mul(gfx::GLYPH_WIDTH);
        for (row, bits) in rows.iter().enumerate() {
            for column in 0..gfx::GLYPH_WIDTH {
                let bit = 1_u8
                    .checked_shl(gfx::GLYPH_WIDTH.saturating_sub(1).saturating_sub(column))
                    .unwrap_or(0);
                let wanted = if bits & bit == 0 {
                    (0, 0, 0)
                } else {
                    (0xFF, 0xFF, 0xFF)
                };
                let at_x = left.saturating_add(cell).saturating_add(column);
                let at_y = top.saturating_add(u32::try_from(row).unwrap_or(0));
                match image.pixel(at_x, at_y) {
                    Ok(color) if color == wanted => {}
                    Ok(color) => {
                        return vec![format!(
                            "the pixel at {at_x},{at_y} of the glyph {character:?} is {color:?}, not {wanted:?}"
                        )];
                    }
                    Err(error) => return vec![format!("the text is not on the screen: {error}")],
                }
            }
        }
    }
    Vec::new()
}

/// The resolution the display server reported, out of the serial output.
fn screen_size(output: &str) -> Option<(u32, u32)> {
    let line = output
        .lines()
        .find_map(|line| line.split("[display] screen=").nth(1))?;
    let mode = line.split_whitespace().next()?;
    let (width, height) = mode.split_once('x')?;
    Some((width.parse().ok()?, height.trim().parse().ok()?))
}

/// The keys the runner injects, by the name QEMU knows them under and the
/// key this system reports for each.
const INPUT_KEYS: [(&str, driver_i8042::KeyCode); 3] = [
    ("a", driver_i8042::KeyCode::A),
    ("b", driver_i8042::KeyCode::B),
    ("c", driver_i8042::KeyCode::C),
];

/// The key that ends the program that listens, which is the last thing
/// injected.
const INPUT_END: (&str, driver_i8042::KeyCode) = ("esc", driver_i8042::KeyCode::Escape);

/// The steps of the pointer path, in pixels. Each goes out as one command,
/// so each is one packet of the mouse.
const INPUT_PATH: [(i32, i32); 3] = [(5, 0), (0, 7), (-3, -2)];

/// One more step, injected after the button has come back up.
const INPUT_LAST: (i32, i32) = (2, -4);

/// The line the program that listens writes for [`INPUT_LAST`]: the last
/// thing the pointer does, and the first packet after the button that says
/// the button is up. A mouse of this machine reports a button coming up in
/// the next packet it sends and not in one of its own, which is why the
/// path has a step behind the button rather than ending at it.
fn last_motion_line() -> String {
    let (dx, dy) = INPUT_LAST;
    format!("[input] pointer {dx} {dy} 0 0")
}

/// The line the program writes when the button goes down.
const BUTTON_PRESSED: &str = "[input] pointer 0 0 0 1";

/// Types at the machine and moves its pointer, once the program that
/// listens says it is subscribed.
///
/// Every event waits for the line the program writes for it before the next
/// one goes out. The controller of this machine holds sixteen bytes and
/// drops what does not fit, which a run that sent everything at once found:
/// six packets of the mouse are twenty-four bytes and the last two of them
/// were never seen.
fn inject_input(socket: &Path, session: &mut Session) -> Result<(), Error> {
    for line in [
        "[checks] input lifecycle and isolation: ok",
        "is gone: ring released",
    ] {
        if !session.wait_for(line, E2E_TIMEOUT) {
            return Err(Error::Usage(format!(
                "input regression did not complete: `{line}`"
            )));
        }
    }
    if !session.wait_for("[input] ready", E2E_TIMEOUT) {
        return Err(Error::Usage(
            "the program that listens never subscribed".to_owned(),
        ));
    }
    let mut qmp = Qmp::connect(socket, E2E_TIMEOUT)?;
    for (qcode, code) in INPUT_KEYS {
        for pressed in [true, false] {
            qmp.send_key(qcode, pressed)?;
            let line = format!("[input] key {} {}", code.code(), u8::from(pressed));
            if !session.wait_for(&line, E2E_TIMEOUT) {
                return Err(Error::Usage(format!("the machine never said `{line}`")));
            }
        }
    }
    for (dx, dy) in INPUT_PATH {
        qmp.move_pointer(dx, dy)?;
        let line = format!("[input] pointer {dx} {dy} 0 0");
        if !session.wait_for(&line, E2E_TIMEOUT) {
            return Err(Error::Usage(format!("the machine never said `{line}`")));
        }
    }
    qmp.button(Button::Left, true)?;
    if !session.wait_for(BUTTON_PRESSED, E2E_TIMEOUT) {
        return Err(Error::Usage(
            "the button of the pointer never went down".to_owned(),
        ));
    }
    // A mouse of this machine reports a button coming up in the next packet
    // it sends and not in one of its own, so the release and the step after
    // it go out together and the step is what is waited for.
    qmp.button(Button::Left, false)?;
    let (dx, dy) = INPUT_LAST;
    qmp.move_pointer(dx, dy)?;
    if !session.wait_for(&last_motion_line(), E2E_TIMEOUT) {
        return Err(Error::Usage(
            "the button of the pointer never came back up".to_owned(),
        ));
    }
    let (qcode, _code) = INPUT_END;
    qmp.send_key(qcode, true)?;
    qmp.send_key(qcode, false)?;
    if !session.wait_for("[input] done", E2E_TIMEOUT) {
        return Err(Error::Usage(
            "the program that listens never saw the key that ends it".to_owned(),
        ));
    }
    Ok(())
}

/// The numbers of the `[input]` lines of one kind, one row per line.
fn input_events(output: &str, kind: &str) -> Vec<Vec<i64>> {
    let head = format!("[input] {kind} ");
    output
        .lines()
        .filter_map(|line| line.split(&head).nth(1))
        .map(|rest| {
            rest.split_whitespace()
                .filter_map(|word| word.parse().ok())
                .collect()
        })
        .collect()
}

/// What the `[input]` lines do not say about what was injected.
///
/// The two devices are checked apart from each other. The controller has one
/// output buffer and two queues behind it, and which of them it hands out
/// first is its own business, so a key and a packet of the mouse may arrive
/// in either order; what each device says on its own is in the order it said
/// it.
fn input_lines(output: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let keys: Vec<Vec<i64>> = input_events(output, "key");
    let wanted: Vec<Vec<i64>> = INPUT_KEYS
        .into_iter()
        .chain(core::iter::once(INPUT_END))
        .flat_map(|(_qcode, code)| {
            [
                vec![i64::from(code.code()), 1],
                vec![i64::from(code.code()), 0],
            ]
        })
        .collect();
    if keys != wanted {
        violations.push(format!(
            "the keys came back as {keys:?} and were injected as {wanted:?}"
        ));
    }
    let pointer = input_events(output, "pointer");
    if pointer.is_empty() {
        violations.push("the pointer never moved".to_owned());
        return violations;
    }
    let moved = |index: usize| -> i64 {
        pointer
            .iter()
            .filter_map(|row| row.get(index).copied())
            .sum()
    };
    let path: Vec<(i32, i32)> = INPUT_PATH
        .into_iter()
        .chain(core::iter::once(INPUT_LAST))
        .collect();
    let sent = (
        path.iter().map(|(dx, _dy)| i64::from(*dx)).sum::<i64>(),
        path.iter().map(|(_dx, dy)| i64::from(*dy)).sum::<i64>(),
    );
    if (moved(0), moved(1)) != sent {
        violations.push(format!(
            "the pointer moved by {:?} and was moved by {sent:?}",
            (moved(0), moved(1))
        ));
    }
    // The button: it goes down in one packet and comes up in a later one.
    let down = pointer
        .iter()
        .position(|row| row.get(3).copied().unwrap_or(0) != 0);
    let up = down.and_then(|first| {
        pointer
            .iter()
            .skip(first)
            .position(|row| row.get(3).copied().unwrap_or(0) == 0)
            .map(|later| first.saturating_add(later))
    });
    match (down, up) {
        (Some(down), Some(up)) if down < up => {}
        _other => violations.push(format!(
            "the button was pressed at {down:?} and released at {up:?}"
        )),
    }
    violations
}

/// How far the pointer is moved for each of the two steps that show the
/// cursor sprite moving. Both are larger than the sprite, so the place it
/// came from and the place it went to do not overlap and each can be looked
/// at on its own.
const CANVAS_CURSOR_STEPS: [(i32, i32); 2] = [(40, 24), (-30, 40)];

/// The stroke: the button goes down, the pointer moves by this, and the
/// button comes up again. It is long enough that its middle lies well clear
/// of the sprite at either end.
const CANVAS_STROKE: (i32, i32) = (120, 60);

/// Where the pointer goes after the stroke, so that the sprite stands clear
/// of the segment when the picture of it is taken.
const CANVAS_ASIDE: (i32, i32) = (-200, 150);

/// What is typed at the canvas, by the name QEMU knows each key under and
/// the character this system types for it.
const CANVAS_TYPING: [(&str, char); 3] = [("h", 'h'), ("i", 'i'), ("1", '1')];

/// Drives the canvas: the escape key, two steps of the pointer, a stroke,
/// a typed string, and the key that ends it, with a picture of the screen
/// taken after each of the three.
///
/// It runs only on the machine that has a screen, and only after the
/// program that listens has ended: that one echoes every event to the
/// console, and what is injected here is not part of what it was checked
/// against.
fn inject_canvas(socket: &Path, session: &mut Session, root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    if !session.wait_for("[canvas] ready ", E2E_TIMEOUT) {
        return vec!["the canvas never said it was ready".to_owned()];
    }
    let mut qmp = match Qmp::connect(socket, E2E_TIMEOUT) {
        Ok(qmp) => qmp,
        Err(error) => return vec![format!("the canvas could not be driven: {error}")],
    };
    // The escape key first: it puts the background down over whatever the
    // program that paints left on the screen, so every pixel looked at
    // below is one the canvas itself wrote. The canvas has cleared itself
    // once already, for the escape that ended the program that listens, so
    // what is counted is the line before this one goes out.
    let cleared = session.count_seen("[canvas] cleared");
    if let Err(error) = press(&mut qmp, "esc") {
        return vec![format!("the escape key: {error}")];
    }
    if !session.wait_for_more("[canvas] cleared", cleared, E2E_TIMEOUT) {
        return vec!["the canvas did not clear itself".to_owned()];
    }
    violations.extend(canvas_cursor(&mut qmp, session, root));
    if violations.is_empty() {
        violations.extend(canvas_stroke(&mut qmp, session, root));
    }
    if violations.is_empty() {
        violations.extend(canvas_text(&mut qmp, session, root));
    }
    let done = session.count_seen("[canvas] done");
    if let Err(error) = press(&mut qmp, ENDS_QCODE) {
        violations.push(format!("the key that ends the canvas: {error}"));
    } else if !session.wait_for_more("[canvas] done", done, E2E_TIMEOUT) {
        violations.push("the canvas never ended".to_owned());
    }
    violations
}

/// Drives everything that draws on the screen, in the order the programs
/// of the image draw.
///
/// The canvas first: it is the two protocols in one program, its picture
/// is checked while the machine still runs, and it ends on a key. The
/// desktop after it, because the compositor paints over the whole screen
/// and every picture of the canvas would otherwise be of what the desktop
/// left there (D-126).
fn drive_the_screen(socket: &Path, session: &mut Session, root: &Path) -> Vec<String> {
    let violations = inject_canvas(socket, session, root);
    if !violations.is_empty() {
        return violations;
    }
    inject_desk(socket, session, root)
}

/// The name QEMU knows the key that shows the desktop under, and the one
/// that ends it.
const DESK_SHOWS_QCODE: &str = "f11";
const DESK_ENDS_QCODE: &str = "f12";

/// What is typed at the shell, by the name QEMU knows each key under.
const SHELL_TYPING: [&str; 10] = ["e", "c", "h", "o", "spc", "h", "e", "l", "l", "o"];

/// What the line typed at the shell asks for, as the shell repeats it.
const SHELL_LINE: &str = "[shell] ran echo hello";

/// Drives the desktop: the key that shows it, a picture of the bar, the
/// clock and the window of the shell, a line typed at the shell, and the
/// key that ends it.
///
/// It runs after the canvas has ended, because the desktop paints over the
/// whole screen and every picture taken before this one is of what the
/// programs that draw on the screen itself left there (D-126).
fn inject_desk(socket: &Path, session: &mut Session, root: &Path) -> Vec<String> {
    if !session.wait_for("[desk] ready ", E2E_TIMEOUT) {
        return vec!["the compositor never said it was ready".to_owned()];
    }
    if !session.wait_for("[desk] frame ", E2E_TIMEOUT) {
        return vec!["the shell never opened a window".to_owned()];
    }
    let Some(frame) = last_four(&session.output(), "[desk] frame ") else {
        return vec!["the compositor said nothing about where the window stands".to_owned()];
    };
    let mut qmp = match Qmp::connect(socket, E2E_TIMEOUT) {
        Ok(qmp) => qmp,
        Err(error) => return vec![format!("the desktop could not be driven: {error}")],
    };
    if let Err(error) = press(&mut qmp, DESK_SHOWS_QCODE) {
        return vec![format!("the key that shows the desktop: {error}")];
    }
    if !session.wait_for("[desk] shown", E2E_TIMEOUT) {
        return vec!["the desktop never painted itself".to_owned()];
    }
    let mut violations = desk_picture(&mut qmp, root, frame);
    if violations.is_empty() {
        violations.extend(shell_line(&mut qmp, session));
    }
    let done = session.count_seen("[desk] done");
    if let Err(error) = press(&mut qmp, DESK_ENDS_QCODE) {
        violations.push(format!("the key that ends the desktop: {error}"));
    } else if !session.wait_for_more("[desk] done", done, E2E_TIMEOUT) {
        violations.push("the desktop never ended".to_owned());
    }
    violations
}

/// What the picture does not show of the desktop: the bar, the clock, and
/// the window of the shell.
fn desk_picture(qmp: &mut Qmp, root: &Path, frame: ((u32, u32), (u32, u32))) -> Vec<String> {
    let image = match picture_of(qmp, root) {
        Ok(image) => image,
        Err(error) => return vec![format!("no picture of the desktop: {error}")],
    };
    let mut violations = Vec::new();
    match image.pixel(0, 0) {
        Ok(color) if color == rgb(server_desk::bar::BACKGROUND) => {}
        Ok(color) => violations.push(format!("the bar begins with {color:?}")),
        Err(error) => violations.push(format!("the bar is not on the screen: {error}")),
    }
    match image.pixel(0, server_desk::bar::HEIGHT.saturating_sub(1)) {
        Ok(color) if color == rgb(server_desk::bar::EDGE) => {}
        Ok(color) => violations.push(format!("the line under the bar is {color:?}")),
        Err(error) => violations.push(format!("the bar has no edge: {error}")),
    }
    let clock = server_desk::bar::clock_rect(image.width());
    match image.count_of(
        clock.x,
        clock.y,
        clock.w,
        clock.h,
        rgb(server_desk::bar::INK),
    ) {
        Ok(0) => violations.push("the clock shows nothing".to_owned()),
        Ok(_ink) => {}
        Err(error) => violations.push(format!("the clock is not on the screen: {error}")),
    }
    let ((x, y), (width, height)) = frame;
    match image.pixel(x, y.saturating_add(height).saturating_sub(1)) {
        Ok(color) if color == rgb(server_desk::window::FOCUSED) => {}
        Ok(color) => violations.push(format!("the frame of the window is {color:?}")),
        Err(error) => violations.push(format!("the window is not on the screen: {error}")),
    }
    let side = server_desk::window::CLOSE_SIZE;
    let close_x = x
        .saturating_add(width)
        .saturating_sub(side)
        .saturating_sub(server_desk::window::CLOSE_MARGIN);
    let close_y = y.saturating_add(server_desk::window::CLOSE_MARGIN);
    let wanted = side.saturating_mul(side);
    match image.count_of(
        close_x,
        close_y,
        side,
        side,
        rgb(server_desk::window::CLOSE_INK),
    ) {
        Ok(found) if found == wanted => {}
        Ok(found) => violations.push(format!(
            "{found} of {wanted} pixels of the close box carry its color"
        )),
        Err(error) => violations.push(format!("the close box is not on the screen: {error}")),
    }
    violations
}

/// Types one line at the shell and waits for the shell to repeat it.
fn shell_line(qmp: &mut Qmp, session: &mut Session) -> Vec<String> {
    let ran = session.count_seen(SHELL_LINE);
    for qcode in SHELL_TYPING {
        if let Err(error) = press(qmp, qcode) {
            return vec![format!("the key `{qcode}`: {error}")];
        }
    }
    if let Err(error) = press(qmp, "ret") {
        return vec![format!("the return key: {error}")];
    }
    if !session.wait_for_more(SHELL_LINE, ran, E2E_TIMEOUT) {
        return vec!["what was typed never reached the shell".to_owned()];
    }
    Vec::new()
}

/// A picture of the screen, taken over a connection that is already open.
///
/// The machine serves one monitor client at a time, so a second connection
/// beside the one that is driving the pointer reads nothing: the picture
/// goes through the same session as the events it is meant to show.
fn picture_of(qmp: &mut Qmp, root: &Path) -> Result<ppm::Image, Error> {
    qmp.screendump(&root.join("target").join("screen.ppm"))
}

/// Presses the key that ends the desktop and waits for the compositor to
/// say it is down.
///
/// Every run that has a screen needs this, as it needs `end_the_canvas`:
/// the compositor reports to the root task when the desktop is down and
/// not before, and the machine ends when every program that reports has
/// reported. The key works whether the desktop was ever painted or not.
fn end_the_desk(socket: &Path, session: &mut Session) -> Result<(), Error> {
    let mut qmp = Qmp::connect(socket, E2E_TIMEOUT)?;
    let done = session.count_seen("[desk] done");
    press(&mut qmp, DESK_ENDS_QCODE)?;
    if !session.wait_for_more("[desk] done", done, E2E_TIMEOUT) {
        return Err(Error::Usage(
            "the desktop never said it was down".to_owned(),
        ));
    }
    Ok(())
}

/// Presses the key that ends the canvas and waits for it to say it did.
///
/// Every run that has a screen needs this: the canvas presents nothing
/// until an event reaches it and ends on nothing but this key (D-126), and
/// a run that never sends it waits for the program until the time limit.
fn end_the_canvas(socket: &Path, session: &mut Session) -> Result<(), Error> {
    let mut qmp = Qmp::connect(socket, E2E_TIMEOUT)?;
    let done = session.count_seen("[canvas] done");
    press(&mut qmp, ENDS_QCODE)?;
    if !session.wait_for_more("[canvas] done", done, E2E_TIMEOUT) {
        return Err(Error::Usage("the canvas never said it was done".to_owned()));
    }
    Ok(())
}

/// The name QEMU knows [`app_canvas::ENDS`] under.
const ENDS_QCODE: &str = "f10";

/// Presses a key and lets it come up again.
fn press(qmp: &mut Qmp, qcode: &str) -> Result<(), Error> {
    qmp.send_key(qcode, true)?;
    qmp.send_key(qcode, false)
}

/// `canvas_cursor`: a pointer path moves the sprite, and the picture before
/// a step and the picture after it differ in exactly that way — the sprite
/// stands where the canvas said the pointer now is, and where it stood
/// before carries the background again.
fn canvas_cursor(qmp: &mut Qmp, session: &mut Session, root: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    let mut before: Option<(u32, u32)> = None;
    for (dx, dy) in CANVAS_CURSOR_STEPS {
        let Some(at) = moved_to(qmp, session, dx, dy) else {
            return vec![format!(
                "the canvas never said where the pointer went after {dx},{dy}"
            )];
        };
        let image = match picture_of(qmp, root) {
            Ok(image) => image,
            Err(error) => return vec![format!("no picture of the canvas: {error}")],
        };
        violations.extend(sprite_at(&image, at));
        if let Some(gone) = before {
            // The sprite is drawn by the display server and taken off by
            // it, so the place it left has to hold what stood under it,
            // which here is the background of the canvas.
            let corner = (gone.0.saturating_add(1), gone.1.saturating_add(5));
            match image.pixel(corner.0, corner.1) {
                Ok(color) if color == rgb(app_canvas::BACKGROUND) => {}
                Ok(color) => violations.push(format!(
                    "the sprite left {corner:?} showing {color:?} and not the background"
                )),
                Err(error) => violations.push(format!("the pixel at {corner:?}: {error}")),
            }
        }
        before = Some(at);
        if !violations.is_empty() {
            break;
        }
    }
    violations
}

/// Moves the pointer and answers with where the canvas says it now is.
fn moved_to(qmp: &mut Qmp, session: &mut Session, dx: i32, dy: i32) -> Option<(u32, u32)> {
    let already = session.count_seen("[canvas] cursor ");
    qmp.move_pointer(dx, dy).ok()?;
    if !session.wait_for_more("[canvas] cursor ", already, E2E_TIMEOUT) {
        return None;
    }
    last_pair(&session.output(), "[canvas] cursor ")
}

/// `canvas_stroke`: the button goes down, the pointer moves, and the pixels
/// along the segment the canvas says it drew carry the pen.
fn canvas_stroke(qmp: &mut Qmp, session: &mut Session, root: &Path) -> Vec<String> {
    let pressed = session.count_seen("[canvas] stroke ");
    if qmp.button(Button::Left, true).is_err() {
        return vec!["the button of the pointer could not be pressed".to_owned()];
    }
    if !session.wait_for_more("[canvas] stroke ", pressed, E2E_TIMEOUT) {
        return vec!["the canvas did not start a stroke when the button went down".to_owned()];
    }
    let held = session.count_seen("[canvas] stroke ");
    let (dx, dy) = CANVAS_STROKE;
    if qmp.move_pointer(dx, dy).is_err() {
        return vec!["the pointer could not be moved with the button down".to_owned()];
    }
    if !session.wait_for_more("[canvas] stroke ", held, E2E_TIMEOUT) {
        return vec!["the canvas drew no segment while the button was held".to_owned()];
    }
    let Some((from, to)) = last_four(&session.output(), "[canvas] stroke ") else {
        return vec!["the canvas said nothing about the segment it drew".to_owned()];
    };
    // The button comes up in the next packet the mouse sends, so the
    // release and the step that carries the pointer clear of the segment go
    // out together.
    if qmp.button(Button::Left, false).is_err() {
        return vec!["the button of the pointer could not be released".to_owned()];
    }
    let (aside_x, aside_y) = CANVAS_ASIDE;
    if moved_to(qmp, session, aside_x, aside_y).is_none() {
        return vec!["the pointer never moved clear of the stroke".to_owned()];
    }
    let image = match picture_of(qmp, root) {
        Ok(image) => image,
        Err(error) => return vec![format!("no picture of the stroke: {error}")],
    };
    pen_along(&image, from, to)
}

/// What the picture does not show of the segment the canvas said it drew.
///
/// The segment is walked along its longer axis, and every step of that walk
/// has to find the pen somewhere across the shorter one: a line drawn a
/// pixel at a time is continuous in the axis it advances fastest in, and
/// checking it that way needs no second copy of the algorithm that drew it.
fn pen_along(image: &ppm::Image, from: (u32, u32), to: (u32, u32)) -> Vec<String> {
    let pen = rgb(app_canvas::PEN);
    let wide = from.0.abs_diff(to.0) >= from.1.abs_diff(to.1);
    let (first, last) = if wide {
        (from.0.min(to.0), from.0.max(to.0))
    } else {
        (from.1.min(to.1), from.1.max(to.1))
    };
    let (across_first, across_last) = if wide {
        (from.1.min(to.1), from.1.max(to.1))
    } else {
        (from.0.min(to.0), from.0.max(to.0))
    };
    for along in first..=last {
        let found = (across_first..=across_last).any(|across| {
            let (x, y) = if wide {
                (along, across)
            } else {
                (across, along)
            };
            image.pixel(x, y) == Ok(pen)
        });
        if !found {
            return vec![format!(
                "the stroke from {from:?} to {to:?} carries no pen at {along} of {first}..={last}"
            )];
        }
    }
    Vec::new()
}

/// `canvas_text`: what is typed appears at the text cursor the canvas
/// named, pixel for pixel against the font this system carries.
fn canvas_text(qmp: &mut Qmp, session: &mut Session, root: &Path) -> Vec<String> {
    let mut cells = Vec::new();
    for (qcode, character) in CANVAS_TYPING {
        let written = session.count_seen("[canvas] text ");
        if let Err(error) = press(qmp, qcode) {
            return vec![format!("the key `{qcode}`: {error}")];
        }
        if !session.wait_for_more("[canvas] text ", written, E2E_TIMEOUT) {
            return vec![format!("the canvas never wrote `{character}`")];
        }
        let Some(at) = last_pair(&session.output(), "[canvas] text ") else {
            return vec![format!(
                "the canvas said nothing about where `{character}` went"
            )];
        };
        cells.push((at, character));
    }
    let image = match picture_of(qmp, root) {
        Ok(image) => image,
        Err(error) => return vec![format!("no picture of the typed text: {error}")],
    };
    let mut violations = Vec::new();
    for (at, character) in cells {
        violations.extend(glyph_at(&image, at, character));
        if !violations.is_empty() {
            break;
        }
    }
    violations
}

/// What the picture does not show of one glyph at one cell.
fn glyph_at(image: &ppm::Image, at: (u32, u32), character: char) -> Vec<String> {
    let rows = gfx::glyph(character);
    let ink = rgb(app_canvas::INK);
    let ground = rgb(app_canvas::BACKGROUND);
    for (row, bits) in rows.iter().enumerate() {
        for column in 0..gfx::GLYPH_WIDTH {
            let bit = 1_u8
                .checked_shl(gfx::GLYPH_WIDTH.saturating_sub(1).saturating_sub(column))
                .unwrap_or(0);
            let wanted = if bits & bit == 0 { ground } else { ink };
            let x = at.0.saturating_add(column);
            let y = at.1.saturating_add(u32::try_from(row).unwrap_or(0));
            match image.pixel(x, y) {
                Ok(color) if color == wanted => {}
                Ok(color) => {
                    return vec![format!(
                        "the pixel at {x},{y} of the typed {character:?} is {color:?}, not {wanted:?}"
                    )];
                }
                Err(error) => return vec![format!("the typed text is not on the screen: {error}")],
            }
        }
    }
    Vec::new()
}

/// What the picture does not show of the sprite standing at `at`.
///
/// Every pixel the arrow covers is checked, and what each should be is
/// asked of the display server itself rather than written out again here:
/// which pixels are the body and which the edge follows from the shape
/// table, and a test that carried its own copy of that rule would agree
/// with a wrong one just as readily.
fn sprite_at(image: &ppm::Image, at: (u32, u32)) -> Vec<String> {
    for row in 0..server_display::CURSOR_HEIGHT {
        for column in 0..server_display::CURSOR_WIDTH {
            let index = usize::try_from(row).unwrap_or(0);
            let Some(color) =
                server_display::pixel_of(server_display::CursorShape::Arrow, index, column)
            else {
                continue;
            };
            let x = at.0.saturating_add(column);
            let y = at.1.saturating_add(row);
            match image.pixel(x, y) {
                Ok(seen) if seen == rgb(color) => {}
                Ok(seen) => {
                    return vec![format!(
                        "the pointer is at {at:?} and the pixel at {x},{y} of its sprite is {seen:?}, not {:?}",
                        rgb(color)
                    )];
                }
                Err(error) => return vec![format!("the sprite is not on the screen: {error}")],
            }
        }
    }
    Vec::new()
}

/// A color of this system as a picture of the screen reports it.
const fn rgb(color: gfx::Color) -> (u8, u8, u8) {
    (color.r, color.g, color.b)
}

/// The two numbers of the last line beginning with `head`.
fn last_pair(output: &str, head: &str) -> Option<(u32, u32)> {
    let numbers = last_numbers(output, head)?;
    Some((*numbers.first()?, *numbers.get(1)?))
}

/// The two points of the last line beginning with `head`.
fn last_four(output: &str, head: &str) -> Option<((u32, u32), (u32, u32))> {
    let numbers = last_numbers(output, head)?;
    Some((
        (*numbers.first()?, *numbers.get(1)?),
        (*numbers.get(2)?, *numbers.get(3)?),
    ))
}

/// The numbers of the last line beginning with `head`.
fn last_numbers(output: &str, head: &str) -> Option<Vec<u32>> {
    let rest = output
        .lines()
        .filter_map(|line| line.split(head).nth(1))
        .next_back()?;
    Some(
        rest.split_whitespace()
            .filter_map(|word| word.parse().ok())
            .collect(),
    )
}

/// The same system on a machine with no graphics adapter: the firmware
/// reports no Graphics Output Protocol, so the kernel finds no framebuffer,
/// the display server answers that there is no screen, and the program that
/// draws says it drew nothing — and the run still ends by itself.
fn test_without_a_framebuffer(machine: &Machine, path: &Path) -> Result<(), Error> {
    let socket = qemu::socket_path("audhsos-qmp-novga")?;
    let _ = std::fs::remove_file(&socket);
    let mut session = Session::start(
        machine,
        path,
        &qemu::Options {
            no_vga: true,
            qmp: Some(socket.clone()),
            ..qemu::Options::plain()
        },
    )?;
    let mut violations = Vec::new();
    for (needle, complaint) in NO_VGA_LINES {
        if !session.wait_for(needle, E2E_TIMEOUT) {
            violations.push((*complaint).to_owned());
            break;
        }
    }
    // This run carries no scratch disk, so the only block device is the
    // one the firmware booted from. The server mounts its volume and does
    // not format it: a volume carrying the loader, the kernel and the
    // boot image has far fewer clusters free than it has clusters, where
    // one just formatted would have all but one.
    if violations.is_empty() {
        if session.wait_for("[files] boot volume: clusters=", E2E_TIMEOUT) {
            violations.extend(boot_volume_lines(&session.output()));
        } else {
            violations.push("the machine without a scratch disk mounted no volume".to_owned());
        }
    }
    // And it has nothing to write to, which it says rather than writing
    // to the volume it must not.
    if violations.is_empty() && !session.wait_for("[files] no scratch disk", E2E_TIMEOUT) {
        violations.push("a machine with nothing to write to did not say so".to_owned());
    }
    if violations.is_empty() && session.wait_for("[hello] ready", E2E_TIMEOUT) {
        session.send(b"typed\n")?;
    }
    // The i8042 is part of the machine whether it has a screen or not, so
    // the input tests run here as well.
    if violations.is_empty() {
        match inject_input(&socket, &mut session) {
            Ok(()) => violations.extend(input_lines(&session.output())),
            Err(error) => violations.push(format!("nothing could be injected: {error}")),
        }
    }
    if violations.is_empty() {
        match session.wait_for_end(E2E_TIMEOUT) {
            None => {
                violations.push("the machine without a screen did not end by itself".to_owned());
            }
            status => {
                let outcome = qemu::outcome_of(status, false);
                if outcome != qemu::Outcome::Success {
                    violations.push(format!("the machine reported a {}", outcome.name()));
                }
            }
        }
    }
    let output = session.finish();
    let _ = std::fs::remove_file(&socket);
    note!(
        "qemu without a graphics adapter: {} line(s)",
        output.lines().count()
    );
    report("no framebuffer", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the run without a graphics adapter ---");
        eprintln!("{output}");
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)
}

/// What a machine without a graphics adapter has to say.
const NO_VGA_LINES: [(&str, &str); 4] = [
    (
        "[info] framebuffer=absent",
        "the kernel did not report that the machine has no framebuffer",
    ),
    (
        "[display] no framebuffer",
        "the display server did not report that there is no screen",
    ),
    (
        "[paint] nothing drawn: ",
        "the program that draws did not say that it drew nothing",
    ),
    (
        "[canvas] no screen: ",
        "the canvas did not say that it has no screen",
    ),
];

/// The lines the run without the two network lines has to carry: the bus is
/// walked, the device is not there, and the machine ends by itself.
const NO_NETWORK_LINES: [(&str, &str); 7] = [
    // The bus walk is read off the volume like every program outside the
    // boot set, so the run reaches it one program at a time rather than
    // waiting once for a line minutes away.
    (
        "[files] boot volume: clusters=",
        "the run without a network mounted no boot volume",
    ),
    (
        "[init] started app-lspci",
        "the run without a network read no bus walk off the volume",
    ),
    (
        "[lspci] window segment=",
        "the program that walks the bus was given no window",
    ),
    (
        "[lspci] 00:00.0 8086:29c0",
        "the bus walk did not find the host bridge of the machine",
    ),
    (
        "[lspci] no virtio device",
        "the bus walk did not report that there is no virtio device",
    ),
    (
        "[net] no interface",
        "the network server did not report that there is no interface",
    ),
    (
        "[net-app] no interface",
        "the program that uses a socket was not told there is none",
    ),
];

/// The same system on a machine without the two network lines: the bus walk
/// reports the rest of the bus, finds no virtio device, and the run ends by
/// itself (13.12).
fn test_without_a_network(machine: &Machine, path: &Path) -> Result<(), Error> {
    let socket = qemu::socket_path("audhsos-qmp-nonet")?;
    let _ = std::fs::remove_file(&socket);
    let mut session = Session::start(
        machine,
        path,
        &qemu::Options {
            qmp: Some(socket.clone()),
            ..qemu::Options::without_network()
        },
    )?;
    let mut violations = Vec::new();
    for (needle, complaint) in NO_NETWORK_LINES {
        if !session.wait_for(needle, E2E_TIMEOUT) {
            violations.push((*complaint).to_owned());
            break;
        }
    }
    if violations.is_empty() && session.wait_for("[hello] ready", E2E_TIMEOUT) {
        session.send(b"typed\n")?;
    }
    // The programs that listen end when they have seen what they wait for,
    // and nothing but the runner sends it: a run that injects nothing waits
    // for them until the time limit. This machine has a screen, so the
    // canvas and the desktop wait for the keys that end them as well.
    if violations.is_empty() {
        match inject_input(&socket, &mut session) {
            Ok(()) => violations.extend(input_lines(&session.output())),
            Err(error) => violations.push(format!("nothing could be injected: {error}")),
        }
    }
    if violations.is_empty() {
        match end_the_canvas(&socket, &mut session) {
            Ok(()) => {}
            Err(error) => violations.push(format!("the canvas did not end: {error}")),
        }
    }
    if violations.is_empty() {
        match end_the_desk(&socket, &mut session) {
            Ok(()) => {}
            Err(error) => violations.push(format!("the desktop did not end: {error}")),
        }
    }
    if violations.is_empty() {
        match session.wait_for_end(E2E_TIMEOUT) {
            None => violations.push("the machine did not end by itself".to_owned()),
            status => {
                let outcome = qemu::outcome_of(status, false);
                if outcome != qemu::Outcome::Success {
                    violations.push(format!("the machine reported a {}", outcome.name()));
                }
            }
        }
    }
    let output = session.finish();
    let _ = std::fs::remove_file(&socket);
    note!("qemu without a network: {} line(s)", output.lines().count());
    report("without a network", &violations);
    if !violations.is_empty() {
        eprintln!("--- serial output of the run without a network ---");
        eprintln!("{output}");
        eprintln!("--- end ---");
    }
    Error::from_violations(violations)
}

/// Every test kernel in QEMU, then the three images that must make the
/// loader report a failure.
fn test_qemu(root: &Path) -> Result<(), Error> {
    let machine = Machine::locate()?;
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
        .env(qemu::ACCELERATOR_VARIABLE, machine.accelerator())
        .env(
            USER_TESTS_VARIABLE,
            root.join(USER_TESTS_DIR).display().to_string(),
        )
        .run()?;
    loader_images(root, &machine)
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
    let run = machine.run_captured(&path, &qemu::Options::plain())?;
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
fn loader_images(root: &Path, machine: &Machine) -> Result<(), Error> {
    let profile = "debug";
    let loader = loader_bytes(root, profile)?;
    let kernel = fs::read_bytes(&kernel_binary(root, profile))?;
    let boot = boot_image::build(&boot_image::placeholder_root_task(), &[], 0)?;
    let cases: [LoaderCase; 3] = [
        ("bad-kernel", Some(corrupt(&kernel)), Some(boot.clone())),
        ("missing-kernel", None, Some(boot)),
        ("missing-boot-image", Some(kernel), None),
    ];
    let mut violations = Vec::new();
    for (name, kernel, boot) in cases {
        let image = disk_image(loader.clone(), kernel, boot)?;
        let path = write_run_image(root, name, &image)?;
        let run = machine.run_captured(&path, &qemu::Options::plain())?;
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

/// The programs that lie on the volume, each under the path
/// `AUDHSOS/BIN/` gives it.
///
/// # Errors
///
/// The errors of reading what the build wrote, and [`Error::Usage`] for a
/// program whose name is no 8.3 one.
fn volume_programs(
    root: &Path,
    profile: &str,
    desktop: bool,
) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let built = root.join("target/x86_64-unknown-none").join(profile);
    let carried: &[&str] = if desktop {
        &archive::DESKTOP
    } else {
        &archive::ON_THE_VOLUME
    };
    let mut files = Vec::new();
    let mut whole = 0usize;
    let mut kept = 0usize;
    for name in carried.iter().copied() {
        let (spelled, len) = user_loader::volume::file_name(name.as_bytes())
            .ok_or_else(|| Error::Usage(format!("`{name}` is no name a volume holds")))?;
        let spelled = std::str::from_utf8(spelled.get(..len).unwrap_or(&[]))
            .map_err(|_| Error::Usage(format!("`{name}` spells no file name")))?;
        let path = format!(
            "{}/{}/{spelled}",
            user_loader::volume::DIRECTORY[0],
            user_loader::volume::DIRECTORY[1]
        );
        // Only the part a loader reads goes onto the volume: the tail of
        // an unoptimized build is debug information, and the root task
        // reads every byte of a program off the disk two kibibytes per
        // message.
        let bytes = fs::read_bytes(&built.join(name))?;
        let read = loadable::trim(name, &bytes)?;
        whole = whole.saturating_add(bytes.len());
        kept = kept.saturating_add(read.len());
        files.push((path, read));
    }
    note!(
        "volume: {} of {} KiB is what the loader reads",
        kept.wrapping_div(1024),
        whole.wrapping_div(1024)
    );
    Ok(files)
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

/// The size of a scratch disk. FAT32 wants 65525 clusters, which at one
/// sector each is thirty-three mebibytes before the two tables above them;
/// this is what the boot volume has by default, and the file is sparse, so
/// a disk nothing wrote costs a directory entry.
const SCRATCH_SIZE: u64 = 64 * 1024 * 1024;

/// Where the scratch disk of the run `name` lies.
///
/// One disk per run name, because two machines writing one file would tear
/// it and QEMU refuses the second one anyway; and the same path across
/// runs of that name, because a test that boots twice to see what survived
/// needs the bytes the first boot wrote (D-136).
pub(crate) fn scratch_path(root: &Path, name: &str) -> PathBuf {
    root.join("target")
        .join("qemu")
        .join(format!("{name}.scratch.img"))
}

/// The scratch disk of the run `name`, blank whatever was on it.
///
/// The end-to-end run boots twice and the second boot reads what the
/// first wrote, so the pair says nothing unless it starts from a disk
/// nothing wrote. A run a person starts keeps its disk instead
/// ([`scratch_image`]).
fn blank_scratch_image(root: &Path, name: &str) -> Result<PathBuf, Error> {
    let path = scratch_path(root, name);
    let _ = std::fs::remove_file(&path);
    scratch_image(root, name)
}

/// The scratch disk of the run `name`, carrying `files` from the first
/// boot on.
///
/// A disk nothing wrote is formatted by `server-fs` on its first boot,
/// which is what [`blank_scratch_image`] leaves it. This one carries a
/// volume the host wrote, so the server mounts it and finds the files
/// there — which is how the key material of the interop run reaches the
/// guest (D-146). There is no partition table on it: the scratch disk is
/// the system's own and a table would make it a disk somebody else
/// partitioned (document 15, 15.10).
fn written_scratch_image(
    root: &Path,
    name: &str,
    files: &[(String, Vec<u8>)],
) -> Result<PathBuf, Error> {
    let path = scratch_path(root, name);
    let _ = std::fs::remove_file(&path);
    let borrowed: Vec<(&str, Vec<u8>)> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    let size = usize::try_from(SCRATCH_SIZE).unwrap_or(0);
    let mut image = vec![0u8; size];
    let geometry = fat32::write(&mut image, &borrowed)?;
    note!(
        "scratch disk {}: {} files, {} clusters",
        path.display(),
        files.len(),
        geometry.clusters
    );
    fs::write_bytes(&path, &image)?;
    Ok(path)
}

/// The scratch disk of the run `name`, blank when it was not there and as
/// it stands when it was.
fn scratch_image(root: &Path, name: &str) -> Result<PathBuf, Error> {
    let path = scratch_path(root, name);
    if fs::create_sparse(&path, SCRATCH_SIZE)? {
        note!(
            "scratch disk {}: blank, {} MiB",
            path.display(),
            SCRATCH_SIZE >> 20
        );
    }
    Ok(path)
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
    // A measurement is written by an image that passed, so it is printed
    // here and not only in the output of a failure. The numbers of
    // 08-roadmap.md 8.10 are read off these lines.
    for measurement in &report.measurements {
        note!(
            "bench {}: {} ticks, the median of {}",
            measurement.name,
            measurement.ticks,
            measurement.samples
        );
    }
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
    match qemu::check(&report, run).and_then(|()| Error::from_violations(missing)) {
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
    let mut scratch = false;
    let mut keys = false;
    let mut build_options = Vec::new();
    let mut image_options = Vec::new();
    for option in options {
        match option.as_str() {
            "--display" => display = true,
            "--scratch" => scratch = true,
            "--ssh" => keys = true,
            "--desktop" => image_options.push("--desktop".to_owned()),
            "--release" => {
                build_options.push("--release".to_owned());
                image_options.push("--release".to_owned());
            }
            other => return Err(Error::Usage(format!("unknown option `{other}` for run"))),
        }
    }
    build(root, &build_options)?;
    image(root, &image_options)?;
    let machine = Machine::locate()?;
    let path = root.join("target").join("audhsos.img");
    // `--ssh` carries the two files the shell of the desktop reads before
    // it opens a connection (D-146): the fingerprints of every public key
    // of `keys/ssh/` and the client's secret. It writes the disk rather
    // than keeping it, because a fingerprint added to that directory has
    // to reach the next run.
    let scratch = if keys {
        Some(written_scratch_image(
            root,
            "audhsos",
            &ssh::trust_files(root)?,
        )?)
    } else {
        scratch
            .then(|| scratch_image(root, "audhsos"))
            .transpose()?
    };
    let machine_options = qemu::Options {
        scratch,
        ..qemu::Options::windowed(display)
    };
    let status = machine.run_attached(&path, &machine_options)?;
    match qemu::outcome_of(status, false) {
        qemu::Outcome::Success => Ok(()),
        outcome => Err(Error::Usage(format!(
            "the machine reported a {}",
            outcome.name()
        ))),
    }
}

/// The documentation as PDF, written under `target/pdf/`.
///
/// The xtask only starts the tool; the tool itself decides what to
/// convert and where it goes, and every option after `pdf` is passed
/// through to it.
pub(crate) fn pdf(root: &Path, options: &[String]) -> Result<(), Error> {
    let mut cmd = Cmd::cargo()
        .cwd(root)
        .args(["run", "--release", "-p", "docpdf", "--"]);
    // `xtask pdf -- --list` and `xtask pdf --list` mean the same thing:
    // the separator belongs to Cargo, and the xtask has already passed it.
    for option in options.iter().skip_while(|option| *option == "--") {
        cmd = cmd.arg(option);
    }
    cmd.run()
}

/// Runs the release-mode JavaScript host with the caller's options.
pub(crate) fn jrs(root: &Path, options: &[String]) -> Result<(), Error> {
    Cmd::cargo()
        .cwd(root)
        .args(["run", "--release", "-p", "jrs-cli", "--"])
        .args(options.iter().map(String::as_str))
        .run()
}

/// The memory benchmark of the development machine.
///
/// It is built with optimizations and never from the `dev` profile: a
/// benchmark of unoptimized code measures the bounds checks, not the
/// machine.
///
/// # Errors
///
/// The errors of the build and of the run.
pub(crate) fn membench(root: &Path, options: &[String]) -> Result<(), Error> {
    Cmd::cargo()
        .cwd(root)
        .args(["run", "--release", "-p", "membench", "--"])
        .args(options.iter().map(String::as_str))
        .run()
}

/// Runs the fuzzer of `norec` with the caller's options.
///
/// The engine it drives is not built here: `sh tools/sqlite.sh` clones and
/// builds it, and the fuzzer refuses the run when it is missing.
///
/// # Errors
///
/// The errors of the build and of the run.
pub(crate) fn norec(root: &Path, options: &[String]) -> Result<(), Error> {
    Cmd::cargo()
        .cwd(root)
        .args(["run", "--release", "-p", "norec", "--"])
        .args(options.iter().map(String::as_str))
        .run()
}

/// Focused checks for the JavaScript runtime, still using the pinned tools.
pub(crate) fn jrs_check(root: &Path, options: &[String]) -> Result<(), Error> {
    let fix_format = match options {
        [] => false,
        [option] if option == "--fix-format" => true,
        _ => {
            return Err(Error::Usage(
                "jrs-check accepts only --fix-format".to_owned(),
            ));
        }
    };
    let mut fmt = Cmd::cargo().cwd(root).args([
        "fmt",
        "-p",
        "jrs",
        "-p",
        "jrs-cli",
        "-p",
        "audhsos-event-target",
        "-p",
        "audhsos-timer-queue",
        "-p",
        "audhsos-json",
        "-p",
        "audhsos-math",
        "-p",
        "audhsos-utf16",
        "-p",
        "xtask",
    ]);
    if !fix_format {
        fmt = fmt.args(["--", "--check"]);
    }
    fmt.run()?;
    Cmd::cargo()
        .cwd(root)
        .args([
            "test",
            "-p",
            "jrs",
            "-p",
            "jrs-cli",
            "-p",
            "audhsos-event-target",
            "-p",
            "audhsos-timer-queue",
            "-p",
            "audhsos-json",
            "-p",
            "audhsos-math",
            "-p",
            "audhsos-utf16",
        ])
        .run()?;
    Cmd::cargo()
        .cwd(root)
        .args([
            "clippy",
            "-p",
            "jrs",
            "-p",
            "jrs-cli",
            "-p",
            "audhsos-event-target",
            "-p",
            "audhsos-timer-queue",
            "-p",
            "audhsos-json",
            "-p",
            "audhsos-math",
            "-p",
            "audhsos-utf16",
            "-p",
            "xtask",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .run()?;
    Cmd::cargo()
        .cwd(root)
        .args(["check", "-p", "jrs", "--target", "x86_64-unknown-none"])
        .run()
}

/// Focused checks for the isolated Thompson automaton.
pub(crate) fn regex_check(root: &Path, options: &[String]) -> Result<(), Error> {
    let fix = match options {
        [] => false,
        [option] if option == "--fix-format" => true,
        _ => {
            return Err(Error::Usage(
                "regex-check accepts only --fix-format".to_owned(),
            ));
        }
    };
    let mut fmt = Cmd::cargo().cwd(root).args([
        "fmt",
        "-p",
        "audhsos-regex",
        "-p",
        "audhsos-regex-bt",
        "-p",
        "xtask",
    ]);
    if !fix {
        fmt = fmt.args(["--", "--check"]);
    }
    fmt.run()?;
    Cmd::cargo()
        .cwd(root)
        .args(["test", "-p", "audhsos-regex", "-p", "audhsos-regex-bt"])
        .run()?;
    Cmd::cargo()
        .cwd(root)
        .args([
            "clippy",
            "-p",
            "audhsos-regex",
            "-p",
            "audhsos-regex-bt",
            "-p",
            "xtask",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .run()?;
    Cmd::cargo()
        .cwd(root)
        .args([
            "check",
            "-p",
            "audhsos-regex",
            "-p",
            "audhsos-regex-bt",
            "--target",
            "x86_64-unknown-none",
        ])
        .run()
}

/// Host coverage against the thresholds.
pub(crate) fn coverage(root: &Path) -> Result<(), Error> {
    let totals = coverage::measure(root)?;
    let (table, violations) = coverage::evaluate(&totals);
    note_raw!("{table}");
    report("coverage", &violations);
    Error::from_violations(violations)
}

/// Miri over the host-executable `unsafe` of the adapter crates and the
/// tests that reach it, which is what `MIRI_TARGETS` names.
///
/// One run per crate rather than one run over all of them, because each
/// carries its own test-name filters. A crate whose filters are empty runs
/// whole.
///
/// # Errors
///
/// The errors of the runs; the violations of
/// [`unsafe_budget::miri_gaps`], which are checked first, because a run
/// that skipped an `unsafe` site would pass and mean nothing.
pub(crate) fn miri(root: &Path) -> Result<(), Error> {
    let gaps = unsafe_budget::miri_gaps(root)?;
    report("miri coverage", &gaps);
    Error::from_violations(gaps)?;
    for target in MIRI_TARGETS {
        let mut cmd = Cmd::cargo_plain()
            .cwd(root)
            .args(["miri", "test", "-p", target.name]);
        if !target.filters.is_empty() {
            cmd = cmd.arg("--");
            for filter in target.filters {
                cmd = cmd.arg(*filter);
            }
        }
        cmd.run()?;
    }
    Ok(())
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
            "--compact" => mode = Job::Compact,
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
    if matches!(mode, Job::Regression) {
        return replay_corpora(root, &targets);
    }
    for target in targets {
        match &mode {
            Job::Fuzz => run_fuzzer(root, target.name, seconds)?,
            Job::Regression => {}
            Job::Merge(from) => merge_corpus(root, target.name, from)?,
            Job::Compact => compact_corpus(root, target.name)?,
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
    /// Fold the stored corpus back to a minimal cover of what it reaches.
    Compact,
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

/// Folds the corpus of `name` back to a minimal cover of what it reaches.
///
/// Fuzzing writes a file for every input that reaches something the run had
/// not reached as cheaply, and a run that reads a part of the corpus calls
/// some of them new that another part already covers. A merge into an empty
/// directory answers which files a minimal cover needs; the rest are deleted,
/// except the ones a person named, which are the regression inputs this
/// project tracks and never the hexadecimal names a find gets.
fn compact_corpus(root: &Path, name: &str) -> Result<(), Error> {
    let corpus = corpus_of(root, name);
    let cover = corpus.with_extension("cover");
    let _ = std::fs::remove_dir_all(&cover);
    std::fs::create_dir_all(&cover)
        .map_err(|source| Error::io(format!("creating {}", cover.display()), source))?;
    note!("compacting the corpus of `{name}`");
    Cmd::cargo()
        .cwd(&root.join("fuzz"))
        .args([
            "run",
            "--release",
            "--bin",
            name,
            "--",
            "-merge=1",
            &cover.display().to_string(),
            &corpus.display().to_string(),
        ])
        .env("RUSTFLAGS", FUZZING_FLAGS)
        .run()?;
    let kept: BTreeSet<String> = fs::walk_files(&cover)?
        .iter()
        .map(|file| fs::file_name(file).to_owned())
        .collect();
    let mut removed = 0usize;
    for file in fs::walk_files(&corpus)? {
        let found = fs::file_name(&file);
        if is_found_name(found) && !kept.contains(found) {
            std::fs::remove_file(&file)
                .map_err(|source| Error::io(format!("deleting {}", file.display()), source))?;
            removed = removed.saturating_add(1);
        }
    }
    let _ = std::fs::remove_dir_all(&cover);
    note!(
        "`{name}`: {removed} redundant inputs removed, {} kept",
        kept.len()
    );
    Ok(())
}

/// Whether `name` is the hexadecimal name a find gets rather than the name a
/// person gave a regression input.
fn is_found_name(name: &str) -> bool {
    matches!(name.len(), 32 | 40) && name.bytes().all(|byte| byte.is_ascii_hexdigit())
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

/// Runs the fuzzer of one target for `seconds` seconds, on every core.
///
/// One process is started, and it starts one worker per core itself. That
/// process is the orchestrator: it reads the corpus once, keeps the pool,
/// and hands its workers seeds to change. Every worker reading the whole
/// corpus was what this used to do, and on a corpus of some thousand inputs
/// of a slow target it spent the greater part of the run computing the same
/// answer on every core.
fn run_fuzzer(root: &Path, name: &str, seconds: u64) -> Result<(), Error> {
    let jobs = test_jobs()?;
    note!("fuzzing `{name}` for {seconds} seconds on {jobs} workers");
    let fuzz = root.join("fuzz");
    let executables = artifacts::build(
        Cmd::cargo()
            .cwd(&fuzz)
            .args(["build", "--release", "--bin", name])
            .env("RUSTFLAGS", FUZZING_FLAGS),
    )?;
    let executable = executables
        .iter()
        .find(|exe| exe.name == name && !exe.test)
        .ok_or_else(|| Error::Parse(format!("Cargo reported no executable for `{name}`")))?;
    let corpus = corpus_of(root, name).display().to_string();
    // The seed must differ between two runs of the whole command, and it
    // must not be zero, which the engine reads as none.
    let seed = u64::from(std::process::id()).wrapping_mul(0x9E37_79B9) | 1;
    executable
        .command()
        .cwd(&fuzz)
        .arg(&corpus)
        .arg(format!("-max_total_time={seconds}"))
        .arg(format!("-workers={jobs}"))
        .arg(format!("-seed={seed}"))
        .run()
}

/// Builds selected regression binaries once, then replays their corpora
/// concurrently. Targets without a corpus directory are reported and skipped.
///
/// The replay is the longest step of `check`, and one corpus is far larger
/// than the rest: it is split over as many shards as there are jobs, so the
/// machine replays it instead of one core of it. The binaries are built in
/// release, because a replay has to reach the panics of a target, not the
/// speed of a debug build.
fn replay_corpora(root: &Path, targets: &[&FuzzTarget]) -> Result<(), Error> {
    let jobs = test_jobs()?;
    let fuzz = root.join("fuzz");
    let mut selected = Vec::new();
    let mut build = Cmd::cargo().cwd(&fuzz).args(["build", "--release"]);
    for target in targets {
        let corpus = corpus_of(root, target.name);
        if corpus.is_dir() {
            selected.push((target.name, corpus));
            build = build.args(["--bin", target.name]);
        } else {
            note!("`{}`: no corpus at {}", target.name, corpus.display());
        }
    }
    if selected.is_empty() {
        return Ok(());
    }
    let executables = artifacts::build(build)?;
    let mut commands = Vec::new();
    for (name, corpus) in selected {
        let executable = executables
            .iter()
            .find(|exe| exe.name == name && !exe.test)
            .ok_or_else(|| Error::Parse(format!("Cargo reported no executable for `{name}`")))?;
        for shard in corpus_shards(&fs::walk_files(&corpus)?, jobs) {
            let paths = shard.iter().map(|path| {
                path.strip_prefix(&fuzz)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            });
            commands.push(executable.command().cwd(&fuzz).args(paths));
        }
    }
    run_parallel(&commands, jobs)
}

/// Deals `files` round robin into at most `jobs` shards.
///
/// Round robin rather than in blocks, because the corpus is named by content
/// hash and a slow neighbourhood of it is no more spread out than any other.
/// A corpus with fewer files than jobs gets one shard per file, and an empty
/// one gets no shard at all. The paths stay relative to the directory the
/// replay runs in, which keeps a shard of some thousand of them far inside
/// the argument limit of one command.
fn corpus_shards(files: &[PathBuf], jobs: usize) -> Vec<Vec<PathBuf>> {
    let count = jobs.max(1).min(files.len());
    (0..count)
        .map(|shard| files.iter().skip(shard).step_by(count).cloned().collect())
        .collect()
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
    let steps: [(&str, Step); 11] = [
        ("lint", lint),
        ("check-layering", check_layering),
        ("check-deps", check_deps),
        ("unsafe-budget", unsafe_budget),
        ("test --host", |root| test(root, &["--host".to_owned()])),
        ("coverage", coverage),
        ("miri", miri),
        ("doc", doc),
        ("test --qemu", |root| test(root, &["--qemu".to_owned()])),
        ("test --e2e", |root| test(root, &["--e2e".to_owned()])),
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

/// The address the root task is linked at, which its linker script repeats
/// and this checks against the layout constant of the interface.
const ROOT_TASK_BASE: u64 = audhsos_abi::layout::ROOT_TASK_BASE;

/// The linker script of the root task.
const ROOT_SCRIPT: &str = "crates/user/programs/root.ld";

/// The address the programs of the archive are linked at, which their
/// linker script repeats and this checks.
const PROGRAM_BASE: u64 = 0x0100_0000;

/// The linker script of those programs.
const PROGRAM_SCRIPT: &str = "crates/user/programs/program.ld";

/// Checks the two linker scripts of the userland against the addresses the
/// code says they hold.
///
/// # Errors
///
/// [`Error::Violations`] when a script and this disagree.
pub(crate) fn check_userland(root: &Path) -> Result<(), Error> {
    check_base(root, ROOT_SCRIPT, "ROOT_TASK_BASE", ROOT_TASK_BASE)?;
    check_base(root, PROGRAM_SCRIPT, "PROGRAM_BASE", PROGRAM_BASE)
}

/// Refuses a linker script whose base address is not the one the code says.
fn check_base(root: &Path, path: &str, name: &str, expected: u64) -> Result<(), Error> {
    let script = fs::read(&root.join(path))?;
    let violations = match linker::constant(&script, name) {
        Some(base) if base == expected => Vec::new(),
        Some(base) => vec![format!(
            "{path}: {name} is {base:#x}, the xtask says {expected:#x}"
        )],
        None => vec![format!("{path}: {name} is missing")],
    };
    report("program base", &violations);
    Error::from_violations(violations)
}

/// The boot image of the real system: the header, the root task as an ELF,
/// and the archive of the eight programs.
///
/// # Errors
///
/// The errors of reading what the build wrote and of writing the archive.
fn boot_image_of(root: &Path, profile: &str) -> Result<Vec<u8>, Error> {
    let built = root.join("target/x86_64-unknown-none").join(profile);
    let task = fs::read_bytes(&built.join("server-init"))?;
    let mut files = Vec::new();
    for name in archive::PROGRAMS {
        files.push((name, fs::read_bytes(&built.join(name))?));
    }
    let archive = archive::build(&files)?;
    note!("archive: {} programs, {} bytes", files.len(), archive.len());
    boot_image::build(&task, &archive, 0)
}

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
    let mut desktop = false;
    for option in options {
        match option.as_str() {
            "--release" => profile = "release",
            "--desktop" => desktop = true,
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
    check_userland(root)?;
    let boot = boot_image_of(root, profile)?;
    let mut files = vec![
        (disk::LOADER_PATH, fs::read_bytes(&loader)?),
        (disk::KERNEL_PATH, fs::read_bytes(&kernel)?),
        (disk::BOOT_IMAGE_PATH, boot.clone()),
    ];
    let programs = volume_programs(root, profile, desktop)?;
    for (path, bytes) in &programs {
        files.push((path.as_str(), bytes.clone()));
    }
    note!(
        "volume: {} programs under AUDHSOS/BIN/{}",
        programs.len(),
        if desktop { ", the desktop alone" } else { "" }
    );
    let list = archive::start_list(&archive::DESKTOP_START);
    if desktop {
        note!(
            "volume: the boot starts {}",
            archive::DESKTOP_START.join(", ")
        );
        files.push((archive::START_LIST_PATH, list));
    }
    let anchors = anchors::of(root)?;
    let table = anchors::table(&anchors)?;
    let named: Vec<&str> = anchors.iter().map(|anchor| anchor.name.as_str()).collect();
    note!(
        "volume: {} trust anchor(s) in {}, {} bytes: {}",
        anchors.len(),
        anchors::VOLUME_PATH,
        table.len(),
        named.join(", ")
    );
    files.push((anchors::VOLUME_PATH, table));
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
