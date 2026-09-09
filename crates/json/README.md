# JSON core

Independent safe `no_std + alloc` JSON grammar and UTF-16 string quoting.
No external dependencies and no JavaScript parser fallback. Numbers retain
their source range and convert to binary64 through the Rust toolchain. Strings
preserve lone UTF-16 surrogates; quoting escapes lone surrogates but keeps pairs.

The parse result is a flat postorder arena, so destruction does not recursively
walk attacker-controlled nesting. Containers hold child indices and object
members retain source order and duplicates; the embedding applies its own
duplicate-key policy. Every node retains its exact source range for revivers.
Parsing is O(n) in input units with bounded nesting (hard cap 48), node count,
input length and total decoded string storage. A caller-supplied work counter
bounds parsing and quoting. Quoting appends to a bounded output buffer. Logical
limits do not catch allocator OOM; the embedding still needs a memory quota.

This crate does not implement JavaScript object hooks or JSON.stringify's
property traversal. Those live in jrs. The xtask QMP-specific integer-only JSON
codec is separate and is not used as the language implementation.

References: local ECMA-262 §25.5 (including strict `ParseJSON`, source records and
`QuoteJSONString`); the grammar is the JSON subset described there, not JavaScript
object literals. Comments, trailing commas, non-JSON whitespace, unquoted names,
leading-zero numbers and non-finite number tokens are rejected.

Unit tests quote/parse every individual UTF-16 code unit and check strict grammar,
source spans and independent quota boundaries. `fuzz/json_codec` drives arbitrary
UTF-16 and byte-expanded input, checks arena/source invariants, requotes strings
and exercises tiny budgets through the project's own `support/fuzz` engine.
