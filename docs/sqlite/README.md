# Reference documents: SQLite

The documents the port of [document 16](../16-sqlite-in-rust.md) is
written against. A structure of a file format is a claim about bytes, and a
claim without the sentence it came from is one nobody can check.

Nothing here is compiled, linked, or read at run time. These are
documents. Rule R8 of the safety policy is about dependencies and is
untouched: `Cargo.lock` still lists only workspace members. The source of
SQLite itself is not here — it is a checkout under `research/`, which the
checks never descend into, and this directory is the specification rather
than the implementation.

## What is here

| File | Document | Retrieved | Bytes | SHA-256 |
|------|----------|-----------|-------|---------|
| `fileformat2.html` | *Database File Format* | 2026-09-13 | 110473 | `905067afacc30583c9ee86778b532094cc1830c7ec7b5f019bcfd2eb9ded97a2` |
| `datatype3.html` | *Datatypes In SQLite* | 2026-09-13 | 38701 | `4eaab766411136c123d392b2a931c69106a60a616f5a52536eacc79729816fe1` |
| `lang_expr.html` | *SQL Language Expressions* | 2026-09-13 | 379232 | `e2d855812998814eef43c4d009a19eade9511b990384400daf8895d25b5f02e8` |
| `lang_keywords.html` | *SQLite Keywords* | 2026-09-13 | 9459 | `0cb8b8361365c28b9186d3358c27d73bcbbfa7902fe8cd6a57acb0d5adf91745` |
| `testing.html` | *How SQLite Is Tested* | 2026-09-13 | 59623 | `d1a8b5c43a42e573f7165483fdf02b91c605076b869a823d924241fb308feb97` |
| `copyright.html` | *SQLite Copyright* | 2026-09-13 | 8351 | `44ca9f793055c8e32fc65f65a4f5bcf813a33f5bdaaa084067dd617a4ed3cc70` |

Each was fetched twice from `https://www.sqlite.org/<name>.html` and the
two fetches agreed byte for byte.

## Terms

`copyright.html` is kept because it is the term the other four are held
under:

> All of the code and documentation in SQLite has been dedicated to the
> public domain by the authors.

So this is the first case of D-124 — a document whose licence permits the
copy — and there is nothing further to state.

## What is cited from them

- `fileformat2.html` is the contract of `db-sqlite`. Section 1.3 is the
  header, 1.6 the b-tree pages and their overflow rules, 2.1 the record
  format, and 2.6 the schema table. Every structure of that crate names
  the section it comes from.
- `datatype3.html` is the type system: the five storage classes, the five
  affinities, and the comparison and conversion rules that decide what
  equals what. It is what the value layer is written against.
- `lang_expr.html` is the expression grammar the parser will follow, and
  the operator precedence it has to reproduce.
- `lang_keywords.html` is the list of words the language keeps for itself,
  and the four ways a name may be quoted to use one anyway. The table in
  `keyword.rs` is that list; the source of the words is
  `tool/mkkeywordhash.c` of the checkout under `research/`, because the
  page names them without saying which token each becomes.
- `testing.html` is the standard document 16, section 16.7, holds this
  port to: branch coverage and MC/DC over the whole of it, with the
  reasoning for why that is the bar.

Documents for the parts not yet written — the write-ahead log, locking,
the opcodes, the rest of the language — are fetched when the code that
cites them is.
