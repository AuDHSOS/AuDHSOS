# 18. More than one database on one connection

## 18.0 How to read this document

Every step below has the same parts, in this order: Status, Depends on,
Size, Needs, Does, Produces, Done when. A step that creates something
nameable carries Produces. Nothing is implied. Every term is defined in
18.1 and has exactly one name, used everywhere.

## 18.1 Terms

| Term | Meaning |
|------|---------|
| connection | One `crate::change::Writer`, which runs the statements of one client and holds what the client was told. |
| reader | One `crate::db::Database`, built over the bytes a connection holds, which answers the rows of one statement. |
| image | The bytes of one SQLite file, header first. |
| held file | One image a connection writes, with the pages, the header and the journal beside it. `main` is one; each `ATTACH` adds one. |
| schema name | The name a statement writes in front of a table: `main`, `temp`, or the name an `ATTACH` gave. |
| schema place | Where a held file stands in the connection's list: 0 for `main`, 1 for `temp`, 2 and up in the order the `ATTACH` statements ran. `sqlite3FindTable` of `research/sqlite/src/build.c:337` reads `db->aDb` at that place. |
| root page | The page a table's tree begins at, which names a tree of one held file only. |
| opening function | The function the client tells the connection, which answers the image of a file name or nothing. `ATTACH` calls it. |
| master journal | The file SQLite writes the names of the journals into where one transaction writes more than one held file, which `research/sqlite/src/pager.c` calls the super-journal. |
| temp schema | The held file `CREATE TEMP TABLE` writes into, which stands at schema place 1 and which no file name names. |

## 18.2 Goal

At the end of this track, five things are true that are not true now:

1. `ATTACH 'file' AS name` adds a held file to a connection, and `DETACH
   name` takes it away.
2. A statement reads and writes `name.table` for every held file, and a
   statement that names no schema reads the temp schema first, `main`
   second and the attached files in the order they were attached.
3. One transaction writes every held file it touched, or none of them.
4. `CREATE TEMP TABLE` writes into the temp schema, and
   `sqlite_temp_schema` answers its rows.
5. The suite's own harness runs the 151 files of SQLite's own corpus
   that write `ATTACH`.

## 18.3 What is already built

| Piece | What it does | Decided in |
|-------|--------------|------------|
| `crate::change::Writer` | One connection over one image: the pages, the header, the journal, the savepoints and what the client was told. | D-162 |
| `crate::db::Database` | One reader over one image: its tables, views and triggers. | D-172 |
| `crate::tree` | The B-tree over a page source, which every read and every write of a row goes through. | D-165 |
| `crate::journal` | The rollback journal of one file, and the log of one file in write-ahead logging. | D-190, D-262 |
| `crate::pragma::HELD` | What a connection was told for each pragma that holds a value. | D-206 |
| `crate::auth::Asked` | The read of one statement, which carries the schema name of every table it names. | D-313 |
| `suite::Session` | One writer per path, and the connections that read each path. | D-201, D-213 |

Two facts of the present code bound the work:

| Fact | Where |
|------|-------|
| A schema name other than `main` names no table. | `crates/db/sqlite/src/db.rs:2746` |
| `CREATE TEMP TABLE` writes into the one file the connection holds. | `crates/db/sqlite/src/change.rs:2744` |

## 18.4 What is missing

| # | What is missing | Why it matters |
|---|-----------------|----------------|
| 1 | The parser reads no `ATTACH` and no `DETACH`. | 118 cases of SQLite's own files stop at the first one, and 151 files write one. |
| 2 | A connection holds one image and one header. | A second held file has a page count, a schema cookie and a journal of its own. |
| 3 | A root page names no held file. | Two tables of two held files may carry the same root page. |
| 4 | The reader resolves a bare name in one schema. | `sqlite3FindTable` reads the temp schema, then `main`, then the attached files. |
| 5 | A commit writes one file. | A transaction over two held files writes both or neither. |
| 6 | The temp schema does not exist. | `sqlite_temp_schema` answers no rows, and a temporary table of one connection is read by another. |
| 7 | The client hands the connection one image at `Writer::opened`. | `ATTACH` names a file the client has to open. |

