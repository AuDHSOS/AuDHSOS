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

# The same rows put in by a key that jumps about, so that every insert
# lands in the middle of a page and the tree is balanced rather than
# appended to.
rm -f "$out/shuffled.db"
"$sqlite" "$out/shuffled.db" "PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO t(rowid,n,s) SELECT (i*137)%401, i, 'row ' || i FROM c;"
printf '%s\t%s bytes\n' shuffled.db "$(wc -c <"$out/shuffled.db" | tr -d ' ')"

# Enough of the same rows that the root of the tree fills and the tree
# grows a third level, so that the balance is one of interior pages and
# runs up from the leaf to the root.
rm -f "$out/deep.db"
"$sqlite" "$out/deep.db" "PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<4000) INSERT INTO t(rowid,n,s) SELECT (i*1373)%4201, i, 'row ' || i FROM c;"
printf '%s\t%s bytes\n' deep.db "$(wc -c <"$out/deep.db" | tr -d ' ')"

# Rows taken out again: one delete that only evens the leaves out, one
# that empties enough of them to put pages on the free list, one that
# takes every row out, and one over rows whose payload runs onto
# overflow pages.
rows400="CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO t(rowid,n,s) SELECT (i*137)%401, i, 'row ' || i FROM c;"
for pair in "deleted.db:rowid%3=0" "emptied.db:rowid%4!=0" "cleared.db:rowid>0"; do
    name="${pair%%:*}"
    where="${pair#*:}"
    rm -f "$out/$name"
    "$sqlite" "$out/$name" "PRAGMA page_size=512; $rows400 DELETE FROM t WHERE $where;"
    printf '%s\t%s bytes\n' "$name" "$(wc -c <"$out/$name" | tr -d ' ')"
done

# Rows written over: one that stays the same length, one that grows into
# the free space the page holds, and one that grows past it.
rm -f "$out/updated.db"
"$sqlite" "$out/updated.db" "PRAGMA page_size=512; $rows400 UPDATE t SET n=n+1000 WHERE rowid%5=0; UPDATE t SET s='x' WHERE rowid%7=0; UPDATE t SET s=s||'-longer-text-here' WHERE rowid%11=0;"
printf '%s\t%s bytes\n' updated.db "$(wc -c <"$out/updated.db" | tr -d ' ')"

# Rows written over where the key moves: a row whose payload doubles, a
# row whose key column is set, a row whose overflow chain is dropped,
# and a row whose key is set under the name `rowid`.
rm -f "$out/moved.db"
"$sqlite" "$out/moved.db" \
    "PRAGMA page_size=512;" \
    "CREATE TABLE t(id INTEGER PRIMARY KEY, s TEXT);" \
    "INSERT INTO t VALUES (1,'a'),(2,'bb'),(3,'ccc'),(4,replace(hex(zeroblob(600)),'0','y')),(5,'e');" \
    "UPDATE t SET s=s||s WHERE id=2;" \
    "UPDATE t SET id=id+100 WHERE id=3;" \
    "UPDATE t SET s='short' WHERE id=4;" \
    "UPDATE t SET rowid=9 WHERE id=5;"
printf '%s\t%s bytes\n' moved.db "$(wc -c <"$out/moved.db" | tr -d ' ')"

# Rows written over where the new payload is the length the old one was,
# so the cell lies where it lay: one row whose payload runs onto three
# overflow pages, and one whose cell is the length it was although the
# payload grew past the leaf, which is the only way the two lengths meet.
rm -f "$out/overwritten.db"
"$sqlite" "$out/overwritten.db" \
    "PRAGMA page_size=512;" \
    "CREATE TABLE t(s TEXT);" \
    "INSERT INTO t VALUES (replace(hex(zeroblob(600)),'0','y')), (substr(replace(hex(zeroblob(60)),'0','y'),1,97));" \
    "UPDATE t SET s=replace(s,'y','z') WHERE rowid=1;" \
    "UPDATE t SET s=replace(hex(zeroblob(300)),'0','z') WHERE rowid=2;"
printf '%s\t%s bytes\n' overwritten.db "$(wc -c <"$out/overwritten.db" | tr -d ' ')"

