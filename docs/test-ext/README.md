# External conformance test inputs

`test262/` is an independent upstream checkout, not a build or runtime dependency.
Its files must remain verbatim under their original copyright and license notices;
the project's AGPL SPDX-header requirement does not apply inside this exact path.
The header check still applies to project-owned runners and neighboring files.

Source: <https://github.com/tc39/test262>
Inspected revision: `419d3e0a2273ba01a3bfcbec423f2801425b8e93`.
Read `test262/INTERPRETING.md` for execution rules and `test262/LICENSE` for
upstream license terms. Tests must not be rewritten to fit jrs. Complete passing
Test262 results, in addition to WPT, are required for jrs acceptance; current
smoke probes do not establish that result. Strict/non-strict variants, negative
phases/types, async completion, module fixtures and `$262` capabilities matter.

Run the current dependency-free diagnostic runner via:

```sh
sh tools/xtask.sh jrs --fuel 1000000 --test262 docs/test-ext/test262 --all --summary
sh tools/xtask.sh jrs --test262 docs/test-ext/test262 test/built-ins/JSON
```

The first command selects the whole checkout, including staging and Intl. The
second reports every strict/non-strict variant of the selection. Nonzero exit
means failed or unsupported cases; unsupported module/host operations and
unverified parse-negative rejections never count as passing. Full acceptance is
still absent. The runner does not invoke npm, Python, Node or an external parser.

The host now implements `$262.evalScript` as same-realm global Script execution,
alongside `$262.gc` and `$262.global`. Nested Scripts defer job checkpoints until
the outer host turn completes. createRealm, agents, ArrayBuffer detachment and
module facilities remain explicit unsupported operations. ECMAScript eval is
distinct and is not supplied by this test-host callback.
