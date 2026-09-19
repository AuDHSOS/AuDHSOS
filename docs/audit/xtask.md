# xtask audit findings

Repository: AuDHSOS/AuDHSOS. Audit of xtask at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #417
Title: xtask: a byte raw string hides every `unsafe` and `global_asm!` that follows it from the counter
Labels: bug, part::tools
Body:
`strip_comments_and_strings` recognizes a raw string only when the `r` is not preceded by an identifier byte (`crates/tools/xtask/src/unsafe_budget.rs:96`, `crates/tools/xtask/src/unsafe_budget.rs:162-172`). `br"..."`, `br#"..."#` and `cr"..."` start with `b` or `c`, so the `r` is refused as a raw string start and the `"` that follows goes to `skip_string` (`crates/tools/xtask/src/unsafe_budget.rs:150-160`), which honors backslash escapes and stops at the first `"`. The doc comment at `crates/tools/xtask/src/unsafe_budget.rs:34` promises that string literals are ignored.

`fn f(p: *const u8) -> u8 { let _ = br"\"; unsafe { core::ptr::read(p) } }` compiles under edition 2024 and `count` reports zero `unsafe` sites for it: `skip_string` reads `\"` as an escaped quote and treats the rest of the file up to the next `"` as string content. `br#"a"b"#` desynchronizes the scanner the same way. Every `unsafe`, `asm!`, `naked_asm!` and `global_asm!` between that literal and the next `"` is uncounted, so `cargo xtask unsafe-budget` (R4) and the `global_asm!` scan of `check-layering` at `crates/tools/xtask/src/layering.rs:272-276` (R3) pass on a file that exceeds its budget or carries `global_asm!`.

Fix: in `strip_comments_and_strings`, treat a `b` or `c` immediately followed by `r"`, `r#` or `"` as the start of the literal and route `br`/`cr` literals through `skip_raw_string`; the option not taken, refusing files that hold `br"` in the check, would reject valid code the repository may need.

---

## F02 — issue #419
Title: xtask: check-layering reads only the host target's dependency graph, so a `[target.'cfg(...)'.dependencies]` table escapes the edge check
Labels: bug, part::tools
Body:
`cargo_tree` runs `cargo tree --workspace --all-features --edges ...` without `--target` (`crates/tools/xtask/src/layering.rs:39-57`). `cargo tree --help` states that dependencies are filtered to the host platform unless `--target all` is passed. `check_edges` therefore sees only the edges active on the development machine.

Adding to `crates/kernel/mm/Cargo.toml` the table `[target.'cfg(target_os = "none")'.dependencies]` with `audhsos-sync.workspace = true` gives `kernel-mm` an adapter dependency on the kernel target. On the host the cfg is false, `cargo tree` omits the edge, `check_edges` reports nothing, and `cargo xtask build` links it into the kernel. `check-deps` accepts the line at `crates/tools/xtask/src/deps.rs:83`, because the key ends with `.workspace`. R5 in `docs/04-safety-policy.md:55` names the layering rule as its enforcement.

Fix: pass `--target all` in `cargo_tree`, or run the tree once per triple of `Target::CROSS` in addition to the host; the option not taken, forbidding `[target.*]` tables in `check-deps`, would not cover a `cfg` on a normal table.

---

## F03 — issue #421
Title: xtask: check-deps accepts a path dependency that points outside the workspace
Labels: bug, part::tools
Body:
`manifest_violations` accepts any inline table that contains `path =` and none of the six markers of `mentions_external` (`crates/tools/xtask/src/deps.rs:83-85`, `crates/tools/xtask/src/deps.rs:95-106`). The path itself is not compared with the workspace members. `lock_violations` reports only packages with a `source` line (`crates/tools/xtask/src/deps.rs:32-43`), and a path dependency has no `source` in `Cargo.lock` (every member block of `Cargo.lock` lacks one; `grep -c '^source' Cargo.lock` is 0).

`foo = { path = "/opt/vendored/foo" }` in any manifest passes `check-deps` and `Cargo.lock` stays free of `source` lines. `check-layering` reports the edge as outside the workspace on the host graph only; combined with the target table of F02 the vendored crate reaches the kernel build with no check reporting it. R8 in `docs/04-safety-policy.md:58` names `check-deps` as the enforcement of "no external code".

