# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# Writes the fixture databases of `db-sqlite` with the `sqlite3` shell, so
# that what the tests read is the format as SQLite writes it and not as
# this repository reads it. Runs without a shebang, so start it as
# `sh tools/sqlite-fixtures.sh`; the SPDX header must be the first line.
#
# It also builds `tools/sqlite-oracle.c` against the amalgamation and
# records what the C library answers, which is how the tests compare
# against SQLite without SQLite being there when they run.
#
# The shell comes from `sh tools/sqlite.sh`; $NOREC_SQLITE or --sqlite
# names another. The fixtures are committed, so this is run when they
# change and not as part of a build.

set -eu
cd "$(dirname "$0")/.." || exit 1

sqlite="${NOREC_SQLITE:-research/sqlite/sqlite3}"
out=crates/db/sqlite/src/tests/fixtures

usage() {
    cat <<'USAGE'
sh tools/sqlite-fixtures.sh [--sqlite PATH] [--out DIR] [--help]

  --sqlite PATH  the sqlite3 shell (default research/sqlite/sqlite3)
  --out DIR      where the fixtures go (default the crate's fixtures)
  --help         this text
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --sqlite) sqlite="$2"; shift 2 || exit 1 ;;
        --out) out="$2"; shift 2 || exit 1 ;;
        --help | -h) usage; exit 0 ;;
        *)
            echo "sqlite-fixtures: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

[ -x "$sqlite" ] || {
    echo "sqlite-fixtures: no shell at $sqlite; run sh tools/sqlite.sh first" >&2
    exit 1
}
mkdir -p "$out"

# The rows every matrix fixture holds, so that one set of assertions reads
# all of them: three rows of the four storage classes, and an index.
content="CREATE TABLE m(i INTEGER, t TEXT, r REAL, b BLOB);
INSERT INTO m VALUES (1,'one',1.5,x'01'), (2,'two',2.5,x'0202'), (3,'three',3.5,x'030303');
CREATE INDEX mi ON m(t);"

# One fixture: a name, the pragmas that configure it, and a file control
# for the reserved bytes, which is not a pragma.
fixture() {
    name="$1"
    control="$2"
    pragmas="$3"
    rm -f "$out/$name"
    if [ -n "$control" ]; then
        "$sqlite" "$out/$name" "$control" "$pragmas $content" >/dev/null
    else
        "$sqlite" "$out/$name" "$pragmas $content" >/dev/null
    fi
    printf '%s\t%s bytes\n' "$name" "$(wc -c <"$out/$name" | tr -d ' ')"
}

# The ones document 6 reads one at a time. Each is written from nothing,
# so whatever is there goes first.
rm -f "$out/small.db" "$out/page512.db" "$out/utf16.db" "$out/overflow.db" \
    "$out/indexed.db" "$out/wide16.db" "$out/keys.db" "$out/generated.db" \
    "$out/joins.db"
