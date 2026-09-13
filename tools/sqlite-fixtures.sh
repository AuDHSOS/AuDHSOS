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

# The five that document 6, section 6.6.75, reads one at a time.
rm -f "$out/small.db" "$out/page512.db" "$out/utf16.db" "$out/overflow.db" "$out/indexed.db"
"$sqlite" "$out/small.db" "CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB); INSERT INTO t VALUES (1,'one',1.5,x'0102'), (2,'two',-2.5,NULL), (-3,'',0.0,x'ff');"
"$sqlite" "$out/page512.db" "PRAGMA page_size=512; VACUUM; CREATE TABLE wide(n INTEGER, s TEXT); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<400) INSERT INTO wide SELECT i, 'row ' || i FROM c;"
"$sqlite" "$out/utf16.db" "PRAGMA encoding='UTF-16le'; CREATE TABLE u(t TEXT); INSERT INTO u VALUES ('abc'), ('äöü');"
"$sqlite" "$out/overflow.db" "CREATE TABLE big(t TEXT); INSERT INTO big VALUES (replace(hex(zeroblob(9000)),'0','x'));"
"$sqlite" "$out/indexed.db" "CREATE TABLE k(a INTEGER, b TEXT); CREATE INDEX ka ON k(a); CREATE UNIQUE INDEX kb ON k(b); WITH RECURSIVE c(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM c WHERE i<100) INSERT INTO k SELECT i, 'v' || i FROM c;"

# The matrix of document 15, section 15.6: the same rows written under
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

# The write-ahead log is merged back, so that what is committed is one
# file and not three.
"$sqlite" "$out/m-wal.db" "PRAGMA wal_checkpoint(TRUNCATE);" >/dev/null
rm -f "$out"/*-wal "$out"/*-shm

# The recorded oracle: doubles, and the text the C library prints them as.
amalgamation="$(dirname "$sqlite")/sqlite3.c"
if [ -f "$amalgamation" ]; then
    oracle="$(mktemp -d)/oracle"
    "${CC:-cc}" -O1 -I "$(dirname "$amalgamation")" -o "$oracle" \
        tools/sqlite-oracle.c "$amalgamation" -lm -lpthread -ldl
    "$oracle" fp-corpus >"$out/fp.corpus"
    "$oracle" fp <"$out/fp.corpus" >"$out/fp.golden"
    rm -rf "$(dirname "$oracle")"
    printf 'fp.corpus\t%s cases\n' "$(wc -l <"$out/fp.corpus" | tr -d ' ')"
else
    echo "sqlite-fixtures: no $amalgamation; the oracle was not rebuilt" >&2
fi

echo "sqlite-fixtures: wrote $(ls "$out" | wc -l | tr -d ' ') files under $out"
