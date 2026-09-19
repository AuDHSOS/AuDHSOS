# audhsos-regex, audhsos-regex-bt, audhsos-utf16 audit findings

Repository: AuDHSOS/AuDHSOS. Audit of audhsos-regex, audhsos-regex-bt, audhsos-utf16 at main bc47df5.
Each `## ` section is one issue, filed 2026-09-19 with the number in its heading. `Title:` is the issue title, `Labels:` the labels, everything after `Body:` is the issue body verbatim.

---

## F01 — issue #405
Title: audhsos-regex: the per-position reset of the seen set costs O(states) and is not charged to the work budget
Labels: bug
Body:
`Search::run` clears the whole `seen` vector with `self.seen.fill(false)` once per input position at `crates/regex/src/matcher.rs:133`. The vector has one entry per compiled instruction (`crates/regex/src/matcher.rs:98`), so the reset costs O(m) per position and O(m × n) per search. `charge` is called for thread pops, register copies and class lookups (`crates/regex/src/matcher.rs:125`, `crates/regex/src/matcher.rs:206`, `crates/regex/src/matcher.rs:220`) and never for the reset, so `Report::work` and the `limits.work` check (`crates/regex/src/matcher.rs:109-118`) omit the dominant cost of a large pattern on a long non-matching input. The doc comment of `Limits::work` says the field counts "work units, including state visits and capture-register copying" (`crates/regex/src/lib.rs:51-52`) and jrs passes its remaining fuel as that budget (`crates/jrs/src/vm/regexp.rs:154-157`).

Pattern `a{10000}b{10000}c{10000}d{10000}e{10000}f{10000}` compiles to 60003 states under `Limits::default()`. `find` on 16,000,000 units of `x` returns `Ok` with `work` 64,000,004 (below the 100,000,000 budget) after 22 s on the host; pattern `a` on the same input reports the same `work` and finishes in 0.7 s. The uncharged part is 60003 × 16,000,001 byte writes, about 10^12, thirty times the charged work. A script that constructs such a pattern and string holds the interpreter for that time inside one `exec` call without exhausting fuel.

Fix: replace `seen: Vec<bool>` with a `Vec<usize>` of position stamps, mark a state with `position + 1` and test equality instead of `true`, which removes the reset entirely; charging `code.len()` per position instead would keep the reset and make the budget reject a 65,536-state pattern after 1,500 positions.

---

## F02 — issue #406
Title: audhsos-regex-bt: a lookahead predicate charges the range count instead of the binary-search bound
Labels: bug
Body:
`Instruction::Predicate` charges `class.ranges.len() + 1` work units per evaluation at `crates/regex-bt/src/matcher.rs:242`. `Class::contains` runs a binary search over the sorted ranges (`crates/regex/src/parse.rs:23-30`), O(log r). `Instruction::Class` in the same VM charges the bit width of the range count at `crates/regex-bt/src/matcher.rs:217-223`, and the NFA charges the same bound for its `Look` instruction at `crates/regex/src/matcher.rs:252-258`.

A class with 8192 disjoint single-unit ranges (for example `\u0000\u0002\u0004…` inside brackets, 49,152 pattern units, below the `ranges` limit of 16,384 and the `pattern_units` limit of 65,536) compiled as `(?=[…])x` charges 8193 units per start position. On a non-matching input the budget of 100,000,000 is exhausted after 12,206 positions and `find` returns `Error::Limit { resource: "backtracking work" }` for an input of 12,207 units; the same pattern and input on the NFA charges 15 units per position.

Fix: charge `usize::BITS - class.ranges.len().leading_zeros()` plus one at `crates/regex-bt/src/matcher.rs:242`, the expression used at `crates/regex-bt/src/matcher.rs:217-223`.

---

## F03 — issue #407
Title: audhsos-regex: the parser accepts `{` and `}` as literals but rejects `]`, `a{1` and `a{1,x}`, which matches neither 22.2.1 nor B.1.2
Labels: bug
Body:
The module doc names ECMA-262 22.2.1 as the grammar (`crates/regex/src/parse.rs:4`). `atom` turns a `{` that is not followed by a digit and every `}` into `Expr::Unit` at `crates/regex/src/parse.rs:397-401`, while `docs/ecma/ecma262.html` 22.2.1 lists `{` and `}` in `SyntaxCharacter` and defines `PatternCharacter` as "SourceCharacter but not SyntaxCharacter". `atom` rejects `]` with `Error::Syntax` at `crates/regex/src/parse.rs:400`, `term` and `counted` reject `a{1` and `a{1,x}` with `Error::Syntax` at `crates/regex/src/parse.rs:279-286` and `crates/regex/src/parse.rs:327-329`, while B.1.2 defines `ExtendedPatternCharacter` as "SourceCharacter but not one of ^ $ \ . * + ? ( ) [ |", which admits `]`, `{` and `}` as literals when they do not form an `InvalidBracedQuantifier`. `lookahead` accepts `]` as a predicate at `crates/regex/src/parse.rs:473-476`, so `(?=])` compiles and `]` does not.

`Regex::compile` returns `Ok` for `a}`, `{`, `{a` and `a{,5}` and `Error::Syntax` for `a]`, `a{1` and `a{1,x}` under `Limits::default()`. jrs maps `Error::Syntax` to a JavaScript `SyntaxError` (`crates/jrs/src/regexp.rs:82`), so `new RegExp("a]")` throws where every B.1.2 engine returns a matcher for the literal `a]`, and `new RegExp("a}")` returns a matcher where a 22.2.1 engine throws.

