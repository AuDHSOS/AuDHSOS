# test-support, fuzz-support, kernel-test-harness, membench, norec, text-demo audit findings

Repository: AuDHSOS/AuDHSOS. Audit of test-support, fuzz-support, kernel-test-harness, membench, norec, text-demo at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #499
Title: fuzz-support: a counter range registered more than once is counted once per registration
Labels: bug, part::tools
Body:
`counters::register` at `crates/support/fuzz/src/counters.rs:49-63` appends every non-empty range to `STARTS` and `LENGTHS` and has no check against the range registered before it. The doc comment at `crates/support/fuzz/src/counters.rs:21-25` states that one object file registers one range. LLVM's `SanitizerCoverage` pass emits one module constructor per codegen unit, and every constructor passes the bounds of the whole counter section (`__start___sancov_cntrs`/`__stop___sancov_cntrs` on ELF, `section$start$__DATA$__sancov_cntrs`/`section$end$...` on Mach-O); the pass deduplicates the constructor through a COMDAT group on ELF and COFF and cannot on Mach-O, which is why libFuzzer's `TracePC::HandleInline8bitCountersInit` drops a registration whose start equals the last one. The same applies to `__sanitizer_cov_pcs_init` at `crates/support/fuzz/src/sancov.rs:179-187`, which adds the table's block count on every call.

On macOS, a fuzz target whose instrumented crates have k codegen units registers the one section k times. `with_regions` at `crates/support/fuzz/src/counters.rs:102-118` then clears and scans k copies of the section per run, `for_each_nonzero` at `crates/support/fuzz/src/counters.rs:133-165` numbers the same counter k times, `feature_of` at `crates/support/fuzz/src/feature.rs:50-55` turns each number into its own feature, `blocks()` reports k times the real block count in the line at `crates/support/fuzz/src/engine.rs:210-214`, and `value_base` at `crates/support/fuzz/src/engine.rs:140` moves up k-fold. With k times c counters above the 262,144 counters that the 2^21 slots of `crates/support/fuzz/src/cover.rs:22` hold, `slot_of` folds different counters onto one slot and the pool stops telling them apart. A release build of the fuzz workspace has 16 codegen units per crate by default, so k is in the tens to hundreds on the machine the README names as a reason for this engine (`crates/support/fuzz/README.md`, "every machine with Apple's clang").