## 18.5 Decision D1: the client answers the file name

**The decision: `ATTACH 'file' AS name` calls an opening function the
client told the connection, which answers the image of that file name or
nothing.**

Reason 1: this crate reads no file system, so the bytes of a file reach
it only from its client. `Writer::opened` already takes the image of
`main` that way.

Reason 2: the opening function is a `fn` pointer, which is what
`crate::auth::Asking` and `crate::func::Defined` already are, so a
connection carries it without an allocation.

Reason 3: a name the function answers nothing for is refused `unable to
open database: <file>`, which is `attachFunc` of
`research/sqlite/src/attach.c:268`.

The option not taken is a connection that takes every image it may ever
attach at `Writer::opened`. Cost: the client cannot answer a file name a
statement computes, and `ATTACH` of a name that was not listed would be
refused a file that exists.

## 18.6 Decision D2: a held file is one struct, and a connection holds a list of them

**The decision: the fields of `Writer` that belong to one file move into
one struct `HeldFile`, of which `Writer` holds the one at schema place 0
by itself and the rest in a list.**

Reason 1: `main` and an attached file differ in nothing but their place,
so one struct holds both and no branch tells them apart.

Reason 2: schema place 0 is there for every connection, so the type says
so rather than a lookup answering nothing for it.

Reason 3: the list is read in the order the files were attached, which
`sqlite3FindTable` reads `db->aDb` in as well.

The option not taken is a second `Writer` per attached file. Cost: a
transaction, a savepoint and the counters of a connection belong to the
connection and not to one file, so each would have to be held outside the
`Writer` that carries them now.

## 18.7 Decision D3: a root page carries its schema place

**The decision: every root page a reader or a connection carries becomes
a pair of the schema place and the page number.**

Reason 1: two held files carry the same page numbers, so a root page
alone names no tree.

Reason 2: the pair is read where the tree is read, so a wrong place is a
refusal and not a row of another file.

The option not taken is a page number that counts on past the end of the
first file. Cost: the number a file holds in `sqlite_schema` would differ
from the number the reader carries, and every write of the schema would
have to add and take away the offset.

## 18.8 Decision D4: the temp schema is a held file of its own

**The decision: the temp schema is the held file at schema place 1, made
empty when the first `CREATE TEMP TABLE` runs, and dropped when the
connection closes.**

Reason 1: `sqlite3FindTable` reads schema place 1 for a bare name before
`main`, so a temporary table of the same name as a table of `main` stands
in front of it.

Reason 2: `sqlite_temp_schema` is the schema table of that file, so it
answers the temporary tables and nothing else.

Reason 3: a temporary table is not in the bytes the client holds for
`main`, so another connection over the same path does not read it.

The option not taken is the temp schema written into `main` with a mark
on each row. Cost: `sqlite_schema` of `main` would answer rows the C
library does not write there, and every reader of the schema would have
to filter them.

## 18.9 Decision D5: a transaction over two held files writes a master journal

**The decision: a commit that writes more than one held file writes the
journal of each file first, then a master journal naming them, and then
the files.**

Reason 1: `sqlite3PagerCommitPhaseOne` of the C library writes the
super-journal for the same reason: a machine that stops between two files
leaves a name that says which journals to replay.

Reason 2: the suite's own harness replays journals to check them, so a
transaction over two files is checked the same way as one over one file.

The option not taken is a commit that writes each file as if it stood
alone. Cost: a run that stops between the two files leaves one file
committed and one not, which no replay repairs.

## 18.10 The order of the steps

