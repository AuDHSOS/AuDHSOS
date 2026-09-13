# 7. Toolchain and Environment

## 7.1 Rust toolchain

The project pins one nightly toolchain.

```toml
# rust-toolchain.toml
[toolchain]
channel = "nightly-2026-08-25"   # exact date is fixed in Phase 0 after verification
components = ["rustfmt", "clippy", "rust-src", "llvm-tools-preview", "miri"]
targets = ["x86_64-unknown-none", "x86_64-unknown-uefi"]
profile = "minimal"
```

Unstable features in use, and nowhere else:

| Feature | Used by | Purpose |
|---------|---------|---------|
| `abi_x86_interrupt` | `kernel-hal-x86_64` | compiler-generated interrupt entry and exit |
| `custom_test_frameworks` | kernel binary and test kernels | running `#[test_case]` functions inside QEMU |
| `-Zsanitizer=fuzzer` | fuzz target builds on the host | links the fuzzer runtime shipped with the toolchain |

Adding an unstable feature requires a decision register entry. The
toolchain date is bumped about once a month in its own commit after `cargo
xtask check` passes.

## 7.2 Targets

| Target | Used for | Linker |
|--------|----------|--------|
| `x86_64-unknown-uefi` | the loader | `rust-lld` bundled with the toolchain, PE output |
| `x86_64-unknown-none` | kernel, test kernels, every userland binary | `rust-lld` bundled with the toolchain |
| `aarch64-apple-darwin` | host tests, xtask, fuzzing on the development machine | system linker |
| `x86_64-unknown-linux-gnu` | host tests, xtask, fuzzing in CI | system linker |

The kernel is linked with the static relocation model at `KERNEL_BASE`.
Userland ELF binaries are linked statically at a fixed base address; the
loader honors their program headers, and so does the kernel, which reads
the root task out of the boot image as an ELF (D-92). What is converted to
a flat binary with `llvm-objcopy` from `llvm-tools-preview` are the user
programs of the kernel test images: a test image maps the bytes it embeds
at one address, with no loader in between.

## 7.3 Host tools

None beyond the toolchain. The xtask calls `cargo`, `rustc`, `rustfmt`,
`cargo-clippy`, `cargo-miri`, `llvm-profdata`, `llvm-cov`, `llvm-objcopy`,
and `qemu-system-x86_64`. Container software is never used, locally or in
CI.

`sh tools/sqlite.sh` is outside all of that. It clones SQLite, checks out
one release tag, and builds it below `research/`, which `.gitignore` keeps
out of the repository, so a reference implementation can be read and run
beside our own. It needs `git`, `make` and a C compiler, and nothing this
project builds depends on it. `--version X.Y.Z` picks the tag
(`version-X.Y.Z`, 3.53.4 by default), `--dir` the checkout, `--jobs` the
parallel compiler jobs, and `--clean` clones again. A second run on an
existing checkout rebuilds only what changed. No `tclsh` is needed:
SQLite's autosetup builds its own `jimsh` and runs every code generator
through it.

```bash
sh tools/sqlite.sh --version 3.53.4
```

## 7.4 QEMU

QEMU 11.1.1 from MacPorts is installed: `/opt/local/bin/qemu-system-x86_64`
with the UEFI firmware `/opt/local/share/qemu/edk2-x86_64-code.fd`. The
xtask finds QEMU on the `PATH` or through `AUDHSOS_QEMU`, and the firmware
next to the QEMU binary (`../share/qemu/`) or through `AUDHSOS_OVMF`.
`AUDHSOS_QEMU_TIMEOUT` overrides the per-kernel timeout in seconds.

On Linux the xtask probes KVM once before the first machine, then TCG if
KVM cannot start. It passes the selected name through
`AUDHSOS_QEMU_ACCELERATOR` to every Cargo runner, so the many kernel test
machines do not repeat the probe. Setting the variable explicitly skips
the probe. macOS keeps using TCG.

Firmware for later targets is present in the same directory:
`edk2-aarch64-code.fd` and `edk2-riscv-code.fd`.

