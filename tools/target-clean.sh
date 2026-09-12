# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# Drops build artifacts below the target directory that no build has touched
# for a while, so a 20 GiB tree shrinks to what the current work needs.
# Cargo and the xtask rebuild whatever goes. Runs without a shebang, so start
# it as `sh tools/target-clean.sh [options]`; the SPDX header must be the
# first line.
#
# Run no build while this runs: a file the compiler is still writing looks
# old the moment its mtime predates the cutoff.

cd "$(dirname "$0")/.." || exit 1

days=14
dry_run=no

usage() {
    cat <<'USAGE'
sh tools/target-clean.sh [--days N] [--dry-run] [--help]

  --days N   drop files whose mtime is older than N days (default 14)
  --dry-run  report what would go, delete nothing
  --help     this text

The target directory is $CARGO_TARGET_DIR, or ./target when that is unset.
CACHEDIR.TAG stays; it keeps backup tools out of the tree.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --days)
            days="$2"
            shift 2 || exit 1
            ;;
        --days=*)
            days="${1#--days=}"
            shift
            ;;
        --dry-run)
            dry_run=yes
            shift
            ;;
        --help | -h)
            usage
            exit 0
            ;;
        *)
            echo "target-clean: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$days" in
    '' | *[!0-9]*)
        echo "target-clean: --days wants a whole number, got '$days'" >&2
        exit 2
        ;;
esac

target="${CARGO_TARGET_DIR:-$PWD/target}"
if [ ! -d "$target" ]; then
    echo "target-clean: no target directory at $target"
    exit 0
fi

# One traversal, O(n) in the entries below target. `-newer` on a marker file
# is the portable cutoff; BSD and GNU find disagree over -mtime's rounding.
marker="$target/.target-clean-cutoff"
touch -t "$(date -v-"${days}"d '+%Y%m%d%H%M' 2>/dev/null \
    || date -d "$days days ago" '+%Y%m%d%H%M')" "$marker" || exit 1

stale="$(mktemp)" || exit 1
trap 'rm -f "$stale" "$marker"' EXIT INT TERM

# NUL-separated throughout, so a newline in a path stays one entry.
find "$target" -type f ! -newer "$marker" \
    ! -name CACHEDIR.TAG ! -name .target-clean-cutoff -print0 >"$stale"

count="$(tr -dc '\0' <"$stale" | wc -c | tr -d ' ')"
if [ "$count" -eq 0 ]; then
    echo "target-clean: nothing older than $days days in $target"
    exit 0
fi
noun=files
[ "$count" -eq 1 ] && noun=file

# du -k prints one line per file; xargs may split the list, so awk sums
# every chunk.
size="$(xargs -0 du -k <"$stale" | awk '
    { s += $1 }
    END {
        if (s >= 1048576) { printf "%.1f GiB", s / 1048576 }
        else { printf "%.1f MiB", s / 1024 }
    }')"

if [ "$dry_run" = yes ]; then
    echo "target-clean: would drop $count $noun, $size, older than $days days"
    tr '\0' '\n' <"$stale"
    exit 0
fi

xargs -0 rm -f <"$stale"
# Empty directories the deletions left behind. -depth walks children first,
# and one rmdir per call so a parent tests empty after its child goes.
find "$target" -mindepth 1 -depth -type d -empty -exec rmdir {} \;
echo "target-clean: dropped $count $noun, $size, older than $days days"
