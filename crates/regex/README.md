# Regular expression core

Dedicated crate for the project's reusable regular expression engine,
`audhsos-regex`: a safe `no_std` + `alloc` Thompson NFA. This component must not live
inside `crates/support/`, `crates/encoding/`, or the JavaScript parser/VM.
The `jrs` adapter translates ECMAScript syntax, flags and string semantics
at its boundary; the matching core is independently auditable and reusable.

## Binding requirements

- Implement a Thompson NFA or DFA, in safe Rust, with no external dependencies.
  The external Rust `regex` crate is a design comparison, not a dependency
  or source-code donor.
- No backtracking matcher, including a hidden fallback for unsupported syntax.
- Bound compilation independently of matching: source length, parse depth,
  repetition expansion, compiled states and character-class storage.
- For an NFA, process each reachable state at most once per input position,
  including epsilon closure. With `m` compiled states and `n` input units,
  the matching bound is O(m × n), linear in text length for a fixed pattern.
  Searching must not restart a complete matcher at each possible start offset.
- If DFA caching is added, bound cache growth and define its exhaustion
  behaviour. Do not unconditionally materialize exponentially many subsets.
- Captures, ordered alternatives and greedy/lazy priority must have explicit
  bounds too; a linear Boolean recognizer is not sufficient evidence for a
  capture-producing matcher.
- Unsupported constructs must return typed errors, not silently change
  semantics. General backreferences are not supported by finite automata;
  they must not trigger a backtracking fallback in this crate. The separate,
  explicitly authorized `crates/regex-bt` engine supports backtracking without
  weakening the algorithm contract here. jrs still uses only this automaton;
  switching its engine is a separate integration step.
- Register dedicated parser/compiler/matcher fuzzing through the repository's
  `crates/support/fuzz` engine, including epsilon cycles, nested quantifiers,
  empty matches, malformed UTF encodings, oversized repetitions, pattern/input
  boundaries and regression cases for any discovered failures.
- Test operation-count bounds rather than wall-clock time alone. Include
  adversarial patterns such as `(a+)+$` and `(a|aa)*b` against increasing input
  lengths; successful termination is not by itself a complexity proof.

These requirements prevent exponential backtracking `ReDoS`. They do not remove
the need for resource quotas, cancellation, bounded output and memory-failure
handling: a large pattern or input can still exhaust finite resources.

Other reusable non-`RegExp` components belong in appropriately named crates
under `crates/`, with workspace layering, documentation, tests and fuzzing as
required by the existing project policy.

## Implemented subset and bounds

UTF-16 code-unit literals and escapes, dot, classes/ranges and negation,
ASCII word/digit classes and ECMAScript whitespace, alternation, groups,
captures, greedy/lazy repetition, counted repetition, anchors and word
boundaries. Positive/negative lookahead of a single literal or character class
is a zero-width NFA predicate at the current offset, including end-of-input;
it performs no secondary search. Options: multiline and dot-all. Unicode/code-point mode,
case-folding, general lookaround, named groups, backreferences, and quantification
of nullable expressions are explicitly unsupported. The last restriction
avoids claiming ECMAScript empty-iteration capture semantics prematurely.

Prioritized epsilon closure is iterative. At each input position, every
instruction is visited at most once across *all* active starts. A candidate
match discards lower-priority threads while higher-priority threads continue.
No matching cursor moves backwards. `Report::state_visits` is bounded by
`states × (input_units + 1)`; tests assert it for adversarial patterns.

Captures use bounded per-thread vectors, not an unbounded history. With `g`
capture registers, worst-case work is O(m × n × (g + log r)), where `r` is
the maximum number of class ranges; memory is O(m × g + r). All are linear
in text length for a fixed compiled pattern. Six state/register matrices
are conservatively charged before matching. Allocation failure still needs
an embedding memory quota; logical resource errors do not catch allocator OOM.

`sh tools/xtask.sh regex-check` runs focused tests, strict Clippy and the
bare-metal cross-check. This crate does not borrow code from another engine.

The core is fuzzed independently with
`sh tools/xtask.sh fuzz --target regex_nfa --time 10`. Its target checks both
byte-derived and arbitrary UTF-16 patterns/inputs, invalid-program errors,
capture-range validity and the unique state-visit bound. Integration into
`jrs` has a separate `jrs_source` fuzz target.

## Shared syntax

The public `syntax` module exposes normalized classes, anchors, the bounded AST
and parser profiles. `syntax::parse` retains the original automaton restrictions.
`syntax::parse_with_profile(..., Profile::Backtracking)` is an explicit frontend
for `regex-bt`, adding decimal references, general lookaround and nullable repeats.
It does not run a matcher or change `Regex::compile`. No dependency points from
the automaton crate to regex-bt. Match/Options/Error types are reused by both.