"$sqlite" "$out/small.db" "CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB); INSERT INTO t VALUES (1,'one',1.5,x'0102'), (2,'two',-2.5,NULL), (-3,'',0.0,x'ff');"
"$sqlite" "$out/page512.db" "PRAGMA page_size=512; VACUUM; CREATE TABLE wide(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO wide SELECT i, 'row ' || i FROM c; CREATE INDEX wides ON wide(s); CREATE TABLE deep(k TEXT PRIMARY KEY, v) WITHOUT ROWID; WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO deep SELECT 'k' || i, i FROM c;"
"$sqlite" "$out/utf16.db" "PRAGMA encoding='UTF-16le'; CREATE TABLE u(t TEXT); INSERT INTO u VALUES ('abc'), ('äöü');"
"$sqlite" "$out/overflow.db" "CREATE TABLE big(t TEXT); INSERT INTO big VALUES (replace(hex(zeroblob(9000)),'0','x'));"
"$sqlite" "$out/wide16.db" "PRAGMA encoding='UTF-16le'; CREATE TABLE s(t TEXT); INSERT INTO s VALUES (char(57344)), (char(65536)), ('z'), (char(65533)), (char(55296)), (char(65536)||'a'); CREATE TABLE n(t TEXT COLLATE NOCASE); INSERT INTO n VALUES ('A'),('b'),('a');"
"$sqlite" "$out/keys.db" "CREATE TABLE r(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO r VALUES (5,'five'),(2,'two'),(9,'nine'); CREATE TABLE w(a TEXT, b INT, PRIMARY KEY(a)) WITHOUT ROWID; INSERT INTO w VALUES ('x',1),('y',2); CREATE TABLE d(k INTEGER PRIMARY KEY DESC, v); INSERT INTO d VALUES (1,'a'); CREATE TABLE u(a, b, c, d, PRIMARY KEY(c,a)) WITHOUT ROWID; INSERT INTO u VALUES (1,2,3,4),(5,6,7,8); CREATE TABLE p(a, b, PRIMARY KEY(a,a,b)) WITHOUT ROWID; INSERT INTO p VALUES ('m',1),('n',2); CREATE TABLE q(a REAL, b, c AS (a+1), PRIMARY KEY(b)) WITHOUT ROWID; INSERT INTO q(a,b) VALUES (1.0,'x'),(2.5,'y');"
"$sqlite" "$out/generated.db" "CREATE TABLE g(a, b AS (a+1), c AS (a*2) STORED); INSERT INTO g(a) VALUES (1),(2); CREATE TABLE h(a, c AS (a*2) STORED); INSERT INTO h(a) VALUES (3),(4); CREATE TABLE f(a, b AS (c+1), c AS (a*2)); INSERT INTO f(a) VALUES (5),(NULL); CREATE TABLE i(a, b TEXT AS (a), c INT AS (a)); INSERT INTO i(a) VALUES ('7'),(8.5); CREATE TABLE j(a, b AS (hex(a)), c AS (nullif(a,1))); INSERT INTO j(a) VALUES (1),('x');"
"$sqlite" "$out/joins.db" "CREATE TABLE a(x INTEGER, y TEXT); INSERT INTO a VALUES (1,'one'),(2,'two'),(3,NULL); CREATE TABLE b(x INTEGER, z TEXT); INSERT INTO b VALUES (1,'B1'),(1,'B1b'),(4,'B4'),(NULL,'Bn'),(-9223372036854775808,'Bmin'); CREATE TABLE c(y TEXT COLLATE NOCASE, w INTEGER); INSERT INTO c VALUES ('ONE',10),('two',20);"
"$sqlite" "$out/indexed.db" "CREATE TABLE k(a INTEGER, b TEXT); CREATE INDEX ka ON k(a); CREATE UNIQUE INDEX kb ON k(b); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<100) INSERT INTO k SELECT i, 'v' || i FROM c;
CREATE TABLE m(p, q TEXT COLLATE NOCASE, r); CREATE INDEX mpq ON m(p, q); CREATE INDEX mq ON m(q); CREATE INDEX mr ON m(r DESC); INSERT INTO m VALUES (1,'A',10),(1,'b',20),(2,'A',30),(2,'c',NULL),(3,NULL,50);
CREATE TABLE e(a, b); CREATE INDEX ea ON e(abs(a)); CREATE INDEX eb ON e(b) WHERE b>0; CREATE INDEX ec ON e(a COLLATE NOCASE); INSERT INTO e VALUES (-1,1),(2,-2),(3,3);
CREATE TABLE o(a, t TEXT); CREATE INDEX ot ON o(t); INSERT INTO o VALUES (1, replace(hex(zeroblob(4000)),'0','y')), (2,'short');
CREATE TABLE u(a TEXT, b, PRIMARY KEY(a)) WITHOUT ROWID; CREATE INDEX ub ON u(b); INSERT INTO u VALUES ('x',1),('y',2);
CREATE TABLE f(v); CREATE INDEX fv ON f(v); INSERT INTO f VALUES (NULL),(7),(1.5),('t'),(x'0102');"