Fix: implement B.1.2 for the non-Unicode mode the crate targets: treat `]` and `}` as literals in `atom`, and in `term` fall back to a literal `{` when `counted` finds no complete `{n}`, `{n,}` or `{n,m}`; the option not taken is 22.2.1 strictness, which rejects the `{` and `}` literals `atom` accepts today.

---

## F04 — issue #408
Title: audhsos-regex: a quantified lookahead returns `Error::Syntax` although B.1.2 admits it
Labels: bug
Body:
`term` returns `Error::Syntax { message: "cannot quantify an assertion" }` when the atom is `Expr::Look` or `Expr::Lookaround` at `crates/regex/src/parse.rs:292-297`. `docs/ecma/ecma262.html` B.1.2 has the production `Term :: [~UnicodeMode] QuantifiableAssertion Quantifier` with `QuantifiableAssertion :: (?= Disjunction ) | (?! Disjunction )`. The regex-bt README lists "quantified assertions" among the constructs that "remain unsupported, not approximated" (`crates/regex-bt/README.md:26-29`).

`Regex::compile` of `(?=a)*` returns `Error::Syntax { offset: 6, message: "cannot quantify an assertion" }` in both crates. jrs maps this variant to a JavaScript `SyntaxError` (`crates/jrs/src/regexp.rs:82`), so a script sees a syntax error for a pattern the specification accepts. The `Expr::Assert` case (`^*`, `\b+`) is a syntax error in both grammars and is reported correctly by the same line.

Fix: return `self.unsupported("quantified lookahead")` for `Expr::Look` and `Expr::Lookaround` at `crates/regex/src/parse.rs:292-297` and keep `self.syntax` for `Expr::Assert`.

---

## F05 — issue #409
Title: audhsos-regex: `(?` at the end of the pattern and `(?x)` return `Error::Unsupported`
Labels: bug
Body:
`atom` returns `Error::Unsupported { feature: "lookaround, named groups or inline modifiers" }` for every `(?` that is not followed by `=`, `!` or `:` at `crates/regex/src/parse.rs:371-375`, without looking at the following unit. `docs/ecma/ecma262.html` 22.2.1 and B.1.2 admit after `(?` only `=`, `!`, `<`, `:` and `RegularExpressionModifiers` (`i`, `m`, `s`, optionally with `-`) followed by `:`.

`Regex::compile` of `(?` returns `Error::Unsupported { offset: 2, .. }` and `(?x)` returns the same, while both are syntax errors in every profile of the specification. jrs maps `Error::Unsupported` to its host `Unsupported` error, not to a JavaScript `SyntaxError` (`crates/jrs/src/regexp.rs:83`), so a script cannot catch the malformed pattern as a `SyntaxError`.

Fix: at `crates/regex/src/parse.rs:371-375` return `self.unsupported(..)` only when the next unit is `<`, `i`, `m`, `s` or `-`, and `self.syntax("invalid group")` otherwise.

---

## F06 — issue #410
Title: audhsos-regex: the `Limits::depth` doc comment is off by one
Labels: bug
Body:
The doc comment of `Limits::depth` says "Maximum group nesting, additionally capped at 48" (`crates/regex/src/lib.rs:37`). `disjunction` increments `depth` for the top-level pattern as well as for every group at `crates/regex/src/parse.rs:235-240`, so a pattern with `k` nested groups needs `depth >= k + 1`.

`Regex::compile` of `(a)` with `Limits { depth: 1, ..Limits::default() }` returns `Error::Limit { resource: "pattern nesting" }` and succeeds with `depth: 2`; the test at `crates/regex/src/tests/mod.rs:274-281` encodes the off-by-one. Under the default of 48 the parser admits 47 nested groups, and the README of regex-bt states "the shared hard nesting cap of 48" (`crates/regex-bt/README.md:23`).

Fix: change the doc comment to "Maximum disjunction depth including the top level, capped at 48; a pattern with k nested groups needs k + 1"; changing the check to `self.depth > ...` would instead admit 48 nested groups and change the accepted set.

---

## F07 — issue #411
Title: audhsos-regex: `compile` accepts a pattern that `find` rejects under the same limits
Labels: enhancement
Body:
`Regex::find` computes `code.len() × registers × 6` and returns `Error::Limit { resource: "capture workspace" }` when the product exceeds `limits.capture_cells` at `crates/regex/src/matcher.rs:70-82`. `Regex::compile` bounds `states` (`crates/regex/src/compile.rs:75-79`) and `captures` (`crates/regex/src/parse.rs:378-382`) separately and never evaluates the product, although `code.len()` and `registers` are fixed at `crates/regex/src/compile.rs:50-55`.

The pattern of 64 groups `(a)` followed by `a{5300}` compiles under `Limits::default()` to 5495 states with 130 registers; 5495 × 130 × 6 = 4,286,100 exceeds the default `capture_cells` of 4,194,304, so every `find` with the default limits returns `Error::Limit`. A script that constructs such a `RegExp` gets the error on each `exec`, not from the constructor, and `Regex::compile` reports no limit that the caller could relate to the pattern.

Fix: evaluate the same product in `Regex::compile` after `crates/regex/src/compile.rs:49` against `limits.capture_cells` and return the limit error there; keeping the check only in `find` leaves the constructor-time error path of jrs unused for this limit.
