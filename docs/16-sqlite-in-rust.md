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
| `header`, `page`, `record`, `image`, `bytes` | The hundred-byte header, the four b-tree page types read and written, the four cell shapes, overflow chains, the record format read and written, the walk of a table tree and of an index tree. | Q1, D-162, D-163 |
| `wal` | The write-ahead log a reader must follow and a commit must write: the header, the frames, the checksum of section 4.2, and the newest committed frame of each page. | D-155, D-171, Q3, Q7 |
| `journal` | The rollback journal a reader must play back and a commit must write: the headers, the records, the checksum of `pager_cksum`, the content each page began with, and what each journal mode leaves behind. | D-156, D-170, Q3, Q7 |
| `token`, `keyword` | SQL text to tokens, the same character classes as `src/tokenize.c`. | Q4 |
| `ast`, `parse` | Tokens to a tree: expressions, `SELECT`, `CREATE TABLE`, `CREATE INDEX`. | Q4 |
| `schema` | The `CREATE` text of `sqlite_schema` to columns, affinities, collations, the rowid rules and the indexes. | D-145, D-159, Q2 |
| `fp`, `number` | A double as decimal text and back: `sqlite3FpDecode`, the `%f`, `%e` and `%g` conversions it feeds, `sqlite3AtoF`, `sqlite3Atoi64`. | D-142, D-161, Q5 |
| `value`, `utf8` | Storage classes, affinity, collation, comparison, the three text encodings. | D-143, D-147, Q5 |
| `eval`, `func`, `agg` | An expression over a row; fifty scalar functions; seven aggregates; the four shapes of statement an expression uses. | D-143, D-144, D-148, D-154, D-160, Q5 |
| `tree` | The pages of a database being written, a row put in the tree its table begins at or taken out of it again, the balance any key order needs, and the free list the pages go on. | D-166 to D-169, Q7 |
| `format` | `format(F,...)` and `printf(F,...)`: the flags, the field width, the precision, and the twenty-three conversions of `sqlite3_str_vappendf`. `unistr(X)` reads the escapes `%#q` writes, and `quote(X)` of text is `%Q` of it. | D-161, Q8 |
| `db` | A statement answered from a file by walking the sides of its `FROM` once, held to the rowids the `WHERE` leaves each. | D-146, D-149, D-150, D-153, D-158, Q5 |
| `change` | A statement that changes a database run from its text: the table a `CREATE TABLE` names, the rows an `INSERT` puts in it, the rows a `DELETE` takes out, the rows an `UPDATE` writes over, what the journal mode leaves beside the file, and what a `PRAGMA` configures. | D-172, D-173, D-174, D-175, D-182, Q8 |
| `pragma` | The settings the file itself holds, which a reader answers out of the header and a writer applies before the first table. | D-182, Q8 |

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
| A `WHERE` that names the rowid, which the walk descends to rather than scanning past | answered |
| A `WHERE` that holds an indexed column equal to a value, which the walk reads out of the index | answered |
| A statement inside a `FROM`, a `WITH` that is not `RECURSIVE` | answered |
| `(SELECT ...)` as a value, `EXISTS`, `IN (SELECT ...)`, `IN table`, correlated or not | answered |
| A window clause: `OVER`, a `WINDOW` clause, a `FILTER` | answered |
| A table-valued function | refused by name |

### What the tests hold it to

| Layer | Cases | Source |
|-------|-------|--------|
| Tokens | 959, of which 800 come out of SQLite's own suite | recorded oracle |
| Expressions, parsed | 541 | recorded oracle |
| Statements, parsed | 792, of which 36 are refused and counted | recorded oracle |
| Schemas | 169 | recorded oracle, `schema.corpus` |
| Doubles as text | 8404, at three precisions | recorded oracle, `fp.corpus` |
| Text as numbers | 215 | recorded oracle, `num.corpus` |
| Expressions, answered | 23836 | recorded oracle, `eval.corpus` |
| Statements, answered | 785 over 32 fixtures | recorded oracle, `query.corpus` |
| A database whose content is in its log | 26 cases over `logged.db` | the fixture and logs built by hand |
| A database caught mid-transaction | 19 cases over `rollback.db`, and the journals two modes leave behind | the fixture and journals built by hand |
| The format under every configuration | 11 fixtures, the same three rows and the same index | the matrix, 16.11 |
| A row written back as the bytes it was read from | 978 rows over 8 files | the fixtures the shell wrote |
| A cell written back, and a page built from its cells | 1663 cells, 56 pages | the fixtures the shell wrote |
| A cell taken off a page and put back | 1333 cells | the fixtures the shell wrote |
| A whole file written back, header and pages | 27 files, 108 pages built again | the fixtures the shell wrote |
| A database written from a schema and its rows | 5 files, byte for byte, one of them a tree of 15 pages | the fixtures the shell wrote |
| The readers against arbitrary bytes | 7 fuzz targets | `fuzz/sqlite_image`, `sqlite_tokens`, `sqlite_expr`, `sqlite_eval`, `sqlite_wal`, `sqlite_journal`, `sqlite_format` |

