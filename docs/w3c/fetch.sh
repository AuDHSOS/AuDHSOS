# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (C) 2026 Manuel Baesler and contributors
# Fetch every document of this directory, byte for byte, and the figure files
# CSS Fonts Module Level 4 and Compositing and Blending Level 1 name by a
# relative path. Run from anywhere; writes beside this script. See README.md.
set -eu

dir=$(cd "$(dirname "$0")" && pwd)

# file:url — a dated address where the publisher has one, because a dated
# address is fixed and the undated one moves to the next draft.
docs='png-3.html:https://www.w3.org/TR/png-3/
css-fonts-4.html:https://www.w3.org/TR/2026/WD-css-fonts-4-20260913/
woff2.html:https://www.w3.org/TR/2024/REC-WOFF2-20240808/
compositing-1.html:https://www.w3.org/TR/2024/CRD-compositing-1-20240321/'

for d in $docs; do
	f=${d%%:*}
	u=${d#*:}
	curl -sSL --fail --max-time 300 "$u" -o "$dir/$f"
	printf '%s  %s\n' "$(shasum -a 256 "$dir/$f" | cut -d' ' -f1)" "$f"
done

# file:base for the two documents that name figures by a relative path.
# `ducky.png` is skipped: Compositing and Blending Level 1 writes it inside
# escaped example markup, `&lt;img src="ducky.png"/>`, so it is example text
# and not a figure, and the publisher answers 404 for it.
figures='css-fonts-4.html:https://www.w3.org/TR/2026/WD-css-fonts-4-20260913
compositing-1.html:https://www.w3.org/TR/2024/CRD-compositing-1-20240321'

for d in $figures; do
	f=${d%%:*}
	base=${d#*:}
	grep -oE 'src="[^"]*"' "$dir/$f" |
		sed 's/src="//;s/"$//;s|^\./||' |
		grep -v '^data:' | grep -v '^/' | grep -v '^https\{0,1\}://' |
		grep -v '^ducky\.png$' |
		LC_ALL=C sort -u |
		while read -r img; do
			mkdir -p "$dir/$(dirname "$img")"
			curl -sSL --fail --max-time 120 "$base/$img" -o "$dir/$img"
			printf '%s  %s\n' \
				"$(shasum -a 256 "$dir/$img" | cut -d' ' -f1)" "$img"
		done
done
