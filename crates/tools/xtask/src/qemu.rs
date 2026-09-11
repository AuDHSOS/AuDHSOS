// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Finding QEMU and its firmware, running one disk image, and reading the
//! serial protocol the machine writes.
//!
//! Invariants: the command line is the one
//! [03-target-platform.md 3.1.1](../../../docs/03-target-platform.md)
//! prescribes and nobody types it by hand; a run that does not end by
//! itself is killed and reported as a crash; the protocol grammar is the
//! one `kernel-test-harness` writes, so that writer and reader cannot
//! drift apart.

use std::io::Read;
#[cfg(target_os = "linux")]
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use kernel_test_harness::protocol::{
    BENCH_PREFIX, FAILED, OK, SEPARATOR, SUMMARY_PREFIX, TEST_PREFIX, TICKS,
};

use crate::error::Error;
#[cfg(target_os = "linux")]
use crate::out::note;

/// The QEMU binary the reference machine runs on.
const QEMU_BINARY: &str = "qemu-system-x86_64";

/// The CPU model of the reference machine. `qemu64` carries neither
/// `rdrand` nor `rdseed`, which was checked against QEMU through
/// `query-cpu-model-expansion` and not assumed; TCG provides both once
/// they are asked for, and `random_bytes` needs `rdseed` (D-110).
const CPU_MODEL: &str = "qemu64,+rdrand,+rdseed";

/// Names the QEMU binary, instead of searching the `PATH`.
const QEMU_VARIABLE: &str = "AUDHSOS_QEMU";

/// Names the firmware image, instead of looking next to QEMU.
const FIRMWARE_VARIABLE: &str = "AUDHSOS_OVMF";

/// The accelerator selected by the parent xtask for every QEMU runner.
pub(crate) const ACCELERATOR_VARIABLE: &str = "AUDHSOS_QEMU_ACCELERATOR";

/// Names the time limit of one run in seconds.
const TIMEOUT_VARIABLE: &str = "AUDHSOS_QEMU_TIMEOUT";

/// Places distributions install the firmware, relative to the directory
/// holding QEMU. `MacPorts` uses the first one; Debian uses the second one.
const FIRMWARE_RELATIVES: [&str; 4] = [
    "../share/qemu/edk2-x86_64-code.fd",
    "../share/OVMF/OVMF_CODE_4M.fd",
    "../share/OVMF/OVMF_CODE.fd",
    "../share/qemu/OVMF.fd",
];

/// The port inside the guest the forwarded host port reaches, which is the
/// echo port of a listener Phase 14 brings up.
const GUEST_PORT: u16 = 7;

/// Time limit of one run in seconds.
const DEFAULT_TIMEOUT: u64 = 60;

/// The width of the mode the firmware is to set.
const SCREEN_WIDTH: u32 = 1920;

/// The height of the mode the firmware is to set.
///
/// The firmware draws its own text console into the mode it sets, and
/// `GraphicsConsoleDxe` needs the twenty-five rows of nineteen pixels that
/// a UEFI console has: below four hundred seventy-five pixels the firmware
/// never reaches its boot manager, and nothing is loaded at all. This is
/// far above that.
const SCREEN_HEIGHT: u32 = 1200;

/// How long the runner waits between two checks on the machine.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The Linux accelerator probe is shared by every QEMU use of one xtask.
#[cfg(target_os = "linux")]
static LINUX_ACCELERATOR: OnceLock<Result<String, String>> = OnceLock::new();

/// The exit status of a machine that reported success.
pub(crate) const EXIT_SUCCESS: i32 = 33;

/// The exit status of a machine that reported a test failure.
pub(crate) const EXIT_TEST_FAILURE: i32 = 35;

/// The exit status of a machine whose loader reported a failure.
pub(crate) const EXIT_LOADER_FAILURE: i32 = 37;

