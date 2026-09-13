# 15. SQLite in Rust

## 15.1 What is being built

A database engine in this repository's Rust that is SQLite: the same file
format, the same SQL, the same answers. It reads a file the C library
wrote, and the C library reads what it writes. It is written under the
rules every other crate here follows — no dependency outside the
workspace, `forbid(unsafe_code)` in the logic, host tests under a coverage
gate — and it is checked against the C library rather than against a
reading of the documentation.

Why this system wants one: a file system is what document 2 gives
userland, and the next thing a program asks for after a file is a table.
D-53 put the file system logic in a crate so that the server and the image
writer share one implementation of the format; a database is the same
argument one layer up. The port is also the largest test this project's
tooling has been given, and the tooling is what makes it possible: the
`sqlite3` shell of `sh tools/sqlite.sh` is a reference implementation that
answers questions, and `norec` already asks it some.

## 15.2 What "the same" means

Three claims, each of them testable, and no claim beyond them.

- **The format.** A database this engine writes is one the C library opens
  and reads without complaint, and the other way round, byte for byte
  where the format fixes the bytes. `docs/sqlite/fileformat2.html` is the
  document, and every structure cites the section it comes from.
- **The SQL.** For a statement both engines accept, the rows come back the
  same, in the same order where the statement orders them, with the same
  types. Where SQLite's behaviour is unspecified, this engine matches what
  the C library does and says so in a comment.
- **The refusals.** A statement the C library refuses is refused here too.
  The message need not be the same string; the error code is.

What is not claimed: the same query plans, the same file sizes for the
same inserts, the same performance, or the loadable extensions.

## 15.3 The architecture, and its layers

Nine rules the port is written to. They are not style; each one is what
makes some later thing possible, and each is checkable.

1. **Sans-I/O.** No layer opens a file, reads a clock, takes a lock or
   starts a thread. A layer is a function of bytes and a state machine over
   them; the I/O is at the edge, in whatever embeds the engine. This is
   what `audhsos-tls` and `audhsos-ssh` already are, and it is why they can
   be tested exhaustively on a host with no machine around them.
2. **One direction.** Format, pager, b-tree, parser, planner, virtual
   machine, interface: each layer knows only the ones below it. The check
   `cargo xtask check-layering` holds the graph to the table, so a cycle
   fails the build rather than the review.
3. **Read without copying.** A value is a slice of the page it was stored
   in, for as long as the page is there. The read path allocates nothing,
   which is what lets the same code run in a kernel with no allocator and
   in a shell with one.
4. **Refusals are data.** Every error names the rule of the format or of
   the language that was broken, as a variant and not a string. A caller
   can act on it, a test can assert it, and a message can be written from
   it in whatever language the caller prints in.
5. **Total functions.** No panic, no unwrap, no index, no arithmetic that
   can overflow unseen: the workspace lints forbid them, so a file that
   lies produces a refusal and never a crash.
6. **Bounded work.** Every walk of a structure a file describes carries its
   own bound — the depth of a tree, the length of a chain, the size of a
   payload — because the file chooses those numbers and the engine must
   not be what they choose.
7. **Determinism.** No global state, no ambient randomness, no clock. The
   same database and the same statements answer the same rows, which is
   what makes a differential test against the C library a test rather than
   a hope.
8. **The oracle is recorded, not trusted.** Where the C library is the
   truth — the tokenizer's answers, the bytes of a file — what it answered
   is committed beside the test, so the comparison runs where SQLite is not
   installed. What generated it is written down; what it generated is
   checked in.
9. **Testing is part of the design.** Each layer is built with the test
   that can hold it: fixtures for the format, a recorded oracle for the
   tokenizer, differential execution for the semantics, property tests for
   the algebra, fuzz targets for every parser, the configuration matrix of
   15.6 for the run-time shapes, and complete coverage over all of it.

Bottom to top, each its own crate or module, each testable without the one
above it:

1. **The format** (`db-sqlite`, reading): the header, b-tree pages, cells,
   overflow chains, the record format. Done.
2. **The pager**: pages in and out of a file, the rollback journal and its
   four modes, the write-ahead log, locking, and the free list.
3. **The b-tree writer**: insert, delete, balance, and the pointer maps
   auto-vacuum needs.
4. **The tokenizer and the parser**: SQL text to a syntax tree, in the
   grammar `docs/sqlite/lang_expr.html` and its neighbours describe.
5. **The code generator and the virtual machine**: the tree to opcodes, and
   the register machine that runs them.
6. **The semantics**: type affinity, the comparison and collation rules,
   the built-in functions, `NULL` everywhere.
7. **The interface**: prepare, step, bind, column, and the shell that drives
   them, so that a person can type at it.

## 15.4 The order of work

Each step ends green: the checks pass, the coverage gate holds, and what
the step claims is tested.

