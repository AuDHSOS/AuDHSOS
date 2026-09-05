# kernel-test-harness

The runner a kernel test image uses: it takes a list of tests, writes one
protocol line per test and one summary line, and ends the machine with the
matching status. The lines are the ones the host-side runner parses, so
this crate and the xtask must agree; they do, because both are tested
against the same examples.

The runner writes through the `DebugConsole` and `TestExit` traits, so the
whole sequence runs on the host against recording doubles.
