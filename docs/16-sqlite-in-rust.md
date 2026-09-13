# 16. SQLite in Rust

## 16.0 How to read this document

Every step below has the same six parts, in the same order: Status,
Depends on, Size, Needs, Does, Done when. A step that leaves something
nameable behind carries Produces between Does and Done when. Nothing is
implied. Every term is defined in 16.1 and has exactly one name, used
everywhere.

## 16.1 Terms

| Term | Meaning |
|------|---------|
| the engine | `crates/db/sqlite`, the crate this document is about. |
| the C library | SQLite 3.53.4, checked out under `research/sqlite` by `sh tools/sqlite.sh`. |
| the shell | `research/sqlite/sqlite3`, built by the same script. |
| the oracle | `tools/sqlite-oracle.c`, a program linked against the amalgamation that answers what no SQL statement can ask. |
| corpus | A file of cases, one per line or one per NUL, committed under `crates/db/sqlite/src/tests/fixtures`. |
| golden | What the C library answered for a corpus, committed beside it. |
| recorded oracle | A corpus and its golden together. The comparison runs where SQLite is not installed. |
| fixture | A database file the shell wrote, committed under the same directory. |
| the matrix | The seven run-time dimensions of 16.11. |
| differential execution | The same statement put to both engines and the rows compared. |
| storage class | One of `NULL`, `INTEGER`, `REAL`, `TEXT`, `BLOB`, which is what a value is. |
| affinity | What a column converts a value to before storing or comparing it. |
| collation | What orders two pieces of text: `BINARY`, `NOCASE`, `RTRIM`. |
| core | One `SELECT` between the compound operators, with its own `WHERE`, `GROUP BY` and `DISTINCT`. |
| compound | Two or more cores put together with `UNION`, `UNION ALL`, `INTERSECT` or `EXCEPT`. |
| side | One table of a `FROM` clause, with the join that attaches it. |
| refuse by name | Answer an error variant that names the shape, rather than answer rows that are near. |

## 16.2 Goal

Six things are true at the end of the track that are not true now.

1. The engine reads a database the C library wrote and answers the same
   rows for the same statements.
2. The C library reads a database the engine wrote.
3. SQLite's own test suite under `research/sqlite/test` runs against the
   engine.
4. Every fuzz target of `fuzz/` replays its corpus without a panic.
5. The engine stands at 100 percent of lines, branches and conditions,
   with no line exempted.
6. Every level of the suite runs across the matrix of 16.11.

## 16.3 What "the same" means

| Claim | What is tested |
|-------|----------------|
| The format | A database the engine writes, the C library opens and reads, and the other way round, byte for byte where the format fixes the bytes. `docs/sqlite/fileformat2.html` is the document, and every structure cites its section. |
| The SQL | For a statement both engines accept, the rows come back the same, in the same order where the statement orders them, with the same storage classes. Where SQLite's behaviour is unspecified, the engine matches the C library and the comment says so. |
| The refusals | A statement the C library refuses, the engine refuses. The message need not be the same string. |

Not claimed: the same query plans, the same file sizes for the same
inserts, the same speed, the loadable extensions.

## 16.4 The rules the port is written to

Each rule is checkable, and each makes a later thing possible.

| # | Rule | What it makes possible |
|---|------|------------------------|
| R1 | No layer opens a file, reads a clock, takes a lock or starts a thread. | The engine is a function of bytes, testable on a host with no machine around it, as `audhsos-tls` and `audhsos-ssh` are. |
| R2 | Each layer of 16.10 knows only the layers below it. | `sh tools/xtask.sh check-layering` holds the graph to the table, so a cycle fails the build. |
| R3 | A value is a slice of the page it was stored in, for as long as the page is there. | The read path allocates nothing, so the same code runs in a kernel with no allocator. |
| R4 | Every error is a variant naming the rule of the format or of the language that was broken. | A caller acts on it, a test asserts it, and a message is written from it in any language. |
| R5 | No panic, no unwrap, no index, no arithmetic that can overflow unseen. | A file that lies produces a refusal. The workspace lints hold it. |
| R6 | Every walk of a structure a file describes carries its own bound. | The file chooses the depth of a tree, the length of a chain and the size of a payload; the engine chooses what it will walk. |
| R7 | No global state, no ambient randomness, no clock. | The same database and the same statements answer the same rows, which is what makes differential execution a test. |
| R8 | What the C library answered is committed beside the test. | The comparison runs in CI, where SQLite is not installed. |
| R9 | Each layer is built together with the test that holds it. | Fixtures for the format, a recorded oracle for the tokenizer, differential execution for the semantics, a fuzz target per parser, the matrix for the run-time shapes. |

