# AuDHSOS

A capability-based microkernel operating system written entirely in Rust,
without external code.

- Kernel mechanisms: address spaces, threads with scheduling, IPC,
  capabilities, interrupt forwarding. Everything else runs in userland.
- No assembly files. `unsafe` only in the eight allowlisted adapter crates
  of [the safety policy](docs/04-safety-policy.md), each block documented
  and budgeted.
- No dependencies outside this repository. Building needs the pinned Rust
  toolchain from `rust-toolchain.toml` and QEMU with its bundled UEFI
  firmware.
- First target: QEMU `q35` on `x86_64`, booted through UEFI by the
  project's own loader.

The design, the rules, and the roadmap are in [docs/](docs/README.md).

## Building and checking

All commands go through the build automation, started by the wrapper
scripts under `tools/`:

```bash
sh tools/xtask-check.sh
```

runs every check that CI runs. `sh tools/xtask.sh <subcommand>` runs a
single one, `sh tools/xtask.sh --help` lists them. The scripts put rustup's
Cargo proxy (`~/.cargo/bin/cargo`), which honors the toolchain pin, in front
of the `PATH`; the xtask stops under any other Cargo. They add nothing else:
the output and the exit status are the xtask's own.

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