# Rows written over where the statement names no rows to leave out, and
# a key set under the name `rowid` on a table that holds no column the
# key is another name for.
rm -f "$out/keyed.db"
"$sqlite" "$out/keyed.db" \
    "PRAGMA page_size=512;" \
    "CREATE TABLE t(a INTEGER, b TEXT);" \
    "INSERT INTO t VALUES (1,'one'),(2,'two'),(3,'three');" \
    "UPDATE t SET b='all';" \
    "UPDATE t SET rowid=rowid+10 WHERE a=2;"
printf '%s\t%s bytes\n' keyed.db "$(wc -c <"$out/keyed.db" | tr -d ' ')"

# The auto-vacuum dimension of document 16, section 16.11, over the write
# path: the same rows under both settings, chains that cross the second
# pointer-map page, free pages a file that vacuums a step at a time
# keeps, and the pages a file that vacuums itself whole moves down and
# gives up at the commit.
chained="CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<40) INSERT INTO t(rowid,n,s) SELECT (i*17)%41, i, replace(hex(zeroblob(i*30)),'0','x') FROM c;"
for case in "v-full.db:full:$rows400" \
    "v-incremental.db:incremental:$rows400" \
    "v-chained.db:full:$chained" \
    "v-freed.db:incremental:$rows400 DELETE FROM t WHERE rowid%4!=0;" \
    "v-moved.db:full:$rows400 DELETE FROM t WHERE rowid%4!=0;" \
    "v-moved-chained.db:full:$chained DELETE FROM t WHERE rowid%3!=0;"; do
    name="${case%%:*}"
    rest="${case#*:}"
    mode="${rest%%:*}"
    sql="${rest#*:}"
    rm -f "$out/$name"
    "$sqlite" "$out/$name" "PRAGMA page_size=512; PRAGMA auto_vacuum=$mode; $sql"
    printf '%s\t%s bytes\n' "$name" "$(wc -c <"$out/$name" | tr -d ' ')"
done

# The matrix of document 16, section 16.11, over the write path as a
# covering array: thirty configurations in which every value of every
# dimension appears and every pair of values from two dimensions appears
# together at least once. The full cross is 3 x 5 x 3 x 6 x 3 = 810;
# thirty is the fewest rows a pair-covering array of these dimensions
# can have, because the two widest are five and six values wide.
#
# Each row names the encoding, the page size, the reserved tail, the
# journal mode and the auto-vacuum setting, and holds the same four
# hundred rows put in by a key that jumps about.
array_case() {
    name="$1"
    encoding="$2"
    page="$3"
    reserved="$4"
    journal="$5"
    vacuum="$6"
    control=""
    [ "$reserved" = 0 ] || control=".filectrl reserve_bytes $reserved"
    encode=""
    case "$encoding" in
        utf16le) encode="PRAGMA encoding='UTF-16le';" ;;
        utf16be) encode="PRAGMA encoding='UTF-16be';" ;;
    esac
    rm -f "$out/$name.db" "$out/$name.db-journal" "$out/$name.db-wal" "$out/$name.db-shm"
    pragmas="PRAGMA page_size=$page; $encode PRAGMA auto_vacuum=$vacuum; PRAGMA journal_mode=$journal;"
    if [ "$journal" = wal ]; then
        # A connection that closes checkpoints the log and takes it
        # away, so the pair is copied while the connection is open, and
        # the reserved tail is set by that same connection.
        "$sqlite" "$out/$name.db" >/dev/null <<ARRAY
$control
$pragmas
PRAGMA wal_autocheckpoint=0;
$rows400
.system cp "$out/$name.db" "$out/$name-db.tmp"
.system cp "$out/$name.db-wal" "$out/$name-wal.tmp"
ARRAY
        mv "$out/$name-db.tmp" "$out/$name.db"
        mv "$out/$name-wal.tmp" "$out/$name.db-wal"
        rm -f "$out/$name.db-shm"
        printf '%s\t%s bytes, log %s bytes\n' "$name.db" \
            "$(wc -c <"$out/$name.db" | tr -d ' ')" \
            "$(wc -c <"$out/$name.db-wal" | tr -d ' ')"
        return
    fi
    if [ -n "$control" ]; then
        "$sqlite" "$out/$name.db" "$control" "$pragmas $rows400" >/dev/null
    else
        "$sqlite" "$out/$name.db" "$pragmas $rows400" >/dev/null
    fi
    printf '%s\t%s bytes\n' "$name.db" "$(wc -c <"$out/$name.db" | tr -d ' ')"
}