The crate is `COMPLETE` in `crates/tools/xtask/src/policy.rs`: 100 percent
of lines and 100 percent of branches, in both instrumentations.

## 16.6 What is missing

| # | What is missing | Which step |
|---|-----------------|------------|
| 1 | The plan that chooses between them: the first index whose column a `=` names is the one taken, and how many rows each would answer is not counted. | Q6 |
| 2 | The virtual machine and the code generator that replaces the tree walker. | Q6 |
| 3 | Writing past the record: the b-tree writer, transactions, the journal in four modes, the WAL. `changes`, `total_changes` and `last_insert_rowid` answer nought until then, which is what a connection that has written nothing answers. | Q7 |
| 4 | The rest of the language: `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `ALTER`, `DROP`, triggers, views, a `WITH` written `RECURSIVE`. | Q8 |
| 5 | The functions whose answers are not exact: `sqrt`, `exp`, `ln`, `log`, `pow` and the trigonometric set. See D-160. | Q8 |
| 6 | The matrix run across every level of the suite rather than the format alone. | Q9 |
| 7 | A report that names MC/DC pairs, which the pinned toolchain does not emit. D4 (16.10) derives MC/DC from condition coverage and `cargo xtask mcdc` instead. | Q10 |

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
percent of branches under both instrumentations, unreachable defensive
code is deleted rather than exempted, and MC/DC is claimed from
condition coverage over decisions `cargo xtask mcdc` holds to the
short-circuit operators and to naming no condition twice.**

Reason 1: `docs/sqlite/testing.html` is the standard SQLite holds itself
to, and a port that claims the file format should claim the testing too.

Reason 2: a refusal no input reaches is either dead code or a missing
test. Both are defects, and an exemption hides which one it is.

Reason 3: `&&` and `||` evaluate their right operand only where the left
one does not settle the answer, so an operand is evaluated only where
every operand that could mask it has taken the value that does not mask
it, and it decides the outcome wherever it is evaluated. Two evaluations
of one operand with opposite values are then the pair MC/DC asks for.
Condition coverage at 100 percent says every operand was evaluated both
ways, so over such decisions it is masking MC/DC, which is the form
DO-178C accepts for the objective.

Reason 4: a bitwise `&`, `|` or `^` over booleans breaks reason 3,
because both operands are evaluated whatever the first says and the
toolchain records no condition for either.

Reason 5: unique-cause MC/DC asks beyond reason 3 that no decision name
one condition twice, because a condition standing in two places cannot
be varied on its own. Naming it twice is what makes masking MC/DC and
unique-cause MC/DC part, so a check that finds none closes the two
together.

Reason 6: `cargo xtask mcdc` reads the typed tree of every crate of
`COMPLETE` through `-Zunpretty=thir-tree`, reports each bitwise operator
over booleans by its span, and reads the source each decision's
conditions lie at to report a decision that names one twice. It passes
over a span the compiler wrote from a macro, because a derived
`PartialEq` compares each field under one span and would read as one
condition repeated. `db-sqlite` holds 409 short-circuit operators, 656
conditions, no bitwise operator over booleans and no decision that names
a condition twice. A reading of the source alone would answer neither,
because `&` there is a reference as often as an operator and the type of
an operand is what tells the two apart.

Reason 7: `cargo xtask check` runs `coverage`, `coverage --condition`
and `mcdc`, so a decision whose second operand no test settles, a
decision written with an operator that hides its operands, and a
decision naming one condition twice each fail the check.

**The option not taken: wait for `-Z coverage-options=mcdc`.** The pin
takes `block`, `branch` and `condition` and refuses `mcdc`, and a goal
does not become measured by waiting for a toolchain.

## 16.11 The configuration matrix

| Dimension | Values |
|-----------|--------|
| Text encoding | UTF-8, UTF-16 little-endian, UTF-16 big-endian |
| Page size | 512, 1024, 4096, 8192, 65536 |
| Reserved bytes per page | 0, 4, 32 |
| Journal mode | `delete`, `truncate`, `persist`, `memory`, `wal`, `off` |
| Auto-vacuum | off, full, incremental |
| Schema format | 1 to 4 |
| Temporary storage | file, memory |

Nineteen fixtures hold one point of the matrix each, and every one of
them holds the same three rows and the same index, so a difference in an
answer is a difference the configuration made. Six tests read them: four
over the format, one that puts sixteen statements to every fixture and
holds the answers to each other, and one that holds `hex`,
`octet_length` and a cast to a blob to the difference the encoding makes,
because those three answer the bytes as they are stored. `query.corpus`
records what the C library answers for all of them, so the answers are
held to SQLite and to each other.

Nine more fixtures, `w-*.db`, hold one point each of the three
dimensions writing changes — the page size, the encoding and the
reserved tail — with the same four hundred rows put in by a key that
jumps about. Each is built here from the schema and the rows and is the
file the shell wrote, byte for byte.

Thirty more fixtures, `x-NN.db`, hold the five dimensions the write path
answers as a covering array: every value of every dimension appears, and
every pair of values from two dimensions appears together at least once.
The full cross is 3 x 5 x 3 x 6 x 3 = 810, and thirty is the fewest rows
such an array can have, because the two widest dimensions are five and
six values wide. Each holds the same four hundred rows, and each is the
file the shell wrote — with the journal or the log beside it where the
mode leaves one — byte for byte.

Six more fixtures, `v-*.db`, hold the auto-vacuum dimension over the
write path: the same four hundred rows under `full` and under
`incremental`, chains that cross the second pointer-map page, the free
pages `incremental` keeps, and the pages `full` moves down and gives up
at the commit, one of them cutting a file of a hundred and eleven pages
back to thirty-five. Each is the file the shell wrote, byte for byte.

The journal-mode dimension needs no fixture of its own, because the mode
changes what lies beside the file and not what is in it: `change::Writer`
runs the same three statements under `delete`, `truncate`, `persist`,
`memory` and `off` and writes `emptied.db` under every one of them, and
what each mode leaves beside it is nothing, an empty file, or
`journalled.db-journal` byte for byte. The sixth mode writes frames
rather than pages, so the file stays as `PRAGMA journal_mode=wal` left it
and the log is `logging.db-wal` byte for byte.

The last two dimensions have no fixture: a schema format below four
needs a database the shell will not write, and temporary storage is not
a file.

Nothing of the five is left over the write path.

The schema-format dimension is read rather than written: no pragma the
shell takes asks for a format below four, because `legacy_file_format`
is answered and ignored, so the oracle asks for it through
`SQLITE_DBCONFIG_LEGACY_FILE_FORMAT` and writes `format1.db`. Format 3
is what `ALTER TABLE ADD COLUMN` raises a file to, which `format3.db`
holds; format 2 has no fixture, because the pinned library writes 3
wherever the file format document allows 2. Fourteen cases of
`query.corpus` hold the three files to the C library, and the write path
answers the same dimension through `defaults.db`.

Temporary storage has no fixture, because it is not a file.

## 16.12 The fixtures

`sh tools/sqlite-fixtures.sh` writes every fixture and every corpus the
tests read, using the shell and the oracle. They are committed, because
CI has no SQLite.

| Fixture | What it holds |
|---------|---------------|
| `m-*.db`, eleven of them | The same three rows and the same index under every configuration of 16.11 the shell can write. |
| `v-*.db`, six of them | The auto-vacuum dimension over the write path: the pointer maps, the free pages `incremental` keeps, and the pages `full` moves down at the commit. |
| `x-NN.db`, thirty of them | The covering array of 16.11 over the write path, each with the journal or the log the mode leaves beside it. |
| `index-*.db`, nine of them | The index trees a `CREATE INDEX` writes and an `INSERT` keeps: over no row, over a few, with a collation and an order of their own, over a key that runs onto a chain, over every storage class, and over enough rows to need a page above the leaves. |
| `format1.db`, `format3.db`, `format4.db` | The schema-format dimension: the whole numbers 0 and 1 stored with and without a payload, the `DESC` of an index kept and ignored, and the columns `ALTER TABLE ADD COLUMN` left the rows short of. |
| `defaults.db` | A statement that names one of three columns, so the other two hold what they fall back to. |
| `w-*.db`, nine of them | The same four hundred rows, put in by a key that jumps about, under every page size, every encoding and every reserved tail. |
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
| L2 | The pager | Pages in and out of a file, the journal in five modes, the WAL, locking, the free list. | built but for locking and the checkpoint |
| L3 | The b-tree writer | Insert, delete, balance, the pointer maps auto-vacuum needs. | built |
| L4 | The tokenizer and parser | SQL text to a tree. | built |
| L5 | The code generator and virtual machine | The tree to opcodes, and the register machine that runs them. | missing |
| L6 | The semantics | Affinity, comparison, collation, the built-in functions, `NULL`. | built for the read half |
| L7 | The interface | Prepare, step, bind, column, and a shell to type at. | missing |

## 16.14 The order of the steps

| Step | Name | Status | Depends on | Size |
|------|------|--------|------------|------|
| Q1 | The format, read | built | nothing | L |
| Q2 | The schema as types | built | Q1 | M |
| Q3 | The pager and the index trees | built | Q1 | L |
| Q4 | The tokenizer and the parser | built | nothing | L |
| Q5 | Values, and a statement answered by walking | built | Q2, Q4 | L |
| Q6 | The virtual machine | open | Q5 | L |
| Q7 | Writing | the record is written | Q3, Q5 | L |
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

Status: built.
Depends on: Q1.
Size: L.

### Needs

- Q1, for the page layout.
- `docs/sqlite/fileformat2.html` section 4, for the write-ahead log.
- `src/pager.c`, for the rollback journal, which that document does not
  describe: `writeJournalHdr`, `readJournalHdr`, `pager_cksum`,
  `pager_playback`.
- The matrix of 16.11, for the journal modes.

### Does

1. Follow a write-ahead log, which a reader must: the header, the
   frames, the checksum of section 4.2, and the newest committed frame
   of each page. Built.
2. Play back a hot rollback journal, which a reader must: the headers,
   the records, the checksum of `pager_cksum`, and the page count the
   database is truncated to.
3. Hold the page a walk stands on rather than parsing it once per cell,
   which is what a page cache is worth over a file already in memory.
   See D-157.

### Produces

`wal` and `journal` in `crates/db/sqlite/src`, and
`Image::open_with_log`, `Image::open_with_journal`,
`Database::open_with_log` and `Database::open_with_journal` beside the
two that read a file alone.

### Done when

A file in each of the six journal modes reads back the same rows;
`fixtures/logged.db`, whose file names no table, reads back the rows its
log holds; `fixtures/rollback.db`, whose file holds a transaction half
written, reads back the rows that transaction started from; a walk
parses each page it visits once rather than once per cell; the crate
meets D4.

## 16.18 Q4. The tokenizer and the parser

Status: built.
Depends on: nothing. The window clauses are recorded in D-219.
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
4. Parse a `FILTER`, an `OVER` and a `WINDOW` clause, reading each of
   the three words as a name where the words around it leave no window.

### Produces

`token`, `keyword`, `ast`, `parse` in `crates/db/sqlite/src`.

### Done when

959 recorded token cases agree; 792 recorded statements are accepted or
refused as the C library does, but for the 36 the parser counts;
`sqlite_tokens` and `sqlite_expr` replay their corpora without a panic.

## 16.19 Q5. Values, and a statement answered by walking

Status: built.
Depends on: Q2, Q4. Recorded in D-208, D-214, D-215, D-219 and D-221.
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
7. Read a term of the `WHERE` on the level that answers it, and read
   the side of an `ON` by an index that names its key, which D-208
   records.
8. Order a compound under the `COLLATE` its `ORDER BY` was written
   with, and take the rows of a recursive term off a queue its own
   `ORDER BY` and `LIMIT` bound, which D-214 and D-215 record.

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

Status: a table is filled in any key order, emptied again, and filled
from the pages the delete freed; the file and the rollback journal
beside it are the ones the shell wrote, byte for byte. The pointer maps and the
checkpoint are not built.
Depends on: Q3, Q5. Recorded in D-162 to D-171 and D-233.
Size: L.

### Needs

- Q3, for the pager and the journal.
- Q5, for the affinity each column applies before a value is stored.

### Does

1. Write a row as a record: the affinity of each column, a serial type
   per value, the header of types and the body of values. Built.
2. Write a cell as the bytes a page holds it in, and a page as the
   cells it holds. Built.
3. Put a cell on a page and take one off: the free list, the space a
   cell is given, and the page moved together where the space is in
   pieces. Built.
4. Write the hundred-byte header and the file its pages make. Built.
5. Put a row in the tree its table begins at, with what does not fit
   on overflow pages. Built.
6. Balance a tree that is filled in key order: a root that grows a
   child under it, and a full right-most leaf that grows a sibling
   beside it. Built.
7. Balance a tree a row lands in the middle of: the page and its
   siblings written again, the dividers on the parent, and the balance
   run up to the root. Built.
8. Take a row out again: the overflow chain it ran onto, the pages a
   delete has emptied, and the free list they go on. Built.
9. Keep the pointer maps of a file that vacuums itself.
10. Write the rollback journal of a transaction and leave of it what
    the journal mode says. Built.
11. Play a journal back over the file it belongs to. Built.
12. Write the frames of a transaction into the write-ahead log. Built.
13. Checkpoint a log back into the file it belongs to.
14. Open a database that is already written and write it again, which
    reads the pages and the free list out of the header. Built, which
    D-233 records.

### Done when

The C library opens a database this engine wrote and reads the rows back;
the matrix of 16.11 runs across the write path, which the `w-*` fixtures
do for the page size, the encoding and the reserved tail.

## 16.22 Q8. The rest of the language

Status: `CREATE TABLE`, `CREATE INDEX`, `CREATE VIEW`, `CREATE
TRIGGER`, the four `DROP`s, `ALTER TABLE ... ADD COLUMN`, `PRAGMA`,
`ANALYZE`, `REINDEX`, `BEGIN`, `COMMIT`, `ROLLBACK`, `INSERT`, `DELETE`
and `UPDATE` are run from their text and write the files the shell wrote,
a foreign key holds the rows of both tables it names, and the JSON
functions answer what the shell answers; the rest is open.
Depends on: Q6. Recorded in D-172 to D-174, D-182, D-186, D-187, D-189,
D-191, D-193, D-195, D-196, D-205 to D-211, D-216 to D-218, D-219,
D-224 to D-232 and D-234 to D-241.
Size: L.

### Does

1. `INSERT`, `UPDATE`, `DELETE`, `REPLACE`. `INSERT` is built, over
   `VALUES` and over a `SELECT`, and so are `DELETE` and `UPDATE`. A
   `DELETE` and an `UPDATE` over a table an index is over write the
   entries of that index as well, which D-187 records. A `UNIQUE` and
   a `PRIMARY KEY` give the table an index of its own and hold every
   row the statement writes to it, under the conflict word the
   statement names or the one the index carries, which D-205 records.
2. `CREATE`, `ALTER`, `DROP` for tables, indexes, views and triggers.
   `CREATE TABLE` is built for a table of columns and for one made
   from a statement, which D-203 records, `CREATE INDEX` for
   an index over columns, which D-186 records, `CREATE VIEW` and
   `DROP VIEW`, which D-195 records, `DROP TABLE` and `DROP INDEX`,
   which D-191 records, and `CREATE TRIGGER` and `DROP TRIGGER`, which
   D-204 records. `ALTER TABLE ... ADD COLUMN` is built, which D-196 records, and
   `ALTER TABLE ... RENAME TO`, which D-237 records, and `DROP COLUMN`,
   which D-240 records; `RENAME COLUMN` is open.
3. Subqueries, `WITH`, and the window clauses. A `WITH` term that reads
   itself is built, which D-198 records, and the window clauses are
   built, which D-219 records.
4. The functions that need a clock or a random source. `random` and
   `randomblob` are built, which D-199 records, and the date and time
   functions are built for every moment but `now`, which D-232
   records; the clock is open.
5. `ANALYZE`, which counts the tables and their indexes into
   `sqlite_stat1`. Built, which D-207 records.
6. `REINDEX`, which writes the entries of an index again out of the
   rows they belong to. Built, which D-209 records.
7. `PRAGMA integrity_check` and `PRAGMA quick_check`, which walk the
   file and answer what it holds against what it says. Built, which
   D-210 records.
8. `sqlite_schema` as a table a statement reads, and `sqlite_sequence`
   as the count a key that counts up is kept in. Built, which D-216
   and D-218 record.
9. The rows a foreign key holds: the check on the row that points, the
   check on the row pointed at, the five actions, and the two pragmas
   that answer the keys and the orphans. Built, which D-224 records.
10. Rows of values: the comparisons, `BETWEEN`, `IN`, a statement that
    stands for a row, and the refusal of a row written anywhere else.
    Built, which D-225 records; reading an index for such a comparison
    is open.
11. The joins: the column a `USING` names after a `RIGHT JOIN`, the
    tables written inside brackets, and the names those tables answer
    under. Built, which D-226 to D-228 record.
12. The constraints a row is held to: the columns that refuse nothing,
    every `CHECK` of the table, and the key the row is written under.
    Built, which D-230 records.
13. The date and time functions, which D-232 records.
14. A table that keeps its rows in the key's own tree: the tree it is
    made as, and the statements that write it. Built, which D-234
    records.
15. The JSON functions: the twenty-six scalar names, the four
    aggregates and the two operators, over the binary form the header
    comment of `src/json.c` states. Built, which D-235 records;
    `json_each` and `json_tree` need a virtual table and are open.
16. `SAVEPOINT`, `RELEASE` and `ROLLBACK TO`, and the statement
    journal a statement of a transaction is put back from. Built, which
    D-236 records.
17. `ALTER TABLE ... RENAME TO`: every statement of the schema that
    names the table written again under the new name. Built, which
    D-237 records.
18. `INSERT ... ON CONFLICT`: the clause a row that shares a key
    reaches, and what it writes. Built, which D-238 records; a table
    that keeps its rows in the key's own tree is open.
19. The statements written inside the expressions of a statement that
    writes: `EXISTS`, `IN (SELECT ...)` and a statement that stands for
    a value. Built, which D-239 records.
20. `ALTER TABLE ... DROP COLUMN`: the column out of the text that
    made the table and out of every row. Built, which D-240 records.
21. `RETURNING`, and `INSERT INTO t DEFAULT VALUES`. Built, which
    D-241 records.
22. The words a statement that writes is refused with, which
    `sqlite3Insert`, `sqlite3StartTable`, `sqlite3CreateIndex` and
    `sqlite3CheckObjectName` write. Built, which D-242 records.
23. An index over an expression and an index over fewer rows than the
    table has. Built, which D-243 records; a statement is planned
    against neither and walks the table.
24. The key an `ON CONFLICT` clause names, by the columns and the
    collations of that key. Built, which D-244 records; a clause that
    names an index over an expression or over fewer rows is open.
25. `ALTER TABLE ... RENAME COLUMN`: every statement of the schema
    that names the column written again under the new name. Built,
    which D-245 records.
26. The order a row is held to the keys of a table in, which the `ON
    CONFLICT` clauses name. Built, which D-246 records; a table that
    keeps its rows in the key's own tree reaches no clause.
27. `ALTER TABLE ... DROP CONSTRAINT`, `... ALTER COLUMN ... DROP NOT
    NULL`, `... ALTER COLUMN ... SET NOT NULL` and `... ADD
    [CONSTRAINT name] CHECK (...)`. Built, which D-247 records.
28. What a foreign key is compared under, which is the affinity and
    the collation of the parent's column, and what says the columns
    pointed at are a key. Built, which D-248 records.
29. The years and the months `timediff` counts, which is the second
    moment walked to the first. Built, which D-249 records.
30. A foreign key over a table that keeps its rows in the key's own
    tree. Built, which D-250 records.
31. What a `CREATE` and a `DROP` name in a refusal. Built, which D-251
    records.
32. The token a statement the parser refuses is named by. Built, which
    D-252 records.
33. `INSERT`, `UPDATE` and `DELETE` over a view, which the `INSTEAD OF`
    triggers of the view answer. Built, which D-253 records.
34. `changes()`, `total_changes()` and `last_insert_rowid()`, which the
    connection carries. Built, which D-254 records.
35. The four bytes a cell takes of a page at the least. Built, which
    D-255 records.
36. `INSERT ... ON CONFLICT` over a table that keeps its rows in the
    key's own tree. Built, which D-256 records.
37. The types a `STRICT` table holds. Built, which D-257 records.
38. The columns a `GENERATED ALWAYS AS` computes, written down where
    the column is `STORED`. Built, which D-258 records.
39. `median`, `percentile`, `percentile_cont` and `percentile_disc`,
    and the `WITHIN GROUP` clause that names their values. Built, which
    D-259 records.
40. The digit separators a number is written with, and the words a
    token the tokenizer read as no token at all is refused with. Built,
    which D-260 records.
41. `UPDATE ... FROM`, and the `WITH` before such a statement. Built,
    which D-261 records; `UPDATE` and `DELETE` with an `ORDER BY` and a
    `LIMIT` are still open, which the suite reads as
    `update_delete_limit`.
42. The foreign keys a transaction is held to at its end. Built, which
    D-262 records.
43. The words a window is refused with. Built, which D-263 records.
44. The words an aggregate written where no group has been made is
    refused with. Built, which D-264 records.
45. A name more than one side of a `FROM` answers to. Built, which
    D-265 records.
46. The names a statement answers its columns under, which two pragmas
    move. Built, which D-266 records.
47. The words a compound is refused with, and which column a term of
    its `ORDER BY` counts to. Built, which D-267 records.
48. What an index entry holds, what a `CREATE INDEX` and a `DROP INDEX`
    are held to, and what `ON CONFLICT ROLLBACK` undoes. Built, which
    D-268 records.
49. When a foreign key is located, and what a `CREATE TABLE` that
    writes one is held to. Built, which D-269 records.
50. The functions an application defines on a connection, and the count
    of pages `PRAGMA max_page_count` holds the file to. Built, which
    D-270 records. An aggregate the application defines, which
    `md5sum` of `testfixture` is, is still open.
51. The journal mode of a connection, and how many columns a statement
    an `IN` looks in answers. Built, which D-271 records.
52. Whether a text ends a statement, and where the place of the nulls
    may be written. Built, which D-272 records.
53. The collations an application defines on a connection. Built, which
    D-273 records. A schema that names a collation the connection was
    not told of is refused where the schema is read, and not where a
    comparison reaches that column, so `SELECT * FROM t` over such a
    table is refused where the C library answers its rows.
54. A function an application defines for any number of arguments, and
    a `COLLATE` written on a whole number of an `ORDER BY` or a
    `GROUP BY`. Built, which D-274 records. A proc the harness calls
    may not run a statement of its own.
55. The same statements under every page size, every encoding and every
    journal mode a connection opens under. Built, which D-275 records.
56. One place that writes text into the encoding the file names, which
    is `record::write_in`. Built, which D-277 records.
57. The room the parent of a `balance_quick` has, counted before the
    routine is chosen. Built, which D-278 records.
58. The schema a constraint reads, kept beside the cookie it was read
    under. Built, which D-279 records.
59. The words a join is written with, and what the `ON` of an outer
    join may name. Built, which D-281 records. An `ON` of an inner join
    that names a table read after it is still refused `no such column`,
    where the C library reads such an `ON` as a `WHERE`.
60. What a trigger may not carry: a variable, and a schema in front of
    the table a write of its body names. Built, which D-282 records.
61. The second argument of `likelihood`, the register `#1` names, and
    what a column that points may fall back to. Built, which D-283
    records.
