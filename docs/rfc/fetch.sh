# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
# Fetch every RFC of this directory, byte for byte, and print the SHA-256 of
# each against the table in README.md. Run from anywhere; writes beside this
# script.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)

rfcs='791 792 826 894 1035 1071 1122 1950 1951 2104 2131 2132 2313 2464 3279
3526 3596 4055 4231 4250 4251 4252 4253 4254 4291 4443 4648 4861 4862 5480 5646 5656
5756 5758 5869 5903 5952 6668 6724 6979 7468 7748 8017 8032 8081 8106 8200 8201
8268 8308 8332 8439 8446 8448 8709 8731 9110 9112 9142 9293 9987'

if [ "$#" -gt 0 ]; then
	rfcs="$*"
fi

for n in $rfcs; do
	curl -sSL --fail --max-time 120 \
		"https://www.rfc-editor.org/rfc/rfc$n.txt" -o "$dir/rfc$n.txt"
	printf '%s  %s\n' \
		"$(shasum -a 256 "$dir/rfc$n.txt" | cut -d' ' -f1)" "rfc$n.txt"
done