/// What one run of the machine amounted to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The machine reported success.
    Success,
    /// The machine reported a failed test.
    TestFailure,
    /// The loader reported a failure.
    LoaderFailure,
    /// The machine did not report anything: a triple fault, a hang the
    /// time limit ended, or QEMU itself failing.
    Crash,
}

impl Outcome {
    /// The name the report uses.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::TestFailure => "test failure",
            Outcome::LoaderFailure => "loader failure",
            Outcome::Crash => "crash",
        }
    }
}

/// The signal that ended a process, where the platform reports one.
#[cfg(unix)]
fn signal_of(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

/// Platforms that do not report signals say nothing.
#[cfg(not(unix))]
const fn signal_of(_status: ExitStatus) -> Option<i32> {
    None
}

/// The outcome an exit status means. A run the time limit ended is a
/// crash whatever the status says.
pub(crate) const fn outcome_of(status: Option<i32>, timed_out: bool) -> Outcome {
    if timed_out {
        return Outcome::Crash;
    }
    match status {
        Some(EXIT_SUCCESS) => Outcome::Success,
        Some(EXIT_TEST_FAILURE) => Outcome::TestFailure,
        Some(EXIT_LOADER_FAILURE) => Outcome::LoaderFailure,
        _ => Outcome::Crash,
    }
}

/// One run of the machine.
#[derive(Clone, Debug)]
pub(crate) struct Run {
    /// The exit status, if QEMU exited by itself.
    pub(crate) status: Option<i32>,
    /// The signal that ended QEMU, if one did.
    pub(crate) signal: Option<i32>,
    /// Everything the serial port carried.
    pub(crate) output: String,
    /// Whether the time limit ended the run.
    pub(crate) timed_out: bool,
}

impl Run {
    /// What the run amounted to.
    pub(crate) const fn outcome(&self) -> Outcome {
        outcome_of(self.status, self.timed_out)
    }

    /// How the run ended, in words.
    ///
    /// A crash is three different things — the time limit ended a machine
    /// that was still going, something killed QEMU, or the machine exited
    /// with a code that is neither of the two the exit device writes — and
    /// they are diagnosed differently. A report that calls all three a
    /// crash sends a reader looking in the wrong place, which is what
    /// happened to the intermittent failure of the `ipc` image.
    pub(crate) fn how(&self) -> String {
        if self.timed_out {
            return "the time limit ended it while it was still running".to_owned();
        }
        match (self.status, self.signal) {
            (Some(code), _) => format!("it exited with code {code}"),
            (None, Some(signal)) => format!("something killed QEMU with signal {signal}"),
            (None, None) => "QEMU ended without a code and without a signal".to_owned(),
        }
    }
}

/// What a run of the machine is to have besides the reference command line.
///
/// The reference machine of
/// [03-target-platform.md 3.1.1](../../../docs/03-target-platform.md) is
/// what a run has when nothing is asked for; each of these adds one thing
/// to it, and each is asked for by exactly one caller.
#[derive(Clone, Debug, Default)]
pub(crate) struct Options {
    /// Open QEMU's own window instead of running headless.
    pub(crate) display: bool,
    /// Where the machine protocol listens, for a run the runner talks to.
    pub(crate) qmp: Option<PathBuf>,
    /// Run without a graphics adapter, which leaves the firmware without a
    /// Graphics Output Protocol and the kernel without a framebuffer.
    pub(crate) no_vga: bool,
    /// The host port QEMU's user-mode network forwards into the guest, and
    /// with it the network device itself. `None` is a machine without the
    /// two network lines, which is the second run Phase 13 and Phase 14 are
    /// accepted on (D-118).
    pub(crate) network: Option<u16>,
}

impl Options {
    /// A headless run of the reference machine, network included.
    pub(crate) fn plain() -> Self {
        Options {
            network: free_port(),
            ..Options::default()
        }
    }

    /// The same, with QEMU's own window.
    pub(crate) fn windowed(display: bool) -> Self {
        Options {
            display,
            ..Options::plain()
        }
    }

    /// The same, without the two network lines.
    pub(crate) fn without_network() -> Self {
        Options::default()
    }
}

/// A port on the loopback of the development machine that nothing listens
/// on, or `None` when the system would give none.
///
/// The port is asked of the system by binding one and letting it go again,
/// which is what every test harness does: nothing can hold a port between
/// the moment it is chosen and the moment QEMU takes it, and a port that
/// was taken in between makes QEMU refuse to start with a message that
/// names it.
pub(crate) fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// A socket path for the machine protocol, short enough for a Unix socket.
///
/// A Unix socket path holds 104 bytes at most and QEMU refuses a longer one
/// before it starts. The scratch directory of a worktree of this project
/// already spends most of that, so the socket goes into the shortest
/// directory that exists on every machine this runs on.
pub(crate) fn socket_path(name: &str) -> Result<PathBuf, Error> {
    let candidates = [
        std::env::temp_dir().join(format!("{name}-{}.sock", std::process::id())),
        PathBuf::from(format!("/tmp/{name}-{}.sock", std::process::id())),
    ];
    for candidate in candidates {
        if candidate.as_os_str().len() < MAX_SOCKET_PATH {
            return Ok(candidate);
        }
    }
    Err(Error::Usage(format!(
        "no directory holds a socket path for `{name}` below {MAX_SOCKET_PATH} bytes"
    )))
}

/// How long a Unix socket path may be, the terminating byte included.
const MAX_SOCKET_PATH: usize = 104;

/// The machine the tests run on.
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    qemu: PathBuf,
    firmware: PathBuf,
    accelerator: String,
    timeout: Duration,
}