62. What the terms of a `WITH` may read. Built, which D-284 records. A
    term that reads itself inside a subquery is still answered rather
    than refused `circular reference`, and a term with more than one
    recursive reference is not refused for that.

### Done when

Every statement of the recorded corpora is accepted or refused as the C
library accepts or refuses it, with no count of what is waiting.

## 16.23 Q9. The suites run whole

Status: `sh tools/xtask.sh sqlite-suite` runs SQLite's own test files
under the `tclsh` of the machine. Of 83 479 cases in 719 files, 71 878
pass, 2373 answer differently, and 9228 name something the engine
refuses or a command that needs the C library's internals. Twenty files
reach the sixty-second deadline and are counted with the cases they ran
by then, four of them cut at a different case each run, so the counts
move by some tens between runs.
Depends on: Q7, Q8. Recorded in D-201, D-212, D-213, D-220, D-222,
D-223 and D-224.
Size: M.

### Needs

- An adapter that speaks the commands `testfixture` drives, which is
  `tools/suite/tester.tcl`: this repository's own tester, read by every
  file, reaching the engine over a socket. Built, which D-223 records.
- The matrix of 16.11 in the test support. Built.

### Does

1. Run SQLite's TCL suite under `research/sqlite/test` against the
   engine. Built.
2. Run every level of this repository's own suite across the matrix
   rather than the format alone. Built for the write path.