## 16.5 What is already built

### The crate

| Module | What it holds | Decided in |
|--------|---------------|------------|
| `header`, `page`, `record`, `image`, `bytes` | The hundred-byte header, the four b-tree page types, the four cell shapes, overflow chains, the record format, the walk of a table tree and of an index tree. | Q1 |
| `wal` | The write-ahead log a reader must follow: the header, the frames, the checksum of section 4.2, and the newest committed frame of each page. | D-155, Q3 |
| `token`, `keyword` | SQL text to tokens, the same character classes as `src/tokenize.c`. | Q4 |
| `ast`, `parse` | Tokens to a tree: expressions, `SELECT`, `CREATE TABLE`, `CREATE INDEX`. | Q4 |
| `schema` | The `CREATE` text of `sqlite_schema` to columns, affinities, collations and the rowid rules. | D-145, Q2 |
| `fp`, `number` | A double as decimal text and back: `sqlite3FpDecode`, `sqlite3AtoF`, `sqlite3Atoi64`. | D-142, Q5 |
| `value`, `utf8` | Storage classes, affinity, collation, comparison, the three text encodings. | D-143, D-147, Q5 |
| `eval`, `func`, `agg` | An expression over a row; thirty-four scalar functions; seven aggregates; the four shapes of statement an expression uses. | D-143, D-144, D-148, D-154, Q5 |
| `db` | A statement answered from a file by walking the sides of its `FROM` once. | D-146, D-149, D-150, D-153, Q5 |

### What a statement may hold

| Clause | State |
|--------|-------|
| `SELECT` over one table, over none, or over several joined | answered |
| `WHERE`, `ORDER BY`, `LIMIT`, `OFFSET`, `DISTINCT` | answered |
| `GROUP BY`, `HAVING`, and `count`, `sum`, `total`, `avg`, `min`, `max`, `group_concat`, each with `DISTINCT` | answered |
| `UNION`, `UNION ALL`, `INTERSECT`, `EXCEPT`, `VALUES` | answered |
| A comma, `JOIN`, `INNER`, `CROSS`, `LEFT`, `RIGHT`, `FULL`, `ON`, `USING`, `NATURAL` | answered |
| A table written with `main` in front of it | answered |
| A column that is computed, stored or not | answered |
| A table whose rows live in the key's own tree | answered |
| A statement inside a `FROM`, a `WITH` that is not `RECURSIVE` | answered |
| `(SELECT ...)` as a value, `EXISTS`, `IN (SELECT ...)`, `IN table`, correlated or not | answered |
| A `WITH` written `RECURSIVE`, a window clause, a table-valued function | refused by name |

### What the tests hold it to

| Layer | Cases | Source |
|-------|-------|--------|
| Tokens | 959, of which 800 come out of SQLite's own suite | recorded oracle |
| Expressions, parsed | 541 | recorded oracle |
| Statements, parsed | 792, of which 36 are refused and counted | recorded oracle |
| Schemas | 169 | recorded oracle, `schema.corpus` |
| Doubles as text | 8404, at three precisions | recorded oracle, `fp.corpus` |
| Text as numbers | 215 | recorded oracle, `num.corpus` |
| Expressions, answered | 17051 | recorded oracle, `eval.corpus` |
| Statements, answered | 361 over 14 fixtures | recorded oracle, `query.corpus` |
| A database whose content is in its log | 26 cases over `logged.db` | the fixture and logs built by hand |
| The format under every configuration | 11 fixtures, the same three rows and the same index | the matrix, 16.11 |
| The readers against arbitrary bytes | 5 fuzz targets | `fuzz/sqlite_image`, `sqlite_tokens`, `sqlite_expr`, `sqlite_eval`, `sqlite_wal` |