for case in \
    "x-01:utf16le:512:32:delete:incremental" \
    "x-02:utf16le:512:4:memory:none" \
    "x-03:utf16be:512:32:off:incremental" \
    "x-04:utf8:512:0:persist:none" \
    "x-05:utf16le:512:32:truncate:none" \
    "x-06:utf16le:512:4:wal:full" \
    "x-07:utf16le:1024:32:delete:none" \
    "x-08:utf16be:1024:0:memory:incremental" \
    "x-09:utf8:1024:32:off:none" \
    "x-10:utf16be:1024:32:persist:none" \
    "x-11:utf8:1024:4:truncate:incremental" \
    "x-12:utf16be:1024:0:wal:full" \
    "x-13:utf16be:4096:0:delete:none" \
    "x-14:utf16be:4096:0:memory:incremental" \
    "x-15:utf8:4096:4:off:full" \
    "x-16:utf16le:4096:32:persist:incremental" \
    "x-17:utf16le:4096:32:truncate:full" \
    "x-18:utf8:4096:32:wal:none" \
    "x-19:utf8:8192:0:delete:full" \
    "x-20:utf16le:8192:4:memory:none" \
    "x-21:utf16le:8192:4:off:none" \
    "x-22:utf16le:8192:32:persist:incremental" \
    "x-23:utf16be:8192:32:truncate:none" \
    "x-24:utf16le:8192:0:wal:incremental" \
    "x-25:utf16be:65536:4:delete:incremental" \
    "x-26:utf8:65536:32:memory:full" \
    "x-27:utf16le:65536:0:off:none" \
    "x-28:utf16be:65536:4:persist:full" \
    "x-29:utf16le:65536:0:truncate:full" \
    "x-30:utf16le:65536:32:wal:incremental" \
    ; do
    IFS=: read -r name encoding page reserved journal vacuum <<CASE
$case
CASE
    array_case "$name" "$encoding" "$page" "$reserved" "$journal" "$vacuum"
done