Fix: in `register`, return when `start_address` equals the start stored in the last used slot (libFuzzer's rule), and apply the same test to `__sanitizer_cov_pcs_init` by remembering the last `beg`; the alternative of building the fuzz workspace with `codegen-units = 1` removes the symptom on Mach-O and leaves the engine wrong for any other caller.

---

## F02 — issue #500
Title: fuzz-support: a run without -seed takes its seed from the process uptime and repeats the previous run
Labels: bug, part::tools
Body:
`now()` at `crates/support/fuzz/src/engine.rs:73-76` returns the milliseconds since `START` was initialized, which `run` does at `crates/support/fuzz/src/engine.rs:84` as its first statement. `Runner::new` at `crates/support/fuzz/src/engine.rs:137-139` seeds the mutator with `now().wrapping_add(GOLDEN_TIME)` when `-seed` is absent, and `worker_command` at `crates/support/fuzz/src/orchestrator.rs:214-223` derives every worker's seed from `options.seed.unwrap_or_else(now)`. Both calls happen before the corpus is read, so `now()` is 0 or a few milliseconds in every run. `crates/support/fuzz/src/options.rs:90-91` documents the seed as taken from the clock, and `crates/support/fuzz/src/engine.rs:569-571` mixes in `GOLDEN_TIME` "so that two runs started in the same millisecond still differ".

Two invocations of `target corpus/` without `-seed` on the same corpus draw the same mutation sequence; two invocations with `-workers=N` hand every worker the same seed both times. `cargo xtask fuzz` is not affected because it passes a `-seed` derived from its process id (`crates/tools/xtask/src/commands.rs:2930-2939`).

Fix: seed from `SystemTime::now()` (nanoseconds since the epoch) mixed with the process id at both sites; a seed from `Instant` cannot differ between processes.

---

## F03 — issue #501
Title: fuzz-support: -len_control=0 holds the length limit at 4 bytes for the whole run
Labels: bug, part::tools
Body:
`fuzz` starts the limit at `SMALLEST_LIMIT.min(options.max_len)` (`crates/support/fuzz/src/engine.rs:259`) and grows it only under `options.len_control > 0` (`crates/support/fuzz/src/engine.rs:276-286`); `fuzz_fleet` does the same at `crates/support/fuzz/src/orchestrator.rs:379` and `crates/support/fuzz/src/orchestrator.rs:406-419`. `crates/support/fuzz/src/options.rs:86-89` documents zero as "holds the limit where it starts". libFuzzer's `FuzzerFlags.def` defines `-len_control=0` as "immediately try inputs with size up to max_len", and `crates/support/fuzz/README.md` claims the flag names and their meaning follow libFuzzer.

A user who passes `-len_control=0` to skip the length ramp gets a run whose mutator produces inputs of at most 4 bytes until the deadline, on every worker.

Fix: when `len_control` is zero, start `limit` at `options.max_len`; the option not taken, renaming the flag, breaks the libFuzzer command lines the README promises to keep working.

---

## F04 — issue #502
Title: fuzz-support: a corpus input longer than the current length limit is cut to the limit on its first mutation
Labels: bug, part::tools
Body:
`Mutator::mutate` truncates the input to `limit` after every applied mutation (`crates/support/fuzz/src/mutate.rs:190`). `round` passes the run's limit unchanged (`crates/support/fuzz/src/engine.rs:305-317`), and the worker passes the batch limit unchanged (`crates/support/fuzz/src/worker.rs:179-183`). The limit starts at 4 bytes (`crates/support/fuzz/src/engine.rs:259`, `crates/support/fuzz/src/engine.rs:567`) whatever the corpus holds. libFuzzer's `Fuzzer::MutateAndTestOne` bounds a mutation by `Min(MaxMutationLen, Max(U.size(), TmpMaxMutationLen))`, so a corpus input is never cut below its own size.

With a seed corpus of 1000-byte files, every round drawn from one of them changes one byte and then keeps its first 4 bytes, and that 4-byte prefix is what the target runs; the corpus files are useful only after the limit has grown past their size, which at the default `len_control` of 100 takes on the order of 10^5 runs. `-max_len` smaller than a corpus file cuts that file in every round for the whole run.

Fix: pass `limit.max(input.len())` to `mutate` in `round` and in the worker's `round`, and keep `options.max_len` as the outer bound as libFuzzer does.

---

## F05 — issue #503
Title: fuzz-support: an empty input claims features with size zero, which the ownership table reads as unowned
Labels: bug, part::tools
Body:
`Pool::offer` stores the input's length as the feature's `smallest` (`crates/support/fuzz/src/pool.rs:167-197`), and `claim` tests ownership with `held.smallest == 0` (`crates/support/fuzz/src/pool.rs:214-238`), the value documented as "nothing reaches" at `crates/support/fuzz/src/pool.rs:57-59`. An input of length zero writes `smallest: 0`, so the slot reads as unowned after the claim. `Cover::claim` at `crates/support/fuzz/src/cover.rs:66-75` has the same test. Empty inputs arrive on purpose from `crates/support/fuzz/src/engine.rs:242-246` and `crates/support/fuzz/src/orchestrator.rs:322-324`, from a zero-byte corpus file through `crates/support/fuzz/src/engine.rs:190-200`, and in a worker through `crates/support/fuzz/src/worker.rs:207-223`. libFuzzer's `Fuzzer::RunOne` returns before running an input of size zero.

A corpus with a zero-byte file whose run reaches f features: `covered` rises by f for the empty file and by f again when the next input reaches the same features, because every one of them reads as fresh; the second input is reported `NEW` for features already reached; `release` at `crates/support/fuzz/src/pool.rs:248-260` never runs for the empty input, so its `features` count stays f and its weight keeps it in every draw; the worker's table forgets the empty input's features and the worker reports them again after every run of it.

Fix: refuse a zero-length input in `Pool::offer`, `Cover::claim` and the worker's `probe` (libFuzzer's rule), and offer a one-byte input where the pool would otherwise start empty; the option not taken, storing `size.max(1)`, keeps the empty input in the corpus files.

---

## F06 — issue #504
Title: norec: --seed N --runs 1 reruns an odd-numbered finding with the other count style
Labels: bug, part::tools
Body:
`generated` at `crates/tools/norec/src/main.rs:174-181` picks `CountStyle::Rows` for an even case number and `CountStyle::Count` for an odd one, where the number is the position in the run and not the seed (`crates/tools/norec/src/main.rs:132-134`, `crates/tools/norec/src/main.rs:115-117`). The count style changes the optimized query between `SELECT *` and `SELECT COUNT(*)` (`crates/tools/norec/src/case.rs:119-129`). `crates/tools/norec/src/options.rs:28-29` and `crates/tools/norec/README.md:72-73` state that `--seed N --runs 1` runs exactly the case reported as N.