The crate is `COMPLETE` in `crates/tools/xtask/src/policy.rs`: 100 percent
of lines and 100 percent of branches, in both instrumentations.

## 16.6 What is missing

| # | What is missing | Which step |
|---|-----------------|------------|
| 1 | The pager: a page cache, and the rollback journal a reader must not read past. The write-ahead log is built. | Q3 |
| 2 | The secondary indexes, used rather than read: a `WHERE` that names an indexed column still scans. | Q6 |
| 3 | The virtual machine and the code generator that replaces the tree walker. | Q6 |
| 4 | Writing: the b-tree writer, transactions, the journal in four modes, the WAL. | Q7 |
| 5 | The rest of the language: `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `ALTER`, `DROP`, triggers, views, a `WITH` written `RECURSIVE`. | Q8 |
| 6 | The window clauses, which the parser refuses. | Q8 |
| 7 | An adapter that speaks the commands SQLite's TCL suite drives. | Q9 |
| 8 | The matrix run across every level of the suite rather than the format alone. | Q9 |
| 9 | MC/DC, which the pinned toolchain does not emit. See D4 (16.9). | Q10 |

## 16.7 Decision D1: the engine is a port of the routines

**The decision: each piece is ported from the C routine that implements
it, named in the comment, rather than written from the documentation.**

Reason 1: the documentation is silent where the answers are. It does not
say which side of a comparison takes affinity first, which is what makes
`'abc' + 1` answer 1; `src/vdbe.c` says.
Reason 2: the documentation is wrong about spelling. `docs/sqlite/datatype3.html`
gives no rule that produces `0.33333333333333331` for `1.0/3`;
`sqlite3FpDecode` produces it.
Reason 3: a named routine is checkable. A reader compares the Rust with
the C beside it, and a corpus case holds the two together.

**The option not taken: implement the documentation and test against the
C library.** It costs one diagnosis per divergence, and every divergence
found so far — float rendering, `substr` over a blob, `replace` with an
empty pattern, `likelihood` refusing a non-literal — was a rule the
documentation does not state.

Recorded in D-141 through D-150.

## 16.8 Decision D2: rows before plans

**The decision: a statement is answered by walking each table's tree
once, with no index, no plan and no compilation, until the rows are
right.**

Reason 1: a wrong row is a bug and a slow scan is a number. Only the
first blocks the next step.
Reason 2: the walker is what the virtual machine of Q6 is tested
against. Two implementations that answer the same rows is a stronger
test than one.
Reason 3: the shapes the walker cannot answer refuse by name, so the
engine's answer is either right or absent.

What it costs: O(n) for a scan, O(n log n) where an `ORDER BY` sorts,
O(n·g) to find which of `g` groups a row belongs to, and the product of
the rows for a join of several tables.

**The option not taken: generate opcodes from the start.** It puts the
code generator, the register machine and the semantics in one step, and
a wrong row could be any of the three.

Recorded in D-146.

## 16.9 Decision D3: the oracle is recorded, not trusted

**The decision: every answer the C library gives is committed as a golden
file, together with the corpus that produced it and the program that
asked.**

Reason 1: CI has no SQLite, and a test that needs one is a test that does
not run.
Reason 2: a golden file is a diff. A change in the engine that changes an
answer shows as a failed comparison against a fixed string.
Reason 3: `sh tools/sqlite-fixtures.sh` regenerates every corpus and
every golden from the shell and the oracle, so the recording is
reproducible rather than remembered.

**The option not taken: link the C library into the tests.** It costs a C
toolchain in CI and an `unsafe` boundary in a crate that has none.

## 16.10 Decision D4: what coverage means here

**The decision: the engine is held to 100 percent of lines and 100
percent of branches in both instrumentations, unreachable defensive code
is deleted rather than exempted, and MC/DC is named as not measured.**

Reason 1: `docs/sqlite/testing.html` is the standard SQLite holds itself
to, and a port that claims the file format should claim the testing too.
Reason 2: a refusal no input reaches is either dead code or a missing
test. Both are defects, and an exemption hides which one it is.
Reason 3: the pinned toolchain takes `-Z coverage-options=block`,
`branch` and `condition`, and `llvm-cov` reports no MC/DC pairs for what
it emits. Condition coverage is measured by `sh tools/xtask.sh coverage
--condition`; the independence half of MC/DC is not.

**The option not taken: claim MC/DC from condition coverage.** Condition
coverage counts each operand both ways. It does not show that each
operand alone decides the outcome, which is the half that finds a
condition masked by another.

The gap closes when the pin emits the records. Until then this document
states it.

## 16.11 The configuration matrix

| Dimension | Values |
|-----------|--------|
| Text encoding | UTF-8, UTF-16 little-endian, UTF-16 big-endian |
| Page size | 512, 1024, 4096, 65536 |
| Reserved bytes per page | 0, 32 |
| Journal mode | `delete`, `truncate`, `persist`, `memory`, `wal`, `off` |
| Auto-vacuum | off, full, incremental |
| Schema format | 1 to 4 |
| Temporary storage | file, memory |

The matrix is a table in the test support and not a `for` loop in each
test: a test names the dimensions it is sensitive to, and the harness
runs it for every value of them.

## 16.12 The fixtures

`sh tools/sqlite-fixtures.sh` writes every fixture and every corpus the
tests read, using the shell and the oracle. They are committed, because
CI has no SQLite.

| Fixture | What it holds |
|---------|---------------|
| `m-*.db`, eleven of them | The same three rows and the same index under every configuration of 16.11 the shell can write. |
| `small.db` | One row of each storage class. |
| `page512.db` | Four hundred rows over 512-byte pages, which makes an interior page, in a table tree and in a key's own tree. |
| `utf16.db`, `wide16.db` | Text in UTF-16, including the widths where the order of UTF-16 and the order of characters part. |
| `overflow.db` | A payload that continues on overflow pages. |
| `indexed.db` | One table with two indexes. |
| `keys.db` | The shapes a key takes: a rowid alias, a key written backwards, and three tables with no rowid — one whose key columns are out of the order they were declared in, one naming a key column twice, one with a column the record does not hold. |
| `generated.db` | Computed columns: stored, not stored, one naming a column computed after it, and one of each declared type. |
| `joins.db` | Three tables: two sharing a column named `x`, one sharing `y` and collating it without case. |

## 16.13 The layers

| # | Layer | What it holds | Status |
|---|-------|---------------|--------|
| L1 | The format | Header, b-tree pages, cells, overflow chains, records. | built |
| L2 | The pager | Pages in and out of a file, the journal in four modes, the WAL, locking, the free list. | missing |
| L3 | The b-tree writer | Insert, delete, balance, the pointer maps auto-vacuum needs. | missing |
| L4 | The tokenizer and parser | SQL text to a tree. | built but for the window clauses |
| L5 | The code generator and virtual machine | The tree to opcodes, and the register machine that runs them. | missing |
| L6 | The semantics | Affinity, comparison, collation, the built-in functions, `NULL`. | built for the read half |
| L7 | The interface | Prepare, step, bind, column, and a shell to type at. | missing |

## 16.14 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| Q1 | The format, read | built | nothing | L |
| Q2 | The schema as types | built | Q1 | M |
| Q3 | The pager and the index trees | the log is built; the cache and the journal are open | Q1 | L |
| Q4 | The tokenizer and the parser | built but for the window clauses | nothing | L |
| Q5 | Values, and a statement answered by walking | built | Q2, Q4 | L |
| Q6 | The virtual machine | open | Q5 | L |
| Q7 | Writing | open | Q3, Q6 | L |
| Q8 | The rest of the language | open | Q6 | L |
| Q9 | The suites run whole | open | Q7, Q8 | M |
| Q10 | Coverage to the standard of D4 | open | Q9 | M |

Q4 depends on nothing and was built beside Q1 and Q2.

## 16.15 Q1. The format, read

Status: built.
Depends on: nothing.
Size: L.

### Needs

- `docs/sqlite/fileformat2.html`, sections 1.3, 1.6 and 2.1.
- Fixtures the shell wrote, one per shape.

### Does

1. Parse the hundred-byte header: page size, encoding, reserved bytes,
   write version, the largest root page.
2. Parse a b-tree page of each of the four types, its cell pointer array
   and its cells.
3. Follow an overflow chain, bounded by the pages the file has.
4. Decode a record: the header of serial types and the body they
   describe.
5. Walk a table tree in rowid order, and an index tree in key order,
   one frame per level, at most 32 levels, without allocating.

### Produces

`header`, `page`, `record`, `image`, `bytes` in `crates/db/sqlite/src`.

### Done when

The eleven matrix fixtures read back the same three rows and the same
index; `sqlite_image` replays its corpus without a panic; the crate meets
D4.

## 16.16 Q2. The schema as types

Status: built.
Depends on: Q1.
Size: M.

### Needs

- Q1, to read `sqlite_schema`.
- The `CREATE TABLE` grammar of Q4.

### Does

1. Read the `sql` column of every `table` row of `sqlite_schema`.
2. Parse it as a `CREATE TABLE`.
3. Give each column its declared type, its affinity by
   `sqlite3AffinityType`, and its collation.
4. Decide the rowid rules: which column is the rowid's alias, whether the
   table has no rowid, whether it is `STRICT`.

### Produces

`schema` in `crates/db/sqlite/src`, and `Table`.

### Done when

169 recorded schemas agree with what the C library made of the same
text, columns, affinities, collations and keys.

## 16.17 Q3. The pager and the index trees

Status: the write-ahead log is built; the page cache and the rollback
journal are open.
Depends on: Q1.
Size: L.

### Needs

- Q1, for the page layout.
- `docs/sqlite/fileformat2.html` section 4, for the write-ahead log.
- The matrix of 16.11, for the journal modes.

### Does

1. Follow a write-ahead log, which a reader must: the header, the
   frames, the checksum of section 4.2, and the newest committed frame
   of each page. Built.
2. Read a page through a cache rather than out of a byte slice. Open.
3. Refuse a database whose rollback journal is hot, which is a file
   mid-write and not a file with rows in it. Open.

### Produces

`wal` in `crates/db/sqlite/src`, and `Image::open_with_log` and
`Database::open_with_log` beside the two that read a file alone.

### Done when

A file in each of the six journal modes reads back the same rows;
`fixtures/logged.db`, whose file names no table, reads back the rows its
log holds; the crate meets D4.

## 16.18 Q4. The tokenizer and the parser

Status: built but for the window clauses.
Depends on: nothing.
Size: L.

### Needs

- `src/tokenize.c`, for the character classes.
- `src/parse.y`, for the grammar.

### Does

1. Tokenize SQL text with the same classes and the same rules as
   `src/tokenize.c`.
2. Parse an expression, with the precedence of `parse.y`.
3. Parse a `SELECT`, a `CREATE TABLE` and a `CREATE INDEX` into an arena
   the walk of which is bounded.

### Produces

`token`, `keyword`, `ast`, `parse` in `crates/db/sqlite/src`.

### Done when

959 recorded token cases agree; 792 recorded statements are accepted or
refused as the C library does, but for the 36 the parser counts;
`sqlite_tokens` and `sqlite_expr` replay their corpora without a panic.

## 16.19 Q5. Values, and a statement answered by walking

Status: built.
Depends on: Q2, Q4.
Size: L.

### Needs

- Q2, for the columns and their affinities.
- Q4, for the tree.
- `src/vdbe.c`, `src/func.c`, `src/select.c`, for what each operator,
  function and clause does.

### Does

1. Answer an expression over constants, each operator taken from the
   opcode it compiles into.
2. Answer an expression over a row, with affinity and collation applied
   as the comparison opcodes apply them.
3. Walk one table's tree and answer `SELECT` over it, with `WHERE`,
   `ORDER BY`, `LIMIT` and `DISTINCT`, computing the columns the record
   does not hold.
4. Group the rows and accumulate the seven aggregates.
5. Put several cores together with the four compound operators.
6. Nest the loops for a join, one level per side.

### Produces

`fp`, `number`, `value`, `utf8`, `eval`, `func`, `agg`, `db` in
`crates/db/sqlite/src`.

### Done when

8404 doubles, 215 numbers, 17051 expressions and 299 statements agree
with the C library; `sqlite_eval` and `sqlite_image` replay their corpora
without a panic; the crate meets D4.

## 16.20 Q6. The virtual machine

Status: open.
Depends on: Q5.
Size: L.

### Needs

- Q5, for the semantics each opcode carries.
- `src/vdbe.c`, for the opcodes.
- `src/select.c`, for what a statement compiles into.

### Does

1. Define the register machine and its opcodes.
2. Generate opcodes from the tree Q4 builds.
3. Answer every statement Q5 answers, by running the opcodes.

### Done when

The 299 recorded statements answer the same rows through the machine as
through the walker; the crate meets D4.

## 16.21 Q7. Writing

Status: open.
Depends on: Q3, Q6.
Size: L.

### Needs

- Q3, for the pager and the journal.
- Q6, for the opcodes that write.

### Does

1. Insert, delete and balance in a b-tree.
2. Keep the free list and the pointer maps.
3. Run a transaction through the rollback journal in each of its four
   modes, then through the WAL.

### Done when

The C library opens a database this engine wrote and reads the rows back;
the matrix of 16.11 runs across the write path.

## 16.22 Q8. The rest of the language

Status: open.
Depends on: Q6.
Size: L.

### Does

1. `INSERT`, `UPDATE`, `DELETE`, `REPLACE`.
2. `CREATE`, `ALTER`, `DROP` for tables, indexes, views and triggers.
3. Subqueries, `WITH`, and the window clauses the parser refuses.
4. The functions that need a clock, a random source or `printf`.

### Done when

Every statement of the recorded corpora is accepted or refused as the C
library accepts or refuses it, with no count of what is waiting.

## 16.23 Q9. The suites run whole

Status: open.
Depends on: Q7, Q8.
Size: M.

### Needs

- An adapter that speaks the commands `testfixture` drives, which is
  where a C ABI would live.
- The matrix of 16.11 in the test support.

### Does

1. Run SQLite's TCL suite under `research/sqlite/test` against the
   engine.
2. Run every level of this repository's own suite across the matrix
   rather than the format alone.
3. Drive differential execution from `norec`'s generator with the engine
   as the second implementation.

### Done when

The TCL suite reports no failure that is not a documented omission.

## 16.24 Q10. Coverage to the standard of D4

Status: open.
Depends on: Q9.
Size: M.

### Does

1. Hold every crate of the port to 100 percent of lines and branches in
   both instrumentations.
2. Measure MC/DC when the pinned toolchain emits the records, and raise
   the gate to it.

### Done when

`sh tools/xtask.sh coverage --condition` reports 100 percent for every
crate of the port, and D4's named gap is closed or restated.

## 16.25 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | The C library changes under the port. | A golden file records an answer the current library no longer gives. | `sh tools/sqlite.sh` pins tag `version-3.53.4`. A change of tag regenerates every golden and shows as a diff. |
| 2 | The walker of Q5 and the machine of Q6 disagree. | Two answers, and no way to say which is SQLite's. | The recorded corpora are the third party. Both are compared against the golden, not against each other. |
| 3 | A refusal is added to reach a green check rather than because SQLite refuses. | The engine answers less and the count of refusals hides it. | The count in `tests/db.rs` is asserted and may only fall. |
| 4 | Coverage is met by deleting a test's reach rather than by reaching. | A branch counted covered by one input that no file produces. | D4 forbids exemptions. A branch no input reaches is deleted, which shows in the diff as deleted code. |
| 5 | The matrix is a `for` loop copied into each test. | A dimension added in one test and forgotten in ten. | 16.11 puts the matrix in the test support and has the test name its dimensions. |
| 6 | The fuzz corpora grow until the regression replay is slow. | The check takes longer than three minutes and is skipped. | `sh tools/xtask.sh fuzz --merge` keeps one input per feature. Hash-named files are not committed; named regression entries are. |
| 7 | MC/DC never becomes measurable on the pinned toolchain. | Goal 5 of 16.2 cannot be met as written. | D4 states the gap rather than claiming it away. Condition coverage is measured and gated today. |