impl Machine {
    /// The machine of this development environment.
    ///
    /// # Errors
    ///
    /// [`Error::Usage`] if QEMU or the firmware image cannot be found.
    pub(crate) fn locate() -> Result<Machine, Error> {
        let qemu = match std::env::var_os(QEMU_VARIABLE) {
            Some(value) => PathBuf::from(value),
            None => search_path(QEMU_BINARY).ok_or_else(|| {
                Error::Usage(format!(
                    "`{QEMU_BINARY}` is not on the PATH; set {QEMU_VARIABLE} to its path"
                ))
            })?,
        };
        if !qemu.is_file() {
            return Err(Error::Usage(format!(
                "{} is not a file; set {QEMU_VARIABLE} to the QEMU binary",
                qemu.display()
            )));
        }
        let accelerator = accelerator(&qemu)?;
        let firmware = match std::env::var_os(FIRMWARE_VARIABLE) {
            Some(value) => PathBuf::from(value),
            None => firmware_next_to(&qemu).ok_or_else(|| {
                let tried = firmware_candidates(&qemu)
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                Error::Usage(format!(
                    "no firmware image was found; tried {tried}; set {FIRMWARE_VARIABLE} to it"
                ))
            })?,
        };
        if !firmware.is_file() {
            return Err(Error::Usage(format!(
                "the firmware {} does not exist; set {FIRMWARE_VARIABLE} to it",
                firmware.display()
            )));
        }
        Ok(Machine {
            qemu,
            firmware,
            accelerator,
            timeout: Duration::from_secs(timeout_seconds()),
        })
    }

    /// The command line of the reference machine for `image`. `display`
    /// opens QEMU's own window instead of running headless.
    pub(crate) fn arguments(&self, image: &Path, options: &Options) -> Vec<String> {
        arguments(&self.firmware, image, &self.accelerator, options)
    }

    /// The QEMU binary of the reference machine.
    pub(crate) fn qemu(&self) -> &Path {
        &self.qemu
    }

    /// The accelerator every run of this machine uses.
    pub(crate) fn accelerator(&self) -> &str {
        &self.accelerator
    }

