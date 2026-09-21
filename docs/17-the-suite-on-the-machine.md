# 17. The suite on the machine

## 17.0 How to read this document

Every step below has the same parts, in the same order: Status, Depends
on, Size, Needs, Does, Produces, Done when. Nothing is implied. Every
term is defined in 17.1. Every term has exactly one name, used
everywhere.

## 17.1 Terms

| Term | Meaning |
|------|---------|
| the suite | The 1190 files under `research/sqlite/test/` that the C library is tested with. `research/` is not part of this repository; `sh tools/sqlite.sh` brings it. |
| the harness | `crates/tools/xtask/src/suite.rs`, which runs a file of the suite and scores it. |
| the engine | `db-sqlite`, the crate this repository tests against the suite. |
| a case | One `do_test`, `do_execsql_test` or `do_catchsql_test`, which names itself, runs a script, and says what the script answers. |
| passed | The engine answered a case what the file says. |
| failed | The engine answered a case something else. |
| refused | The engine refused a statement of a case, so the case answered nothing to compare. |
| the interpreter | `tclsh`, the TCL interpreter of the machine the harness runs on. It is not part of this repository. |
| the tester | `tools/suite/tester.tcl`, which this repository holds: the commands a file of the suite drives, written to reach the engine. |
| the runner | `tools/suite/runner.tcl`, which the interpreter is started with: it opens the line to the harness, reads the tester, and reads one file of the suite. |
| the line | A TCP connection over the loopback address, which the harness listens on and the runner connects to. Every request the tester makes and every answer the harness gives goes over it. |
| a connection | One database the tester holds under a name. `sqlite3 db test.db` makes one; `db eval` reads it. |
| a held file | The bytes of one database, which the harness holds in memory under the path the suite opened it by. Two connections to one path read one held file. |
| `testfixture` | The program SQLite's own suite is driven by, which is the C library with a TCL interpreter and eighty commands of its own linked into it. This repository has no such program. |

## 17.2 Goal

At the end of the track, three things are true that are not true now:

1. Every command of a file of the suite runs, so a case is refused only
   where the engine refused a statement.
2. Every case of every file is scored: passed, failed or refused.
3. The harness reports which command a file stopped at, so the count of
   files that stop says what to write next.

## 17.3 What is already built

| Piece | What it does | Decided in |
|-------|--------------|------------|
| `suite::run` | Reads a file into steps, runs the steps, scores the cases. | D-201, D-213 |
| `suite::answer` | Runs the statements of one case against the engine and writes the answer as `execsql` writes one. | D-201 |
| `suite::elements` | Reads a TCL list into its words. | D-201 |
| `db_sqlite::change::Writer` | A connection that writes, held across the steps of a file. | D-162 |
| `db_sqlite::db::Database` | A connection that reads, opened over the bytes the writer holds. | D-172 |
| `process::Cmd` | Runs a program and reads what it writes. | Phase 2 |
| `suite::CONFIGURATIONS` | The nine page-size, encoding and journal-mode configurations `--configuration` opens a connection under. | D-275 |
| `suite::Prepared` | One statement the tester prepared, which it steps and reads the columns of. | D-314 |
| `db_sqlite::auth::Authorizer` | The reading of one statement against the function `sqlite3_set_authorizer` told the connection. | D-313 |
| `suite::opening` | The image of the file an `ATTACH` names, out of what the session holds. | D-326 |

Before this track the harness read nine commands of a file and counted
every other command as one it could not run: 1171 files held 17 724
cases it knew about, and 13 086 of them were refused because a step
before them was such a command. Running the files under `tclsh` makes
118 450 cases of 847 files: 107 885 pass, 3527 answer differently and
7038 are refused.

What the 7038 refusals are for, most first: `sqlite3_memdebug_fail`
(1261), which fails one allocation of the C library; `crash_on_write`
(960) and the crash the harness does not simulate (435); a table an
earlier refusal left unmade (683); `EXPLAIN` and `EXPLAIN QUERY PLAN`
(634), which name the program a statement compiles to; a connection an
earlier case left inside a transaction (632); a statement the engine
does not read (203); `sqlite3_quota_glob` (108), which counts the bytes a
file may take; and `db format` (105), which writes a row the way the
shell writes it. `sqlite3_stmt_readonly` was among them until D-338,
`REGEXP` until D-362 and `db status` until D-365, and none is.

`--configuration` opens every connection of a run under one of nine
page-size, encoding and journal-mode settings, which D-275 decides.
What each answers, over the same files:

| Configuration | Passed | Answered differently | Refused |
|---------------|-------:|---------------------:|--------:|
| `utf8-4096-delete` | 107 885 | 3527 | 7038 |
| `utf16le-4096-delete` | 61 214 | 2392 | 9500 |
| `utf16be-4096-delete` | 61 229 | 2392 | 9500 |
| `utf8-512-delete` | 61 073 | 2452 | 9435 |
| `utf8-1024-delete` | 107 685 | 3439 | 7136 |
| `utf8-65536-delete` | 61 042 | 2358 | 9446 |
| `utf8-4096-persist` | 61 574 | 2372 | 9504 |
| `utf8-4096-truncate` | 61 311 | 2370 | 9501 |
| `utf8-4096-wal` | 59 759 | 2490 | 10 294 |

The counts move between runs of one configuration only where a file
reaches the deadline, which D-302 sets at three minutes. The first row
and the fourth are runs of 2026-09-21; the other seven were measured
before D-353 and are lower than they would read now.
`testfixture` is built with `SQLITE_DEFAULT_PAGE_SIZE=1024`, which
`main.mk` line 1784 sets, so the fourth row is the page size the files
were written for, and it answers 200 cases fewer than the first because
four files that answer tens of thousands of rows reach the deadline
sooner at that size.

One row is behind the others by more than that: `utf8-4096-wal` refuses
684 more than `utf8-4096-delete`, which a connection in write-ahead
logging reads its newest pages out of the log for.

## 17.4 What is missing

| # | What is missing | Why it matters |
|---|-----------------|----------------|
| 1 | Every command of TCL: substitution, `expr`, `if`, `for`, `foreach`, `proc`, `catch`, the list and string commands. | 2927 `set`, 1030 `proc`, 650 `foreach` and 484 `if` commands stand between a file's first line and its cases. |
| 2 | The commands `tester.tcl` defines. | Every file reads it, and nothing else defines `do_test`. |
| 3 | Connections by name. | A file opens `db`, `db2` and `db3` over one path and reads what each wrote. |
| 4 | The errors the engine raises, as text. | 1224 `do_catchsql_test` cases compare the message. |

## 17.5 Decision D1: the interpreter is `tclsh` and not one written here

**The decision: the harness starts `tclsh` and speaks to it.**

Reason 1: the suite is TCL, and a file drives the whole language: a
`proc` that a `foreach` calls with a `$var` an `expr` counted. An
interpreter written here answers a subset, and every file that leaves
the subset scores its cases wrong rather than refusing them.

Reason 2: `tclsh` is on the machine already, and the `sqlite-suite`
subcommand is never a step of `cargo xtask check`, which is the same
reason the suite itself may live outside this repository.

Reason 3: a probe of this design ran 983 of the 1190 files of the suite
to their last command against a tester of two hundred lines.

**The option not taken: a TCL interpreter in `xtask`.** It is the words,
the substitutions, `expr`, the control commands, the list commands, the
string commands and `format`, which is a language and not a feature,
and every one of them is a way to score a case wrong.

## 17.6 Decision D2: the tester is written here and not taken

**The decision: `tools/suite/tester.tcl` is this repository's own, and
the suite's `tester.tcl` is never read.**

Reason 1: the suite's own tester is 2626 lines and calls eighty
commands `testfixture` links in, which are the C library's internals:
`sqlite3_soft_heap_limit`, `sqlite3_test_control`, `testvfs`. A
repository with no such program cannot read it.

Reason 2: a file of the suite reads the tester by `source
$testdir/tester.tcl`, and `$testdir` is whatever the runner sets, so a
tester of this repository's own is read by every file without a file of
the suite being edited.

Reason 3: the tester says what the harness answers. A command that
needs the C library's internals is a `proc` that answers nothing, and
the file runs on.

**The option not taken: reading the suite's tester and stubbing what it
calls.** The stubs are the same eighty commands, and the 2626 lines
around them run code this repository does not have.

## 17.7 Decision D3: the line is a socket and not the standard streams

**The decision: the harness listens on the loopback address and the
runner connects to it.**

Reason 1: a file of the suite writes to standard output — `puts
"Skipping ..."` — and a request written to the same stream is read as
that text.

Reason 2: `tclsh` opens a socket in one command, and the harness
listens with `std::net::TcpListener`, so neither side needs a named
pipe, which the standard library cannot make.

**The option not taken: a file descriptor of its own.** The standard
library cannot hand a child an inherited descriptor without `unsafe`,
which `docs/04-safety-policy.md` refuses.

## 17.8 Decision D4: what a connection reads