3. Drive differential execution from `norec`'s generator with the engine
   as the second implementation.

### How a case is counted

A file keeps one database per path a connection opened, at the page size
of 4096, and `tclsh` runs every command of the file. A statement is run
through the connection that reads or the one that writes by its first
word, which is read with the comments taken out, and a `WITH` clause
carries a statement that writes as well, so the words after it say
which. A real is written with the fifteen significant digits
`tcl_precision` holds.

A case is counted passed where what it answered is what the file writes,
compared as `do_test` of the suite's own tester compares it: `/RE/` is a
regular expression, `~/RE/` one that must not match, `#/A..B/` a range,
`*GLOB*` a pattern, and anything else the text itself, with a token that
is a number compared to fifteen significant digits. A case the engine
refused a statement of is refused and not failed, because the rows a
later case reads are then short. A file runs in a process of its own and
is ended after sixty seconds, with the cases it ran counted;
`process::test_jobs` files run beside each other.

The deadline is wall-clock, so the score of a file it ends is what that
file reached in sixty seconds and not a fixed number. Twenty files reach
it, and `alterdropcol.test`, `rowvalue2.test`, `savepoint6.test` and
`with1.test` are cut at a different case each run: two runs one after
another differ by 53 cases of 74 028. A total of the suite is therefore
exact to about a tenth of a percent, which is what a comparison of two
runs must allow for.

