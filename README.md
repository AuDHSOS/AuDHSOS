# AuDHSOS

A capability-based microkernel operating system written entirely in Rust,
without external code.

- Kernel mechanisms: address spaces, threads with scheduling, IPC,
  capabilities, interrupt forwarding. Everything else runs in userland.
- No assembly files. `unsafe` only in five allowlisted adapter crates, each
  block documented and budgeted.
- No dependencies outside this repository. Building needs the pinned Rust
  toolchain from `rust-toolchain.toml` and QEMU with its bundled UEFI
  firmware.
- First target: QEMU `q35` on `x86_64`, booted through UEFI by the
  project's own loader.

The design, the rules, and the roadmap are in [docs/](docs/README.md).

## Building and checking

All commands go through the build automation:

```bash
cargo xtask check
```

runs every check that CI runs. `cargo xtask --help` lists the subcommands.
Use rustup's Cargo proxy (`~/.cargo/bin/cargo`), which honors the toolchain
pin; the xtask stops otherwise.

## License

AGPL-3.0-only. See [LICENSE](LICENSE).