| Step | What | Status | Depends on | Size |
|------|------|--------|------------|------|
| A1 | One held file, as one struct | done | — | M |
| A2 | `ATTACH` and `DETACH` read | open | A1 | M |
| A3 | A reader over more than one image | open | A2 | L |
| A4 | A statement that writes an attached file | open | A3 | M |
| A5 | One transaction over more than one held file | open | A4 | M |
| A6 | The temp schema | open | A3 | M |
| A7 | The harness over more than one path | open | A4 | S |

## 18.11 A1. One held file, as one struct

### Status

done.

### Depends on

Nothing.

### Size

M.

### Needs

- `crate::change::Writer` as it stands, with the fields that belong to
  one file listed: `pages`, `schema_bytes`, `header`, `mode`, `nonce`,
  `wanted_page`, `sector`, `journal`, `log`, `origin`, `restarting` and
  `began`.

### Does

1. Move those fields into a struct `HeldFile`.
2. Hold one `HeldFile` in `Writer` under the field `held`, which is the
   file at schema place 0.
3. Read every use of `self.pages` and `self.header` as a use of that
   field.

### Produces

`HeldFile` in `crates/db/sqlite/src/change.rs`.

### Done when

`sh tools/xtask-check.sh` exits 0 and the suite answers the same counts
as before the step, because the step changes no behavior.

## 18.12 A2. `ATTACH` and `DETACH` read

### Status

open.

### Depends on

A1.

### Size

M.

### Needs

- A1.
- `crate::parse`, which reads every other statement of the language.

### Does

1. Read `ATTACH ?DATABASE? expr AS name` and `DETACH ?DATABASE? name`
   into `crate::ast::Definition`.
2. Answer the file name by evaluating the expression, which
   `sqlite3Attach` does as well.
3. Call the opening function of D1 with that file name, and add a
   `HeldFile` at the end of the list under the name the statement gave.
4. Refuse `too many attached databases - max 10` past ten attached files,
   `database <name> is already in use` for a name the list holds,
   `database is already attached` for a file name the list holds, and
   `attached databases must use the same text encoding as main database`
   for an image whose encoding differs from the one at schema place 0.
5. Refuse `no such database: <name>` for a `DETACH` of a name the list
   does not hold and `cannot detach database <name>` for `main` and for
   `temp`.
6. Answer the list from `PRAGMA database_list`, one row per held file
   with the schema place, the name and the file name.
7. Refuse `cannot ATTACH database within transaction` inside a
   transaction, which `sqlite3Attach` of
   `research/sqlite/src/attach.c:391` refuses.

### Produces

`crate::ast::Attach` and `crate::ast::Detach`.

### Done when

`PRAGMA database_list` answers two rows after one `ATTACH`, and one after
the `DETACH` that follows it.

## 18.13 A3. A reader over more than one image

### Status

open.

### Depends on

A2.

### Size

L.

### Needs

- A2.
- `crate::db::Database`, which holds one image and the tables, views and
  triggers read out of it.

### Does

1. Move the image, the tables, the views and the triggers of
   `Database` into a struct `Schema`, with the name and the schema place
   beside them.
2. Hold `Vec<Schema>` in `Database`, built from the held files of the
   connection.
3. Answer `Database::table(schema, name)` for a name a statement wrote a
   schema in front of, and `Database::table(name)` for a bare name by
   reading schema place 1, then 0, then 2 and up, which is
   `sqlite3FindTable` of `research/sqlite/src/build.c:373`.
4. Carry the schema place beside every root page, which D3 decides.
5. Answer `<name>.sqlite_schema`, `<name>.sqlite_master` and
   `sqlite_temp_schema` out of the schema they name.
6. Refuse `no such table: <schema>.<name>` for a schema the list holds
   and a table it does not, and for a schema the list does not hold.
7. Read a trigger and a view of the schema the table stands in.

### Produces

`Schema` in `crates/db/sqlite/src/db.rs`.

### Done when

`SELECT * FROM aux.t1 JOIN main.t1` answers the rows of both files, and
`SELECT * FROM t1` answers the rows of `main` where both files hold a
`t1`.

## 18.14 A4. A statement that writes an attached file

### Status

open.