Fix: in `manifest_violations`, extract the `path` value, resolve it against the manifest's directory and require the result to be a workspace member path of `CRATES` (or, for `fuzz/` and `tools/tls-probe/`, a path under the repository root); the option not taken, listing every package of `Cargo.lock` against `CRATES`, would not cover the two other lock files (F05).

---

## F04 — issue #422
Title: xtask: check-deps skips a dependency table whose header is dotted or carries a comment
Labels: bug, part::tools
Body:
`is_dependency_section` strips the brackets from the whole header line and compares the last dot-separated segment with the three table names (`crates/tools/xtask/src/deps.rs:57-64`). TOML allows `[dependencies.rand]` as a dotted table and `[dependencies] # note` as a header with a trailing comment. For the first the last segment is `rand`; for the second the string does not end in `]`, so `trim_matches` leaves `dependencies] # note`. In both cases `in_dependencies` stays `false` and every line of the table is skipped (`crates/tools/xtask/src/deps.rs:71-74`).

A manifest holding `[dependencies.rand]` followed by `version = "0.8"` reports no violation from `manifest_violations`. In the root workspace `lock_violations` still catches the `source` line once `Cargo.lock` is regenerated; in `fuzz/` and `tools/tls-probe/` nothing does (F05).

Fix: cut the header at the first `#` outside quotes, then accept a name whose first segment, or whose segment after `target.<...>`, is one of the three table names, and treat `[dependencies.<name>]` as a dependency entry of its own; the option not taken, a full TOML parser, is more code than the check needs.

---

## F05 — issue #423
Title: xtask: check-deps reads only the root `Cargo.lock`; `fuzz/Cargo.lock` and `tools/tls-probe/Cargo.lock` are never checked
Labels: bug, part::tools
Body:
`deps::check` reads `root.join("Cargo.lock")` once (`crates/tools/xtask/src/deps.rs:13-14`) and walks the tree only for files named `Cargo.toml` (`crates/tools/xtask/src/deps.rs:15-27`). The repository holds two more workspaces with their own lock files, `fuzz/Cargo.lock` and `tools/tls-probe/Cargo.lock`, and neither `check-layering` nor `lint` runs `cargo tree` or clippy in them (`crates/tools/xtask/src/layering.rs:39-57`, `crates/tools/xtask/src/commands.rs:26-57`).

`fuzz/der/Cargo.toml` gaining `[dependencies.rand]` with `version = "0.8"` passes `manifest_violations` (F04); `fuzz/Cargo.lock` records the registry source and nobody reads it; `cargo xtask fuzz --regression` builds and links the crate. `cargo xtask check` exits 0 with external code compiled into a fuzz target. R8 in `docs/04-safety-policy.md:58` says `Cargo.lock` lists only workspace members.

Fix: in `deps::check`, walk for every file named `Cargo.lock` under the root with the same `EXCLUDED_DIRECTORIES` and apply `lock_violations` to each, prefixing the violation with the relative path; the option not taken, adding the two workspaces to the root workspace, is refused by `fuzz/Cargo.toml:4-7` and `tools/tls-probe/Cargo.toml:4-6` for the sanitizer flags and `std`.

---

## F06 — issue #424
Title: xtask: the fuzz targets are outside every safety check, and 23 of the 33 lack `#![forbid(unsafe_code)]`
Labels: bug, part::tools
Body:
The fuzz targets under `fuzz/` are not entries of `CRATES` (`crates/tools/xtask/src/policy.rs:69-1083`), so `check_crate_roots` (`crates/tools/xtask/src/layering.rs:213-233`) and `unsafe_budget::check` (`crates/tools/xtask/src/unsafe_budget.rs:246-262`) skip them. `lint` runs clippy on the root workspace only (`crates/tools/xtask/src/commands.rs:31-53`). The fuzz workspace's own lint table sets only `unexpected_cfgs` (`fuzz/Cargo.toml:87-89`); `unsafe_code` is not denied there. `grep -L 'forbid(unsafe_code)' fuzz/*/src/main.rs` lists 23 files, among them `fuzz/der/src/main.rs`, `fuzz/elf/src/main.rs` and `fuzz/tls_handshake/src/main.rs`; the other 10 carry the attribute by hand.

An `unsafe` block, an `unwrap`, or an `asm!` in `fuzz/der/src/main.rs` compiles, runs in `cargo xtask fuzz --regression` inside `cargo xtask check`, and no step reports it. `docs/04-safety-policy.md:44` states that no crate outside the allowlist may contain `unsafe`, and `crates/tools/xtask/src/policy.rs:969-971` states that every fuzz target is safe code.

