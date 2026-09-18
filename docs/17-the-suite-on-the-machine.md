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

Before this track the harness read nine commands of a file and counted
every other command as one it could not run: 1171 files held 17 724
cases it knew about, and 13 086 of them were refused because a step
before them was such a command. Running the files under `tclsh` makes
98 354 cases of 777 files: 83 667 pass, 3278 answer differently and
11 409 are refused.

`--configuration` opens every connection of a run under one of nine
page-size, encoding and journal-mode settings, which D-275 decides.
What each answers, over the same files:

| Configuration | Passed | Answered differently | Refused |
|---------------|-------:|---------------------:|--------:|
| `utf8-4096-delete` | 83 667 | 3278 | 11 409 |
| `utf16le-4096-delete` | 61 214 | 2392 | 9500 |
| `utf16be-4096-delete` | 61 229 | 2392 | 9500 |
| `utf8-512-delete` | 61 073 | 2452 | 9435 |
| `utf8-1024-delete` | 79 640 | 3128 | 11 358 |
| `utf8-65536-delete` | 61 042 | 2358 | 9446 |
| `utf8-4096-persist` | 61 574 | 2372 | 9504 |
| `utf8-4096-truncate` | 61 311 | 2370 | 9501 |
| `utf8-4096-wal` | 59 759 | 2490 | 10 294 |

The counts move between runs of one configuration only where a file
reaches the deadline, which D-302 sets at three minutes. Only the
first row is a run after D-286 to D-303, the fourth one after D-296; the other seven
were measured before them and are lower than they would read now.
`testfixture` is built with `SQLITE_DEFAULT_PAGE_SIZE=1024`, which
`main.mk` line 1784 sets, so the fourth row is the page size the files
were written for; it scores 269 cases below the first.

One row is behind the others by more than that: `utf8-4096-wal` refuses
786 more than `utf8-4096-delete`, which a connection in write-ahead
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