A finding made at seed 124 in a run started with `--seed 1` (number 123, style `Count`) is rerun by `--seed 124 --runs 1` with number 0 and style `Rows`; a disagreement that only the `COUNT(*)` plan shows does not reproduce, and `--script` prints the wrong query for it.

Fix: derive the style from the seed (`seed % 2`) instead of the number, which keeps the alternation and makes the reproduction claim true.

---

## F07 — issue #505
Title: text-demo: the workspace documentation claims a lint configuration and a catalog entry the crate does not have
Labels: bug, part::tools
Body:
`docs/05-code-organization.md:324-325` states that every crate declares `[lints] workspace = true`. `crates/text-demo/Cargo.toml:10-14` omits the declaration on purpose, and `crates/text-demo/src/main.rs` uses `as` casts, slice indexing and `expect` (`crates/text-demo/src/main.rs:590`) that the workspace lints of `docs/04-safety-policy.md` rule R7 deny. The crate is a workspace member (`Cargo.toml:9`) and a policy entry (`crates/tools/xtask/src/policy.rs:1041-1048`), and the catalog at `docs/05-code-organization.md:145-235` and the tree at `docs/05-code-organization.md:1-140` do not list it.

A reader of section 5.4 concludes that R7 holds in every crate of the workspace; `cargo clippy` on `text-demo` checks none of the R7 lints.

Fix: add a catalog row for `text-demo` and state in 5.4 that it is the one crate outside the workspace lints, with the reason from its manifest; the option not taken, applying the workspace lints, rewrites a crate the manifest calls a throwaway.

---

## F08 — issue #506
Title: fuzz-support: -max_len=0 makes every round run the empty input
Labels: bug, part::tools
Body:
`fuzz` sets the limit to `SMALLEST_LIMIT.min(options.max_len)`, which is 0 for `-max_len=0` (`crates/support/fuzz/src/engine.rs:259`). `erase_bytes` at `crates/support/fuzz/src/mutate.rs:225-236` ignores the limit and succeeds on any input of two or more bytes, after which `crates/support/fuzz/src/mutate.rs:190` truncates the input to 0 bytes; every other mutation refuses an empty input under a limit of 0, so the round at `crates/support/fuzz/src/engine.rs:305-327` runs the target once on the empty input and ends. libFuzzer's `FuzzerFlags.def` defines `-max_len=0` as a length guessed from the corpus.

`target corpus/ -max_len=0` runs the target on the empty input once per round until the deadline or `-runs`, and reports the run as normal.

Fix: treat `max_len` of 0 as the larger of `DEFAULT_MAX_LEN` and the longest corpus file, which `load` already measures (`crates/support/fuzz/src/engine.rs:187-193`).

---

## F09 — issue #507
Title: fuzz-support: the counters module claims to hold the only unsafe code of a fuzzing run
Labels: bug, part::tools
Body:
`crates/support/fuzz/src/counters.rs:10-11` states that the module is "the only place in the project where a fuzzing run is unsafe code". `crates/support/fuzz/src/sancov.rs:63` has an `unsafe impl Sync`, and `crates/support/fuzz/src/sancov.rs:83`, `crates/support/fuzz/src/sancov.rs:107`, `crates/support/fuzz/src/sancov.rs:256` and `crates/support/fuzz/src/sancov.rs:267` hold unsafe blocks that run during a fuzzing run. `crates/support/fuzz/README.md` and `docs/05-code-organization.md:220` name both modules.

A reviewer following the checklist of `docs/04-safety-policy.md` 4.9 who trusts the module comment reviews half of the crate's unsafe code.

Fix: name `sancov` beside `counters` in the comment.

---

## F10 — issue #508
Title: fuzz-support: three fuzz targets are built without the coverage instrumentation
Labels: enhancement, part::tools
Body:
`fuzz/Cargo.toml:107-114` instruments every non-member dependency, and the comment at `fuzz/Cargo.toml:97-105` says the members are listed one by one because their own branches are worth steering by. The members `jrs_backend`, `tcp_segment` and `virtio_net_rx` (`fuzz/Cargo.toml:22`, `fuzz/Cargo.toml:39`, `fuzz/Cargo.toml:42`) have no `[profile.release.package.<name>]` section among `fuzz/Cargo.toml:118-387`.

The `main.rs` of each of those three targets carries neither counters nor compare callbacks, so a branch in the target's own input splitting (a length field, a mode byte) gives the mutator no feature and no dictionary entry.

Fix: add the three sections, and add a test in the xtask policy that every member of `fuzz/Cargo.toml` has one, so the next target cannot be forgotten.

---