Fix: add `unsafe_code = "forbid"` to `[workspace.lints.rust]` in `fuzz/Cargo.toml`, and make `check_crate_roots` and `unsafe_budget::check` iterate the members of `fuzz/Cargo.toml` (read through `workspace_members`) as `Kind::Host` crates with budget zero; the option not taken, adding 33 entries to `CRATES`, duplicates `FUZZ_TARGETS`.

---

## F07 — issue #425
Title: xtask: check-layering checks one crate root per package, not every crate root, so `examples/`, `tests/` and the binaries beside a library are not read
Labels: bug, part::tools
Body:
`crate_roots` returns only `src/lib.rs` when it exists, only `src/main.rs` otherwise, and `src/bin/*.rs` only for a package with neither (`crates/tools/xtask/src/layering.rs:238-258`). A package's `examples/*.rs`, `tests/*.rs`, `benches/*.rs` and the `src/bin/*.rs` next to a `lib.rs` are separate crate roots with their own attribute set. R1 in `docs/04-safety-policy.md:51` states that `check-layering` reads every crate root.

`crates/text-core/examples/unicode_gen.rs` is the crate root of a Logic crate's example and holds no `#![forbid(unsafe_code)]` (`grep -c forbid` is 0); `check-layering` passes. `crates/user/programs/src/bin/` (15 roots) and `crates/user/net-programs/src/bin/` (4 roots) are never read either. The workspace lint `unsafe_code = "deny"` (`Cargo.toml`, `[workspace.lints.rust]`) is a `-D` flag and a crate root's `#![allow(unsafe_code)]` overrides it; only `unsafe_budget::check`, which counts every `.rs` file below the crate path, still catches an actual `unsafe` in such a root, and F01 defeats that count.

Fix: make `crate_roots` collect `src/lib.rs`, `src/main.rs`, every `src/bin/**/*.rs` file, every direct `.rs` file of `examples/`, `tests/` and `benches/`, and every `path =` value of `[[bin]]`, `[[example]]`, `[[test]]` and `[[bench]]` in the manifest; the option not taken, changing the workspace lint to `forbid`, is refused by the adapter crates that need `#![allow(unsafe_code)]`.

---

## F08 — issue #426
Title: xtask: one non-UTF-8 byte on the serial line ends the session's reader thread, and every later wait times out
Labels: bug, part::tools
Body:
`Session::start` reads QEMU's stdout with `BufReader::lines()` and returns from the thread on the first `Err` (`crates/tools/xtask/src/session.rs:60-63`). `Lines` yields `Err(InvalidData)` for a line that is not valid UTF-8. After the return the channel is disconnected and `wait_for` answers `false` at once (`crates/tools/xtask/src/session.rs:99`); `output()` never receives another line.

A guest program that writes one byte of 0x80..0xFF that is not part of a UTF-8 sequence on the console, for example a program echoing what was typed at it after a 2 KiB message boundary cut a multibyte character, silences the run: every following `wait_for` of the end-to-end run reports "did not start" or "never said", the serial dump printed on failure ends at the offending line, and the reason is nowhere in the report.

Fix: read the pipe with `BufRead::read_until(b'\n')` into a `Vec<u8>` and convert each line with `String::from_utf8_lossy`, so a bad byte becomes U+FFFD in the transcript; the option not taken, ending the thread with a note, still loses the rest of the run.

---

## F09 — issue #427
Title: xtask: `run_captured` discards the whole serial transcript when any byte of it is not UTF-8
Labels: bug, part::tools
Body:
`read_all` reads the pipe with `read_to_string` (`crates/tools/xtask/src/qemu.rs:576-582`). `Read::read_to_string` returns an error and leaves the buffer at its original length when the data is not valid UTF-8, so `text` stays empty. The doc comment says a pipe that cannot be read yields what came through before the failure, which is not what `read_to_string` does.

A test kernel that writes one invalid byte at any point of a run produces `Run { output: "" }`: `qemu::parse` finds no lines, `check` reports "the machine wrote no summary line" and the serial output printed by `report_tests` (`crates/tools/xtask/src/commands.rs:2376-2378`) is empty, so the failing test and its message are unrecoverable from the log.

Fix: read into a `Vec<u8>` with `read_to_end` and return `String::from_utf8_lossy(&bytes).into_owned()`; the option not taken, keeping `read_to_string`, cannot return the partial data.