`--why` counts what each refusal was for, by the first two words of the
statement and what the engine answered, which is what says which missing
feature stops the most files.

### Done when

The TCL suite reports no failure that is not a documented omission.

## 16.24 Q10. Coverage to the standard of D4

Status: built for `db-sqlite`, which is at 100 percent of lines and 100
percent of branches under both instrumentations and breaks neither
premise of D4, so the crate meets unique-cause MC/DC by that argument.
What is left is every further crate of the port as it is written.
Depends on: Q9.
Size: S.

### Does

1. Hold every crate of the port to 100 percent of lines and branches in
   both instrumentations. Built.
2. Hold every decision of those crates to the short-circuit operators,
   which is what makes condition coverage masking MC/DC. Built.
3. Hold every decision to naming no condition twice, which is what
   carries masking MC/DC to unique-cause MC/DC. Built.

### Done when

`sh tools/xtask.sh coverage`, `sh tools/xtask.sh coverage --condition`
and `sh tools/xtask.sh mcdc` each report no violation for every crate of
the port.

## 16.25 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | The C library changes under the port. | A golden file records an answer the current library no longer gives. | `sh tools/sqlite.sh` pins tag `version-3.53.4`. A change of tag regenerates every golden and shows as a diff. |
| 2 | The walker of Q5 and the machine of Q6 disagree. | Two answers, and no way to say which is SQLite's. | The recorded corpora are the third party. Both are compared against the golden, not against each other. |
| 3 | A refusal is added to reach a green check rather than because SQLite refuses. | The engine answers less and the count of refusals hides it. | The count in `tests/db.rs` is asserted and may only fall. |
| 4 | Coverage is met by deleting a test's reach rather than by reaching. | A branch counted covered by one input that no file produces. | D4 forbids exemptions. A branch no input reaches is deleted, which shows in the diff as deleted code. |
| 5 | The matrix is a `for` loop copied into each test. | A dimension added in one test and forgotten in ten. | 16.11 puts the matrix in the test support and has the test name its dimensions. |
| 5 | The fuzz corpora grow until the regression replay is slow. | The check takes longer than three minutes and is skipped. | `sh tools/xtask.sh fuzz --merge` keeps one input per feature. Hash-named files are not committed; named regression entries are. |
| 6 | MC/DC never becomes measurable on the pinned toolchain. | Goal 5 of 16.2 cannot be met by reading a report. | D4 derives MC/DC from condition coverage and checks both of its premises with `cargo xtask mcdc`, so the goal is met by argument and by check rather than by a report the pin does not emit. |
| 7 | A `RIGHT` or a `FULL` join reads every row of every side against every row of the sides before it, because a row of such a join is marked matched where the levels under it are read, so no term of the `WHERE` may be read above it. | `joinD.test` reaches the sixty-second deadline and is counted with the cases it ran, as nineteen other files are. | D-208 reads every other term on the level that answers it, and reads the side of an `ON` by an index. What is left is marking a row by its key rather than by where it stands, which lets such a side be read by an index as well. |