# The matrix of document 16, section 16.11: the same rows written under
# every configuration the shell can write them under.
fixture m-utf8-512.db        "" "PRAGMA page_size=512;"
fixture m-utf8-1024.db       "" "PRAGMA page_size=1024;"
fixture m-utf8-4096.db       "" "PRAGMA page_size=4096;"
fixture m-utf8-65536.db      "" "PRAGMA page_size=65536;"
fixture m-utf16le-512.db     "" "PRAGMA page_size=512; PRAGMA encoding='UTF-16le';"
fixture m-utf16le-4096.db    "" "PRAGMA page_size=4096; PRAGMA encoding='UTF-16le';"
fixture m-utf16be-4096.db    "" "PRAGMA page_size=4096; PRAGMA encoding='UTF-16be';"
fixture m-reserved32.db      ".filectrl reserve_bytes 32" "PRAGMA page_size=4096;"
fixture m-wal.db             "" "PRAGMA page_size=4096; PRAGMA journal_mode=wal;"
fixture m-autovacuum-full.db "" "PRAGMA page_size=4096; PRAGMA auto_vacuum=FULL;"
fixture m-autovacuum-incr.db "" "PRAGMA page_size=4096; PRAGMA auto_vacuum=INCREMENTAL;"
fixture m-delete.db          "" "PRAGMA page_size=4096; PRAGMA journal_mode=delete;"
fixture m-truncate.db        "" "PRAGMA page_size=4096; PRAGMA journal_mode=truncate;"
fixture m-persist.db         "" "PRAGMA page_size=4096; PRAGMA journal_mode=persist;"
fixture m-memory.db          "" "PRAGMA page_size=4096; PRAGMA journal_mode=memory;"
fixture m-off.db             "" "PRAGMA page_size=4096; PRAGMA journal_mode=off;"
fixture m-utf16be-512.db     "" "PRAGMA page_size=512; PRAGMA encoding='UTF-16be';"
fixture m-utf8-8192.db       "" "PRAGMA page_size=8192;"
fixture m-reserved4.db       ".filectrl reserve_bytes 4" "PRAGMA page_size=1024;"

# The write-ahead log is merged back, so that what is committed is one
# file and not three.
"$sqlite" "$out/m-wal.db" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null
rm -f "$out"/*-wal "$out"/*-shm

# A table that outgrows one page, filled in key order, which is the tree
# a balance makes of it: one interior page over the leaves it split into.
rm -f "$out/tall.db"
"$sqlite" "$out/tall.db" "PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO t SELECT i, 'row ' || i FROM c;"
printf '%s\t%s bytes\n' tall.db "$(wc -c <"$out/tall.db" | tr -d ' ')"

# Every serial type and every affinity, so that a record written by this
# repository can be held to the bytes the C library writes. `wide` has a
# header of exactly 127 code bytes and `wider` one of 130, which are the
# two sides of the size varint counting itself.
wide_columns=""
at=1
while [ $at -le 130 ]; do
    [ $at -gt 1 ] && wide_columns="$wide_columns,"
    wide_columns="${wide_columns}c$at"
    at=$((at + 1))
done
narrow_columns="$(printf '%s' "$wide_columns" | cut -d, -f1-127)"
wide_values="$(printf '%s' "$wide_columns" | sed 's/c[0-9]*/1/g')"
narrow_values="$(printf '%s' "$narrow_columns" | sed 's/c[0-9]*/2/g')"
rm -f "$out/records.db"
"$sqlite" "$out/records.db" >/dev/null <<RECORDS
PRAGMA page_size=1024;
CREATE TABLE plain(a, b, c, d);
INSERT INTO plain VALUES
 (NULL, 0, 1, -1),
 (2, 127, 128, -128),
 (-129, 32767, 32768, -32768),
 (-32769, 8388607, 8388608, -8388608),
 (-8388609, 2147483647, 2147483648, -2147483648),
 (-2147483649, 140737488355327, 140737488355328, -140737488355328),
 (-140737488355329, 9223372036854775807, -9223372036854775808, 0),
 (2.5, -2.5, 1e300, -1e-300),
 (0.0, -0.0, 9e999, -9e999),
 ('', 'a', 'abc', x''),
 (x'41', x'4142', CAST(x'00' AS TEXT), 'ä'),
 (zeroblob(0), zeroblob(1), zeroblob(100), zeroblob(56)),
 (hex(zeroblob(150)), hex(zeroblob(2000)), zeroblob(4000), 'end');