---

## F10 — issue #428
Title: xtask: `fuzz --target <unknown>` exits 0 without running anything
Labels: bug, part::tools
Body:
`fuzz` filters `FUZZ_TARGETS` by the `--target` value and, when the filter leaves nothing, prints "no fuzz targets are registered yet" and returns `Ok(())` (`crates/tools/xtask/src/commands.rs:2762-2769`). The message describes an empty `FUZZ_TARGETS`, which holds 33 entries (`crates/tools/xtask/src/policy.rs:1220-1278`).

`cargo xtask fuzz --target tls_reocrd --time 600` (a typo) reports "nothing to run" and exits 0; a script or a person that reads the exit status takes ten minutes of fuzzing as done. `--regression --target <typo>` in the same way reports a passed replay of nothing.

Fix: when `selected` is `Some(name)` and no target matches, return `Error::Usage` naming the target and the known names; the option not taken, keeping the note, hides a typo behind a success status.

---

## F11 — issue #429
Title: xtask: the crate table of `docs/05-code-organization.md` lists nine of the sixteen dependencies of `xtask`
Labels: bug, part::tools
Body:
`docs/05-code-organization.md:221` names the dependencies of `xtask` as `audhsos-abi`, `kernel-test-harness`, `audhsos-encoding`, `crypto-hash`, `audhsos-symbols`, `audhsos-time`, `fs-fat`, `fs-gpt` and `user-loader`. `crates/tools/xtask/Cargo.toml:14-30` and `crates/tools/xtask/src/policy.rs:1062-1079` add `app-canvas`, `audhsos-tls`, `audhsos-x509`, `crypto-rng`, `driver-i8042`, `gfx` and `server-display`.

A reader of the table concludes that `xtask` links no TLS or X.509 code and no user server, which the TLS run of `crates/tools/xtask/src/tls.rs:25-32` and the sprite check of `crates/tools/xtask/src/commands.rs:1651-1675` contradict.

Fix: extend the row with the seven missing crates and the reason each is there (the TLS acceptance run, D-149; the canvas and the cursor sprite of the end-to-end run; the key codes of the input run); the option not taken, generating the table from `policy.rs`, is a larger change to the documents.

---

## F12 — issue #430
Title: xtask: the SPDX check does not read linker scripts
Labels: bug, part::tools
Body:
`HEADER_FILE_TYPES` names `rs`, `toml`, `yml`, `yaml` and `sh` (`crates/tools/xtask/src/policy.rs:1185-1191`); `comment_prefix` skips every other extension (`crates/tools/xtask/src/spdx.rs:43-52`). The repository holds four `.ld` files, `crates/kernel/bin/kernel.ld`, `crates/user/programs/program.ld`, `crates/user/programs/root.ld` and `crates/user/test-programs/user.ld`, each beginning with the two header lines in `/* */` comments.

Removing the header from `crates/kernel/bin/kernel.ld` passes `cargo xtask lint`; the four scripts carry the header by convention only.

Fix: add `("ld", "/*")` to `HEADER_FILE_TYPES` and let `header_problem` accept a line that ends in ` */` for that prefix; the option not taken, leaving the scripts unchecked, is a check that covers every source type but one.

---

## F13 — issue #431
Title: xtask: `Session::finish` can drop the last lines the machine wrote
Labels: bug, part::tools
Body:
`finish` kills and waits for the child and then calls `output()`, which drains the channel with `try_recv` (`crates/tools/xtask/src/session.rs:219-225`, `crates/tools/xtask/src/session.rs:211-217`). The reader thread of `crates/tools/xtask/src/session.rs:56-68` is not joined; it may still be splitting the bytes it read before the pipe closed when `try_recv` returns `Empty`.

The serial dump printed on a failed end-to-end run (`crates/tools/xtask/src/commands.rs:573-577`) omits the lines the reader thread had not sent at that instant, which are the last lines before the kill: the ones that say why the run stopped.

Fix: keep the `JoinHandle` in `Session`, join it in `finish` after `child.wait()` and before `output()`; the option not taken, a sleep before the drain, is a guess at a time.

---

## F14 — issue #432
Title: xtask: a comment inside the `members` array makes check-layering report a member that does not exist
Labels: bug, part::tools
Body:
`workspace_members` takes the text between the first `[` after `members` and the first `]`, splits it at commas and strips quotes (`crates/tools/xtask/src/layering.rs:166-191`). A commented line such as `# "crates/old",` or a trailing `# note` becomes an item that begins with `#`.