Graphical test runs (Phase 9 and later) use the same QEMU. The runner adds
a QMP socket in the scratch directory and drives it with the xtask's own
QMP client; `screendump` writes a PPM file that the runner reads. For
interactive use `cargo xtask run --display` opens QEMU's `cocoa` display on
macOS and `gtk` on Linux. CI never opens a display.

## 7.5 Findings about the development machine

Recorded on 2026-09-04. These influence Phase 0.

| Finding | Consequence |
|---------|-------------|
| Apple Silicon (`arm64`) | `x86_64` guests run under TCG without acceleration. An `aarch64` port runs under HVF. |
| rustup with `stable-aarch64-apple-darwin` 1.97.0; no nightly installed | Phase 0 installs the pinned nightly through `rust-toolchain.toml` on first use. |
| `/opt/local/bin/rustc` (MacPorts) precedes `~/.cargo/bin` on the `PATH` | A bare `rustc` is not rustup-managed, and rustup's Cargo would pick it up. The xtask therefore sets `RUSTC` and `RUSTDOC` for every Cargo it starts. |
| `cargo` is a shell alias for a locally built Cargo 1.95 outside rustup | That Cargo ignores the toolchain pin. Project commands go through the wrapper scripts below, which put `~/.cargo/bin` in front of the `PATH` and call rustup's proxy; `rustup run <pinned> cargo ...` is the equivalent by hand. The xtask verifies at start that `RUSTUP_TOOLCHAIN` names the pinned nightly and stops with this instruction otherwise. |
| QEMU 11.1.1 from MacPorts with the EDK2 firmware files; no Homebrew | Reference configuration in 3.1.1 works unchanged. |
| Git repository initialized on `main` with no commits | Phase 0 makes the first commit. |
| Container software must not be used on this machine | CI and local runs are native. |

Three wrapper scripts under `tools/` are the entry point that follows from
these findings. They are what the commands in these documents mean on the
development machine:

| Command as these documents write it | What is run here |
|-------------------------------------|------------------|
| `cargo xtask <subcommand>` | `sh tools/xtask.sh <subcommand>` |
| `cargo xtask check` | `sh tools/xtask-check.sh` |
| any other Cargo command | `sh tools/cargo.sh <arguments>` |

Each script changes into the workspace root, puts `~/.cargo/bin` in front of
the `PATH`, turns the pager and the colors off, and then replaces itself
with the proxy through `exec`. They add nothing to the run: the output is
the run's own and so is the exit status, so `$?` and `&&` mean what they
say. They are tracked, so a worktree has them. CI calls the proxy directly,
where the `PATH` is already right.

The third exists for what is not an xtask subcommand, `cargo fmt -p <crate>`
above all: `cargo fmt --all` fails while another session has a half-written
crate in the workspace, and a single package is then formatted on its own.

A warm full check writes about three thousand lines, which is worth
watching and worth nothing in a log. `--quiet` reduces it to one line per
step and prints the output of a step only when that step fails:

```bash
sh tools/xtask-check.sh --quiet
```

`sh tools/target-clean.sh` is maintenance, not a wrapper. The target
directory of this workspace passes twenty gigabytes — the host kernel
under `x86_64-unknown-none`, the coverage build, and the QEMU artifacts
each hold gigabytes of their own — and most of it belongs to work that is
finished. The script deletes the files below `$CARGO_TARGET_DIR`, or
`./target`, whose mtime predates a cutoff of fourteen days by default,
then removes the directories that empties. `CACHEDIR.TAG` stays, because
it is what keeps a backup out of the tree. Cargo and the xtask rebuild
what goes.

```bash
sh tools/target-clean.sh --dry-run --days 7
```

`--dry-run` names every file and the total instead of deleting. The cutoff
is a marker file and `find ! -newer`, because the two `find`
implementations round `-mtime` differently. No build may run at the same
time: a file the compiler is still writing is old as soon as its mtime
predates the cutoff.

## 7.6 Continuous integration

GitHub Actions on Linux runners with QEMU and its UEFI firmware from the
distribution packages (`qemu-system-x86`, `ovmf`). The pipeline order is
in [6.7](06-testing-strategy.md#67-ci-pipeline). The rust-toolchain file
drives the toolchain in CI exactly as on the development machine.