# The index trees a `CREATE INDEX` writes: over no row at all, over a
# few, over terms with a collation and an order of their own, over
# enough rows to fill a second leaf, and over enough to need a page
# above the leaves. The entries are sorted before any is written, so the
# pages fill left to right and the left one is filled before the right
# one is begun, which is what `BTREE_BULKLOAD` asks for.
# Nine hundred rows whose text runs from four bytes to fifty-seven,
# so the index is three levels deep and the entries of one page
# differ in length by more than one of them takes.
rows900="CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<900) INSERT INTO t(rowid,n,s) SELECT (i*541)%1501, i, 'row ' || i || substr('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1, i%53) FROM c;"
rows60="CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<60) INSERT INTO t(rowid,n,s) SELECT i, i, 'row ' || i FROM c;"
for case in "index-empty.db:CREATE TABLE t(a,b); CREATE INDEX ta ON t(a);" \
    "index-few.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(3,'c'),(1,'a'),(2,'b'); CREATE INDEX ta ON t(a);" \
    "index-collated.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(3,'c'),(1,'a'),(2,'b'); CREATE INDEX tb ON t(b COLLATE nocase, a DESC);" \
    "index-split.db:$rows60 CREATE INDEX ts ON t(s);" \
    "index-deep.db:$rows400 CREATE INDEX ts ON t(s);" \
    "index-repeated.db:CREATE TABLE t(a,b); INSERT INTO t VALUES('x',1),('x',2),('y',3); CREATE INDEX ta ON t(a);" \
    "index-wide.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(replace(hex(zeroblob(400)),'0','a'),1),('b',2); CREATE INDEX ta ON t(a);" \
    "index-kept.db:CREATE TABLE t(a,b); CREATE INDEX ta ON t(a); INSERT INTO t VALUES(3,'c'),(1,'a'),(2,'b'),(2,'d');" \
    "index-classes.db:CREATE TABLE t(a,b); CREATE INDEX ta ON t(a); INSERT INTO t VALUES(NULL,1),(2.5,2),(x'0102',3),('t',4),(7,5),(NULL,6),(2.5,7);" \
    "index-added.db:$rows400 CREATE INDEX ts ON t(s); INSERT INTO t(rowid,n,s) VALUES(500,500,'row 1 and a half'),(501,501,'row 999');" \
    "index-gone.db:$rows60 CREATE INDEX ts ON t(s); DELETE FROM t WHERE n%3=0;" \
    "index-hollow.db:$rows400 CREATE INDEX ts ON t(s); DELETE FROM t WHERE n%2=0;" \
    "index-emptied.db:$rows60 CREATE INDEX ts ON t(s); DELETE FROM t WHERE n>0;" \
    "index-moved.db:$rows60 CREATE INDEX ts ON t(s); UPDATE t SET s='moved ' || n WHERE n%7=0;" \
    "index-rekeyed.db:CREATE TABLE t(a,b); CREATE INDEX ta ON t(a); INSERT INTO t VALUES(1,'a'),(2,'b'),(3,'c'); UPDATE t SET rowid=9 WHERE a=2;" \
    "index-alias.db:CREATE TABLE t(a INTEGER PRIMARY KEY, b); INSERT INTO t VALUES(7,'x'),(3,'y'),(9,'z'); CREATE INDEX ta ON t(a); DELETE FROM t WHERE b='y';" \
    "index-tall.db:$rows900 CREATE INDEX ts ON t(s); DELETE FROM t WHERE n%3=0;" \
    "drop-one.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(1,'x'),(2,'y'); CREATE TABLE u(c); INSERT INTO u VALUES(9); DROP TABLE t;" \
    "drop-deep.db:$rows400 CREATE TABLE u(c); INSERT INTO u VALUES(9); DROP TABLE t;" \
    "drop-wide.db:CREATE TABLE t(a); INSERT INTO t VALUES(replace(hex(zeroblob(900)),'0','a')),(replace(hex(zeroblob(900)),'0','b')); CREATE TABLE u(c); DROP TABLE t;" \
    "drop-indexed.db:$rows60 CREATE INDEX ts ON t(s); CREATE TABLE u(c); INSERT INTO u VALUES(9); DROP TABLE t;" \
    "drop-index.db:$rows60 CREATE INDEX ts ON t(s); DROP INDEX ts;" \
    "drop-last.db:CREATE TABLE t(a); INSERT INTO t VALUES(1); CREATE TABLE u(c); INSERT INTO u VALUES(9); DROP TABLE u;" \
    "tx-one.db:CREATE TABLE t(a,b); BEGIN; INSERT INTO t VALUES(1,'x'); INSERT INTO t VALUES(2,'y'); COMMIT;" \
    "tx-grown.db:$rows60 BEGIN; DELETE FROM t WHERE n%3=0; INSERT INTO t(rowid,n,s) VALUES(500,500,'row 500'); COMMIT;" \
    "tx-back.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(1,'x'); BEGIN; INSERT INTO t VALUES(2,'y'); DELETE FROM t WHERE a=1; ROLLBACK;" \
    "tx-kept.db:CREATE TABLE t(a,b); INSERT INTO t VALUES(1,'x');"; do
    name="${case%%:*}"
    sql="${case#*:}"
    rm -f "$out/$name"
    "$sqlite" "$out/$name" "PRAGMA page_size=512; $sql"
    printf '%s\t%s bytes\n' "$name" "$(wc -c <"$out/$name" | tr -d ' ')"
done

rm -f "$out/unchained.db"
"$sqlite" "$out/unchained.db" "PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<40) INSERT INTO t(rowid,n,s) SELECT (i*17)%41, i, replace(hex(zeroblob(i*30)),'0','x') FROM c; DELETE FROM t WHERE rowid%3!=0;"
printf '%s\t%s bytes\n' unchained.db "$(wc -c <"$out/unchained.db" | tr -d ' ')"

# Statements this crate runs from their text: a table whose key is one
# of its columns, rows with the key given and rows without, rows named
# in another order, and rows read out of one table into another.
rm -f "$out/stated.db"
"$sqlite" "$out/stated.db" \
    "CREATE TABLE r(id INTEGER PRIMARY KEY, v TEXT);" \
    "INSERT INTO r VALUES (5,'five'),(2,'two'),(9,'nine');" \
    "INSERT INTO r(v) VALUES ('ten');" \
    "CREATE TABLE s(a, b);" \
    "INSERT INTO s SELECT id, v FROM r;" \
    "INSERT INTO s(b,a) VALUES ('x',1);"
