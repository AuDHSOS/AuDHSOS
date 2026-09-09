# External conformance suites

The test suites this system is measured against, kept beside the code so
that a claim about conformance can be checked without asking a server that
has moved on, and so that the suite cannot change under a number this
repository prints.

This is the arrangement of [`docs/rfc/`](../rfc/README.md) applied to
something that is not a document. What the other reference directories
hold is read; what this one holds is executed, as input, by a runner in
this workspace. The distinction matters for one rule and no other: rule R8
of the safety policy is about dependencies, and it is untouched. Nothing
here is compiled or linked, `Cargo.lock` still lists only workspace
members, no manifest names anything outside the workspace, and the runner
invokes no npm, Python, Node, or external parser. The files are data that
a test reads at test time.

## What is here

| Directory | Suite | Source | Revision | Retrieved |
|-----------|-------|--------|----------|-----------|
| `test262/` | *Test262*, the official ECMAScript conformance suite, Ecma International (TC39) | `https://github.com/tc39/test262`, branch `main` | `419d3e0a2273ba01a3bfcbec423f2801425b8e93`, committed 2026-09-02 | 2026-09-09 |

At that revision a full clone is 366 MB, of which 127 MB is its own `.git`;
what `cargo xtask test-ext` fetches is that revision without the history,
264 MB with a `.git` of 26 MB. `test/` holds 53876 JavaScript files: 24009 under `language/`, 23817 under
`built-ins/`, 3357 under `intl402/`, 1491 under `staging/`, 1086 under
`annexB/`, and 116 under `test/harness/`, which are the tests of the
harness itself. 313 of the 53876 are `_FIXTURE` files, which are inputs a
test loads and not tests. The 34 files of the top-level `harness/`
directory are the prelude a test includes by name.

## Why there is no checksum, and how to obtain it

The other reference directories record a checksum per file, because each
holds one document that was fetched once and does not move. This holds a
git repository of some fifty thousand files whose upstream branch advances
several times a day, and the thing that pins it is the revision above: a
commit hash is the checksum of a tree, and git checks it on checkout.

The checkout is not part of this repository. It is 366 MB with a `.git` of
its own, the outer repository does not track it, and it is not needed to
build, test, or check this system — only to measure it. To obtain it:

```sh
sh tools/xtask.sh test-ext
sh tools/xtask.sh test-ext --status
```

The first brings every suite named in the table to its pinned revision:
it creates the directory, fetches that one revision, checks it out
detached, and reads back what landed on the disk rather than reporting what
it asked for. It is idempotent — a checkout already at the revision costs
one `git rev-parse` and no network — and it refuses rather than discards: a
directory that is not a checkout is left alone, and a checkout with
uncommitted changes in it is reported instead of reset, because a test that
somebody edited is evidence and not litter. A checkout it creates itself
gets the revision without the history; a checkout that is already there
keeps whatever history it has, since `--depth` against a full clone would
truncate it. The second form only reports, reaches no network, and exits
nonzero when a suite is missing or stands somewhere else.

By hand the same thing is

```sh
git clone https://github.com/tc39/test262 docs/test-ext/test262
git -C docs/test-ext/test262 checkout 419d3e0a2273ba01a3bfcbec423f2801425b8e93
```

which downloads the history as well.

`cargo xtask test-ext` is the only subcommand that uses the network, and it
is therefore the only one that is never a step of `cargo xtask check`. A
check runs offline; a suite that is absent means a measurement that did not
happen, which is not the same as a check that failed.

The revision lives in four places that must agree: the table
`EXTERNAL_SUITES` in `crates/tools/xtask/src/policy.rs`, which is what the
subcommand checks out; here; [document 6](../06-testing-strategy.md)
section 6.7; and the README of the crate `jrs`. Moving it is a change to
all four and to the numbers that were measured under the old one, not a
silent `git pull`.

## What the revision is a snapshot of

Test262 has no editions and no releases. It is a single branch that grows
with the specification and with the bugs people find in implementations,
so there is no version to name — only a moment. This one is the tip of
`main` on the day it was cloned. `git describe` finds a tag on it, but the
tag is a generated web-features manifest and not a release; it names the
same hash and says nothing more than the hash does.

The suite is larger than what the specification alone requires. `intl402/`
tests ECMA-402, `annexB/` tests the web-compatibility annex, and
`staging/` holds tests that have not yet been reviewed into the suite
proper. Which of those a run selects is a decision of the run, and the
`--all` selection below takes every one of them.

## Terms

Test262 is not covered by this repository's licence. It carries the BSD
Licence, © 2012 Ecma International, printed in full in `test262/LICENSE`,
which permits redistribution provided the notice, the conditions, and the
disclaimer travel with the copy. The files are therefore kept exactly as
upstream wrote them, with their own headers intact.

That is why the SPDX check makes one exception. D-25 requires the header
`AGPL-3.0-only` and the project's copyright line in every file; adding it
to a file here would both modify a verbatim copy and assert a copyright
over someone else's work. `crates/tools/xtask/src/spdx.rs` exempts the
path `docs/test-ext/test262` and nothing else. The exemption is
root-relative rather than a filename pattern, so it cannot be inherited by
a file that merely sits in a directory of that name elsewhere, and the
project's own runners and every neighbouring file still need the header.

A test is also not to be rewritten to fit `jrs`. A test that fails is a
result; a test that was edited until it passed is not.

## Who runs it

The crate `jrs` has a Test262 runner, `crates/tools/jrs/src/test262.rs`,
which reads the suite in place:

```sh
sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 --all --summary
sh tools/xtask.sh jrs --test262 docs/test-ext/test262 test/built-ins/JSON
```

The first selects the whole checkout, `staging/` and `intl402/` included,
and prints a summary. The second selects a subtree and reports every
strict and non-strict variant of it. A nonzero exit means something failed
or was unsupported. Frontmatter is read by a bounded parser written for
this purpose, not by a YAML dependency, and the runner honours the
execution rules of `test262/INTERPRETING.md`: the harness files a test
includes, its flags and variants, the phase and type of a negative test,
async completion, and the `$262` host object.

An unsupported host operation is not a passing test, and neither is a
parse-negative case whose rejection was not verified. `$262.evalScript`,
`$262.gc`, and `$262.global` are implemented; `createRealm`, agents,
`ArrayBuffer` detachment, and the module facilities are explicit
unsupported operations, and modules count as unsupported wherever they
appear.

## What this does not yet establish

Full acceptance of `jrs` requires the whole suite to pass under those
rules, together with WPT. That has not happened. What exists is a
diagnostic runner and a set of focused selections whose pass counts the
`jrs` README records directory by directory; those numbers measure
progress and are not a conformance claim. The RegExp engine is a finite
automaton by choice, so the full backreference tests state a known and
deliberate incompatibility rather than a defect to be fixed.