Commenting out one member in `Cargo.toml` makes `check_crate_list` (`crates/tools/xtask/src/layering.rs:136-142`) report the violation `workspace member `# "crates/old"` is not in the policy table` and `cargo xtask check` fails on a manifest Cargo accepts.

Fix: drop the part of every line after `#` before splitting; the option not taken, forbidding comments in the array, is a rule Cargo does not have.

---

## F15 — issue #433
Title: xtask: `disk::build` retries every error, not only a full volume, up to a 4 GiB image
Labels: enhancement, part::tools
Body:
`build` grows the image by one mebibyte on any `Err` until 4 GiB (`crates/tools/xtask/src/image/disk.rs:45-51`). `try_build` and `fat32::write` also fail for a name that is not 8.3, a path that is in the image twice, and a component that is a file where a directory is needed (`crates/tools/xtask/src/image/fat32.rs:129-131`, `crates/tools/xtask/src/image/fat32.rs:184-192`).

Today every caller passes the constant paths of `crates/tools/xtask/src/image/disk.rs:20-27`, so only `Full` occurs. A caller that adds a file with a name that is not 8.3 gets about 4000 rounds of `vec![0u8; size]`, a format and a write, up to 4 GiB each, before the name error is reported, which is O(n²) in bytes over the growth.

Fix: return at once on every error other than the one `no_room` maps from `fs_fat::Error::Full` (give it its own variant), and grow only on that; the option not taken, computing the exact size up front, duplicates the geometry search of `fs-fat`.

---

## F16 — issue #434
Title: xtask: the serial readers hold every byte the guest writes with no cap
Labels: enhancement, part::tools
Body:
`read_all` reads the pipe to its end into one `String` (`crates/tools/xtask/src/qemu.rs:576-582`), and the session's reader accumulates a line until `\n` (`crates/tools/xtask/src/session.rs:60`). Neither has a byte limit; the only bound is the run's time limit (`crates/tools/xtask/src/qemu.rs:421-438`, `E2E_TIMEOUT` per wait in `crates/tools/xtask/src/commands.rs:179`).

A test kernel in a print loop without a newline, or one that writes at full speed for the 60 seconds of `DEFAULT_TIMEOUT`, makes the runner allocate whatever the serial line carries in that time before it is killed and reported as a crash. The bound holds today because the guests are this repository's own images.

Fix: cap what either reader keeps at a fixed number of bytes (a few megabytes), drop the rest and note the truncation in the transcript; the option not taken, a shorter time limit, changes what a slow machine passes.

---

## F17 — issue #435
Title: xtask: a comma in the checkout path breaks the `-drive file=` arguments of QEMU
Labels: enhancement, part::tools
Body:
`arguments` writes the firmware, the boot image and the scratch disk as `file={}` with the path unescaped (`crates/tools/xtask/src/qemu.rs:467-472`, `crates/tools/xtask/src/qemu.rs:536-538`). QEMU splits a `-drive` option at commas and requires a comma inside a value to be doubled.

A checkout under a directory whose name holds a comma makes `qemu-system-x86_64` refuse the drive and every `test --qemu`, `test --e2e` and `run` fails with a QEMU option error. `runner_command` refuses a path with a space for the same class of reason (`crates/tools/xtask/src/commands.rs:2005-2016`).

Fix: replace `,` with `,,` in every path placed into a QEMU option value, in one helper used by the three sites; the option not taken, refusing such a path as `runner_command` does, blocks a run that can work.

---

## F18 — issue #436
Title: xtask: `coverage::evaluate` tests a condition that is always true
Labels: enhancement, part::tools
Body:
`evaluate` iterates `CRATES` and, for each entry, gates the threshold check on `find(krate.name).is_some()` (`crates/tools/xtask/src/coverage.rs:202`). `find` searches `CRATES` for that same name (`crates/tools/xtask/src/policy.rs:1331-1333`), so the condition holds for every iteration and adds one O(|CRATES|) scan per crate.

A reader takes the condition for a case that exists (a crate with coverage data but no policy entry), which the loop over `CRATES` excludes.

Fix: drop `&& find(krate.name).is_some()`; the option not taken, iterating `totals` instead of `CRATES`, loses the "has no coverage data" violation.