printf '%s\t%s bytes\n' stated.db "$(wc -c <"$out/stated.db" | tr -d ' ')"

# The same rows and the same delete under a journal mode that leaves the
# journal behind, so that the file the commit wrote is there to read.
# The database is `emptied.db` byte for byte, because the journal mode
# changes what is beside the file and not what is in it.
rm -f "$out/journalled.db" "$out/journalled.db-journal"
"$sqlite" "$out/journalled.db" "PRAGMA page_size=512; PRAGMA journal_mode=persist; $rows400 DELETE FROM t WHERE rowid%4!=0;" >/dev/null
printf '%s\t%s bytes\n' journalled.db-journal "$(wc -c <"$out/journalled.db-journal" | tr -d ' ')"
cmp -s "$out/journalled.db" "$out/emptied.db" || {
    echo "sqlite-fixtures: journalled.db is not emptied.db" >&2
    exit 1
}
rm -f "$out/journalled.db"

# The same under a statement that only puts rows in, so that the commit
# is what puts page one in the journal and puts it last. The database is
# `shuffled.db` byte for byte.
rm -f "$out/appended.db" "$out/appended.db-journal"
"$sqlite" "$out/appended.db" "PRAGMA page_size=512; PRAGMA journal_mode=persist; $rows400" >/dev/null
printf '%s\t%s bytes\n' appended.db-journal "$(wc -c <"$out/appended.db-journal" | tr -d ' ')"
cmp -s "$out/appended.db" "$out/shuffled.db" || {
    echo "sqlite-fixtures: appended.db is not shuffled.db" >&2
    exit 1
}
rm -f "$out/appended.db"

# The same rows written into a write-ahead log rather than the file: two
# statements, each its own transaction, and no checkpoint, so the log
# holds every page and the database holds the one page `PRAGMA
# journal_mode=wal` left.
rm -f "$out/logging.db" "$out/logging.db-wal" "$out/logging.db-shm"
"$sqlite" "$out/logging.db" >/dev/null <<LOGGING
PRAGMA page_size=512;
PRAGMA journal_mode=wal;
PRAGMA wal_autocheckpoint=0;
CREATE TABLE t(n INTEGER, s TEXT);
WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO t(rowid,n,s) SELECT (i*137)%401, i, 'row ' || i FROM c;
DELETE FROM t WHERE rowid%3=0;
.system cp "$out/logging.db" "$out/logging-db.tmp"
.system cp "$out/logging.db-wal" "$out/logging-wal.tmp"
LOGGING
mv "$out/logging-db.tmp" "$out/logging.db"
mv "$out/logging-wal.tmp" "$out/logging.db-wal"
rm -f "$out/logging.db-shm"
printf '%s\t%s bytes, log %s bytes\n' logging.db \
    "$(wc -c <"$out/logging.db" | tr -d ' ')" \
    "$(wc -c <"$out/logging.db-wal" | tr -d ' ')"

# Rows taken out and put in again, so that the pages the delete freed
# are the ones the insert takes. Four thousand rows over 512-byte pages
# leave a free list of two trunks.
rm -f "$out/reused.db"
"$sqlite" "$out/reused.db" "PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<4000) INSERT INTO t(rowid,n,s) SELECT (i*1373)%4201, i, 'row ' || i FROM c; DELETE FROM t WHERE rowid%8!=0; WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<1000) INSERT INTO t(rowid,n,s) SELECT 5000+(i*379)%1009, i, 'more ' || i FROM c;"
printf '%s\t%s bytes\n' reused.db "$(wc -c <"$out/reused.db" | tr -d ' ')"

# The matrix of document 16, section 16.11, over the write path: the
# same four hundred rows put in by a key that jumps about, under every
# page size, every encoding and every reserved tail the shell writes.
written() {
    name="$1"
    control="$2"
    pragmas="$3"
    rm -f "$out/$name"
    rows="CREATE TABLE t(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO t(rowid,n,s) SELECT (i*137)%401, i, 'row ' || i FROM c;"
    if [ -n "$control" ]; then
        "$sqlite" "$out/$name" "$control" "$pragmas $rows" >/dev/null
    else
        "$sqlite" "$out/$name" "$pragmas $rows" >/dev/null
    fi
    printf '%s\t%s bytes\n' "$name" "$(wc -c <"$out/$name" | tr -d ' ')"
}