CREATE TABLE typed(i INTEGER, t TEXT, r REAL, n NUMERIC, b BLOB);
INSERT INTO typed VALUES (5, 5, 5, 5, 5);
INSERT INTO typed VALUES ('5', '5', '5', '5', '5');
INSERT INTO typed VALUES (2.5, 2.5, 2.5, 2.5, 2.5);
INSERT INTO typed VALUES (NULL, NULL, NULL, NULL, NULL);
INSERT INTO typed VALUES ('abc', 'abc', 'abc', 'abc', 'abc');
INSERT INTO typed VALUES (x'41', x'41', x'41', x'41', x'41');
INSERT INTO typed VALUES (9223372036854775807, 9223372036854775807,
 9223372036854775807, 9223372036854775807, 9223372036854775807);
INSERT INTO typed VALUES (1e300, 1e300, 1e300, 1e300, 1e300);
INSERT INTO typed VALUES (0, 0, 0, 0, 0);
INSERT INTO typed VALUES (1, 1, 1, 1, 1);
INSERT INTO typed VALUES (-0.0, -0.0, -0.0, -0.0, -0.0);
INSERT INTO typed VALUES (4.0, 4.0, 4.0, 4.0, 4.0);
CREATE TABLE narrow($narrow_columns);
INSERT INTO narrow VALUES ($narrow_values);
CREATE TABLE wide($wide_columns);
INSERT INTO wide VALUES ($wide_values);
CREATE TABLE keyed(a TEXT, b INTEGER, PRIMARY KEY(a)) WITHOUT ROWID;
INSERT INTO keyed VALUES ('one', 1), ('two', 2), ('three', 3);
CREATE TABLE aliased(k INTEGER PRIMARY KEY, v);
INSERT INTO aliased VALUES (1, 'one'), (2, NULL), (3, 2.5);
RECORDS
printf '%s\t%s bytes\n' records.db "$(wc -c <"$out/records.db" | tr -d ' ')"

# A database whose content is in the log and not in the file, which is
# what a reader that does not follow the log reads back as empty. The
# three files are copied while the connection is open, because closing
# the last one checkpoints the log away.
rm -f "$out/m-logged.db" "$out/m-logged.db-wal" "$out/logged.db" "$out/logged.db-wal"
"$sqlite" "$out/m-logged.db" >/dev/null <<LOGGED
PRAGMA page_size=4096;
PRAGMA journal_mode=wal;
PRAGMA wal_autocheckpoint=0;
$content
UPDATE m SET t='ONE' WHERE i=1;
DELETE FROM m WHERE i=3;
INSERT INTO m VALUES (4,'four',4.5,x'04040404');
.system cp "$out/m-logged.db" "$out/logged.db"
.system cp "$out/m-logged.db-wal" "$out/logged.db-wal"
LOGGED
rm -f "$out/m-logged.db" "$out/m-logged.db-wal" "$out/m-logged.db-shm"
printf '%s\t%s bytes, log %s bytes\n' logged.db \
    "$(wc -c <"$out/logged.db" | tr -d ' ')" \
    "$(wc -c <"$out/logged.db-wal" | tr -d ' ')"

