# jrs CLI

The host executable of [jrs](../../jrs). It grants only a `print` capability
to a script. `sh tools/xtask.sh jrs --help` lists the command line options.
Runtime failures and invalid command lines exit unsuccessfully.

`sh tools/xtask.sh jrs --wpt ROOT TEST_FILE...` runs selected tests from a
local WPT checkout with its original `resources/testharness.js`. The runner
extracts classic HTML scripts and JS META dependencies and reports the actual
result/completion callbacks. Failed assertions, harness errors, missing results
or completion, unsupported modules/variants and runtime failures exit nonzero.
Resource paths cannot leave ROOT; source and execution budgets remain active.
The runner explicitly installs queueMicrotask; its callbacks share the Promise
FIFO queue. Uncaught microtask exceptions fail the run even if subsequent
test-result callbacks report success. This does not implement ErrorEvent routing.
It also installs standalone Event/CustomEvent/EventTarget/DOMException and supplies
event timestamps from a monotonic host clock. The global is not a Window or
EventTarget, and there is no DOM parent path. Listener exceptions are reported
and fail the run; no implicit document/global event behavior is fabricated.
WPT is test input, never a runtime/build dependency. HTML extraction reuses
the internal `doc-html` crate; no external dependency was added.

This is a shell runner, not a browser or `wpt run` product adapter. Scripts
are evaluated independently in a shared Realm with declaration instantiation
and Promise job checkpoints, never concatenated into one compilation unit.
The WPT shell explicitly installs timers backed by a monotonic active-time clock
and pumps one due task plus its microtask checkpoint at a time. It sleeps in
bounded slices, never fabricates elapsed time, and caps the post-script timer
loop at 30 seconds. Lack of completion, reported errors and fuel exhaustion are
failures. This shell is always active; document lifecycle, worker suspension,
Trusted Types/CSP metadata and cross-realm timer routing remain absent.
The start callback reads the original harness properties. For `single_test` and
test-requested `explicit_done`, the runner waits for the test's own completion;
it must never call done merely because all source files have loaded. Missing
completion and delayed assertion failures are covered by transport regressions.
DOM, rendering, network and worker facilities
are absent. See the core README for the exact pinned WPT results and gaps.
Project-owned runner transport fixtures are not WPT conformance tests.

`jrs [--fuel N] --realm FIRST.js SECOND.js` exposes the same persistent realm
execution for local scripts. Files execute in order; print is the output
capability. Failures stop the command. Fuel is shared across the sequence.
