# xtask

The build automation. Invoked as `cargo xtask <subcommand>`; `cargo xtask
--help` lists the subcommands. Uses only the standard library and the
binaries of the pinned toolchain. The policy tables (layering, unsafe
budgets, SPDX header, coverage thresholds, fuzz targets) live in
`src/policy.rs`.