    /// The command line for messages.
    pub(crate) fn display(&self, image: &Path, options: &Options) -> String {
        let mut text = self.qemu.display().to_string();
        for argument in self.arguments(image, options) {
            text.push(' ');
            text.push_str(&argument);
        }
        text
    }

    /// Runs `image` with the serial port captured and the time limit
    /// enforced.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if QEMU cannot be started or waited for.
    pub(crate) fn run_captured(&self, image: &Path, options: &Options) -> Result<Run, Error> {
        let mut command = Command::new(&self.qemu);
        command
            .args(self.arguments(image, options))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|source| Error::io(format!("starting {}", self.qemu.display()), source))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let reader = std::thread::spawn(move || read_all(stdout));
        let errors = std::thread::spawn(move || read_all(stderr));
        let (status, timed_out) = self.wait(&mut child)?;
        let mut output = reader.join().unwrap_or_default();
        output.push_str(&errors.join().unwrap_or_default());
        Ok(Run {
            status: status.code(),
            signal: signal_of(status),
            output,
            timed_out,
        })
    }

    /// Runs `image` with QEMU's streams attached to the terminal and no
    /// time limit.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if QEMU cannot be started or waited for.
    pub(crate) fn run_attached(
        &self,
        image: &Path,
        options: &Options,
    ) -> Result<Option<i32>, Error> {
        eprintln!("$ {}", self.display(image, options));
        let status = Command::new(&self.qemu)
            .args(self.arguments(image, options))
            .status()
            .map_err(|source| Error::io(format!("running {}", self.qemu.display()), source))?;
        Ok(status.code())
    }

    /// Waits for the machine, killing it once the time limit is up.
    fn wait(&self, child: &mut std::process::Child) -> Result<(ExitStatus, bool), Error> {
        let deadline = Instant::now().checked_add(self.timeout);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok((status, false)),
                Ok(None) => {}
                Err(source) => return Err(Error::io("waiting for QEMU", source)),
            }
            if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
                let _ = child.kill();
                let status = child
                    .wait()
                    .map_err(|source| Error::io("waiting for QEMU", source))?;
                return Ok((status, true));
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// The time limit of one run.
    pub(crate) const fn timeout(&self) -> Duration {
        self.timeout
    }
}

/// The command line of the reference machine, as
/// [03-target-platform.md 3.1.1](../../../docs/03-target-platform.md)
/// prescribes it.
pub(crate) fn arguments(
    firmware: &Path,
    image: &Path,
    accelerator: &str,
    options: &Options,
) -> Vec<String> {
    let mut line = vec![
        "-machine".to_owned(),
        "q35".to_owned(),
        "-accel".to_owned(),
        accelerator.to_owned(),
        "-cpu".to_owned(),
        CPU_MODEL.to_owned(),
        "-smp".to_owned(),
        "1".to_owned(),
        "-m".to_owned(),
        "256M".to_owned(),
        "-drive".to_owned(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            firmware.display()
        ),
        "-drive".to_owned(),
        format!("format=raw,file={}", image.display()),
        "-serial".to_owned(),
        "stdio".to_owned(),
        "-display".to_owned(),
        if options.display {
            display_backend().to_owned()
        } else {
            "none".to_owned()
        },
    ];
    line.extend(graphics(options.no_vga));
    line.push("-no-reboot".to_owned());
    line.push("-device".to_owned());
    line.push("isa-debug-exit,iobase=0xf4,iosize=0x04".to_owned());
    if let Some(socket) = &options.qmp {
        line.push("-qmp".to_owned());
        line.push(format!("unix:{},server,nowait", socket.display()));
    }
    line.extend(network(options.network));
    line
}

