# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors

# Clones SQLite, checks out one release tag, and builds it, for reading a
# reference implementation beside our own. The tree lands below research/,
# which .gitignore keeps out of this repository. Runs without a shebang, so
# start it as `sh tools/sqlite.sh [options]`; the SPDX header must be the
# first line.
#
# No tclsh needed: SQLite's autosetup builds its own jimsh and runs every
# code generator through it.

set -eu
cd "$(dirname "$0")/.." || exit 1

version=3.53.4
dir=research/sqlite
jobs=
clean=no

usage() {
    cat <<'USAGE'
sh tools/sqlite.sh [--version X.Y.Z] [--dir PATH] [--jobs N] [--clean] [--help]

  --version X.Y.Z  release to build, the tag version-X.Y.Z (default 3.53.4)
  --dir PATH       where the checkout goes (default research/sqlite)
  --jobs N         parallel compiler jobs (default: the CPU count)
  --clean          drop the checkout first and clone again
  --help           this text

Leaves the sqlite3 shell, libsqlite3.a and libsqlite3.so in the checkout.
USAGE
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) version="$2"; shift 2 || exit 1 ;;
        --version=*) version="${1#--version=}"; shift ;;
        --dir) dir="$2"; shift 2 || exit 1 ;;
        --dir=*) dir="${1#--dir=}"; shift ;;
        --jobs) jobs="$2"; shift 2 || exit 1 ;;
        --jobs=*) jobs="${1#--jobs=}"; shift ;;
        --clean) clean=yes; shift ;;
        --help | -h) usage; exit 0 ;;
        *)
            echo "sqlite: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$version" in
    '' | *[!0-9.]*)
        echo "sqlite: --version wants digits and dots, got '$version'" >&2
        exit 2
        ;;
esac
case "${jobs:-1}" in
    '' | *[!0-9]*)
        echo "sqlite: --jobs wants a whole number, got '$jobs'" >&2
        exit 2
        ;;
esac
[ -n "$jobs" ] || jobs="$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)"

tag="version-$version"
[ "$clean" = yes ] && rm -rf "$dir"

if [ -d "$dir/.git" ]; then
    # The release commit carries `release` as well, so ask which tags point
    # at HEAD instead of letting describe pick one of them.
    if ! git -C "$dir" tag --points-at HEAD | grep -qx "$tag"; then
        # A depth-1 clone carries one commit, so the wanted tag is fetched
        # the same way rather than looked for in history that is not there.
        echo "sqlite: fetching $tag into $dir"
        git -C "$dir" fetch --depth 1 origin "refs/tags/$tag:refs/tags/$tag"
        git -C "$dir" checkout --force "$tag"
        git -C "$dir" clean -xdf
    fi
else
    rm -rf "$dir"
    mkdir -p "$(dirname "$dir")"
    echo "sqlite: cloning $tag into $dir"
    git clone --depth 1 --branch "$tag" https://github.com/sqlite/sqlite.git "$dir"
fi

cd "$dir"
[ -f Makefile ] || ./configure
make -j"$jobs"

echo "sqlite: built $(./sqlite3 --version)"
echo "sqlite: $PWD/sqlite3"