written w-utf8-512.db       "" "PRAGMA page_size=512;"
written w-utf8-1024.db      "" "PRAGMA page_size=1024;"
written w-utf8-4096.db      "" "PRAGMA page_size=4096;"
written w-utf8-8192.db      "" "PRAGMA page_size=8192;"
written w-utf8-65536.db     "" "PRAGMA page_size=65536;"
written w-utf16le-512.db    "" "PRAGMA page_size=512; PRAGMA encoding='UTF-16le';"
written w-utf16be-512.db    "" "PRAGMA page_size=512; PRAGMA encoding='UTF-16be';"
written w-reserved4.db      ".filectrl reserve_bytes 4" "PRAGMA page_size=1024;"
written w-reserved32.db     ".filectrl reserve_bytes 32" "PRAGMA page_size=512;"

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
    # The schema format dimension of document 16, section 16.11. No
    # pragma the shell takes asks for a format below four —
    # `legacy_file_format` is answered and ignored — so the oracle asks
    # for it through `SQLITE_DBCONFIG_LEGACY_FILE_FORMAT`. ALTER TABLE
    # ADD COLUMN raises the format to three, and this library writes
    # three where the file format document allows two, so format two has
    # no fixture.
    rows="INSERT INTO t VALUES(0,1,'x'),(2,3,'y')"
    printf 'format1.db\t%s\n' \
        "$("$oracle" legacy "$out/format1.db" \
            "CREATE TABLE t(a,b,c)" "$rows" "CREATE INDEX tc ON t(c DESC)")"
    printf 'format3.db\t%s\n' \
        "$("$oracle" legacy "$out/format3.db" \
            "CREATE TABLE t(a,b,c)" "$rows" "ALTER TABLE t ADD COLUMN d DEFAULT 7" \
            "ALTER TABLE t ADD COLUMN e")"
    rm -f "$out/format4.db" "$out/defaults.db"
    "$sqlite" "$out/format4.db" \
        "CREATE TABLE t(a,b,c); $rows; ALTER TABLE t ADD COLUMN d DEFAULT 7; ALTER TABLE t ADD COLUMN e;"
    printf 'format4.db\tschema format %s\n' \
        "$("$sqlite" "$out/format4.db" "PRAGMA schema_version" >/dev/null; \
            od -An -tu1 -j47 -N1 "$out/format4.db" | tr -d ' ')"
    # The five affinities in one table, each holding the same three
    # values, which is what document 16, section 16.11 calls the
    # comparison rules of `sqlite3BinaryCompareCollSeq`.
    rm -f "$out/affinity.db"
    "$sqlite" "$out/affinity.db" \
        "CREATE TABLE t(xi INTEGER, xr REAL, xb BLOB, xn NUMERIC, xt TEXT);" \
        "INSERT INTO t(rowid,xi,xr,xb,xn,xt) VALUES(1,1,1,1,1,1);" \
        "INSERT INTO t(rowid,xi,xr,xb,xn,xt) VALUES(2,'2','2','2','2','2');" \
        "INSERT INTO t(rowid,xi,xr,xb,xn,xt) VALUES(3,'03','03','03','03','03');"
    printf 'affinity.db\t%s bytes\n' "$(wc -c <"$out/affinity.db" | tr -d ' ')"
    "$sqlite" "$out/defaults.db" \
        "CREATE TABLE t(a, b DEFAULT 7, c TEXT DEFAULT 'z'); INSERT INTO t(a) VALUES(1);"
    printf 'defaults.db\t%s bytes\n' "$(wc -c <"$out/defaults.db" | tr -d ' ')"

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
    # The same database before the transaction, which is what playing
    # the journal back over the pair has to give.
    cp "$out/rollback.db" "$out/rolled.db"
    "$sqlite" "$out/rollback.db" \
        "UPDATE t SET b='changed'; INSERT INTO t VALUES (4,'four');" >/dev/null
    printf '%s\t%s bytes\n' rolled.db "$(wc -c <"$out/rolled.db" | tr -d ' ')"
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
