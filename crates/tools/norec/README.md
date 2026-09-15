# norec

A NoREC fuzzer: random SQL against a database engine, checked against the
same query in a form that engine cannot optimize.

```sh
sh tools/sqlite.sh          # clone and build the engine, once
sh tools/xtask.sh norec -- --runs 1000
```

## What it checks

One case is a random database, a random predicate, and two queries over
them:

```sql
SELECT * FROM t0 WHERE <predicate>;
SELECT SUM(count) FROM (SELECT (<predicate>) IS TRUE AS count FROM t0);
```

The first puts the predicate where the optimizer works: a `WHERE` clause is
what an index is chosen for and what a plan is built around. The second
evaluates the same predicate on every row and adds the truths up, which
leaves the optimizer nothing to apply. The predicate is the same text in
both, so the number of rows the first returns must equal the number the
second sums. When the two differ, the engine answered its own query two
ways, and one of them is wrong.

That is NoREC, from Manuel Rigger and Zhendong Su, *Detecting Optimization
Bugs in Database Engines via Non-Optimizing Reference Engine Construction*,
ESEC/FSE 2020. The copy this crate is written against is in
[`docs/acm/`](../../../docs/acm/README.md); section 3.1 has the two query
forms, 3.2 has the translation, 3.3 has the two ways of counting the first
query, and 3.4 has the limits this generator keeps to.

## What a case holds

One or two tables, each with up to four columns and up to six rows. A
column is declared with one of SQLite's affinities, or with none, and
sometimes with a collating sequence; a value is an integer, a double, text
or `NULL`, drawn from a small pool so that comparisons meet rather than
being false by accident. Half the literals in a predicate are values that
are actually stored.

Over that: indexes, partial indexes, indexes over expressions, and
`ANALYZE`, which are what give the optimizer something to do. A second
table is joined by a comma, a `JOIN` or a `LEFT JOIN`, and section 3.2 has
the `FROM` clause copied into the second query unchanged.

A `UNIQUE` index is created only where the rows are provably distinct as
SQLite compares them — one plain column, no collation, no conversion in
the comparison — because a statement that fails costs the whole case.

Three things are never generated, following section 3.4: a subquery, whose
result the two forms are allowed to disagree over; a function of the clock
or of a random source, which answers differently per call; and `DISTINCT`,
aggregates and window functions, which compute over several rows and do not
survive the translation.

## What a finding leaves behind

The case is shrunk first: rows, indexes, joins, `ANALYZE` and the predicate
are each dropped or simplified as far as the disagreement survives, one
engine run per candidate, within a budget. What comes out is written to
`target/norec/norec-<seed>.sql`, which is a file the shell reproduces the
finding from and nothing else:

```sh
sqlite3 -batch :memory: < target/norec/norec-123.sql
```

The seed of a case is the first seed plus its number, so `--seed 123 --runs
1` runs case 123 again, and `--script` prints what it would run instead of
running it.

## What it costs

One process per case, one script per process. Generation is O(1) in the
size of the case; a run is the engine's own cost; reduction is O(budget)
engine runs and takes a case to its smallest form in a few dozen of them.
Twenty thousand cases against SQLite take a few minutes on one core.
