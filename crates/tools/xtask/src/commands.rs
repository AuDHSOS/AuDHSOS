// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The subcommands.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;
use crate::image::{archive, boot_image, disk};
use crate::out::{self, note, note_raw};
use crate::policy::{FUZZ_TARGETS, FuzzTarget, MIRI_TARGETS, Target, crates_for};
use crate::ppm;
use crate::process::{Cmd, run_parallel, test_jobs};
use crate::qemu::{self, Machine, Run};
use crate::qmp::{Button, Qmp};
use crate::session::Session;
use crate::symbolize;
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
    let mut profile = Vec::new();
    for option in options {
        match option.as_str() {
            "--host" => host = true,
            "--qemu" => qemu = true,
            "--e2e" => e2e = true,
            "--release" => profile.push("--release".to_owned()),
            other => return Err(Error::Usage(format!("unknown option `{other}` for test"))),
        }
    }
    if !(host || qemu || e2e) {
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
    Ok(())
}

/// How long an end-to-end run waits for a line before it gives up.
const E2E_TIMEOUT: Duration = Duration::from_secs(60);

/// What the run has to see, in this order, for the system to have worked.
///
/// Each line is written by a different part of it, so the first one that
/// does not come says where the boot stopped: the root task read the
/// archive, the memory server answered, the name server answered, the
/// console driver took the port, and the application found it and said
/// something through it.
const E2E_LINES: [(&str, &str); 16] = [
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
        "[faulter] about to write to nowhere",
        "the program that faults on purpose never ran",
    ),
    (
        "faulted: PageFault",
        "the root task did not report the fault of its child",
    ),
];

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
    let mut session = Session::start(
        &machine,
        &path,
        &qemu::Options {
            qmp: Some(socket.clone()),
            ..qemu::Options::plain()
        },
    )?;

    let mut violations = Vec::new();
    for (needle, complaint) in E2E_LINES {
        if !session.wait_for(needle, E2E_TIMEOUT) {
            violations.push((*complaint).to_owned());
            break;
        }
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
    // Two clients wrote at once and no line of either may be torn: every
    // line the second one wrote has to stand whole and once, which the
    // last one arriving does not by itself say.
    if violations.is_empty() {
        violations.extend(torn_lines(&session.output()));
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
    test_without_a_framebuffer(&machine, &path)
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
const NO_VGA_LINES: [(&str, &str); 3] = [
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
];

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
    let status = machine.run_attached(&path, &qemu::Options::windowed(display))?;
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

/// Builds selected regression binaries once, then replays their corpora
/// concurrently. Targets without a corpus directory are reported and skipped.
fn replay_corpora(root: &Path, targets: &[&FuzzTarget]) -> Result<(), Error> {
    let jobs = test_jobs()?;
    let mut selected = Vec::new();
    let mut build = Cmd::cargo().cwd(&root.join("fuzz")).arg("build");
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
        commands.push(
            executable
                .command()
                .cwd(&root.join("fuzz"))
                .arg(corpus.display().to_string()),
        );
    }
    run_parallel(&commands, jobs)
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
    check_userland(root)?;
    let boot = boot_image_of(root, profile)?;
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
