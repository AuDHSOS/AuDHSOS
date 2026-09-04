# Changelog

All notable changes to this project are documented in this file. The format
follows Keep a Changelog; the project follows Semantic Versioning.

## [Unreleased]

### Added

- Planning documents and the decision register under `docs/`.
- Workspace foundation: pinned toolchain, workspace lint set, SPDX headers,
  license, CI workflow.
- `audhsos-abi`: error codes, rights, object types, handles, layout
  constants.
- `kernel-types`: physical and virtual addresses, frames, pages, ranges,
  alignment.
- `kernel-hal-api`: hardware abstraction traits with test doubles.
- `audhsos-sync`: the `Global<T>` cell for global state.
- `test-support`: property-test engine with integrated shrinking and
  model-test runner.
- Unit tests live in `src/tests/` so that coverage measures product code
  only.
- `xtask`: `lint`, `check-layering`, `check-deps`, `unsafe-budget`,
  `test`, `coverage`, `miri`, `doc`, `check`.