/// The network of the reference machine, as
/// [13.12](../../../docs/13-the-network-on-the-machine.md) prescribes it, or
/// nothing for a run without one.
///
/// `disable-legacy=on` makes it a non-transitional virtio 1.0 device, whose
/// device identifier is then `0x1041` and not the transitional `0x1000`;
/// `mq=off` is the default and is written down because the driver depends on
/// it. The forwarded port reaches a listener inside the guest and needs no
/// host network and no privileges.
fn network(host_port: Option<u16>) -> Vec<String> {
    let Some(port) = host_port else {
        return Vec::new();
    };
    vec![
        "-netdev".to_owned(),
        format!("user,id=n0,hostfwd=tcp:127.0.0.1:{port}-:{GUEST_PORT}"),
        "-device".to_owned(),
        "virtio-net-pci,netdev=n0,disable-legacy=on,mq=off".to_owned(),
    ]
}

/// The graphics adapter of the reference machine and the mode the firmware
/// is to set on it, or an empty adapter slot when `none` is asked for.
///
/// The default adapter of the `q35` machine is left out and the same device
/// named instead, because the mode wanted is not the firmware's default and
/// it takes two halves to get it. The firmware offers a mode through the
/// Graphics Output Protocol only when the adapter's EDID carries it, which
/// is what `edid=on` with a size does; and it picks among the modes it is
/// offered by the two settings the firmware configuration device carries,
/// which is what the `-fw_cfg` pair does. Either half alone leaves the
/// firmware on its own default, which is 1280x800.
fn graphics(none: bool) -> Vec<String> {
    let mut line = vec!["-vga".to_owned(), "none".to_owned()];
    if none {
        return line;
    }
    line.push("-device".to_owned());
    line.push(format!(
        "VGA,edid=on,xres={SCREEN_WIDTH},yres={SCREEN_HEIGHT}"
    ));
    line.push("-fw_cfg".to_owned());
    line.push(format!(
        "name=opt/ovmf/PcdVideoHorizontalResolution,string={SCREEN_WIDTH}"
    ));
    line.push("-fw_cfg".to_owned());
    line.push(format!(
        "name=opt/ovmf/PcdVideoVerticalResolution,string={SCREEN_HEIGHT}"
    ));
    line
}

/// Reads a pipe to its end; a pipe that cannot be read yields what came
/// through before the failure.
fn read_all(stream: Option<impl Read>) -> String {
    let mut text = String::new();
    if let Some(mut stream) = stream {
        let _ = stream.read_to_string(&mut text);
    }
    text
}

/// The display backend `--display` asks for.
const fn display_backend() -> &'static str {
    if cfg!(target_os = "macos") {
        "cocoa"
    } else {
        "gtk"
    }
}

/// The time limit in seconds, from the environment or the default.
fn timeout_seconds() -> u64 {
    std::env::var(TIMEOUT_VARIABLE)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(DEFAULT_TIMEOUT)
}

/// The configured accelerator, or the first one the host can start.
fn accelerator(qemu: &Path) -> Result<String, Error> {
    if let Some(value) = std::env::var_os(ACCELERATOR_VARIABLE) {
        let name = value
            .to_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                Error::Usage(format!(
                    "{ACCELERATOR_VARIABLE} must name a QEMU accelerator"
                ))
            })?;
        return Ok(name.to_owned());
    }

    #[cfg(target_os = "linux")]
    {
        LINUX_ACCELERATOR
            .get_or_init(|| detect_linux_accelerator(qemu))
            .clone()
            .map_err(Error::Usage)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = qemu;
        Ok("tcg".to_owned())
    }
}

/// Tries KVM before TCG and returns the first accelerator that works.
///
/// Only Linux picks an accelerator; the tests exercise the order on every
/// host.
#[cfg(any(test, target_os = "linux"))]
pub(crate) fn choose_accelerator(mut works: impl FnMut(&str) -> bool) -> Option<&'static str> {
    ["kvm", "tcg"].into_iter().find(|name| works(name))
}