**The decision: the harness holds one set of bytes per path, and every
connection over that path reads and writes those bytes.**

Reason 1: a file opens `db` and `db2` over `test.db` and reads through
one what the other wrote, which is what a second connection is for.

Reason 2: the engine's writer holds the bytes of one database, so a
path names a writer and a connection names a path.

**The option not taken: one writer per connection.** Two connections
over one path would then answer two databases, and every case that
opens a second one would read nothing.

## 17.9 The order of the steps

| Step | What it adds | Status | Depends on | Size |
|------|--------------|--------|------------|------|
| T1 | The line: the harness listens, the runner connects, and a request is answered. | built | nothing | M |
| T2 | The tester: the commands a file drives. | built | T1 | M |
| T3 | The engine behind the line: connections, held files, and the answers. | built | T1 | M |
| T4 | The score: a case is passed, failed or refused, a file that stops says where, and a file that refuses the same reason 5000 times in a row is ended. | built | T2, T3 | S |
| T5 | The commands that need the C library's internals answer nothing, and the helper files the suite reads are this repository's own. | built | T2 | S |
| T6 | The messages the engine refuses with are the ones the C library writes, which a `do_catchsql_test` compares. | open | T3 | M |

## 17.10 T1. The line

Status: built.
Depends on: nothing.
Size: M.

### Needs

- `socket.n` of TCL 8.6, for the client side.
- `std::net::TcpListener`, for the harness side.

### Does

1. The harness binds the loopback address on a port the machine
   chooses, and starts `tclsh` with the runner, the file to read and
   the port.
2. The runner connects, and every request of the tester goes over the
   line.
3. A request is a verb, a count, and that many values, each written as
   its length and its bytes.
4. An answer is `OK`, a count and that many values, or `ERR`, a length
   and a message.
5. A real the tester writes into a statement for a variable carries
   every digit the bits of the double hold, because `testfixture` hands
   the C library the double itself and never the fifteen digits
   `tcl_precision` prints.
6. A method of the connection is matched by any beginning of its name
   that names exactly one method, which is what TCL does for every
   command and which SQLite's own files rely on.
7. An answer may be preceded by `CALL`, what kind of proc is called, a
   count and that many values, which the tester answers with `RET`, a
   count and that many values; the harness writes one where a collation
   or a function the tester defined is reached from inside a statement.
8. A proc the tester runs for a `CALL` may write no request of its own,
   because the answer to the call is read from that line, so the tester
   refuses one.
9. The harness stops reading when the runner closes the line or the
   deadline passes, and ends the interpreter either way.

### Produces

`tools/suite/runner.tcl`, and the line in
`crates/tools/xtask/src/suite.rs`.

### Done when

A file of one case runs, and the harness reads the case's name and what
it answered.

## 17.11 T2. The tester

Status: built.
Depends on: T1.
Size: M.

### Needs

- `research/sqlite/test/tester.tcl`, for the names and the arguments of
  the commands, which this repository answers rather than reads.

### Does

1. `sqlite3 NAME FILE`, which makes a connection and a command of that
   name.
2. `NAME eval SQL`, `NAME eval SQL ARRAY BODY`, `NAME close`, `NAME
   one`, `NAME onecolumn`, `NAME changes`, `NAME last_insert_rowid`,
   `NAME nullvalue`.
3. `execsql`, `catchsql`, over the connection called `db`.
4. `do_test`, `do_execsql_test`, `do_catchsql_test`, which score a
   case.
5. `reset_db`, `forcedelete`, `forcecopy`, `finish_test`,
   `integrity_check`.
6. `ifcapable`, which answers what this engine has.

### Produces

`tools/suite/tester.tcl`.

### Done when

`window3.test` scores its 1222 cases through the interpreter.

## 17.12 T3. The engine behind the line

Status: built.
Depends on: T1.
Size: M.

### Does

1. Hold one writer per path and one path per connection name.
2. `open`, `close`, `delete`, `copy` over the held files.
3. `eval`, which runs the statements of a text in order: a statement
   that reads is answered by a reader over the bytes the writer holds,
   and every other statement goes to the writer.
4. `names`, which answers the names of the columns a statement answers.
5. A refusal of the engine is an `ERR` with the text of it, which
   `catchsql` reads.

### Produces

The rewritten `crates/tools/xtask/src/suite.rs`.

### Done when

A file that opens `db` and `db2` over one path reads through `db2` what
`db` wrote.

## 17.13 T4. The score

Status: built.
Depends on: T2, T3.
Size: S.

### Does