# The recorded oracle: what the C library answers where SQL cannot ask.
amalgamation="$(dirname "$sqlite")/sqlite3.c"
if [ -f "$amalgamation" ]; then
    oracle="$(mktemp -d)/oracle"
    # SQLITE_PRIVATE is defined away so that the routines the
    # amalgamation keeps to itself can be asked directly.
    # The same options the shell of `sh tools/sqlite.sh` is built with,
    # so that the two answer the same: a library without
    # SQLITE_ENABLE_MATH_FUNCTIONS has no `pi` and no `ceil`, and the
    # golden would record a refusal the shell does not make.
    "${CC:-cc}" -O1 -DSQLITE_PRIVATE= -DSQLITE_ENABLE_MATH_FUNCTIONS \
        -I "$(dirname "$amalgamation")" -o "$oracle" \
        tools/sqlite-oracle.c "$amalgamation" -lm -lpthread -ldl
    for mode in fp num schema; do
        "$oracle" "$mode-corpus" >"$out/$mode.corpus"
        "$oracle" "$mode" <"$out/$mode.corpus" >"$out/$mode.golden"
        printf '%s.corpus\t%s cases\n' "$mode" \
            "$(wc -l <"$out/$mode.corpus" | tr -d ' ')"
    done
    # The expression cases are named after the module that answers them.
    "$oracle" expr-corpus >"$out/eval.corpus"
    "$oracle" expr <"$out/eval.corpus" >"$out/eval.golden"
    printf 'eval.corpus\t%s cases\n' "$(wc -l <"$out/eval.corpus" | tr -d ' ')"
    # The query cases read the fixtures written above, so they come last.
    "$oracle" query-corpus >"$out/query.corpus"
    "$oracle" query "$out" <"$out/query.corpus" >"$out/query.golden"

    # A database caught between the sync of its journal and the sync of
    # its own pages, which is the one state a rollback journal is hot in.
    # The pair is built rather than caught: SQLite writes the journal's
    # magic only once its records are on disk, so a crash reachable from
    # a script leaves a journal that is not hot.
    rm -f "$out/rollback.db" "$out/rollback.db-journal"
    "$sqlite" "$out/rollback.db" \
        "CREATE TABLE t(a INTEGER, b TEXT); INSERT INTO t VALUES (1,'one'),(2,'two'),(3,'three');" \
        >/dev/null
    cp "$out/rollback.db" "$(dirname "$oracle")/old.db"
    "$sqlite" "$out/rollback.db" \
        "UPDATE t SET b='changed'; INSERT INTO t VALUES (4,'four');" >/dev/null
    printf 'rollback.db-journal\t%s\n' \
        "$("$oracle" journal "$(dirname "$oracle")/old.db" "$out/rollback.db" \
            "$out/rollback.db-journal")"
    # What the C library does with the pair is what the tests hold this
    # crate to: it rolls the journal back and answers the rows the
    # transaction started from.
    cp "$out/rollback.db" "$(dirname "$oracle")/check.db"
    cp "$out/rollback.db-journal" "$(dirname "$oracle")/check.db-journal"
    rolled="$("$sqlite" "$(dirname "$oracle")/check.db" "SELECT count(*) || ' ' || max(b) FROM t")"
    if [ "$rolled" != "3 two" ]; then
        echo "sqlite-fixtures: the journal did not roll back ($rolled)" >&2
        exit 1
    fi
    printf 'query.corpus\t%s cases\n' "$(wc -l <"$out/query.corpus" | tr -d ' ')"
    rm -rf "$(dirname "$oracle")"
else
    echo "sqlite-fixtures: no $amalgamation; the oracle was not rebuilt" >&2
fi

echo "sqlite-fixtures: wrote $(ls "$out" | wc -l | tr -d ' ') files under $out"