/// Starts QEMU once per candidate and records the choice for this process.
#[cfg(target_os = "linux")]
fn detect_linux_accelerator(qemu: &Path) -> Result<String, String> {
    let mut failures = Vec::new();
    let selected = choose_accelerator(|name| match probe_accelerator(qemu, name) {
        Ok(()) => true,
        Err(problem) => {
            failures.push(format!("{name}: {problem}"));
            false
        }
    });
    match selected {
        Some(name) => {
            note!("QEMU accelerator: {name}");
            Ok(name.to_owned())
        }
        None => Err(format!(
            "QEMU can start neither KVM nor TCG: {}",
            failures.join("; ")
        )),
    }
}

/// Starts and immediately stops the reference machine with one accelerator.
#[cfg(target_os = "linux")]
fn probe_accelerator(qemu: &Path, accelerator: &str) -> Result<(), String> {
    let mut child = Command::new(qemu)
        .args([
            "-machine",
            "q35",
            "-accel",
            accelerator,
            "-cpu",
            CPU_MODEL,
            "-display",
            "none",
            "-nodefaults",
            "-S",
            "-monitor",
            "stdio",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| format!("could not start {}: {source}", qemu.display()))?;
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(b"quit\n");
    }
    let output = child
        .wait_with_output()
        .map_err(|source| format!("could not wait for {}: {source}", qemu.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let diagnostic = String::from_utf8_lossy(&output.stderr)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if diagnostic.is_empty() {
        Err(format!("exited with {}", output.status))
    } else {
        Err(diagnostic)
    }
}

/// The first installed firmware image next to a QEMU binary.
fn firmware_next_to(qemu: &Path) -> Option<PathBuf> {
    firmware_candidates(qemu)
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// The firmware locations used by `MacPorts` and Linux distributions.
pub(crate) fn firmware_candidates(qemu: &Path) -> Vec<PathBuf> {
    let directory = qemu.parent().unwrap_or_else(|| Path::new(""));
    FIRMWARE_RELATIVES
        .iter()
        .map(|relative| directory.join(relative))
        .collect()
}

/// The first entry of the `PATH` that holds an executable file `name`.
fn search_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// The outcome of one test the machine reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TestOutcome {
    /// The test passed.
    Passed,
    /// The test failed, with the message the machine wrote.
    Failed(String),
}

/// One measurement the machine reported: what was measured, the median in
/// ticks, and how many round trips it is the median of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Measurement {
    /// The name of the measurement.
    pub(crate) name: String,
    /// The median, in ticks of the time-stamp counter.
    pub(crate) ticks: u64,
    /// How many round trips it stands for.
    pub(crate) samples: u32,
}

/// Everything the serial protocol carried.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    /// One entry per `[test]` line, in the order they arrived.
    pub(crate) tests: Vec<(String, TestOutcome)>,
    /// One entry per `[bench]` line, in the order they arrived.
    pub(crate) measurements: Vec<Measurement>,
    /// The counts of the `[summary]` line, if one arrived.
    pub(crate) summary: Option<(u32, u32)>,
}

impl Report {
    /// The number of tests that passed.
    pub(crate) fn passed(&self) -> u32 {
        self.count(&TestOutcome::Passed)
    }

