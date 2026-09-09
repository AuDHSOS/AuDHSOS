# Backtracking regular expressions

`audhsos-regex-bt` is an explicitly selected safe `no_std + alloc` backtracking
engine. It depends only on the internal `audhsos-regex` crate, reusing its bounded
syntax parser, normalized character classes, anchors, options, errors and match
representation. It does not change the Thompson matcher or cause an implicit
fallback. jrs integration is separate and is not enabled by adding this crate.

The iterative VM implements ordered alternatives, greedy/lazy repetition,
captures (including clearing each iteration), decimal backreferences, positive
and negative lookahead/lookbehind, and nullable quantifiers. Lookaround is atomic:
internal alternatives are discarded on success. Lookbehind traverses sequences
backwards, not through repeated forward guesses. Unmatched/forward references
match empty; empty optional iterations fail before the continuation, following
ECMA-262 `RepeatMatcher`. Required empty iterations remain possible.

This is **not a linear-time engine** and does not claim complete `ReDoS` prevention.
Adversarial patterns can require exponential work. Execution has one shared work
budget across all candidate starts, comparisons, capture copies and lookarounds.
Explicit choice and assertion stacks, input size, capture/register cells and
compiled states have independent quotas, checked before growth. Limit exhaustion
is a typed error, never a non-match. Matching never recurses on the Rust stack;
parser/compiler recursion has the shared hard nesting cap of 48. Allocation OOM
still requires an embedding allocator policy; limits count logical resources.

The current grammar is UTF-16 code-unit mode with multiline/dot-all options.
Named groups, Unicode/code-point mode, ignore-case folding, Unicode properties,
legacy octal/identity escapes and quantified assertions remain unsupported, not
approximated. This is a reusable core, not full ECMAScript `RegExp` conformance.
Source: local `docs/ecma/ecma262.html` §22.2.2 (`RepeatMatcher`, assertions,
`BackreferenceMatcher`). No external code or library is required.

Run `sh tools/xtask.sh regex-check --fix-format` for both independent engines,
or `sh tools/xtask.sh fuzz --target regex_bt --time 10` for the dedicated fuzz
target. Regular-subset results are compared to the Thompson engine; tests also
check backreferences, reverse captures, atomic assertions, empty iterations and
all execution limits. Never use this engine on untrusted inputs without quotas.

```rust
use audhsos_regex_bt::{Limits, Options, Regex};
let pattern: Vec<u16> = r"(a+)\1".encode_utf16().collect();
let input: Vec<u16> = "aaaa".encode_utf16().collect();
let limits = Limits::default();
let regex = Regex::compile(&pattern, Options::default(), limits)?;
let report = regex.find(&input, 0, false, limits)?;
let found = report.matched.expect("matching pair");
assert_eq!(found.range, 0..4);
assert_eq!(found.captures, vec![Some(0..2)]);
# Ok::<(), audhsos_regex_bt::Error>(())
```