1. A case that the engine refused a statement of is refused, not
   failed.
2. A file that stops at a command the tester does not have is counted
   with that command's name, which `--why` answers.
3. `--show` writes what a case answered and what the file wanted.
4. A file that refuses 5000 cases in a row for the same reason is ended
   there, because a loop whose end a command the harness has none of
   decides runs without bound.
5. A file that stops is counted by the first line of what it stopped at,
   cut to 72 characters, which names the command the file wanted.

### Produces

The score in `crates/tools/xtask/src/suite.rs`.

### Done when

The table `sqlite-suite` writes holds one line per file, and the totals
are of every case of every file the interpreter ran.

## 17.14 T5. The commands that need the C library

Status: built.
Depends on: T2.
Size: S.

### Does

1. A command that reads the C library's internals is a `proc` that
   answers nothing: `sqlite3_test_control`, `testvfs`,
   `sqlite3_soft_heap_limit` and the seventy-odd beside them.
2. The helper files the suite reads — `malloc_common.tcl`,
   `lock_common.tcl`, `wal_common.tcl`, `fts3_common.tcl` — are this
   repository's own, under `tools/suite/`, and hold the same kind of
   `proc`.
3. A command of `testfixture` that reads nothing of the C library but
   its own code reaches this engine's code instead:
   `btree_varint_test` reaches `bytes::varint_again`, the
   `sqlite3_mprintf_*` family reaches `format::format`, and
   `save_prng_state` and `restore_prng_state` reach the random source of
   every writer the file holds.
4. A command of that family hands the format the C type it names, so the
   harness reads `%x`, `%X`, `%o` and `%u` of an `int` argument as the
   same 32 bits without the sign and `%c` as the character the number
   names, which is what `va_arg` of `sqlite3_str_vappendf` reads.
5. The limits of `src/sqliteLimit.h` are variables of the tester, each
   this engine's own limit where the engine holds one, so a file that
   reads `$SQLITE_MAX_LENGTH` to skip a case runs.
6. `file size` and `file exists` over a database the harness holds, the
   log beside it or the journal beside it answer out of the harness,
   which the `size` request asks for, and every other name reaches TCL's
   own `file`.
7. `do_multiclient_test` runs the round where all three connections
   stand in this interpreter, because the harness holds one writer per
   file and as many connections over it as a file opens.
8. A command that names something this engine holds none of answers
   rather than raises where the answer changes nothing a case reads:
   the size of a page cache, the sector under the file, the byte-range a
   lock takes, the version of the library, and every `sqlite3_test_control`.
9. `hexio_read` and `hexio_write` reach the bytes the harness holds,
   which the `read` and `write` requests carry as hexadecimal digits; a
   write reads the database again from what it left.
10. The compile options a file reads as `$::sqlite_options(name)` are an
    array of the tester, each nought where `ifcapable` answers that this
    engine has none of it, and `$::cmdlinearg` carries the defaults of
    the suite's own tester.
11. `testfixture` runs the script it was given in this interpreter, and
    `launch_testfixture` names no channel.
12. `sqlite3_table_column_metadata` answers out of the schema the
    harness holds, and a connection that opens again writes `NULL` as
    the empty string.
13. A connection that closes leaves the file it read no transaction,
    which `sqlite3_close` rolls back.
14. `sqlite3_next_stmt` answers the statements the harness holds for one
    connection, in the order it made them, which D-319 records.
15. `sqlite3_bind_text` and `sqlite3_bind_blob` bind the first `BYTES`
    bytes of the value, and `sqlite3_bind_text16` and `sqlite3_prepare16`
    read the UTF-16 text they are given back as UTF-8, which D-320
    records.
16. The connection command answers the errors `tclsqlite.c` writes for a
    method that is no method and for a count of values a method does not
    take, `db transaction` opens a savepoint inside a transaction the
    connection holds already, a parameter that begins with `@` binds a
    blob, a parameter no variable is set for reaches the script
    `db bind_fallback` named, and a function's answer carries the kind of
    value it stands for, which D-332 records.
17. `sqlite3_stmt_readonly`, `sqlite3_stmt_busy` and
    `sqlite3_stmt_isexplain` answer off the text of the statement and
    how far it has run, which D-338 records.
18. The traces of a connection are told of each statement of the text it
    runs, and the text of a prepared statement carries the semicolon
    that ends it, which D-339 records.
19. A statement `sqlite3_prepare` made is refused `SQLITE_SCHEMA` where
    the schema, a function, a collation, an authorizer or a `DETACH`
    changed since the statement was made, and `sqlite3_expired` answers
    the same test, which D-341 records.