### Depends on

A3.

### Size

M.

### Needs

- A3.

### Does

1. Read the schema name of every statement that writes, and write the
   held file it names.
2. Write `CREATE TABLE aux.t`, `DROP TABLE aux.t`, `CREATE INDEX`,
   `CREATE VIEW` and `CREATE TRIGGER` into the schema they name.
3. Count the rows a statement wrote once for the connection, whichever
   held file they stand in, which `changes()` answers.
4. Refuse a foreign key whose parent stands in another held file, which
   is `sqlite3FkLocateIndex` refusing `foreign key mismatch`.
5. Answer `last_insert_rowid()` for the last row written, whichever held
   file it stands in.

### Done when

`INSERT INTO aux.t1 VALUES(1)` writes the image of `aux` and leaves the
image of `main` as it was.

## 18.15 A5. One transaction over more than one held file

### Status

open.

### Depends on

A4.

### Size

M.

### Needs

- A4.
- `crate::journal`, which writes the rollback journal of one file.

### Does

1. Open a transaction on every held file a statement writes, and not on
   the ones it does not.
2. Write the journal of each held file the transaction wrote, then the
   master journal of D5, then the files.
3. Roll back every held file the transaction wrote where one of them
   refuses.
4. Hold a savepoint as the pages and the header of every held file,
   which is one `Saved` carrying a list rather than one pair.
5. Answer `sqlite3_get_autocommit` for the connection and not for one
   held file.

### Done when

A `ROLLBACK` after a write into two held files leaves both as they stood
at the `BEGIN`.

## 18.16 A6. The temp schema

### Status

open.

### Depends on

A3.

### Size

M.

### Needs

- A3.

### Does

1. Make an empty held file at schema place 1 when the first
   `CREATE TEMP TABLE`, `CREATE TEMP VIEW`, `CREATE TEMP TRIGGER` or
   `CREATE TEMP INDEX` runs.
2. Write a temporary table into that held file, and read a bare name out
   of it first.
3. Answer its rows from `sqlite_temp_schema` and from
   `sqlite_temp_master`.
4. Answer `temp` as the schema name of a temporary table where the
   authorizer is asked.
5. Leave the held file out of what `Writer::written` answers, because no
   file on the client's side holds it.

### Done when

`CREATE TEMP TABLE t(a)` leaves `sqlite_schema` of `main` with no row,
and `sqlite_temp_schema` with one.

## 18.17 A7. The harness over more than one path

### Status

open.

### Depends on

A4.

### Size

S.

### Needs

- A4.
- `suite::Session`, which holds one writer per path.

### Does

1. Tell every connection an opening function that answers the bytes of
   the writer the session holds for that path.
2. Write the bytes of every attached held file back into the writer of
   that path after each statement, where the connection holds no open
   transaction over it.
3. Read the bytes of every attached held file out of the writer of that
   path before each statement, under the same condition.
4. Take `ATTACH` and `DETACH` out of the statements the harness counts as
   ones the engine does not read.

### Done when

SQLite's own `attach.test` runs to its end.

## 18.18 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | A1 touches 96 uses of `self.pages` and 153 of `self.header`. | One wrong place reads the wrong file. | A1 changes no behavior, and the compiler refuses a field that is no longer where it was. |
| 2 | A3 carries a schema place beside every root page. | A root page read without its place names a tree of the wrong file. | The pair is one type, so a bare page number does not compile where a pair is wanted. |
| 3 | A5 writes a master journal the replay of the harness does not know. | A journal the harness replays leaves a file the run did not write. | The replay reads the master journal first, which names the journals it then replays. |
| 4 | A6 holds a file no client writes. | A connection that closes loses the temporary tables, which is what the C library does as well. | `Writer::written` answers the image of `main` only, which A6 states. |
| 5 | The 151 files that write `ATTACH` also write other commands the harness refuses. | The count rises by less than the refusals the steps take away. | A7 measures `attach.test` alone before the other 150 files are counted. |