| Step | What it delivers |
|------|------------------|
| Q1 | The format, read-only: header, pages, cells, overflow, records. **Done.** |
| Q2 | Reading a schema into types: columns, affinities, indexes, and the `sqlite_schema` text parsed rather than handed on. |
| Q3 | The pager reading: page cache, the journal a reader must ignore, the WAL a reader must follow. |
| Q4 | The tokenizer, the expression parser and the statement parser for the read half of SQL. **Done**, but for the window clauses. |
| Q5 | A tree walker that answers those statements from a file, with affinity and collation. Differential tests against the C shell begin here. |
| Q6 | The virtual machine, and the code generator that replaces the walker. |
| Q7 | Writing: the b-tree writer, transactions, the rollback journal in all four modes, then the WAL. |
| Q8 | The rest of the language: `CREATE`, `ALTER`, `DROP`, triggers, views, the built-in functions. |
| Q9 | The configuration matrix and the test suites of 15.5 run whole. |
| Q10 | Coverage to the standard of 15.6. |

## 15.5 How it is tested

Four sources of truth, in the order they were built:

- **Fixtures the C library wrote.** A database written by the shell is the
  format as it is. They are small, they are committed, and the statement
  that produced each is in the test that reads it.
- **Differential execution.** The same statement to both engines, the rows
  compared. `norec` already drives the shell one case at a time; the same
  harness, given a second engine, is a differential tester, and the
  generator it has is the one that writes the cases.
- **SQLite's own tests.** The suite under `research/sqlite/test` is TCL
  driving a `testfixture` that links the library. Running it against this
  engine needs a fixture that speaks the same commands; that is an adapter
  crate (rule R4 territory: it is where the C ABI would live), and the
  order in 15.4 puts it after the engine can answer statements at all.
  Until then the `.test` files are read as specifications — each names the
  behaviour it checks — and the ones that are pure SQL are run through the
  differential harness.
- **Fuzzing.** The project's own engine (`crates/support/fuzz`) over every
  parser this port has: the file format reader, the tokenizer, the
  parser, and the record decoder, each with a corpus under `fuzz/`.

## 15.6 The configuration matrix

A test that ran under one configuration tested one configuration. What
varies, and what every level of the suite runs across:

| Dimension | Values |
|-----------|--------|
| Text encoding | UTF-8, UTF-16 little-endian, UTF-16 big-endian |
| Page size | 512, 1024, 4096, 65536 |
| Reserved bytes per page | 0, 32 |
| Journal mode | `delete`, `truncate`, `persist`, `memory`, `wal`, `off` |
| Auto-vacuum | off, full, incremental |
| Schema format | 1 to 4 |
| Temporary storage | file, memory |

The matrix is a table in the test support, not a `for` loop in each test:
a test names the dimensions it is sensitive to, and the harness runs it
for every value of them.

## 15.7 Coverage

The standard is the one SQLite holds itself to and documents in
`docs/sqlite/testing.html`: every branch taken both ways, and modified
condition/decision coverage over every compound condition. Two things
follow for this port.

- The `coverage` step measures lines and branches. `sh tools/xtask.sh
  coverage --condition` measures more: it builds with
  `-Z coverage-options=branch,condition`, so that every operand of a
  compound decision is counted and not only the decision, and the same
  thresholds apply to that column. What the pinned toolchain does not
  emit is LLVM's MC/DC records — `-Z coverage-options` takes `block`,
  `branch` and `condition`, and `llvm-cov` reports no MC/DC pairs for
  what it produces — so the independence half of MC/DC is not measured
  today. Condition coverage is what is measured; the gap is named here
  rather than claimed away, and it closes when the pin emits the
  records.
- Unreachable defensive code is a defect and not a line to exempt. Where a
  refusal cannot be reached by any input, either the refusal is dead and
  goes, or the input that reaches it exists and is missing from the tests.
  `db-sqlite` is held to all of it — `COMPLETE` in the policy table, 100
  percent of lines and 100 percent of branches, in both instrumentations —
  and it meets it: the refusals that no file could reach were removed
  rather than excused, and the ones that a file can reach are reached by a
  test, most of them by a database laid out by hand for that purpose.

## 15.8 The fixtures

`sh tools/sqlite-fixtures.sh` writes every fixture the tests read, with the
shell of `sh tools/sqlite.sh`. They are committed, because CI has no
SQLite; the script is what makes them reproducible rather than
remembered. Eleven of them are the matrix of 15.6 holding the same three
rows and the same index, and the rest are the cases one test each reads:
a table of every storage class, four hundred rows over 512-byte pages, text
in UTF-16, a payload that overflows, and a table with two indexes.

## 15.9 Where it stands

Q1 and the tokenizer of Q4 are in `crates/db/sqlite`, and the crate is
held to complete coverage: every line, every region and every branch, in
both instrumentations.

A database is opened, its schema walked, its
tables read in rowid order, its overflow chains followed, and its records
decoded, over any page size and any of the three encodings, without
allocating. The matrix of 15.6 is a test, the reader is fuzzed by
`sqlite_image`, and the first bug that target found — a child pointer of
zero, which is a page no file has — is in the regression corpus. A statement is read
into a tree that agrees with SQLite's parser over seven hundred and ninety
-two recorded cases — everything it refuses is refused, and all but
thirty-six of what it accepts is read — an expression over five hundred
and forty-one more, and SQL text is tokenized exactly as
`src/tokenize.c` tokenizes it — the same character
classes, the same rules, the same answers, checked against nine hundred
and fifty-nine recorded cases of which eight hundred come out of SQLite's
own test suite. What the crate cannot do is everything else in 15.3.