## F11 — issue #509
Title: fuzz-support: a worker's coverage table is sent once and never refreshed
Labels: enhancement, part::tools
Body:
`orchestrate` broadcasts `Down::Cover` once after the corpus is loaded (`crates/support/fuzz/src/orchestrator.rs:325`). A worker replaces its table on that message (`crates/support/fuzz/src/worker.rs:114`) and afterwards claims only what it reaches itself (`crates/support/fuzz/src/worker.rs:207-223`). The orchestrator refuses a report of a feature the pool already owns (`crates/support/fuzz/src/orchestrator.rs:507-511`).

With W workers and F features found during the run, each feature first reached by one worker is reported again by each of the other W-1 workers the first time they reach it, each report carrying the whole input and its feature list, and each costing a `Pool::offer` over the list: O(W times F) redundant messages per run.

Fix: after every accepted find, send the claimed features to the other workers as a small `Down` message, or re-broadcast `Down::Cover` every few batches.

---

## F12 — issue #510
Title: test-support: shrinking a vector materializes every candidate of a level before the first is tested
Labels: enhancement, part::tools
Body:
`sequence` at `crates/support/testing/src/tree.rs:113-149` builds the whole candidate list eagerly: one clone of the element vector per chunk removal (`crates/support/testing/src/tree.rs:125`) and one per element candidate (`crates/support/testing/src/tree.rs:144`), and `shrink` at `crates/support/testing/src/property.rs:171` receives the full `Vec` before it evaluates the first candidate. For a vector of n elements with k candidates per element the list has O(n + n times k) entries of O(n) size each: O(n^2 times k) memory per level.

A failing case of `bytes(0..=600)` (`crates/deflate/src/tests/roundtrip.rs:188`, `crates/net/ssh/src/tests/packet.rs:60`) at full length allocates about 1,200 chunk candidates and up to 4,800 element candidates, each holding a cloned vector of 600 trees and a 600-byte value, before the first is tested, and again at every level the shrink descends.

Fix: make `shrinks` return an iterator and yield the chunk removals first, so the element candidates are built only after every removal has been tested.

---

## F13 — issue #511
Title: fuzz-support: corpus collection follows a directory symlink cycle without bound
Labels: enhancement, part::tools
Body:
`collect` at `crates/support/fuzz/src/corpus.rs:63-72` calls `std::fs::read_dir` on every entry, which follows symbolic links, and recurses with no depth bound and no record of visited directories.

A corpus directory holding `loop -> ..` sends `files_under` into unbounded recursion until the stack overflows, in `replay_paths` and in `load`.

Fix: skip an entry whose `symlink_metadata` is a symbolic link, or bound the depth.

---

## F14 — issue #512
Title: kernel-test-harness: begin during a running test leaves the previous line without an outcome
Labels: enhancement, part::tools
Body:
`Harness::begin` at `crates/kernel/test-harness/src/harness.rs:104-107` writes a new `[test] <name> ... ` prefix (`crates/kernel/test-harness/src/protocol.rs:53-55`) and sets `running` without checking that it is already set. The invariant at `crates/kernel/test-harness/src/harness.rs:6-8` says every started test gets exactly one outcome.

An image that calls `begin("a")`, `begin("b")`, `end()` writes `[test] a ... [test] b ... ok` on one line. `parse_test` at `crates/tools/xtask/src/qemu.rs:827-836` takes the first separator, reads the outcome `[test] b ... ok`, matches neither `ok` nor `FAILED: `, and drops the line, so both tests vanish from the report and the run fails only through the summary count check.

Fix: in `begin`, close an open line with `FAILED: begun again` before writing the new prefix.

---

## F15 — issue #513
Title: fuzz-support: the corpus loader computes the longest file and discards it
Labels: enhancement, part::tools
Body:
`load` at `crates/support/fuzz/src/engine.rs:187` and `crates/support/fuzz/src/engine.rs:193` tracks `longest` over every corpus file and drops it with `let _ = longest;` at `crates/support/fuzz/src/engine.rs:216`.

The value is dead code; F08 names the use libFuzzer has for it.

Fix: use it for the `-max_len=0` default, or delete the three lines.

---

## F16 — issue #514
Title: membench: the README omits the --chase option
Labels: enhancement, part::tools
Body:
`crates/tools/membench/README.md:9` lists `--size`, `--passes` and `--steps`. The usage text at `crates/tools/membench/src/config.rs:8-14` and the parser at `crates/tools/membench/src/config.rs:81` also take `--chase`, and `cargo xtask membench` forwards every option (`crates/tools/xtask/src/commands.rs:2467-2472`).

A reader of the README cannot lower the 512 MiB working set of the pointer chase on a small machine without reading the source.

Fix: add `[--chase <mebibytes>]` to the command line in the README.