    /// The number of tests that failed.
    pub(crate) fn failed(&self) -> u32 {
        u32::try_from(
            self.tests
                .iter()
                .filter(|(_, outcome)| *outcome != TestOutcome::Passed)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    fn count(&self, wanted: &TestOutcome) -> u32 {
        u32::try_from(
            self.tests
                .iter()
                .filter(|(_, outcome)| outcome == wanted)
                .count(),
        )
        .unwrap_or(u32::MAX)
    }

    /// The names of the tests that failed, with their messages.
    pub(crate) fn failures(&self) -> Vec<String> {
        self.tests
            .iter()
            .filter_map(|(name, outcome)| match outcome {
                TestOutcome::Passed => None,
                TestOutcome::Failed(message) if message.is_empty() => Some(name.clone()),
                TestOutcome::Failed(message) => Some(format!("{name}: {message}")),
            })
            .collect()
    }
}

/// Reads every protocol line out of `output`. Lines that are not protocol
/// lines are ignored: the firmware writes its own.
pub(crate) fn parse(output: &str) -> Report {
    let mut report = Report::default();
    for line in output.lines() {
        let line = strip_escapes(line);
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix(TEST_PREFIX) {
            if let Some(test) = parse_test(rest) {
                report.tests.push(test);
            }
        } else if let Some(rest) = line.strip_prefix(BENCH_PREFIX) {
            if let Some(measurement) = parse_measurement(rest) {
                report.measurements.push(measurement);
            }
        } else if let Some(rest) = line.strip_prefix(SUMMARY_PREFIX) {
            report.summary = parse_summary(rest).or(report.summary);
        }
    }
    report
}

/// One `[test]` line without its prefix.
fn parse_test(rest: &str) -> Option<(String, TestOutcome)> {
    let position = rest.find(SEPARATOR)?;
    let name = rest.get(..position)?.to_owned();
    let outcome = rest.get(position.checked_add(SEPARATOR.len())?..)?;
    if outcome == OK {
        return Some((name, TestOutcome::Passed));
    }
    let message = outcome.strip_prefix(FAILED)?;
    Some((name, TestOutcome::Failed(message.to_owned())))
}

/// One `[bench]` line without its prefix: a name, the separator, the
/// median in ticks, and how many round trips it is the median of.
fn parse_measurement(rest: &str) -> Option<Measurement> {
    let position = rest.find(SEPARATOR)?;
    let name = rest.get(..position)?.to_owned();
    let figure = rest.get(position.checked_add(SEPARATOR.len())?..)?;
    let (ticks, count) = figure.split_once(TICKS)?;
    let samples = count
        .trim()
        .strip_prefix("(n=")?
        .strip_suffix(')')?
        .parse()
        .ok()?;
    Some(Measurement {
        name,
        ticks: ticks.parse().ok()?,
        samples,
    })
}

/// One `[summary]` line without its prefix.
fn parse_summary(rest: &str) -> Option<(u32, u32)> {
    let mut passed = None;
    let mut failed = None;
    for field in rest.split_whitespace() {
        if let Some(value) = field.strip_prefix("passed=") {
            passed = value.parse().ok();
        } else if let Some(value) = field.strip_prefix("failed=") {
            failed = value.parse().ok();
        }
    }
    Some((passed?, failed?))
}

/// Removes the terminal escape sequences the firmware writes, so that a
/// protocol line that follows one on the same line is still recognized.
pub(crate) fn strip_escapes(line: &str) -> &str {
    match line.rfind('\u{1b}') {
        None => line,
        Some(start) => {
            let rest = line.get(start..).unwrap_or("");
            let end = rest
                .find(|c: char| c.is_ascii_alphabetic())
                .map_or(line.len(), |offset| {
                    start.saturating_add(offset).saturating_add(1)
                });
            line.get(end..).unwrap_or("")
        }
    }
}

/// What the report says about a run that was supposed to pass.
///
/// # Errors
///
/// [`Error::Violations`] naming every failed test, a summary that
/// disagrees with the lines, or a missing summary.
pub(crate) fn check(report: &Report, run: &Run) -> Result<(), Error> {
    let outcome = run.outcome();
    let mut violations = report.failures();
    match report.summary {
        None => violations.push("the machine wrote no summary line".to_owned()),
        Some((passed, failed)) if passed != report.passed() || failed != report.failed() => {
            violations.push(format!(
                "the summary says passed={passed} failed={failed}, \
                 the lines say passed={} failed={}",
                report.passed(),
                report.failed()
            ));
        }
        Some(_) => {}
    }
    if outcome != Outcome::Success {
        violations.push(format!(
            "the machine reported a {}: {}",
            outcome.name(),
            run.how()
        ));
    }
    Error::from_violations(violations)
}