20. `sqlite3_commit_hook`, `sqlite3_rollback_hook` and
    `sqlite3_update_hook` name a script per connection, which the engine
    asks over the line, and a commit hook that answers true refuses the
    statement, which D-342 records. A script that runs a statement of
    its own reaches the engine while the engine waits for its answer,
    which the line has no path for.
21. `$db preupdate hook` names a script per connection, which the engine
    calls before it writes a row, and `$db preupdate count`, `depth`,
    `old` and `new` answer out of what the call handed the tester, which
    D-343 records.
22. `do_test` runs the body of a case at the outermost level, the tester
    answers `sqlite3_open`, `sqlite3_open16`, `sqlite3_open_v2`,
    `sqlite3_close` and `sqlite3_close_v2`, and `file isfile` over a name
    the harness holds answers one, which D-344 records.
23. `sqlite3_exec_printf`, `sqlite_exec_printf` and
    `sqlite3_get_table_printf` write their format with one argument and
    answer as `test1.c` writes them, and the pointer `sqlite3_open`
    answers is a connection of the tester's own, which D-351 records.
24. `ifcapable autovacuum` answers that this engine has it, so the 16
    files that stopped at it run, which D-354 records.
25. A script is split at the semicolons `sqlite3_complete` ends a
    statement at, `verify_ex_errcode` scores a case, `strftime` answers
    what the C library of the machine writes, and the commands that
    size what the C library keeps for itself answer nought rather than
    raising, which D-355 records.
26. The commands of `test_blob.c` answer the name of the code they
    carry, `DB incrblob` answers a channel of the interpreter's own
    making, and `ifcapable incrblob` answers that this engine has it,
    which D-357 records.
27. A command of a blob handle the tester refuses tells the harness the
    code, which `sqlite3_errcode` answers, which D-359 records.
28. `decode_hexdb` reads the text `dbtotxt` writes as the bytes of a
    database file and `DB deserialize` writes them back as the database
    of the connection, which D-360 records.
29. `DB serialize` answers the bytes of the database of the connection,
    which D-361 records.
30. `load_static_extension` names the module to the harness, which
    registers the functions of `regexp` on the connection, and the
    runner reads every file under the system encoding UTF-8, which
    D-362 and D-363 record.
31. `DB status (step|sort|autoindex|vmstep)` answers what the last
    statement of the connection counted, which D-365 records.
32. A statement of a connection that did not begin the transaction open
    on its path reads the file as that transaction found it, which
    D-370 records.
33. `::sqlite_sort_count`, which `cksort` reads, is answered from the
    sorts the last statement counted, which D-372 records.
34. A statement opening with `EXPLAIN QUERY PLAN` is answered by the
    reader and not by the writer, which D-375 records.

### Produces

`tools/suite/*.tcl`.

### Done when

No file of the suite stops at an undefined command.

## 17.15 T6. The messages the engine refuses with

Status: the names are carried, which D-229 records: `no such table`,
`no such column`, `no such function`, `wrong number of arguments to
function`, `no such collation sequence`, `row value misused` and the
messages of the transactions and the foreign keys. The constraint
messages, which name the table and the column, are open.
Depends on: T3.
Size: M.

### Needs

- `sqlite3ErrorMsg` of `src/util.c`, and the calls of it, for the text
  of each message.

### Does

1. Every refusal of the engine carries the names the message holds: the
   table, the column, the index or the function.
2. `db-sqlite` writes the message the C library writes for it.
3. The harness answers a refusal with that message, which `catchsql`
   reads.

### Done when

The `do_catchsql_test` cases of the suite compare the message the file
holds against the same text.

## 17.16 Risks

| # | Risk | Effect | What reduces it |
|---|------|--------|-----------------|
| 1 | A command the tester answers with nothing is one a case reads the answer of. | The case scores wrong, which reads as a defect of the engine. | Such a command answers an error rather than a value wherever a case could read it, so the case is refused. |
| 2 | `tclsh` is not on the machine. | The subcommand cannot run. | It says so by name, and it is never a step of `cargo xtask check`. |
| 3 | A file loops without end. | The run does not finish. | Every file runs under a deadline, and the interpreter is ended when it passes. |
| 4 | A file reads a path the harness does not hold. | The file stops. | The harness holds a path the moment a connection opens it, so a file that opens one and reads it finds it. |
| 5 | The engine refuses a statement the file needed to write its rows. | Every case after it reads a table that is short. | Such a case is refused and not failed, which D-213 already holds for the steps the harness could not run. |
